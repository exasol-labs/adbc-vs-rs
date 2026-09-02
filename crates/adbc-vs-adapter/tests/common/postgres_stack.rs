//! The one-time setup the PostgreSQL end-to-end tests run against.
//!
//! Structural twin of [`super::stack`], which serves the MySQL end-to-end
//! binary, and separate from it on purpose: two binaries keep each driver's
//! shared setup independent, so a MySQL-only run provisions no PostgreSQL
//! virtual schema and a failure names which driver's tier broke.
//!
//! What this module brings up that its twin does not is a *second* virtual
//! schema, over MySQL, beside the PostgreSQL one. The two exist together so the
//! two-driver claims can be made at all: that each driver's library installs
//! under its own BucketFS path, that a query over each reaches its own source,
//! and that each virtual schema advertises the capability set its own dialect
//! declares. None of those is observable from a stack carrying one driver.
//!
//! Setup never returns half-ready. Each step waits for the thing it produced to
//! be usable and fails the run otherwise, so a test that starts always starts
//! against a stack that is actually up.

use std::sync::OnceLock;

use adbc_vs_scan::driver::Driver;

use super::exasol::Exasol;
use super::mysql::{self, MySql};
use super::postgres::{self, Postgres};
use super::postgres_mirror;
use super::schema;
use super::slc;
use super::udf;

/// The virtual schema over the PostgreSQL source.
pub const POSTGRES_VIRTUAL_SCHEMA: &str = "PGVS";

/// The virtual schema over the MySQL source.
///
/// Named apart from [`super::stack::VIRTUAL_SCHEMA`] rather than reusing it:
/// the two end-to-end binaries drive the same Exasol, and a shared name would
/// make one binary's setup silently replace the other's schema.
pub const MYSQL_VIRTUAL_SCHEMA: &str = "PGRUN_MYSQLVS";

/// The `CONNECTION` object addressing the seeded PostgreSQL database.
pub const POSTGRES_CONNECTION: &str = "ADBC_VS_POSTGRES";

/// The `CONNECTION` object addressing the seeded MySQL database. The same
/// object the MySQL binary creates, from the same address and credentials, so
/// the two runs cannot disagree about what it points at.
pub use super::stack::MYSQL_CONNECTION;

/// The stack under test, ready to be queried.
pub struct Stack {
    pub exasol: Exasol,
    /// The handle a test uses to change the PostgreSQL source mid-run.
    ///
    /// Kept on `Stack` rather than reconstructed per call, because
    /// `Postgres::from_stack` re-runs the readiness wait and the seed check —
    /// mirrors [`super::stack::Stack::mysql`].
    pub postgres: Postgres,
}

/// The stack, brought up on first use.
///
/// A failure anywhere in setup panics rather than returning, so no test can
/// run — or pass — against a stack that never came up.
pub fn shared() -> &'static Stack {
    static STACK: OnceLock<Stack> = OnceLock::new();
    STACK.get_or_init(bring_up)
}

/// Constructing `MySql` is the contract this function runs it for, not a
/// value either the caller or [`Stack`] needs to keep: `MySql::from_stack`
/// installs the MySQL fixtures, a side effect the MySQL virtual schema
/// created below depends on having already happened, so it does not outlive
/// this function.
fn bring_up() -> Stack {
    let exasol = Exasol::from_stack();
    let postgres = Postgres::from_stack();
    let _mysql = MySql::from_stack();

    slc::install(&exasol);
    udf::deploy(&exasol);
    schema::create_bucketfs_connection(&exasol);

    schema::create_postgres_connection(&exasol, POSTGRES_CONNECTION, postgres::DATABASE);
    schema::create_virtual_schema(
        &exasol,
        Driver::Postgres,
        POSTGRES_VIRTUAL_SCHEMA,
        POSTGRES_CONNECTION,
    );

    schema::create_mysql_connection(&exasol, MYSQL_CONNECTION, mysql::DATABASE);
    schema::create_virtual_schema(
        &exasol,
        Driver::MySql,
        MYSQL_VIRTUAL_SCHEMA,
        MYSQL_CONNECTION,
    );

    postgres_mirror::install(&exasol);

    Stack { exasol, postgres }
}
