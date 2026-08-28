use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use arrow::array::{
    ArrayRef, Float64Array, Int64Array, RecordBatch, RecordBatchReader, StringArray,
};
use arrow::datatypes::{DataType, Field, Schema, SchemaRef};
use arrow::error::ArrowError;

use super::coerce::{Coercion, DeclaredColumn, DeclaredType};
use super::scan::*;

const STATEMENT: &str = "SELECT `price`, `id` FROM `adbc_vs`.`orders`";
const PASSWORD: &str = "s3cret";

/// The driver's own schema, deliberately unlike the declared one: a scan
/// takes its names and types from the declaration, never from the driver.
fn driver_schema() -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("whatever_the_driver_called_it", DataType::Float64, true),
        Field::new("and_this", DataType::Int64, true),
    ]))
}

/// `price` is declared as what the driver already returns, so the coerced
/// batch shares its array and a retained batch of either kind keeps that
/// array alive; `id` is declared as something the driver does not return, so
/// every batch also exercises a real conversion.
fn declared_columns() -> Vec<DeclaredColumn> {
    vec![
        DeclaredColumn {
            name: "price".to_string(),
            declared_type: DeclaredType::Double,
        },
        DeclaredColumn {
            name: "id".to_string(),
            declared_type: DeclaredType::Decimal {
                precision: 19,
                scale: 0,
            },
        },
    ]
}

fn batch_of(rows: &[(f64, i64)]) -> RecordBatch {
    let prices: ArrayRef = Arc::new(Float64Array::from(
        rows.iter().map(|(price, _)| *price).collect::<Vec<_>>(),
    ));
    let ids: ArrayRef = Arc::new(Int64Array::from(
        rows.iter().map(|(_, id)| *id).collect::<Vec<_>>(),
    ));
    RecordBatch::try_new(driver_schema(), vec![prices, ids])
        .expect("the fixture batch matches the driver schema")
}

/// A reader that hands out prepared batches and, every time the consumer
/// asks for the next one, counts how many previously handed-out batches the
/// consumer is still holding.
///
/// A batch is "still held" when the Arrow array it was built from has more
/// than one strong reference: this reader's own bookkeeping clone is the
/// first, and any second one can only be a batch — driver-shaped or coerced
/// — that the scan never released.
struct ReleaseCountingReader {
    schema: SchemaRef,
    remaining: std::vec::IntoIter<Result<RecordBatch, ArrowError>>,
    handed_out: Vec<ArrayRef>,
    peak_retained: Arc<AtomicUsize>,
}

impl ReleaseCountingReader {
    fn boxed(
        batches: Vec<Result<RecordBatch, ArrowError>>,
        peak_retained: Arc<AtomicUsize>,
    ) -> Box<dyn RecordBatchReader + Send> {
        Box::new(ReleaseCountingReader {
            schema: driver_schema(),
            remaining: batches.into_iter(),
            handed_out: Vec::new(),
            peak_retained,
        })
    }

    fn record_retained(&self) {
        let retained = self
            .handed_out
            .iter()
            .filter(|array| Arc::strong_count(array) > 1)
            .count();
        self.peak_retained.fetch_max(retained, Ordering::SeqCst);
    }
}

impl Iterator for ReleaseCountingReader {
    type Item = Result<RecordBatch, ArrowError>;

    fn next(&mut self) -> Option<Self::Item> {
        self.record_retained();
        let produced = self.remaining.next()?;
        if let Ok(batch) = &produced {
            self.handed_out.push(Arc::clone(batch.column(0)));
        }
        Some(produced)
    }
}

impl RecordBatchReader for ReleaseCountingReader {
    fn schema(&self) -> SchemaRef {
        Arc::clone(&self.schema)
    }
}

fn stream_over(batches: Vec<Result<RecordBatch, ArrowError>>) -> (BatchStream, Arc<AtomicUsize>) {
    let peak_retained = Arc::new(AtomicUsize::new(0));
    let reader = ReleaseCountingReader::boxed(batches, Arc::clone(&peak_retained));
    let coercion =
        Coercion::for_columns(&declared_columns()).expect("the declaration is representable");
    (
        BatchStream::over(reader, coercion, Diagnostic::new(STATEMENT, PASSWORD)),
        peak_retained,
    )
}

#[test]
fn stream_yields_every_batch_the_reader_produces() {
    let (stream, _) = stream_over(vec![
        Ok(batch_of(&[(1.0, 1), (2.0, 2)])),
        Ok(batch_of(&[(3.0, 3)])),
    ]);

    let rows: usize = stream
        .map(|batch| {
            batch
                .expect("the fixture reader produces no failure")
                .num_rows()
        })
        .sum();

    assert_eq!(rows, 3);
}

#[test]
fn stream_yields_batches_in_the_declared_output_shape() {
    let (mut stream, _) = stream_over(vec![Ok(batch_of(&[(19.99, 9_000_000_001)]))]);
    let declared = stream.schema();

    let batch = stream
        .next()
        .expect("the only batch is produced")
        .expect("the fixture batch coerces");

    assert_eq!(batch.schema(), declared);
    assert_eq!(declared.field(0).name(), "price");
    assert_eq!(declared.field(1).name(), "id");
    assert_eq!(
        declared.field(1).data_type(),
        &DataType::Decimal128(19, 0),
        "the driver's Int64 was not driven to the declared DECIMAL"
    );
}

#[test]
fn stream_releases_each_batch_before_requesting_the_next() {
    let (stream, peak_retained) = stream_over(vec![
        Ok(batch_of(&[(1.0, 1)])),
        Ok(batch_of(&[(2.0, 2)])),
        Ok(batch_of(&[(3.0, 3)])),
    ]);

    for batch in stream {
        batch.expect("the fixture reader produces no failure");
    }

    assert_eq!(
        peak_retained.load(Ordering::SeqCst),
        0,
        "the stream was still holding a batch when it asked for the next one"
    );
}

#[test]
fn stream_over_an_exhausted_reader_yields_no_batch() {
    let (stream, _) = stream_over(Vec::new());

    assert_eq!(stream.count(), 0);
}

#[test]
fn stream_failure_names_the_statement_and_the_driver_message() {
    let (mut stream, _) = stream_over(vec![
        Ok(batch_of(&[(1.0, 1)])),
        Err(ArrowError::ExternalError(
            "MySQL server has gone away".into(),
        )),
    ]);

    stream
        .next()
        .expect("the first batch is produced")
        .expect("the first batch is not a failure");
    let error = stream
        .next()
        .expect("the failed batch is reported")
        .expect_err("the second batch is a failure");

    let message = error.to_string();
    assert!(
        message.contains(STATEMENT),
        "error message {message:?} does not name the statement"
    );
    assert!(
        message.contains("MySQL server has gone away"),
        "error message {message:?} does not carry the driver's own message"
    );
}

#[test]
fn stream_stops_after_a_failure_rather_than_polling_a_broken_reader() {
    let (mut stream, _) = stream_over(vec![
        Err(ArrowError::ExternalError(
            "MySQL server has gone away".into(),
        )),
        Ok(batch_of(&[(1.0, 1)])),
    ]);

    stream
        .next()
        .expect("the failed batch is reported")
        .expect_err("the first batch is a failure");

    assert!(
        stream.next().is_none(),
        "the stream kept reading past a failure"
    );
}

#[test]
fn stream_stops_after_a_batch_the_declaration_cannot_accept() {
    let unexpected = RecordBatch::try_new(
        Arc::new(Schema::new(vec![Field::new("only", DataType::Utf8, true)])),
        vec![Arc::new(StringArray::from(vec!["Widget"])) as ArrayRef],
    )
    .expect("the fixture batch matches its schema");
    let (mut stream, _) = stream_over(vec![Ok(unexpected), Ok(batch_of(&[(1.0, 1)]))]);

    let error = stream
        .next()
        .expect("the mismatched batch is reported")
        .expect_err("a one-column batch cannot fill a two-column declaration");

    assert!(error.to_string().contains('2'));
    assert!(
        stream.next().is_none(),
        "the stream kept reading past a coercion failure"
    );
}

#[test]
fn stream_failure_never_contains_the_password() {
    let (mut stream, _) = stream_over(vec![Err(ArrowError::ExternalError(
        format!("Access denied (using password: {PASSWORD})").into(),
    ))]);

    let error = stream
        .next()
        .expect("the failed batch is reported")
        .expect_err("the only batch is a failure");

    assert!(
        !error.to_string().contains(PASSWORD),
        "error message leaked the password"
    );
}
