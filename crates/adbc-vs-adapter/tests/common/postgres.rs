//! The PostgreSQL source of the Compose stack, and the fixture the end-to-end
//! tests read through it.
//!
//! Structural twin of [`super::mysql`]: one module owns where PostgreSQL sits,
//! how to authenticate to it, and what it holds, so no test spells out an
//! address or a credential of its own.
//!
//! It differs from the MySQL module in one way, and deliberately. MySQL's
//! module installs its own fixture tables on every run, because their shape is
//! decided in Rust; PostgreSQL's fixture is authored as SQL in
//! `docker/postgres/init.sql` and seeded by Compose, so this module creates
//! nothing and instead *checks* that the seed a run depends on actually ran.
//! A container whose volume outlived a change to that file would otherwise
//! present an older fixture, and every differential comparison over it would
//! report a pushdown defect that is really a stale table.

use super::readiness::{self, SERVICE_DEADLINE};
use super::{run, try_run};

/// The address the Exasol UDF sandbox reaches PostgreSQL at.
///
/// A fixed address on the Compose network, never the Compose service name:
/// Exasol's `exaconf` rewrites `/etc/hosts` and `/etc/resolv.conf` at start-up,
/// so Docker's service-name resolution does not survive into a UDF sandbox.
pub const ADDRESS: &str = "172.28.5.11";
pub const PORT: u16 = 5432;

/// The `host:port` the stack publishes PostgreSQL at on the loopback
/// interface.
const PUBLISHED_ENDPOINT: &str = "127.0.0.1:5432";

/// The database, user, and password `docker-compose.yml` seeds.
pub const DATABASE: &str = "adbc_vs";
pub const USER: &str = "adbc_vs";
pub const PASSWORD: &str = "adbc_vs_password";

/// The Compose service whose container hosts PostgreSQL.
const SERVICE: &str = "postgres";

/// The tables `docker/postgres/init.sql` seeds beyond the `orders` table the
/// integration tier reads, and which the end-to-end tier depends on. A stack
/// presenting fewer of them is running an older seed.
const SEEDED_TABLES: &[&str] = &[
    "orders",
    "items",
    "item_notes",
    "numeric_specials",
    "all_unmapped",
];

/// The PostgreSQL instance under test.
pub struct Postgres {
    container: String,
}

impl Postgres {
    /// Binds to the PostgreSQL of the Compose stack, waits until it answers
    /// queries, and checks that the Compose seed created the fixture the
    /// end-to-end tier reads.
    ///
    /// Panics when the stack is down or the seed is stale, so a test can never
    /// quietly pass against the wrong fixture.
    pub fn from_stack() -> Self {
        let container = run(
            "locating the PostgreSQL container of the Compose stack",
            "docker",
            &["compose", "ps", "--quiet", SERVICE],
        );
        let container = container.trim().to_owned();
        assert!(
            !container.is_empty(),
            "the Compose stack runs no `{SERVICE}` container; start it with \
             `make stack-up-postgres`"
        );

        let postgres = Self { container };
        postgres.await_queries();
        postgres.check_fixture();
        postgres
    }

    /// Runs a query of one row and one column and returns that value as the
    /// text PostgreSQL rendered it.
    ///
    /// A rejected query fails the run rather than returning, so a caller
    /// reading a row count can never mistake an unreachable database, or a
    /// table that is not there, for an empty one.
    pub fn query_scalar(&self, sql: &str) -> String {
        let rendered = self
            .client(&format!("querying PostgreSQL:\n{sql}"), sql)
            .unwrap_or_else(|failure| panic!("{failure}"));
        let value = rendered.trim();
        assert!(
            !value.contains('\n'),
            "a single-value result was expected from `{sql}`, but PostgreSQL answered:\n{rendered}"
        );
        value.to_owned()
    }

    /// Fails loudly when the Compose seed did not create every fixture table,
    /// naming the recovery that recreates the service from the current
    /// `docker/postgres/init.sql`.
    fn check_fixture(&self) {
        let names = SEEDED_TABLES
            .iter()
            .map(|table| format!("'{table}'"))
            .collect::<Vec<_>>()
            .join(", ");
        let seeded = self.query_scalar(&format!(
            "SELECT count(*) FROM information_schema.tables \
             WHERE table_schema = 'public' AND table_name IN ({names})"
        ));

        assert_eq!(
            seeded,
            SEEDED_TABLES.len().to_string(),
            "database `{DATABASE}` holds {seeded} of the {} seeded tables {SEEDED_TABLES:?}; the \
             Compose seed did not run, which happens when the PostgreSQL container outlived a \
             change to docker/postgres/init.sql. Recreate the service with \
             `docker compose rm -sfv postgres && make stack-up-postgres`",
            SEEDED_TABLES.len()
        );
    }

    fn await_queries(&self) {
        readiness::wait_for_service(SERVICE, PUBLISHED_ENDPOINT, SERVICE_DEADLINE);
        readiness::wait_until(
            &format!("{SERVICE} at {ADDRESS}:{PORT}"),
            SERVICE_DEADLINE,
            || {
                self.client("checking whether PostgreSQL answers queries", "SELECT 1")
                    .is_ok()
            },
        );
    }

    /// Runs `psql` inside the container and returns its output.
    ///
    /// The password travels in the environment rather than on the command
    /// line, so it never appears in the container's process list.
    fn client(&self, description: &str, sql: &str) -> Result<String, String> {
        let arguments = client_arguments(&self.container, sql);
        let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
        try_run(description, "docker", &arguments)
    }
}

/// Builds the `docker exec` argument list that runs `sql` through `psql`
/// inside `container`.
///
/// `--tuples-only` and `--no-align` strip the header and the column padding, so
/// a single-value result arrives as that value alone; `--variable=ON_ERROR_STOP=1`
/// is what makes a rejected statement a non-zero exit rather than a message on
/// standard error and a successful run.
fn client_arguments(container: &str, sql: &str) -> Vec<String> {
    vec![
        "exec".to_string(),
        "--env".to_string(),
        format!("PGPASSWORD={PASSWORD}"),
        container.to_string(),
        "psql".to_string(),
        format!("--username={USER}"),
        format!("--dbname={DATABASE}"),
        "--tuples-only".to_string(),
        "--no-align".to_string(),
        "--variable=ON_ERROR_STOP=1".to_string(),
        "--command".to_string(),
        sql.to_string(),
    ]
}

/// The driver URI an Exasol `CONNECTION` object carries for `database`.
///
/// The scheme is the driver's own canonical name, which `create_schema`
/// compares the statement's `DRIVER` property against before it hands the
/// address to the driver unchanged.
pub fn driver_uri(database: &str) -> String {
    format!("postgresql://{ADDRESS}:{PORT}/{database}")
}

/// A driver URI addressing a port on the PostgreSQL host that nothing
/// listens on.
pub fn unreachable_uri() -> String {
    format!("postgresql://{ADDRESS}:{}/{DATABASE}", PORT + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn driver_uri_addresses_postgres_by_its_fixed_compose_address() {
        assert_eq!(driver_uri("shop"), "postgresql://172.28.5.11:5432/shop");
    }

    #[test]
    fn container_address_invariants_hold() {
        super::super::assert_container_address_invariants(
            &driver_uri(DATABASE),
            &unreachable_uri(),
            PUBLISHED_ENDPOINT,
            PORT,
            PASSWORD,
        );
    }

    #[test]
    fn the_driver_uri_scheme_is_the_drivers_own_name() {
        assert!(
            driver_uri(DATABASE).starts_with(&format!(
                "{}://",
                adbc_vs_scan::driver::Driver::Postgres.as_str()
            )),
            "create_schema refuses an address whose scheme is not the DRIVER property's value"
        );
    }

    #[test]
    fn the_published_endpoint_and_the_in_network_address_share_a_port() {
        assert!(PUBLISHED_ENDPOINT.ends_with(&format!(":{PORT}")));
    }

    #[test]
    fn the_client_stops_on_the_first_rejected_statement() {
        let arguments = client_arguments("some-container", "SELECT 1");
        assert!(
            arguments.contains(&"--variable=ON_ERROR_STOP=1".to_string()),
            "without this flag psql reports a rejected statement and still exits zero, so a \
             failed check would read as a passing one: {arguments:?}"
        );
    }

    #[test]
    fn the_client_passes_the_password_through_the_environment() {
        let arguments = client_arguments("some-container", "SELECT 1");
        assert!(
            arguments
                .iter()
                .all(|argument| argument == &format!("PGPASSWORD={PASSWORD}")
                    || !argument.contains(PASSWORD)),
            "the password must reach psql only as an environment assignment, never as a \
             command-line argument the container's process list shows: {arguments:?}"
        );
    }
}
