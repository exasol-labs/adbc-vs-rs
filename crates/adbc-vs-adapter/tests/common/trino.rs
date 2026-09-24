//! The Trino source of the Compose stack, and the fixture the end-to-end
//! tests read through it.
//!
//! Structural twin of [`super::postgres`]: Trino's own `memory` catalog
//! fixture is authored as SQL in `docker/trino/init.sql` and seeded through
//! the container's own CLI rather than templated from Rust, so this module
//! creates nothing and instead *checks* that the seed a run depends on
//! actually ran. Unlike PostgreSQL's container, which skips its seed on a
//! reused volume, the `memory` catalog holds no data across a restart, so
//! `docker/trino/init.sql` is re-applied by `stack-up-trino` on every stack
//! start rather than only on a fresh one. That still leaves one way a run can
//! see a stale fixture: a container left running from before a change to
//! that file, with no intervening `make stack-up-trino`. `check_fixture`
//! catches that case the same way [`super::postgres::Postgres::check_fixture`]
//! catches PostgreSQL's own volume-persistence case, for the same reason: a
//! differential comparison over a stale table would report a pushdown defect
//! that is really a stale fixture.
//!
//! Trino's own identifier folding is the one way this module's shape differs
//! from its PostgreSQL twin. Measured directly against this container: the
//! `memory` connector lowercases every table and column name regardless of
//! how the DDL that created it quoted the identifier, so
//! `docker/trino/init.sql` declares no mixed-case object — there is no case
//! left for the source to preserve by the time discovery reads it back.
//! `e2e_trino_test.rs`'s own identifier-case test asserts instead that
//! discovery and rendering keep whatever case Trino's own catalog reports,
//! byte for byte.

use super::readiness::{self, SERVICE_DEADLINE};
use super::{run, try_run};

/// The address the Exasol UDF sandbox reaches Trino at.
///
/// A fixed address on the Compose network, never the Compose service name:
/// Exasol's `exaconf` rewrites `/etc/hosts` and `/etc/resolv.conf` at
/// start-up, so Docker's service-name resolution does not survive into a UDF
/// sandbox.
pub const ADDRESS: &str = "172.28.5.12";

/// The TLS port `docker/trino/config.properties` serves authenticated
/// queries on. `PORT_PLAIN_HTTP` is the port that answers no query at all —
/// `scan-udf/trino-scan`'s own transport assertions are what address it.
pub const PORT: u16 = 8443;
pub const PORT_PLAIN_HTTP: u16 = 8080;

/// The `host:port` the stack publishes Trino's TLS endpoint at on the
/// loopback interface.
const PUBLISHED_ENDPOINT: &str = "127.0.0.1:8443";

/// The catalog and schema `docker/trino/init.sql` seeds, and every generated
/// statement qualifies a table by.
pub const CATALOG: &str = "memory";
pub const SCHEMA: &str = "adbc_vs";

/// The principal `docker/trino/password.db` configures for the benign path,
/// and the plaintext `docker-compose.yml` and the `Makefile` declare as
/// `TRINO_PASSWORD`.
pub const USER: &str = "adbc_vs";
pub const PASSWORD: &str = "adbc_vs_password";

/// The second principal `docker/trino/password.db` configures, per
/// `scan-udf/trino-scan`, whose password carries `@`, `/`, `:`, `?`, `#`, `&`, `%`, and a
/// space in one value — every character that would have delimited, ended, or
/// escaped something in a URI. Task 8.5's own `CONNECTION` addresses this
/// principal rather than a benign one, so the end-to-end path exercises the
/// same credential the integration tier (`trino_it_test.rs`) does.
pub const HOSTILE_USER: &str = "adbc_vs_hostile";
pub const HOSTILE_PASSWORD: &str = "p@ss/w:o?r#d&x% y";

/// The Compose service whose container hosts Trino.
const SERVICE: &str = "trino";

/// The objects `docker/trino/init.sql` seeds beyond the `orders` table the
/// integration tier reads, and which the end-to-end tier depends on. A stack
/// presenting fewer of them is running an older seed.
///
/// `orders_view` is a view, not a table, but `information_schema.tables`
/// lists views alongside base tables with `table_type = 'VIEW'`, so the one
/// query in [`Trino::check_fixture`] finds both kinds without a second check.
const SEEDED_OBJECTS: &[&str] = &[
    "orders",
    "items",
    "item_notes",
    "all_unmapped",
    "refresh_target",
    "orders_view",
];

/// The Trino instance under test.
pub struct Trino {
    container: String,
}

impl Trino {
    /// Binds to the Trino of the Compose stack, waits until it answers
    /// authenticated queries, and checks that the seed the end-to-end tier
    /// reads actually ran.
    ///
    /// Panics when the stack is down or the seed is stale, so a test can
    /// never quietly pass against the wrong fixture.
    pub fn from_stack() -> Self {
        let container = run(
            "locating the Trino container of the Compose stack",
            "docker",
            &["compose", "ps", "--quiet", SERVICE],
        );
        let container = container.trim().to_owned();
        assert!(
            !container.is_empty(),
            "the Compose stack runs no `{SERVICE}` container; start it with \
             `make stack-up-trino`"
        );

        let trino = Self { container };
        trino.await_queries();
        trino.check_fixture();
        trino
    }

    /// Runs a query of one row and one column and returns that value as the
    /// text Trino rendered it.
    ///
    /// A rejected query fails the run rather than returning, so a caller
    /// reading a row count can never mistake an unreachable server, or a
    /// table that is not there, for an empty one.
    pub fn query_scalar(&self, sql: &str) -> String {
        let rendered = self
            .client(&format!("querying Trino:\n{sql}"), sql)
            .unwrap_or_else(|failure| panic!("{failure}"));
        let value = rendered.trim();
        assert!(
            !value.contains('\n'),
            "a single-value result was expected from `{sql}`, but Trino answered:\n{rendered}"
        );
        value.to_owned()
    }

    /// Runs one or more statements against the seeded catalog and schema.
    ///
    /// `docker/trino/init.sql` owns the fixture's shape; this method exists
    /// for the one test that must change the source mid-run. A rejected
    /// statement panics rather than returning, so a DDL no-op can never read
    /// as a pass.
    pub fn execute(&self, sql: &str) {
        self.client(&format!("executing SQL against Trino:\n{sql}"), sql)
            .unwrap_or_else(|failure| panic!("{failure}"));
    }

    /// Fails loudly when the seed did not create every fixture object,
    /// naming the recovery that re-applies the current
    /// `docker/trino/init.sql`.
    fn check_fixture(&self) {
        let names = SEEDED_OBJECTS
            .iter()
            .map(|table| format!("'{table}'"))
            .collect::<Vec<_>>()
            .join(", ");
        let seeded = self.query_scalar(&format!(
            "SELECT count(*) FROM {CATALOG}.information_schema.tables \
             WHERE table_schema = '{SCHEMA}' AND table_name IN ({names})"
        ));

        assert_eq!(
            seeded,
            SEEDED_OBJECTS.len().to_string(),
            "catalog `{CATALOG}` schema `{SCHEMA}` holds {seeded} of the {} seeded fixture \
             objects {SEEDED_OBJECTS:?}; the seed did not run against the running container, \
             which happens when it outlived a change to docker/trino/init.sql. Re-apply it with \
             `make stack-up-trino`",
            SEEDED_OBJECTS.len()
        );
    }

    fn await_queries(&self) {
        readiness::wait_for_service(SERVICE, PUBLISHED_ENDPOINT, SERVICE_DEADLINE);
        readiness::wait_until(
            &format!("{SERVICE} at {ADDRESS}:{PORT}"),
            SERVICE_DEADLINE,
            || {
                self.client(
                    "checking whether Trino answers authenticated queries",
                    "SELECT 1",
                )
                .is_ok()
            },
        );
    }

    /// Runs the `trino` CLI inside the container and returns its output.
    ///
    /// The password travels in the environment rather than on the command
    /// line, so it never appears in the container's process list — the same
    /// shape `docker-compose.yml`'s own `TRINO_PASSWORD` convention and
    /// `stack-up-trino`'s Make recipe use.
    fn client(&self, description: &str, sql: &str) -> Result<String, String> {
        let arguments = client_arguments(&self.container, sql);
        let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
        try_run(description, "docker", &arguments)
    }
}

/// Builds the `docker exec` argument list that runs `sql` through the
/// `trino` CLI inside `container`, authenticated as [`USER`].
///
/// `--output-format CSV_UNQUOTED` is what makes a single-value result arrive
/// as that value alone, with no header and no quoting around a text value;
/// `--insecure` is what accepts the self-signed certificate `stack-up-trino`
/// generates.
fn client_arguments(container: &str, sql: &str) -> Vec<String> {
    vec![
        "exec".to_string(),
        "--env".to_string(),
        format!("TRINO_PASSWORD={PASSWORD}"),
        container.to_string(),
        "trino".to_string(),
        "--server".to_string(),
        format!("https://localhost:{PORT}"),
        format!("--user={USER}"),
        "--password".to_string(),
        "--insecure".to_string(),
        "--output-format".to_string(),
        "CSV_UNQUOTED".to_string(),
        "--execute".to_string(),
        sql.to_string(),
    ]
}

/// The driver URI an Exasol `CONNECTION` object carries for the seeded
/// catalog and schema.
///
/// The scheme is the driver's own canonical name, which `create_schema`
/// compares the statement's `DRIVER` property against before it hands the
/// address to the driver unchanged. `SSLVerification=NONE` is the one
/// spelling this driver was measured honouring against the self-signed
/// certificate `stack-up-trino` generates.
pub fn driver_uri() -> String {
    format!("trino://{ADDRESS}:{PORT}/{CATALOG}/{SCHEMA}?SSLVerification=NONE")
}

/// A driver URI addressing a port on the Trino host that nothing listens on.
pub fn unreachable_uri() -> String {
    format!(
        "trino://{ADDRESS}:{}/{CATALOG}/{SCHEMA}?SSLVerification=NONE",
        PORT + 1
    )
}

/// An address naming the plain-HTTP port with no `SSL=false` parameter.
/// The driver still attempts HTTPS by default here, as measured against this
/// container, so this address is expected to fail inside the driver rather
/// than reach the server at all.
pub fn plain_http_port_uri() -> String {
    format!("trino://{ADDRESS}:{PORT_PLAIN_HTTP}/{CATALOG}/{SCHEMA}")
}

/// An address naming the plain-HTTP port with `SSL=false`, which is what
/// makes the driver actually send the request in the clear. Trino's own
/// server was measured refusing it,
/// `403 Forbidden: "Error 403 Forbidden: Authentication over HTTP is not
/// enabled"`, because `http-server.authentication.allow-insecure-over-http`
/// defaults to `false`.
pub fn ssl_false_uri() -> String {
    format!("trino://{ADDRESS}:{PORT_PLAIN_HTTP}/{CATALOG}/{SCHEMA}?SSL=false")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn driver_uri_addresses_trino_by_its_fixed_compose_address() {
        assert_eq!(
            driver_uri(),
            "trino://172.28.5.12:8443/memory/adbc_vs?SSLVerification=NONE"
        );
    }

    #[test]
    fn container_address_invariants_hold() {
        super::super::assert_container_address_invariants(super::super::ContainerAddresses {
            driver_uri: &driver_uri(),
            unreachable_uri: &unreachable_uri(),
            published_endpoint: PUBLISHED_ENDPOINT,
            port: PORT,
            password: PASSWORD,
        });
    }

    #[test]
    fn the_driver_uri_scheme_is_the_drivers_own_name() {
        assert!(
            driver_uri().starts_with(&format!(
                "{}://",
                adbc_vs_scan::driver::Driver::Trino.as_str()
            )),
            "create_schema refuses an address whose scheme is not the DRIVER property's value"
        );
    }

    #[test]
    fn the_plain_http_port_uri_carries_no_ssl_false_parameter() {
        assert!(
            !plain_http_port_uri().contains("SSL=false"),
            "this address is meant to fail inside the driver, which attempts HTTPS by default \
             on every port: {}",
            plain_http_port_uri()
        );
    }

    #[test]
    fn the_ssl_false_uri_carries_the_parameter_that_selects_plain_http() {
        assert!(ssl_false_uri().contains("SSL=false"));
    }

    #[test]
    fn the_hostile_password_carries_every_character_gate_2_1_measured() {
        for character in ['@', '/', ':', '?', '#', '&', '%', ' '] {
            assert!(
                HOSTILE_PASSWORD.contains(character),
                "HOSTILE_PASSWORD must carry {character:?}: {HOSTILE_PASSWORD}"
            );
        }
    }

    #[test]
    fn the_client_requests_an_unheaded_unquoted_result() {
        let arguments = client_arguments("some-container", "SELECT 1");
        assert!(
            arguments.contains(&"--output-format".to_string()),
            "the client must request an unheaded, unquoted result: {arguments:?}"
        );
    }

    #[test]
    fn the_client_passes_the_password_through_the_environment() {
        let arguments = client_arguments("some-container", "SELECT 1");
        assert!(
            arguments
                .iter()
                .all(|argument| argument == &format!("TRINO_PASSWORD={PASSWORD}")
                    || !argument.contains(PASSWORD)),
            "the password must reach the trino CLI only as an environment assignment, never as \
             a command-line argument the container's process list shows: {arguments:?}"
        );
    }
}
