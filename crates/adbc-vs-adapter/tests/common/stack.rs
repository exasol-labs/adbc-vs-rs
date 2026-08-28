//! The one-time setup every end-to-end test runs against.
//!
//! Bringing the stack to a usable state costs a shared-object upload, an
//! archive extraction, and a driver install, so it happens once per test
//! binary and every test shares the result. That sharing is what
//! `--test-threads=1` is for: the tests run against one Exasol, one MySQL, one
//! virtual schema, and one local mirror of the fixture, in sequence.
//!
//! Setup never returns half-ready. Each step waits for the thing it produced
//! to be usable and fails the run otherwise, so a test that starts always
//! starts against a stack that is actually up.

use std::sync::OnceLock;

use super::exasol::Exasol;
use super::mirror;
use super::mysql::{self, MySql};
use super::schema;
use super::slc;
use super::udf;

/// The virtual schema the tests query.
pub const VIRTUAL_SCHEMA: &str = "MYSQLVS";

/// The `CONNECTION` object addressing the seeded MySQL database.
pub const MYSQL_CONNECTION: &str = "ADBC_VS_MYSQL";

/// The stack under test, ready to be queried.
pub struct Stack {
    pub exasol: Exasol,
    pub mysql: MySql,
}

/// The stack, brought up on first use.
///
/// A failure anywhere in setup panics rather than returning, so no test can
/// run — or pass — against a stack that never came up.
pub fn shared() -> &'static Stack {
    static STACK: OnceLock<Stack> = OnceLock::new();
    STACK.get_or_init(bring_up)
}

fn bring_up() -> Stack {
    let exasol = Exasol::from_stack();
    let mysql = MySql::from_stack();

    slc::install(&exasol);
    udf::deploy(&exasol);
    schema::create_bucketfs_connection(&exasol);
    schema::create_mysql_connection(&exasol, MYSQL_CONNECTION, mysql::DATABASE);
    schema::create_virtual_schema(&exasol, VIRTUAL_SCHEMA, MYSQL_CONNECTION);
    mirror::install(&exasol);

    Stack { exasol, mysql }
}
