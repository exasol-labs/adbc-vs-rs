//! Builds the `pushdown` response: the query-side call that invokes the scan
//! UDF with a dynamic `EMITS` clause, over the source statement a
//! [`SqlDialect`](crate::sql_dialect::SqlDialect) renders from the request's
//! query model. Nothing here names a target SQL dialect.
//!
//! Column metadata for the table being pushed down arrives on the request
//! itself, in `involvedTables[].columns[]` — Exasol resends it on every
//! pushdown, so nothing here needs a live MySQL connection or a second
//! metadata discovery. The state `createVirtualSchema` could not hand back
//! any other way — the driver path, the `CONNECTION` name, and the database
//! — travels in `schemaMetadataInfo.adapterNotes` instead.
//!
//! Every identifier the generated `EMITS` clause declares is quoted: an
//! unquoted clause is upper-cased by Exasol before it is compared against
//! the case-preserved column names the scan UDF actually supplies, and the
//! mismatch is a hard runtime failure (`F-UDF-CL-RUST-1001`).
//!
//! The generated statement calls the scan script from its select list over
//! `DUAL`, not as a table in its `FROM` clause: Exasol's parser accepts an
//! emitting script only as a select-list expression, and `DUAL` supplies the
//! single input row that makes the script run exactly once.

use std::fmt;

use adbc_vs_scan::driver::Driver;
use serde_json::Value;

use crate::adapter_notes::AdapterNotes;
use crate::exa_type;
use crate::pushdown_query::{self, Query};
use crate::scan_spec::{ScanColumn, ScanSpec};
use crate::sql_dialect::SqlDialect;

/// The schema-qualified name the scan UDF is registered under, matching the
/// entry symbol `#[exasol_udf]` derives from its function identifier.
///
/// Public because whoever registers the script has to register it under
/// exactly this name — a `CREATE SCRIPT` spelling it independently would leave
/// every pushed-down statement calling a script that does not exist. The
/// module that generates the call owns the name; the test harness imports it.
pub const SCAN_SCRIPT_NAME: &str = "ADBC_VS.ADBC_SCAN";

/// A `pushdown` request could not be turned into a scan call.
#[derive(Debug)]
pub struct PushdownError(String);

impl fmt::Display for PushdownError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "building the pushdown response: {}", self.0)
    }
}

impl std::error::Error for PushdownError {}

/// Builds the response to a `pushdown` request, rendering the delegated
/// fragment through `dialect`.
pub fn pushdown_response(
    request: &Value,
    dialect: &dyn SqlDialect,
) -> Result<Value, PushdownError> {
    let adapter_notes = read_adapter_notes(request)?;
    let query = pushdown_query::parse(request, &adapter_notes.database).map_err(PushdownError)?;
    let source_sql = dialect
        .render(&query)
        .map_err(|error| PushdownError(error.to_string()))?;

    let emitted_columns = emitted_columns(&query);

    let driver =
        Driver::parse(&adapter_notes.driver).map_err(|error| PushdownError(error.to_string()))?;

    let scan_spec = ScanSpec {
        driver,
        driver_path: adapter_notes.driver_path,
        connection: adapter_notes.connection,
        sql: source_sql,
        columns: emitted_columns.clone(),
    };

    let call = format!(
        "SELECT {SCAN_SCRIPT_NAME}({}) {} FROM DUAL",
        quote_exasol_string_literal(&scan_spec.to_json_string()),
        emits_clause(&emitted_columns),
    );

    Ok(serde_json::json!({"type": "pushdown", "sql": call}))
}

fn read_adapter_notes(request: &Value) -> Result<AdapterNotes, PushdownError> {
    AdapterNotes::read(request).map_err(PushdownError)
}

/// The columns the scan UDF must emit, in the order the `EMITS` clause
/// declares them: one per select-list item, carrying the type the query
/// model declared for it.
///
/// Named from position alone, never from a source column name: a join may
/// project two columns that share a name, and an aggregate or a delegated
/// predicate has no source name at all. Position is the one naming rule
/// that holds for every select-list entry this dialect can render.
fn emitted_columns(query: &Query) -> Vec<ScanColumn> {
    query
        .select_list
        .iter()
        .enumerate()
        .map(|(index, item)| ScanColumn {
            name: positional_column_name(index),
            exasol_type: item.declared_type,
        })
        .collect()
}

/// The `EMITS`/scan-specification name for the select-list entry at
/// `index`, one-based so the first declared column reads `c1`.
fn positional_column_name(index: usize) -> String {
    format!("c{}", index + 1)
}

fn emits_clause(columns: &[ScanColumn]) -> String {
    let rendered = columns
        .iter()
        .map(|column| {
            format!(
                "{} {}",
                quote_exasol_identifier(&column.name),
                exa_type::to_ddl(column.exasol_type)
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!("EMITS ({rendered})")
}

/// Quotes an Exasol identifier with double quotes, doubling any double
/// quote the identifier contains. Every `EMITS` column name goes through
/// this: an unquoted clause is upper-cased by Exasol before comparison
/// against the case-preserved names the scan UDF supplies.
fn quote_exasol_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

/// Quotes an Exasol string literal, doubling any embedded single quote.
fn quote_exasol_string_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

#[cfg(test)]
#[path = "pushdown_tests.rs"]
mod tests;
