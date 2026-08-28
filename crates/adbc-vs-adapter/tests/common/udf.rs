//! Putting the built adapter shared object into the stack and registering its
//! two entry points as Exasol scripts.
//!
//! The names here are not free: `#[exasol_udf]` derives an exported symbol
//! from each entry point's identifier and the script name selects it, so
//! `ADBC_ADAPTER` and `ADBC_SCAN` MUST match `crates/adbc-vs-adapter/src/lib.rs`.
//! The scan script's own name is not spelled here at all: it is imported from
//! [`adbc_vs_adapter::pushdown`], the module that generates the call, so the
//! registered name and the called name cannot drift apart.

use std::path::PathBuf;

use super::exasol::{self, Exasol};
use super::script;

/// The shared object `make cross-udf-build` produces, relative to the
/// workspace root.
const BUILT_LIBRARY: &str = "target/release/libadbc_vs_adapter.so";

/// Where that shared object lives in the bucket.
const BUCKET_PATH: &str = "udf/libadbc_vs_adapter.so";

/// The Exasol schema both scripts are created in.
pub const SCRIPT_SCHEMA: &str = "ADBC_VS";

/// The adapter script a `CREATE VIRTUAL SCHEMA` statement names.
pub const ADAPTER_SCRIPT: &str = "ADBC_VS.ADBC_ADAPTER";

/// The scan script every pushed-down statement calls.
pub const SCAN_SCRIPT: &str = adbc_vs_adapter::pushdown::SCAN_SCRIPT_NAME;

/// The scan script's single argument: the scan specification the adapter
/// embeds in its pushdown response, whose length grows with the column count.
const SCAN_ARGUMENT: &str = "scan_spec VARCHAR(2000000)";

/// The scan script's output is declared per call, by the `EMITS` clause the
/// pushdown response generates, because the emitted columns differ per virtual
/// table.
const DYNAMIC_EMITS: &str = "EMITS (...)";

/// Installs the built shared object and creates both scripts over it.
///
/// Repeat calls are safe: both statements replace what they find.
pub fn deploy(exasol: &Exasol) {
    exasol.put_library(&built_library(), BUCKET_PATH);

    let udf_object = exasol::udf_object_path(BUCKET_PATH);
    exasol.execute(&format!("CREATE SCHEMA IF NOT EXISTS {SCRIPT_SCHEMA}"));
    script::create(exasol, &adapter_declaration(), &udf_object);
    script::create(exasol, &scan_declaration(), &udf_object);
}

/// The shared object to install, or a failure naming how to build it.
///
/// It is never built here: it MUST come out of the `rust:1.94-trixie`
/// container so its glibc fingerprint matches the script language container,
/// and a host build that happened to load would prove nothing about the
/// artifact this project ships.
fn built_library() -> PathBuf {
    let library = exasol::workspace_root().join(BUILT_LIBRARY);
    assert!(
        library.is_file(),
        "{} does not exist; build it with `make cross-udf-build`",
        library.display()
    );
    library
}

fn adapter_declaration() -> String {
    format!("ADAPTER SCRIPT {ADAPTER_SCRIPT}")
}

fn scan_declaration() -> String {
    format!("SCALAR SCRIPT {SCAN_SCRIPT}({SCAN_ARGUMENT}) {DYNAMIC_EMITS}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_adapter_declaration_names_no_parameter_list() {
        assert_eq!(adapter_declaration(), "ADAPTER SCRIPT ADBC_VS.ADBC_ADAPTER");
    }

    #[test]
    fn the_scan_declaration_takes_one_specification_and_emits_dynamically() {
        assert_eq!(
            scan_declaration(),
            "SCALAR SCRIPT ADBC_VS.ADBC_SCAN(scan_spec VARCHAR(2000000)) EMITS (...)"
        );
    }

    #[test]
    fn both_scripts_live_in_the_schema_the_harness_creates() {
        assert!(ADAPTER_SCRIPT.starts_with(&format!("{SCRIPT_SCHEMA}.")));
        assert!(SCAN_SCRIPT.starts_with(&format!("{SCRIPT_SCHEMA}.")));
    }
}
