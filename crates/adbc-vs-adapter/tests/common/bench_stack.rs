//! The one-time setup the benchmark tier runs against.
//!
//! Its own [`OnceLock`], deliberately separate from [`super::stack`]'s: the
//! e2e tier never needs the JDBC-based virtual schema this module also
//! provisions, so keeping the two setups apart means `make test-e2e` never
//! pays for a jar download, a `settings.cfg` upload, or an `exaplus` adapter
//! script it has no use for.
//!
//! One [`BenchStack`] per source. [`shared_mysql`] brings up both virtual
//! schemas the MySQL benchmark compares — this repo's ADBC-fronted adapter
//! and Exasol's official JDBC-based `mysql-virtual-schema` — over the same
//! MySQL fixture, plus the schema that holds the CTAS sink table a benchmark
//! run creates and drops repeatedly. A source with no JDBC baseline carries
//! `jdbc: None`, and the runner then times the ADBC side alone. Every step
//! panics rather than returning on failure, so a benchmark can never time a
//! run against a half-provisioned stack.

use std::sync::OnceLock;

use super::bench_data::SourceDb;
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

/// A JDBC-based virtual schema a benchmark compares the ADBC side against,
/// and the versions its report provenance names.
///
/// The jar, driver, and `FETCHSIZE` values are mirrored from
/// [`super::jdbc_vs`], whose own constants are private to building BucketFS
/// upload paths and the `settings.cfg` text: the two modules must be bumped
/// together if either one's version or `FETCHSIZE` ever changes.
#[derive(Debug, Clone, Copy)]
pub struct JdbcBaseline {
    pub virtual_schema: &'static str,
    /// The virtual-schema adapter artifact, e.g. `mysql-virtual-schema`.
    pub vs_jar_name: &'static str,
    pub vs_jar_version: &'static str,
    /// The JDBC driver's product name, e.g. `MySQL Connector/J`.
    pub driver_name: &'static str,
    pub driver_version: &'static str,
    pub fetchsize: &'static str,
}

/// The MySQL benchmark's JDBC baseline, as [`super::jdbc_vs`] provisions it.
pub const MYSQL_JDBC_BASELINE: JdbcBaseline = JdbcBaseline {
    virtual_schema: JDBC_VIRTUAL_SCHEMA,
    vs_jar_name: "mysql-virtual-schema",
    vs_jar_version: "6.0.3",
    driver_name: "MySQL Connector/J",
    driver_version: "9.6.0",
    fetchsize: "100000",
};

/// The stack a benchmark run measures against, ready to be queried.
pub struct BenchStack {
    pub exasol: Exasol,
    /// The source every tier is seeded into.
    pub source: SourceDb,
    /// The ADBC-fronted virtual schema over `source`.
    pub adbc_virtual_schema: &'static str,
    /// The JDBC comparison target, or `None` for an ADBC-only run.
    pub jdbc: Option<JdbcBaseline>,
}

impl BenchStack {
    /// Every virtual schema a sample reads through, ADBC first.
    pub fn virtual_schemas(&self) -> Vec<&'static str> {
        std::iter::once(self.adbc_virtual_schema)
            .chain(self.jdbc.map(|jdbc| jdbc.virtual_schema))
            .collect()
    }
}

/// The MySQL benchmark stack, brought up on first use.
///
/// A failure anywhere in setup panics rather than returning, so no benchmark
/// sample can be timed — or reported — against a stack that never came up.
pub fn shared_mysql() -> &'static BenchStack {
    static STACK: OnceLock<BenchStack> = OnceLock::new();
    STACK.get_or_init(bring_up_mysql)
}

fn bring_up_mysql() -> BenchStack {
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

    BenchStack {
        exasol,
        source: SourceDb::Mysql(mysql),
        adbc_virtual_schema: ADBC_VIRTUAL_SCHEMA,
        jdbc: Some(MYSQL_JDBC_BASELINE),
    }
}
