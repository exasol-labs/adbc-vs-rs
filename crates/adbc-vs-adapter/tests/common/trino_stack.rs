//! The one-time setup the Trino end-to-end tests run against.
//!
//! Structural twin of [`super::postgres_stack`], which serves the PostgreSQL
//! end-to-end binary, and separate from it on purpose: two binaries keep each
//! driver's shared setup independent, so a Trino-only run provisions no
//! PostgreSQL virtual schema and a failure names which driver's tier broke.
//!
//! What this module brings up that its MySQL twin does not is a *second*
//! virtual schema, over MySQL, beside the Trino one. The two exist together
//! so the per-schema capability difference `vs-adapter/trino-pushdown` states is observable at all:
//! `drivers/trino.toml` advertises `JOIN_TYPE_FULL_OUTER` and `mysql.toml`
//! does not, and that difference shows only when both schemas are planned
//! over the same shape of query against one Exasol.
//!
//! Setup never returns half-ready. Each step waits for the thing it produced
//! to be usable and fails the run otherwise, so a test that starts always
//! starts against a stack that is actually up.

use std::sync::OnceLock;

use adbc_vs_scan::driver::Driver;

use super::exasol::Exasol;
use super::mysql::{self, MySql};
use super::schema;
use super::slc;
use super::trino::{self, Trino};
use super::trino_mirror;
use super::udf;

/// The virtual schema over the Trino source.
pub const TRINO_VIRTUAL_SCHEMA: &str = "TRVS";

/// The virtual schema over the MySQL source.
///
/// Named apart from [`super::stack::VIRTUAL_SCHEMA`] and
/// [`super::postgres_stack::MYSQL_VIRTUAL_SCHEMA`] rather than reusing
/// either: every end-to-end binary drives the same Exasol, and a shared name
/// would make one binary's setup silently replace another's schema.
pub const MYSQL_VIRTUAL_SCHEMA: &str = "TRRUN_MYSQLVS";

/// The `CONNECTION` object addressing the seeded Trino catalog and schema,
/// authenticated as [`trino::USER`] — the benign principal every test but
/// `scan-udf/trino-scan`'s credential tests read through.
pub const TRINO_CONNECTION: &str = "ADBC_VS_TRINO";

/// The `CONNECTION` object addressing the seeded MySQL database. The same
/// object the MySQL and PostgreSQL binaries create, from the same address
/// and credentials, so the runs cannot disagree about what it points at.
pub use super::stack::MYSQL_CONNECTION;

/// The stack under test, ready to be queried.
pub struct Stack {
    pub exasol: Exasol,
    /// The handle a test uses to change the Trino source mid-run.
    ///
    /// Kept on `Stack` rather than reconstructed per call, because
    /// `Trino::from_stack` re-runs the readiness wait and the seed check —
    /// mirrors [`super::postgres_stack::Stack::postgres`].
    pub trino: Trino,
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
    let source = Trino::from_stack();
    let _mysql = MySql::from_stack();

    slc::install(&exasol);
    udf::deploy(&exasol);
    schema::provide_drivers(&exasol, &[Driver::Trino, Driver::MySql]);

    exasol.execute(&format!(
        "CREATE OR REPLACE CONNECTION {TRINO_CONNECTION} TO {} USER {} IDENTIFIED BY {}",
        schema::quote_literal(&trino::driver_uri()),
        schema::quote_literal(trino::USER),
        schema::quote_literal(trino::PASSWORD)
    ));
    schema::create_virtual_schema(
        &exasol,
        Driver::Trino,
        TRINO_VIRTUAL_SCHEMA,
        TRINO_CONNECTION,
    );

    schema::create_mysql_connection(&exasol, MYSQL_CONNECTION, mysql::DATABASE);
    schema::create_virtual_schema(
        &exasol,
        Driver::MySql,
        MYSQL_VIRTUAL_SCHEMA,
        MYSQL_CONNECTION,
    );

    trino_mirror::install(&exasol);

    Stack {
        exasol,
        trino: source,
    }
}
