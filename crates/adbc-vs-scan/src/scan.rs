//! Runs the pushed-down MySQL statement and streams its result back as the
//! Arrow batches Exasol's declared output columns require.
//!
//! One UDF instance streams a whole MySQL result set, so the scan must never
//! hold more than the batch it is currently handing over: [`BatchStream`]
//! yields exactly one batch per step and asks the driver for the next only
//! after the caller has dropped the previous one, which keeps peak memory a
//! function of the batch size rather than of the result size.
//!
//! Coercion is part of the same step rather than a call the caller has to
//! remember: a batch straight from the driver is not in the shape Exasol's
//! declared columns require, and there is no legitimate reason to hand one
//! out.

use adbc_core::error::Error as AdbcError;
use adbc_core::{Connection, Statement};
use adbc_driver_manager::{ManagedConnection, ManagedStatement};
use arrow::array::{RecordBatch, RecordBatchReader};
use arrow::datatypes::SchemaRef;
use arrow::error::ArrowError;

use crate::coerce::{Coercion, DeclaredColumn};
use crate::connection::redact;
use crate::error::ScanError;

/// One scan: the MySQL statement to run and the Exasol output columns its
/// result must be emitted as.
///
/// `password` is sent nowhere — the connection already carries it. It is
/// here as the secret to strip out of any diagnostic this scan reports,
/// defence in depth against a driver that echoes back what it was
/// configured with.
pub struct ScanRequest<'a> {
    pub statement: &'a str,
    pub columns: &'a [DeclaredColumn],
    pub password: &'a str,
}

/// Executes `request` on `connection` and returns its result as a stream of
/// batches already in the declared output shape.
///
/// The declared columns are resolved before the statement runs, so an output
/// declaration Arrow cannot carry fails without touching MySQL at all.
pub fn execute_scan(
    connection: &mut ManagedConnection,
    request: ScanRequest<'_>,
) -> Result<BatchStream, ScanError> {
    let coercion = Coercion::for_columns(request.columns)?;
    let statement = request.statement;
    let diagnostic = Diagnostic::new(statement, request.password);

    let mut prepared = connection.new_statement().map_err(|error| {
        diagnostic.failure("allocating a statement for", &driver_source(&error))
    })?;
    prepared
        .set_sql_query(statement)
        .map_err(|error| diagnostic.failure("setting the SQL query", &driver_source(&error)))?;
    let reader = prepared
        .execute()
        .map_err(|error| diagnostic.failure("executing", &driver_source(&error)))?;

    Ok(BatchStream::over(
        Box::new(StatementReader {
            reader,
            _statement: prepared,
        }),
        coercion,
        diagnostic,
    ))
}

/// The context a scan failure names: the statement that was running and the
/// password to strip out of any diagnostic, so a driver that echoes back
/// what it was configured with cannot leak it.
pub(crate) struct Diagnostic {
    statement: String,
    password: String,
}

impl Diagnostic {
    pub(crate) fn new(statement: &str, password: &str) -> Self {
        Diagnostic {
            statement: statement.to_owned(),
            password: password.to_owned(),
        }
    }

    fn failure(&self, step: &str, source: &dyn std::fmt::Display) -> ScanError {
        ScanError::new(format!(
            "{step} `{}`: {}",
            self.statement,
            redact(&source.to_string(), &self.password)
        ))
    }
}

/// Formats an ADBC driver error the way [`Diagnostic::failure`] expects to
/// receive it: the status ahead of the driver's own message, so redaction
/// (applied to the whole rendered text) still strips the password from the
/// message half.
fn driver_source(source: &AdbcError) -> String {
    format!("ADBC status {:?}: {}", source.status, source.message)
}

/// The result of a scan, one coerced Arrow record batch at a time.
///
/// Iteration is the whole interface on purpose: a `for` loop over a stream
/// drops each batch at the end of its body, before the next step asks the
/// driver for another, so a caller cannot accidentally accumulate the whole
/// result. A failure ends the stream — a driver that has failed mid-result
/// has nothing further to give, and polling it again would only repeat the
/// same diagnosis.
pub struct BatchStream {
    reader: Box<dyn RecordBatchReader + Send>,
    coercion: Coercion,
    diagnostic: Diagnostic,
    finished: bool,
}

impl BatchStream {
    pub(crate) fn over(
        reader: Box<dyn RecordBatchReader + Send>,
        coercion: Coercion,
        diagnostic: Diagnostic,
    ) -> Self {
        BatchStream {
            reader,
            coercion,
            diagnostic,
            finished: false,
        }
    }

    /// The Arrow schema every batch this stream yields carries — known
    /// before the first batch, and unchanged by a result that holds no row.
    pub fn schema(&self) -> SchemaRef {
        self.coercion.schema()
    }

    fn stream_failure(&self, source: ArrowError) -> ScanError {
        self.diagnostic.failure("streaming the result of", &source)
    }
}

impl Iterator for BatchStream {
    type Item = Result<RecordBatch, ScanError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        }
        let produced = match self.reader.next() {
            None => {
                self.finished = true;
                return None;
            }
            Some(Ok(batch)) => self.coercion.apply(&batch),
            Some(Err(error)) => Err(self.stream_failure(error)),
        };
        if produced.is_err() {
            self.finished = true;
        }
        Some(produced)
    }
}

/// A reader that outlives nothing: it keeps the ADBC statement it was
/// executed from alive for exactly as long as the result stream it produced.
///
/// ADBC's result stream is handed over as a bare C data interface object
/// with no back-reference to the statement, so dropping the statement first
/// would leave the driver free to release buffers the stream still reads
/// from. Field order is the guarantee: Rust drops `reader` before
/// `_statement`.
struct StatementReader {
    reader: Box<dyn RecordBatchReader + Send>,
    _statement: ManagedStatement,
}

impl Iterator for StatementReader {
    type Item = Result<RecordBatch, ArrowError>;

    fn next(&mut self) -> Option<Self::Item> {
        self.reader.next()
    }
}

impl RecordBatchReader for StatementReader {
    fn schema(&self) -> SchemaRef {
        self.reader.schema()
    }
}
