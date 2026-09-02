//! The one-time setup the benchmark tier runs against.
//!
//! Its own [`OnceLock`], deliberately separate from [`super::stack`]'s: the
//! e2e tier never needs the JDBC-based virtual schema this module also
//! provisions, so keeping the two setups apart means `make test-e2e` never
//! pays for a jar download, a `settings.cfg` upload, or an `exaplus` adapter
//! script it has no use for.
//!
//! On first access this brings up both virtual schemas the benchmark compares
//! — this repo's ADBC-fronted adapter and Exasol's official JDBC-based
//! `mysql-virtual-schema` — over the same MySQL fixture, plus the schema that
//! holds the CTAS sink table a benchmark run creates and drops repeatedly.
//! Every step panics rather than returning on failure, so a benchmark can
//! never time a run against a half-provisioned stack.

use std::sync::OnceLock;

use super::exasol::Exasol;
use super::jdbc_vs;
use super::mysql::{self, MySql};
use super::schema;
use super::slc;
use super::udf;

/// The ADBC-fronted virtual schema this repo's adapter provisions.
///
/// Named distinctly from [`super::stack::VIRTUAL_SCHEMA`] so both can exist in
/// the same Exasol instance without one replacing the other, should a local
/// dev session ever bring up both tiers against a stack left standing by the
/// other.
pub const ADBC_VIRTUAL_SCHEMA: &str = "MYSQLVS_ADBC";

/// The JDBC-based virtual schema — the benchmark's comparison target.
pub const JDBC_VIRTUAL_SCHEMA: &str = "MYSQLVS_JDBC";

/// The `CONNECTION` object the ADBC virtual schema resolves.
///
/// Named distinctly from [`super::stack::MYSQL_CONNECTION`] for the same
/// reason as [`ADBC_VIRTUAL_SCHEMA`].
pub const MYSQL_CONNECTION: &str = "ADBC_VS_MYSQL_BENCH";

/// The schema holding the CTAS sink table a benchmark run creates and drops
/// repeatedly while timing samples.
pub const SINK_SCHEMA: &str = "BENCH";

/// The stack a benchmark run measures against, ready to be queried.
pub struct BenchStack {
    pub exasol: Exasol,
    pub mysql: MySql,
}

/// The stack, brought up on first use.
///
/// A failure anywhere in setup panics rather than returning, so no benchmark
/// sample can be timed — or reported — against a stack that never came up.
pub fn shared() -> &'static BenchStack {
    static STACK: OnceLock<BenchStack> = OnceLock::new();
    STACK.get_or_init(bring_up)
}

fn bring_up() -> BenchStack {
    let exasol = Exasol::from_stack();
    let mysql = MySql::from_stack();

    slc::install(&exasol);
    udf::deploy(&exasol);
    schema::create_bucketfs_connection(&exasol);
    schema::create_mysql_connection(&exasol, MYSQL_CONNECTION, mysql::DATABASE);
    schema::create_virtual_schema(
        &exasol,
        adbc_vs_scan::driver::Driver::MySql,
        ADBC_VIRTUAL_SCHEMA,
        MYSQL_CONNECTION,
    );

    jdbc_vs::install(&exasol, JDBC_VIRTUAL_SCHEMA);

    exasol.execute(&format!("CREATE SCHEMA IF NOT EXISTS {SINK_SCHEMA}"));

    BenchStack { exasol, mysql }
}
