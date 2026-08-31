//! The MySQL source of the Compose stack, and the fixture databases the
//! end-to-end tests read through it.
//!
//! One module owns where MySQL sits, how to authenticate to it, and what it
//! holds, so no test spells out an address, a credential, or a `CREATE TABLE`
//! of its own.

use super::items;
use super::readiness::{self, SERVICE_DEADLINE};
use super::{run, try_run};

/// The address the Exasol UDF sandbox reaches MySQL at.
///
/// A fixed address on the Compose network, never the Compose service name:
/// Exasol's `exaconf` rewrites `/etc/hosts` and `/etc/resolv.conf` at start-up,
/// so Docker's service-name resolution does not survive into a UDF sandbox.
pub const ADDRESS: &str = "172.28.5.10";
pub const PORT: u16 = 3306;

/// The `host:port` the stack publishes MySQL at on the loopback interface.
const PUBLISHED_ENDPOINT: &str = "127.0.0.1:3306";

/// The database, user, and passwords `docker-compose.yml` seeds.
pub const DATABASE: &str = "adbc_vs";
pub const USER: &str = "adbc_vs";
pub const PASSWORD: &str = "adbc_vs_password";
const ROOT_PASSWORD: &str = "adbc_vs_root";

/// A second database, created empty, so a virtual schema over a source with no
/// table can be observed.
pub const EMPTY_DATABASE: &str = "adbc_vs_empty";

/// The Compose service whose container hosts MySQL.
const SERVICE: &str = "mysql";

/// The fixture tables the end-to-end tests read, beyond the `orders` table the
/// Compose seed creates.
///
/// Dropped and recreated on every run, so a re-run against a stack left
/// standing by an earlier one starts from the same state — in particular
/// `refresh_target`, which one test grows a column onto.
///
/// `items` and its join partner `item_notes` carry the filter, aggregation,
/// join, and ordering tiers' fixture needs in one place, for a local Exasol
/// mirror to reproduce exactly. Their row values come from
/// [`super::items`] rather than being spelled here, so the mirror renders the
/// same bytes from the same constants instead of a second author's reading of
/// them; the types, the `latin1` character set, and the primary keys stay
/// here, because they are the source's own and the mirror declares its own:
///
/// - `category`: the aggregation `GROUP BY`/`DISTINCT` column. It holds the
///   measured `Widget`/`widget`/`Gizmo`/`Gîzmo` set (decision-log `[17]`,
///   `[19]`), each value seeded onto two rows so every group is multi-row.
///   MySQL's default `utf8mb4_0900_ai_ci` collation folds the set to 2
///   groups; Exasol's `utf8mb4_0900_as_cs` comparison collation keeps 4.
/// - `code`: a `CHAR(8)` column, so the CHAR-to-VARCHAR mapping (decision
///   `[15]`) is exercised end to end.
/// - `batch`: a `VARCHAR`, not `CHAR`, column carrying the `'ab'`/`'ab '`/
///   `'ab!'` set decision-log `[6]`'s review round measured `MIN` diverging
///   over under `utf8mb4_bin` — a `CHAR` column would strip the trailing
///   space on retrieval and never reproduce the hazard.
/// - `quantity`: nullable, with a NULL, a value above, and a value below the
///   threshold a select-list predicate compares against.
/// - `rank_key`/`tie_break`: a sort key that repeats (`rank_key`) with a
///   second key (`tie_break`) that disambiguates it; seeded out of the order
///   `ORDER BY rank_key, tie_break` would return.
/// - `region`: `latin1`, beside the table's other `utf8mb4` columns, for the
///   non-utf8mb4-unmapping omission test.
/// - `item_notes`: the join partner, keyed by `item_id`. `items.id` 1 carries
///   no matching row (unmatched on the `items` side), and `item_id` 99
///   matches no `items` row (unmatched on the `item_notes` side).
fn fixture_script() -> String {
    format!(
        "\
    DROP TABLE IF EXISTS customers; \
    CREATE TABLE customers (id INT NOT NULL, name VARCHAR(50) NOT NULL, \
        score DECIMAL(9,0) NOT NULL, PRIMARY KEY (id)); \
    INSERT INTO customers VALUES (1, 'Ada', 42), (2, 'Grace', 7); \
    DROP VIEW IF EXISTS order_names; \
    CREATE VIEW order_names AS SELECT id, name FROM orders; \
    DROP TABLE IF EXISTS mixed_types; \
    CREATE TABLE mixed_types (id INT NOT NULL, label VARCHAR(20) NOT NULL, payload JSON NULL); \
    INSERT INTO mixed_types VALUES (1, 'first', '{{\"a\": 1}}'); \
    DROP TABLE IF EXISTS unmapped_only; \
    CREATE TABLE unmapped_only (payload JSON NULL, notes TEXT NULL); \
    DROP TABLE IF EXISTS refresh_target; \
    CREATE TABLE refresh_target (id INT NOT NULL, name VARCHAR(30) NOT NULL); \
    INSERT INTO refresh_target VALUES (1, 'before'); \
    DROP TABLE IF EXISTS item_notes; \
    DROP TABLE IF EXISTS items; \
    CREATE TABLE items (id INT NOT NULL, category VARCHAR(50) NOT NULL, \
        code CHAR(8) NOT NULL, batch VARCHAR(20) NOT NULL, quantity DECIMAL(9,0) NULL, \
        rank_key INT NOT NULL, tie_break INT NOT NULL, \
        region VARCHAR(30) CHARACTER SET latin1 NOT NULL, PRIMARY KEY (id)); \
    INSERT INTO items (id, category, code, batch, quantity, rank_key, tie_break, region) \
        VALUES {}; \
    CREATE TABLE item_notes (item_id INT NOT NULL, note VARCHAR(50) NOT NULL, \
        PRIMARY KEY (item_id)); \
    INSERT INTO item_notes (item_id, note) VALUES {};",
        items::source_values(),
        items::note_values()
    )
}

/// The MySQL instance under test.
pub struct MySql {
    container: String,
}

impl MySql {
    /// Binds to the MySQL of the Compose stack, waits until it answers
    /// queries, and installs the fixture databases.
    ///
    /// Panics when the stack is down, so a test can never quietly pass against
    /// no source database.
    pub fn from_stack() -> Self {
        let container = run(
            "locating the MySQL container of the Compose stack",
            "docker",
            &["compose", "ps", "--quiet", SERVICE],
        );
        let container = container.trim().to_owned();
        assert!(
            !container.is_empty(),
            "the Compose stack runs no `{SERVICE}` container; start it with `make stack-up`"
        );

        let mysql = Self { container };
        mysql.await_queries();
        mysql.install_fixtures();
        mysql
    }

    /// Runs one or more statements against `DATABASE` as the root user.
    pub fn execute(&self, sql: &str) {
        self.execute_in(DATABASE, sql);
    }

    /// Runs a query of one row and one column against `DATABASE` and returns
    /// that value as the text MySQL rendered it.
    ///
    /// A rejected query fails the run rather than returning, so a caller
    /// reading a row count can never mistake an unreachable database, or a
    /// table that is not there, for an empty one.
    pub fn query_scalar(&self, sql: &str) -> String {
        let rendered = self
            .client(&format!("querying MySQL:\n{sql}"), Some(DATABASE), sql)
            .unwrap_or_else(|failure| panic!("{failure}"));
        let value = rendered.trim();
        assert!(
            !value.contains('\n'),
            "a single-value result was expected from `{sql}`, but MySQL answered:\n{rendered}"
        );
        value.to_owned()
    }

    /// Runs one or more statements against `database` as the root user.
    pub fn execute_in(&self, database: &str, sql: &str) {
        self.client(
            &format!("executing SQL against MySQL:\n{sql}"),
            Some(database),
            sql,
        )
        .unwrap_or_else(|failure| panic!("{failure}"));
    }

    /// Installs everything the end-to-end tests expect to find, and fails
    /// loudly when the Compose seed is missing.
    fn install_fixtures(&self) {
        let seeded = self
            .client(
                "checking that the Compose seed created the orders fixture",
                Some(DATABASE),
                "SELECT COUNT(*) FROM information_schema.tables \
                 WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = 'orders'",
            )
            .unwrap_or_else(|failure| panic!("{failure}"));
        assert!(
            seeded.contains('1'),
            "database `{DATABASE}` holds no `orders` table; the Compose seed did not run, \
             which happens when the MySQL volume outlived a change to \
             docker/mysql/init.sql. Recreate the service with \
             `docker compose rm -sfv mysql && make stack-up-mysql`"
        );

        self.execute(&fixture_script());
        self.execute_in(
            "mysql",
            &format!(
                "DROP DATABASE IF EXISTS {EMPTY_DATABASE}; \
                 CREATE DATABASE {EMPTY_DATABASE}; \
                 GRANT ALL ON {EMPTY_DATABASE}.* TO '{USER}'@'%'; FLUSH PRIVILEGES;"
            ),
        );
    }

    fn await_queries(&self) {
        readiness::wait_for_service(SERVICE, PUBLISHED_ENDPOINT, SERVICE_DEADLINE);
        readiness::wait_until(
            &format!("{SERVICE} at {ADDRESS}:{PORT}"),
            SERVICE_DEADLINE,
            || {
                self.client("checking whether MySQL answers queries", None, "SELECT 1")
                    .is_ok()
            },
        );
    }

    /// Runs the MySQL client inside the container and returns its output.
    ///
    /// The password travels in the environment rather than on the command
    /// line, so it never appears in the container's process list.
    fn client(
        &self,
        description: &str,
        database: Option<&str>,
        sql: &str,
    ) -> Result<String, String> {
        let arguments = client_arguments(&self.container, database, sql);
        let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
        try_run(description, "docker", &arguments)
    }
}

/// Builds the `docker exec` argument list that runs `sql` through the MySQL
/// client inside `container`.
///
/// `--default-character-set=utf8mb4` is required, not cosmetic: without it the
/// client falls back to a charset that is not `utf8mb4`, and a non-ASCII
/// literal such as the fixture's `Gîzmo` round-trips through `docker exec` as
/// mis-decoded bytes rather than the accented character it was written as —
/// measured against this stack's own MySQL 8.4 container.
fn client_arguments(container: &str, database: Option<&str>, sql: &str) -> Vec<String> {
    let mut arguments = vec![
        "exec".to_string(),
        "--env".to_string(),
        format!("MYSQL_PWD={ROOT_PASSWORD}"),
        container.to_string(),
        "mysql".to_string(),
        "--default-character-set=utf8mb4".to_string(),
        "--user=root".to_string(),
        "--batch".to_string(),
        "--skip-column-names".to_string(),
        "--execute".to_string(),
        sql.to_string(),
    ];
    if let Some(database) = database {
        arguments.push(database.to_string());
    }
    arguments
}

/// The driver URI an Exasol `CONNECTION` object carries for `database`.
pub fn driver_uri(database: &str) -> String {
    format!("mysql://{ADDRESS}:{PORT}/{database}")
}

/// A driver URI addressing a port on the MySQL host that nothing listens on.
pub fn unreachable_uri() -> String {
    format!("mysql://{ADDRESS}:{}/{DATABASE}", PORT + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_fixture_statement_carries_a_credential() {
        let script = fixture_script();
        assert!(!script.contains(ROOT_PASSWORD) && !script.contains(PASSWORD));
    }

    #[test]
    fn driver_uri_addresses_mysql_by_its_fixed_compose_address() {
        assert_eq!(driver_uri("shop"), "mysql://172.28.5.10:3306/shop");
    }

    #[test]
    fn driver_uri_carries_no_credential() {
        let uri = driver_uri(DATABASE);
        assert!(
            !uri.contains(PASSWORD) && !uri.contains('@'),
            "user and password travel as separate ADBC options, never in the URI: {uri}"
        );
    }

    #[test]
    fn the_unreachable_uri_names_the_same_host_on_a_port_nothing_serves() {
        assert_eq!(unreachable_uri(), "mysql://172.28.5.10:3307/adbc_vs");
    }

    #[test]
    fn the_published_endpoint_and_the_in_network_address_share_a_port() {
        assert!(PUBLISHED_ENDPOINT.ends_with(&format!(":{PORT}")));
    }

    #[test]
    fn client_arguments_set_the_client_charset_to_utf8mb4() {
        let arguments = client_arguments("some-container", Some("adbc_vs"), "SELECT 1");
        assert!(
            arguments.contains(&"--default-character-set=utf8mb4".to_string()),
            "without this flag a non-ASCII fixture literal round-trips through the client as \
             mis-decoded bytes: {arguments:?}"
        );
    }

    #[test]
    fn fixtures_seed_case_and_accent_variants_in_the_grouped_column() {
        let script = fixture_script();
        for value in ["'Widget'", "'widget'", "'Gizmo'", "'Gîzmo'"] {
            assert!(
                script.contains(value),
                "items.category must hold {value} so grouping under MySQL's default collation \
                 yields fewer groups than Exasol's comparison collation: {script}"
            );
        }
    }

    #[test]
    fn fixtures_seed_each_grouped_value_onto_more_than_one_row() {
        let script = fixture_script();
        for value in ["'Widget'", "'widget'", "'Gizmo'", "'Gîzmo'"] {
            let occurrences = script.matches(value).count();
            assert!(
                occurrences >= 2,
                "items.category value {value} must seed more than one row so aggregation \
                 groups are multi-row, but it occurs {occurrences} time(s): {script}"
            );
        }
    }

    #[test]
    fn fixtures_seed_a_trailing_space_variant_beside_its_bare_counterpart() {
        let script = fixture_script();
        assert!(
            script.contains("'ab '") && script.contains("'ab'") && script.contains("'ab!'"),
            "items.batch must hold a value differing from another only by a trailing space, \
             to expose the PAD SPACE hazard fixed in decision [6]: {script}"
        );
    }

    #[test]
    fn fixtures_declare_the_code_column_char_not_varchar() {
        let script = fixture_script();
        assert!(
            script.contains("code CHAR(8) NOT NULL"),
            "items.code must be declared CHAR so the CHAR-to-VARCHAR mapping is exercised end \
             to end: {script}"
        );
    }

    #[test]
    fn fixtures_declare_a_latin1_column() {
        let script = fixture_script();
        assert!(
            script.contains("CHARACTER SET latin1"),
            "items.region must be declared latin1 so metadata discovery has a non-utf8mb4 \
             column to omit: {script}"
        );
    }

    #[test]
    fn fixtures_declare_a_nullable_quantity_column() {
        let script = fixture_script();
        assert!(
            script.contains("quantity DECIMAL(9,0) NULL"),
            "items.quantity must be nullable so a delegated select-list predicate can produce \
             a null: {script}"
        );
    }

    #[test]
    fn fixtures_seed_quantity_with_a_true_a_false_and_a_null_case() {
        let script = fixture_script();
        assert!(
            script.contains("NULL"),
            "items.quantity must carry at least one NULL row: {script}"
        );
    }

    #[test]
    fn fixtures_declare_a_repeating_sort_key_with_a_tie_breaking_second_key() {
        let script = fixture_script();
        assert!(
            script.contains("rank_key INT NOT NULL, tie_break INT NOT NULL"),
            "items must carry a rank_key column that repeats and a tie_break column that \
             disambiguates it: {script}"
        );
        let rank_key_values = [3, 3, 1, 1, 4, 4, 2, 2];
        for value in rank_key_values {
            let occurrences = script.matches(&format!(", {value}, ")).count();
            assert!(
                occurrences >= 1,
                "rank_key {value} must appear seeded: {script}"
            );
        }
    }

    #[test]
    fn fixtures_create_a_join_partner_table() {
        let script = fixture_script();
        assert!(
            script.contains("CREATE TABLE item_notes"),
            "a join partner table is required for the outer-join tests: {script}"
        );
    }

    #[test]
    fn fixtures_leave_an_unmatched_row_on_each_side_of_the_join() {
        let script = fixture_script();
        let item_notes = script
            .split("CREATE TABLE item_notes")
            .nth(1)
            .expect("item_notes must be declared");
        assert!(
            !item_notes.contains("(1, "),
            "item_notes must carry no row for items.id 1, so that row is unmatched on the \
             items side: {script}"
        );
        assert!(
            item_notes.contains("(99, "),
            "item_notes must carry a row no items row matches, so that row is unmatched on \
             the item_notes side: {script}"
        );
    }
}
