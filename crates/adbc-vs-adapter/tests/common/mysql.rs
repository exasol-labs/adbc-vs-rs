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

/// The table holding the `items` rows with every `utf8mb4` character column
/// declared [`CODE_POINT_COLLATION`].
///
/// A delegated character comparison, grouping, `MIN`, or `MAX` over it
/// returns what Exasol returns over the local mirror of `items`, so every
/// differential test over a character column reads it rather than `items`.
pub const CODE_POINT_ITEMS: &str = "items_bin";

/// The MySQL collation `vs-adapter/mysql-collation` names as the source-side
/// remedy for Exasol's code-point comparison. It weighs the `utf8mb4`
/// encoding bytes, so it orders by code point, and it is `NO PAD`, so a
/// trailing space is significant, as it is in Exasol.
const CODE_POINT_COLLATION: &str = "utf8mb4_0900_bin";

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
///   Under MySQL's default `utf8mb4_0900_ai_ci`, which `items` keeps, a
///   delegated `GROUP BY` folds the set to 2 groups; under
///   [`CODE_POINT_COLLATION`] in [`CODE_POINT_ITEMS`] it keeps the 4 Exasol
///   keeps over the mirror.
/// - `code`: a `CHAR(8)` column, so the CHAR-to-VARCHAR mapping (decision
///   `[15]`) is exercised end to end.
/// - `batch`: a `VARCHAR`, not `CHAR`, column carrying the `'ab'`/`'ab '`/
///   `'ab!'` set. Measured on MySQL 8.4, `MIN` over it returns `'ab '` under
///   the `PAD SPACE` `utf8mb4_bin`, where Exasol and the `NO PAD`
///   [`CODE_POINT_COLLATION`] return `'ab'` — a `CHAR` column would strip the
///   trailing space on retrieval and never show the difference.
/// - `quantity`: nullable, with a NULL, a value above, and a value below the
///   threshold a select-list predicate compares against.
/// - `rank_key`/`tie_break`: a sort key that repeats (`rank_key`) with a
///   second key (`tie_break`) that disambiguates it; seeded out of the order
///   `ORDER BY rank_key, tie_break` would return.
/// - `region`: `latin1`, beside the table's other `utf8mb4` columns, for the
///   non-utf8mb4-unmapping omission test. It carries no collation clause in
///   either table, because MySQL rejects [`CODE_POINT_COLLATION`] for
///   `latin1`, so it stays unmapped in [`CODE_POINT_ITEMS`] too.
/// - [`CODE_POINT_ITEMS`]: the same rows as `items`, created in the same
///   script, so it exists before the shared setup creates the virtual schema.
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
    {} \
    {} \
    CREATE TABLE item_notes (item_id INT NOT NULL, note VARCHAR(50) NOT NULL, \
        PRIMARY KEY (item_id)); \
    INSERT INTO item_notes (item_id, note) VALUES {};",
        items_table("items", None),
        items_table(CODE_POINT_ITEMS, Some(CODE_POINT_COLLATION)),
        items::note_values()
    )
}

/// The statements that recreate `table` holding every `items` row, each
/// `utf8mb4` character column declaring `collation`, or the server default
/// when it is `None`.
fn items_table(table: &str, collation: Option<&str>) -> String {
    let collate = collation
        .map(|collation| format!(" COLLATE {collation}"))
        .unwrap_or_default();
    format!(
        "DROP TABLE IF EXISTS {table}; \
         CREATE TABLE {table} (id INT NOT NULL, category VARCHAR(50){collate} NOT NULL, \
         code CHAR(8){collate} NOT NULL, batch VARCHAR(20){collate} NOT NULL, \
         quantity DECIMAL(9,0) NULL, rank_key INT NOT NULL, tie_break INT NOT NULL, \
         region VARCHAR(30) CHARACTER SET latin1 NOT NULL, PRIMARY KEY (id)); \
         INSERT INTO {table} (id, category, code, batch, quantity, rank_key, tie_break, region) \
         VALUES {};",
        items::source_values()
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
    fn container_address_invariants_hold() {
        super::super::assert_container_address_invariants(super::super::ContainerAddresses {
            driver_uri: &driver_uri(DATABASE),
            unreachable_uri: &unreachable_uri(),
            published_endpoint: PUBLISHED_ENDPOINT,
            port: PORT,
            password: PASSWORD,
        });
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
                 yields fewer groups than Exasol's code-point comparison: {script}"
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
            "items.batch must hold a value differing from another only by a trailing space, so \
             MIN and an equality over it show whether the source collation pads: {script}"
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

    /// The `CREATE TABLE` statement the fixture script issues for `table`.
    fn create_statement_of(table: &str) -> String {
        let script = fixture_script();
        let statement = script
            .split(&format!("CREATE TABLE {table} ("))
            .nth(1)
            .unwrap_or_else(|| panic!("the fixture must create {table}: {script}"));
        statement
            .split_once(';')
            .map(|(declaration, _)| declaration.to_owned())
            .unwrap_or_else(|| panic!("the CREATE TABLE {table} statement must end: {script}"))
    }

    #[test]
    fn fixtures_declare_every_utf8mb4_column_of_the_code_point_items_utf8mb4_0900_bin() {
        let statement = create_statement_of(CODE_POINT_ITEMS);
        for column in [
            "category VARCHAR(50) COLLATE utf8mb4_0900_bin NOT NULL",
            "code CHAR(8) COLLATE utf8mb4_0900_bin NOT NULL",
            "batch VARCHAR(20) COLLATE utf8mb4_0900_bin NOT NULL",
        ] {
            assert!(
                statement.contains(column),
                "{CODE_POINT_ITEMS} must declare `{column}`, the collation that reproduces \
                 Exasol's code-point comparison: {statement}"
            );
        }
        assert!(
            statement.contains("region VARCHAR(30) CHARACTER SET latin1 NOT NULL"),
            "region must stay latin1 with no collation clause, because MySQL rejects \
             utf8mb4_0900_bin for latin1: {statement}"
        );
    }

    #[test]
    fn fixtures_leave_the_items_columns_under_the_server_default_collation() {
        let statement = create_statement_of("items");
        assert!(
            !statement.contains("COLLATE"),
            "items must declare no collation, so its character columns take MySQL's default \
             utf8mb4_0900_ai_ci: {statement}"
        );
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
