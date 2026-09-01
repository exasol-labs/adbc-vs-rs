//! The scan UDF's own body: parses the scan specification a `pushdown`
//! response embedded, opens the source connection it names, and streams
//! `adbc-vs-scan`'s already-coerced batch stream straight to Exasol.
//!
//! [`adbc_vs_scan::scan::execute_scan`] owns both running the statement and
//! coercing each batch to the declared Exasol shape; this function only
//! translates between the adapter's [`scan_spec::ScanColumn`] and the scan
//! crate's own [`adbc_vs_scan::coerce::DeclaredColumn`] vocabulary and emits
//! what it is handed.

use exasol_udf_sdk::context::{EmitBatch, UdfContext};
use exasol_udf_sdk::error::UdfError;

use adbc_vs_scan::coerce::{DeclaredColumn, DeclaredType};
use adbc_vs_scan::scan::ScanRequest;

use crate::exa_type::ExasolType;
use crate::scan_spec::{self, ScanColumn};

/// Runs the scan specification encoded in `scan_spec_json` and emits every
/// resulting Arrow batch.
pub fn run_scan(ctx: &mut dyn UdfContext, scan_spec_json: &str) -> Result<(), UdfError> {
    let spec = scan_spec::parse(scan_spec_json).map_err(UdfError::User)?;

    let connection_object = ctx.connection(&spec.connection)?;
    let source_connection = adbc_vs_scan::connection::SourceConnection {
        uri: connection_object.address,
        username: connection_object.user,
        password: connection_object.password,
    };

    let mut connection =
        adbc_vs_scan::connection::open(spec.driver, &spec.driver_path, &source_connection)
            .map_err(|error| UdfError::User(error.to_string()))?;

    let declared_columns: Vec<DeclaredColumn> = spec.columns.iter().map(declared_column).collect();
    let request = ScanRequest {
        statement: &spec.sql,
        columns: &declared_columns,
        password: &source_connection.password,
    };
    let batches = adbc_vs_scan::scan::execute_scan(&mut connection, request)
        .map_err(|error| UdfError::User(error.to_string()))?;

    for batch in batches {
        let batch = batch.map_err(|error| UdfError::User(error.to_string()))?;
        ctx.emit_batch(&batch)?;
    }

    Ok(())
}

/// Translates a pushdown-declared column into the vocabulary
/// `adbc-vs-scan`'s coercion owns. `Varchar` and `Char` both collapse to
/// [`DeclaredType::Text`]: coercion only needs to know a column is text, not
/// its declared length — Exasol enforces the length itself when it accepts
/// the row.
fn declared_column(column: &ScanColumn) -> DeclaredColumn {
    let declared_type = match column.exasol_type {
        ExasolType::Decimal { precision, scale } => DeclaredType::Decimal { precision, scale },
        ExasolType::Varchar { .. } | ExasolType::Char { .. } => DeclaredType::Text,
        ExasolType::Date => DeclaredType::Date,
        ExasolType::Timestamp => DeclaredType::Timestamp,
        ExasolType::Double => DeclaredType::Double,
        ExasolType::Boolean => DeclaredType::Boolean,
    };
    DeclaredColumn {
        name: column.name.clone(),
        declared_type,
    }
}
