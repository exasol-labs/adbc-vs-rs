//! Registering a Rust UDF as an Exasol script.
//!
//! Every such script has the same body — a `%udf_object` directive naming a
//! shared object in BucketFS — and differs only in its declaration. This module
//! owns the body and the statement terminator, so callers write the
//! declaration and nothing else.

use super::exasol::Exasol;
use super::slc::LANGUAGE_ALIAS;

/// Creates or replaces one script over the shared object at `udf_object`.
///
/// `declaration` is everything between the language alias and the `AS` keyword:
/// the script kind, its qualified name, and its parameter and result clauses —
/// for example `SCALAR SCRIPT ADBC_VS.ADBC_SCAN(scan_spec VARCHAR(2000000))
/// EMITS (...)`, or `ADAPTER SCRIPT ADBC_VS.ADBC_ADAPTER`. The script name
/// selects the entry point, so it has to match one the shared object exports.
///
/// `udf_object` is a path from [`udf_object_path`].
///
/// [`udf_object_path`]: super::exasol::udf_object_path
pub fn create(exasol: &Exasol, declaration: &str, udf_object: &str) {
    exasol.execute(&create_statement(declaration, udf_object));
}

/// The `CREATE OR REPLACE` statement `create` submits.
fn create_statement(declaration: &str, udf_object: &str) -> String {
    format!("CREATE OR REPLACE {LANGUAGE_ALIAS} {declaration} AS\n%udf_object {udf_object};")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_statement_wraps_a_scalar_declaration_around_the_shared_object() {
        assert_eq!(
            create_statement(
                "SCALAR SCRIPT ADBC_VS.ADBC_SCAN(scan_spec VARCHAR(2000000)) EMITS (...)",
                "/buckets/bfsdefault/default/udf/libadbc_vs_adapter.so"
            ),
            "CREATE OR REPLACE RUST SCALAR SCRIPT ADBC_VS.ADBC_SCAN(scan_spec VARCHAR(2000000)) \
             EMITS (...) AS\n\
             %udf_object /buckets/bfsdefault/default/udf/libadbc_vs_adapter.so;"
        );
    }

    #[test]
    fn create_statement_wraps_an_adapter_declaration_that_has_no_parameter_list() {
        assert_eq!(
            create_statement(
                "ADAPTER SCRIPT ADBC_VS.ADAPTER",
                "/buckets/bfsdefault/default/udf/libadbc_vs_adapter.so"
            ),
            "CREATE OR REPLACE RUST ADAPTER SCRIPT ADBC_VS.ADAPTER AS\n\
             %udf_object /buckets/bfsdefault/default/udf/libadbc_vs_adapter.so;"
        );
    }

    #[test]
    fn create_statement_terminates_the_script_body() {
        let statement = create_statement("SCALAR SCRIPT s() RETURNS BIGINT", "/buckets/b/x.so");
        assert!(
            statement.ends_with(';'),
            "script body must end with its statement terminator: {statement}"
        );
    }
}
