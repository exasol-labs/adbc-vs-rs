pub mod adapter_notes;
pub mod capabilities;
pub mod create_schema;
pub mod dispatch;
pub mod emit_type;
pub mod exa_type;
pub mod metadata;
pub mod mysql_dialect;
pub mod mysql_source;
pub mod mysql_type_mapping;
pub mod postgres_dialect;
pub mod postgres_source;
pub mod postgres_type_mapping;
pub mod provisioning;
pub mod pushdown;
pub mod pushdown_query;
pub mod registry_index;
pub mod scan_entry;
pub mod scan_spec;
pub mod sql_dialect;
pub mod trino_dialect;
pub mod trino_source;
pub mod trino_type_mapping;
pub mod type_mapping;

/// The rules that bind the driver identity's three tables, which belong to no
/// one of them.
#[cfg(test)]
mod driver_selection_tests;

/// The dialect-agnostic query model every dialect's own tests are driven with.
#[cfg(test)]
mod dialect_test_fixtures;

use exasol_udf_macros::exasol_udf;
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;

/// The `ADAPTER SCRIPT` entry point. `#[exasol_udf]`'s `vs_adapter(...)`
/// form wires [`dispatch::adapter_call`] into the
/// `virtual_schema_adapter_call` vtable slot and, from this function's own
/// identifier, exports `__exa_udf_entry_ADBC_ADAPTER` — the symbol
/// `adapter-request-dispatch`'s artifact test looks for. This function's own
/// body never runs: the adapter protocol calls only the wired-in
/// `virtual_schema_adapter_call` slot.
#[exasol_udf(vs_adapter(dispatch::adapter_call))]
fn adbc_adapter(_ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    Ok(())
}

/// The scan `SCALAR ... EMITS` entry point, called once per pushed-down
/// statement with the scan specification as its single string argument. No
/// static `emits(...)` is declared: the pushdown response supplies a
/// query-side dynamic `EMITS` clause instead, because the emitted columns
/// vary per virtual table. Exports `__exa_udf_entry_ADBC_SCAN`, the symbol
/// `arrow-scan`'s artifact test looks for.
#[exasol_udf(input(scan_spec: String))]
fn adbc_scan(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    let scan_spec_json = ctx
        .get_string(0)?
        .ok_or_else(|| UdfError::User("adbc_scan's scan_spec argument is NULL".to_string()))?
        .to_string();
    scan_entry::run_scan(ctx, &scan_spec_json)
}
