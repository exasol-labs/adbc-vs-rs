//! The `scalars` table the scalar-function tests read at every source, and
//! the scenario bodies those tests share.
//!
//! One set of rows serves MySQL, PostgreSQL, Trino, and each Exasol mirror,
//! written with typed date and timestamp literals every engine accepts. Trino
//! maps no `CHAR` and no `BOOLEAN` column, so its table leaves `ch` and `flag`
//! out. PostgreSQL and Trino seed the table from their `init.sql`, which holds
//! the same values; a unit test below checks it does.

use serde_json::{Map, Value};

use super::exasol::{cell, Exasol};
use super::mirror_table::{column, Column};
use adbc_vs_adapter::pushdown::SCAN_SCRIPT_NAME;
use adbc_vs_adapter::scan_spec;

pub const TABLE: &str = "scalars";

/// Every column, as the MySQL and PostgreSQL virtual tables declare it.
pub const COLUMNS: [Column; 14] = [
    column("id", "DECIMAL(10,0)"),
    column("i", "DECIMAL(19,0)"),
    column("d", "DECIMAL(18,4)"),
    column("f", "DOUBLE"),
    column("num", "VARCHAR(20) UTF8"),
    column("s", "VARCHAR(20) UTF8"),
    column("dt", "DATE"),
    column("ts", "TIMESTAMP(3)"),
    column("ts2", "TIMESTAMP(3)"),
    column("ch", "CHAR(5) UTF8"),
    column("dtext", "VARCHAR(30) UTF8"),
    column("flag", "BOOLEAN"),
    column("us1", "TIMESTAMP(6)"),
    column("us2", "TIMESTAMP(6)"),
];

/// The positions of `ch`, `flag`, `us1`, and `us2`, which Trino leaves out:
/// it maps no `CHAR` and no `BOOLEAN`, and declares every timestamp
/// `TIMESTAMP(3)`.
const NOT_AT_TRINO: [usize; 4] = [9, 11, 12, 13];

/// The rows, one value per column of [`COLUMNS`], separated by `, `, which
/// no value contains. `i` reaches the `BIGINT`
/// maximum, `d` carries fractional days, `num` holds numbers as text, `s`
/// holds blanks, multi-byte characters, and a value only blanks, `ts` and
/// `ts2` lie a millisecond, days, and more than a year apart, and `us1` and
/// `us2` lie a millisecond and a half apart in both directions.
const ROWS: [&str; 7] = [
    "1, 9223372036854775807, 12.5, 1.5, '1.5', '  ab  ', DATE '2024-02-29', TIMESTAMP '2024-02-28 23:59:59.500', TIMESTAMP '2024-01-01 00:00:00.000', 'ab', '2024-01-05', TRUE, TIMESTAMP '2024-01-01 00:00:00.001500', TIMESTAMP '2024-01-01 00:00:00.000000'",
    "2, -5, -0.25, 0.25, '-2', 'abcdef', DATE '2023-12-31', TIMESTAMP '2024-01-01 00:00:00.000', TIMESTAMP '2023-12-31 23:59:59.999', 'abcde', '2023-12-31', FALSE, TIMESTAMP '2024-01-01 00:00:00.000000', TIMESTAMP '2024-01-01 00:00:00.001500'",
    "3, 0, 1000, -3.5, '10', 'xxaxx', DATE '2024-01-01', TIMESTAMP '2025-03-01 12:00:00.250', TIMESTAMP '2024-02-29 06:00:00.000', 'a', '2024-02-29 10:11:12', TRUE, TIMESTAMP '2024-01-02 00:00:00.123456', TIMESTAMP '2024-01-01 00:00:00.000001'",
    "4, 7, 0.0001, 2.0, '0.5', '   ', DATE '2024-03-01', TIMESTAMP '2024-03-01 00:00:00.001', TIMESTAMP '2024-03-01 00:00:00.000', 'xy', '2024-03-01', FALSE, TIMESTAMP '2024-01-01 00:00:00.000999', TIMESTAMP '2024-01-01 00:00:00.000000'",
    "5, 2, 3.75, 0.5, '3', 'é', DATE '2020-06-15', TIMESTAMP '2020-06-15 08:30:00.000', TIMESTAMP '2020-06-14 20:30:00.000', 'é', '2020-06-15', TRUE, TIMESTAMP '2020-06-15 08:30:00.250000', TIMESTAMP '2020-06-15 08:30:00.250000'",
    "6, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL",
    "7, 1, 1, 1.0, '1', 'ß', DATE '2021-01-01', TIMESTAMP '2021-01-01 00:00:00.000', TIMESTAMP '2021-01-01 00:00:00.000', 'ß', '2021-01-01', TRUE, TIMESTAMP '2021-01-01 00:00:00.000000', TIMESTAMP '2021-01-01 00:00:00.000000'",
];

/// The rows as a `VALUES` list over every column.
pub fn values() -> String {
    render(|_| true)
}

/// The columns Trino's table declares.
pub fn trino_columns() -> Vec<Column> {
    COLUMNS
        .into_iter()
        .enumerate()
        .filter(|(index, _)| !NOT_AT_TRINO.contains(index))
        .map(|(_, column)| column)
        .collect()
}

/// The rows as a `VALUES` list over the columns Trino's table declares.
pub fn trino_values() -> String {
    render(|index| !NOT_AT_TRINO.contains(&index))
}

fn render(keep: impl Fn(usize) -> bool) -> String {
    ROWS.iter()
        .map(|row| {
            let values: Vec<&str> = row
                .split(", ")
                .enumerate()
                .filter(|(index, _)| keep(*index))
                .map(|(_, value)| value)
                .collect();
            format!("({})", values.join(", "))
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// The MySQL statements that create and seed the table.
pub fn mysql_script() -> String {
    format!(
        "DROP TABLE IF EXISTS {TABLE}; \
         CREATE TABLE {TABLE} (id INT NOT NULL, i BIGINT NULL, d DECIMAL(18,4) NULL, \
             f DOUBLE NULL, num VARCHAR(20) NULL, s VARCHAR(20) NULL, dt DATE NULL, \
             ts DATETIME(3) NULL, ts2 DATETIME(3) NULL, ch CHAR(5) NULL, \
             dtext VARCHAR(30) NULL, flag BIT(1) NULL, us1 DATETIME(6) NULL, \
             us2 DATETIME(6) NULL, PRIMARY KEY (id)); \
         INSERT INTO {TABLE} VALUES {};",
        values()
    )
}

/// The source a [`Scenario`] reads, which decides the fragments its generated
/// statements carry and which columns its table declares.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Source {
    MySql,
    Postgres,
    Trino,
}

impl Source {
    /// Whether the source maps the `ch` and `flag` columns.
    fn maps_char_and_boolean(self) -> bool {
        self != Source::Trino
    }

    /// Whether the dialect advertises `FN_TRIM`.
    fn delegates_trim(self) -> bool {
        self != Source::MySql
    }

    fn pick(self, fragments: [&'static str; 3]) -> &'static str {
        fragments[self as usize]
    }
}

/// One virtual schema over a source's `scalars`, beside the mirror holding
/// the same rows.
pub struct Scenario<'a> {
    pub exasol: &'a Exasol,
    pub source: Source,
    pub virtual_schema: &'a str,
    pub mirror_schema: &'a str,
}

pub type Rows = Vec<Map<String, Value>>;

impl Scenario<'_> {
    /// Runs `select` (a select list), with `rest` (a `WHERE` or `GROUP BY`
    /// clause, or nothing) over the virtual table and the mirror, ordered by
    /// `id` unless `rest` groups, and asserts both answers are identical.
    pub fn assert_matches_mirror(&self, select: &str, rest: &str) -> Rows {
        let query = |schema: &str| {
            let order = if rest.contains("GROUP BY") {
                "ORDER BY 1, 2"
            } else {
                "ORDER BY \"id\""
            };
            let select = if rest.contains("GROUP BY") {
                select.to_string()
            } else {
                format!("\"id\", {select}")
            };
            format!("SELECT {select} FROM {schema}.\"{TABLE}\" {rest} {order}")
        };
        let virtual_rows = self.exasol.query_rows(&query(self.virtual_schema));
        let mirror_rows = self.exasol.query_rows(&query(self.mirror_schema));
        assert_eq!(
            virtual_rows,
            mirror_rows,
            "`{}` must return exactly the mirror's rows: virtual {virtual_rows:?}, mirror \
             {mirror_rows:?}",
            query(self.virtual_schema)
        );
        virtual_rows
    }

    /// The statement the adapter generated for `select` and `rest` over the
    /// virtual table, which must be exactly one.
    pub fn generated_statement(&self, select: &str, rest: &str) -> String {
        let query = format!(
            "SELECT {select} FROM {}.\"{TABLE}\" {rest}",
            self.virtual_schema
        );
        let calls: Vec<String> = self
            .exasol
            .query_rows(&format!(
                "SELECT PUSHDOWN_SQL FROM (EXPLAIN VIRTUAL {query})"
            ))
            .iter()
            .filter_map(|row| cell(row, "PUSHDOWN_SQL"))
            .collect();
        assert_eq!(calls.len(), 1, "`{query}` must be one statement: {calls:?}");
        scan_specification(&calls[0]).sql
    }

    /// The one pushdown response `EXPLAIN VIRTUAL` reports for the query.
    pub fn pushdown_call(&self, select: &str) -> String {
        let query = format!("SELECT {select} FROM {}.\"{TABLE}\"", self.virtual_schema);
        self.exasol
            .query_scalar(&format!(
                "SELECT PUSHDOWN_SQL FROM (EXPLAIN VIRTUAL {query})"
            ))
            .unwrap_or_else(|| panic!("`{query}` must be delegated"))
    }

    /// Asserts each `(select, rest)` reaches the source carrying the
    /// source's fragment in its generated statement, and matches the mirror.
    pub fn assert_delegated(&self, cases: &[(&str, &str, [&'static str; 3])]) {
        for (select, rest, fragments) in cases {
            let fragment = self.source.pick(*fragments);
            let statement = self.generated_statement(select, rest);
            assert!(
                statement.contains(fragment),
                "`{select} {rest}` must reach the source carrying `{fragment}`: {statement}"
            );
            self.assert_matches_mirror(select, rest);
        }
    }

    /// Runs one whole query, `query(schema)`, over the virtual schema and the
    /// mirror, and asserts both answers are identical.
    pub fn assert_query_matches_mirror(&self, query: impl Fn(&str) -> String) -> Rows {
        let virtual_rows = self.exasol.query_rows(&query(self.virtual_schema));
        let mirror_rows = self.exasol.query_rows(&query(self.mirror_schema));
        assert_eq!(
            virtual_rows,
            mirror_rows,
            "`{}` must return exactly the mirror's rows",
            query(self.virtual_schema)
        );
        virtual_rows
    }

    /// `pushdown-scalar-function`: `YEAR`, `MONTH`, and `DAY` over a `DATE`
    /// and a `TIMESTAMP` column, projected and filtered, each declared a
    /// `DECIMAL`.
    pub fn date_parts(&self) {
        let year = ["YEAR(`dt`)", "EXTRACT(YEAR FROM \"dt\")", "YEAR(\"dt\")"];
        let day = ["DAY(`ts`)", "EXTRACT(DAY FROM \"ts\")", "DAY(\"ts\")"];
        self.assert_delegated(&[
            (
                "YEAR(\"dt\") AS Y, MONTH(\"dt\") AS M, DAY(\"dt\") AS D",
                "",
                year,
            ),
            (
                "YEAR(\"ts\") AS Y, MONTH(\"ts\") AS M, DAY(\"ts\") AS D",
                "",
                day,
            ),
            ("\"dt\"", "WHERE YEAR(\"dt\") = 2024", year),
            ("\"ts\"", "WHERE DAY(\"ts\") = 1", day),
        ]);
        let call = self.pushdown_call("YEAR(\"dt\"), DAY(\"ts\")");
        assert!(
            call.contains("\"c1\" DECIMAL(") && call.contains("\"c2\" DECIMAL("),
            "each date part must be declared a DECIMAL: {call}"
        );
    }

    /// `pushdown-scalar-function`: exact arithmetic past the `BIGINT` range is
    /// returned, a null operand yields null, and a result beyond the declared
    /// type fails as it does over the mirror.
    pub fn exact_arithmetic(&self) {
        let add = ["+ CAST(1 AS", "+ CAST(1 AS", "+ CAST(1 AS"];
        let multiply = ["* CAST(2 AS", "* CAST(2 AS", "* CAST(2 AS"];
        self.assert_delegated(&[
            ("\"i\" + 1 AS A", "", add),
            (
                "\"i\" - -1 AS B",
                "",
                ["- CAST(-1 AS", "- CAST(-1 AS", "- CAST(-1 AS"],
            ),
            ("\"i\" * 2 AS C", "", multiply),
            (
                "\"d\" * \"d\" AS E, \"d\" - \"d\" AS F",
                "",
                ["* CAST(", "* CAST(", "* CAST("],
            ),
            ("\"i\"", "WHERE \"i\" * 2 > 10", multiply),
            (
                "\"d\"",
                "WHERE \"d\" * \"d\" > \"d\" - \"d\"",
                ["* CAST(", "* CAST(", "* CAST("],
            ),
        ]);
        let overflow = self.generated_statement("\"i\" * \"i\" * \"i\"", "");
        assert!(
            overflow.contains("* CAST("),
            "the overflowing product must be delegated: {overflow}"
        );
        for schema in [self.virtual_schema, self.mirror_schema] {
            self.exasol.execute_expecting_failure(&format!(
                "SELECT \"i\" * \"i\" * \"i\" FROM {schema}.\"{TABLE}\""
            ));
        }
    }

    /// `pushdown-scalar-function`: `DOUBLE` and mixed arithmetic in double
    /// precision, and text read as a number.
    pub fn double_and_character_arithmetic(&self) {
        let double = ["AS DOUBLE)", "AS DOUBLE PRECISION)", "AS DOUBLE)"];
        self.assert_delegated(&[
            (
                "\"f\" + \"d\" AS A, \"f\" - \"d\" AS B, \"f\" * \"d\" AS C",
                "",
                double,
            ),
            (
                "\"num\" + 1 AS A, \"num\" - 1 AS B, \"num\" * 2 AS C",
                "",
                double,
            ),
            ("\"f\"", "WHERE \"num\" * 2 > 1", double),
        ]);
    }

    /// `pushdown-scalar-function`: a number shifts a date or timestamp by days
    /// rounded half away from zero, and a date minus a date counts days.
    pub fn date_arithmetic(&self) {
        let shift = ["DATE_ADD(`dt`", "(\"dt\" + CAST(", "date_add('day', "];
        self.assert_delegated(&[
            (
                "\"dt\" + 1 AS A, 1 + \"dt\" AS B, \"dt\" + 1.5 AS C",
                "",
                shift,
            ),
            (
                "\"dt\" - 1 AS A",
                "",
                ["DATE_SUB(`dt`", "(\"dt\" - ", "date_add('day', -"],
            ),
            (
                "\"dt\" - DATE '2024-01-01' AS A",
                "",
                ["DATEDIFF(", "(\"dt\" - DATE", "date_diff('day', "],
            ),
            (
                "\"ts\" + 1.5 AS A, \"ts\" - \"d\" AS B",
                "",
                ["DATE_ADD(`ts`", "INTERVAL '1 day'", "date_add('day', "],
            ),
            ("\"dt\"", "WHERE \"dt\" + 1 = DATE '2024-03-01'", shift),
        ]);
        if self.source == Source::MySql {
            let statement = self.generated_statement("\"dt\" + 1", "");
            assert!(
                !statement.contains("`dt` +"),
                "MySQL's `+` reads a date as a number: {statement}"
            );
        }
    }

    /// `pushdown-scalar-function`: a `TIMESTAMP` difference in the select list
    /// is emitted as seconds and converted back to an interval in Exasol.
    pub fn timestamp_difference(&self) {
        let difference = [
            "TIMESTAMPDIFF(MICROSECOND",
            "date_trunc('milliseconds', \"ts\" - \"ts2\")",
            "(\"ts\" - \"ts2\")",
        ];
        self.assert_delegated(&[
            (
                "\"ts\" - \"ts2\" AS A, \"ts\" - \"dt\" AS B",
                "",
                difference,
            ),
            ("\"ts\" + (\"ts\" - \"ts2\") AS A", "", difference),
            (
                "\"id\" AS K",
                "WHERE \"ts\" - \"ts2\" < \"ts\" - \"dt\"",
                difference,
            ),
            (
                "(\"ts\" - \"ts2\") * 1.5 AS S, (\"ts\" - \"ts2\") + (\"ts\" - \"dt\") AS T",
                "",
                difference,
            ),
        ]);
        if self.source != Source::Trino {
            self.assert_delegated(&[(
                "\"us1\" - \"us2\" AS U",
                "",
                ["TIMESTAMPDIFF(MICROSECOND", "date_trunc('milliseconds'", ""],
            )]);
        }
        self.assert_query_matches_mirror(|schema| {
            format!("SELECT MIN(\"ts\" - \"ts2\") AS M FROM {schema}.\"{TABLE}\"")
        });
        let call = self.pushdown_call("\"ts\" - \"ts2\"");
        assert!(
            call.contains("NUMTODSINTERVAL(\"c1\", 'SECOND')"),
            "the interval must be converted in an outer SELECT: {call}"
        );
    }

    /// `pushdown-scalar-function`: the string functions over `VARCHAR` text.
    pub fn string_functions(&self) {
        let length = ["CHAR_LENGTH(", "CHAR_LENGTH(", "LENGTH("];
        self.assert_delegated(&[
            (
                "UPPER(\"s\") AS U, LOWER(\"s\") AS L",
                "",
                ["UPPER(", "UPPER(", "UPPER("],
            ),
            ("LENGTH(\"s\") AS N", "", length),
            (
                "SUBSTR(\"s\", 2) AS A, SUBSTR(\"s\", 2, 3) AS B",
                "",
                ["SUBSTR(", "SUBSTR(", "SUBSTR("],
            ),
            (
                "\"s\"",
                "WHERE UPPER(\"s\") = 'ABCDEF'",
                ["UPPER(", "UPPER(", "UPPER("],
            ),
            ("\"s\"", "WHERE LENGTH(\"s\") = 1", length),
        ]);
        let trim = ["", "TRIM(", "TRIM("];
        if self.source.delegates_trim() {
            self.assert_delegated(&[
                ("TRIM(\"s\") AS A, TRIM(\"s\", 'x') AS B", "", trim),
                ("\"s\"", "WHERE TRIM(\"s\") = 'ab'", trim),
            ]);
        } else {
            let statement = self.generated_statement("TRIM(\"s\") AS A", "");
            assert!(
                !statement.contains("TRIM"),
                "MySQL withholds FN_TRIM: {statement}"
            );
            self.assert_matches_mirror("TRIM(\"s\") AS A, TRIM(\"s\", 'x') AS B", "");
        }
    }

    /// `pushdown-scalar-function`: `SUBSTR` positions follow Exasol's rule.
    pub fn substr_positions(&self) {
        let substr = ["SUBSTR(", "SUBSTR(", "SUBSTR("];
        self.assert_delegated(&[(
            "SUBSTR(\"s\", 0) AS A, SUBSTR(\"s\", 1) AS B, SUBSTR(\"s\", -2) AS C, \
             SUBSTR(\"s\", 10) AS D, SUBSTR(\"s\", 2, 100) AS E, SUBSTR(\"s\", -10, 3) AS F, \
             SUBSTR(\"s\", 0, 2) AS G, SUBSTR(\"s\", \"d\") AS H",
            "",
            substr,
        )]);
    }

    /// `pushdown-scalar-function`: an empty result is null, so the blank row
    /// and the null row group together and match `IS NULL`.
    pub fn empty_string_results(&self) {
        let substr = ["SUBSTR(", "SUBSTR(", "SUBSTR("];
        self.assert_delegated(&[("\"id\" AS K", "WHERE SUBSTR(\"s\", 10) IS NULL", substr)]);
        let mut keys = vec!["SUBSTR(\"s\", 4)", "UPPER(SUBSTR(\"s\", 4))"];
        if self.source.delegates_trim() {
            keys.push("TRIM(\"s\")");
        }
        for key in keys {
            let grouped = format!("{key} AS K, COUNT(*) AS N");
            let statement = self.generated_statement(&grouped, &format!("GROUP BY {key}"));
            assert!(statement.contains("GROUP BY"), "{statement}");
            self.assert_matches_mirror(&grouped, &format!("GROUP BY {key}"));
            self.assert_query_matches_mirror(|schema| {
                format!("SELECT COUNT(DISTINCT {key}) AS N FROM {schema}.\"{TABLE}\"")
            });
            self.assert_matches_mirror("\"s\"", &format!("WHERE {key} IS NULL"));
        }
    }

    /// `pushdown-scalar-function`: `LENGTH` over a `CHAR` value counts the
    /// characters the source keeps, which differs from Exasol's padded count.
    pub fn string_function_results_follow_the_source(&self, char_length: &str) {
        assert!(self.source.maps_char_and_boolean());
        let query = |schema: &str| {
            format!("SELECT LENGTH(\"ch\") AS N FROM {schema}.\"{TABLE}\" WHERE \"id\" = 1")
        };
        let virtual_length = self.exasol.query_scalar(&query(self.virtual_schema));
        let mirror_length = self.exasol.query_scalar(&query(self.mirror_schema));
        assert_eq!(
            virtual_length.as_deref(),
            Some(char_length),
            "the source's own count"
        );
        assert_eq!(
            mirror_length.as_deref(),
            Some("5"),
            "Exasol counts the padding"
        );
    }

    /// `pushdown-scalar-function`: a number, a date, text, and a boolean
    /// convert as Exasol converts them under its default formats.
    pub fn implicit_conversions(&self) {
        let length = ["CHAR_LENGTH(", "CHAR_LENGTH(", "LENGTH("];
        self.assert_delegated(&[
            (
                "LENGTH(\"d\") AS A, SUBSTR(\"d\", 1) AS B",
                "",
                length.map(|_| "SUBSTR("),
            ),
            ("LENGTH(\"dt\") AS A, SUBSTR(\"dt\", 6) AS B", "", length),
            (
                "YEAR(\"dtext\") AS A",
                "",
                [
                    "YEAR(CAST(`dtext`",
                    "EXTRACT(YEAR FROM CAST(\"dtext\"",
                    "YEAR(CAST(\"dtext\"",
                ],
            ),
        ]);
        if self.source.maps_char_and_boolean() {
            self.assert_delegated(&[("\"flag\" + 1 AS A", "", ["CASE WHEN", "CASE WHEN", ""])]);
        }
        if self.source == Source::Trino {
            let statement = self.generated_statement("LENGTH(\"f\")", "");
            assert!(statement.contains("LENGTH("), "{statement}");
            let length = self.exasol.query_scalar(&format!(
                "SELECT LENGTH(\"f\") FROM {}.\"{TABLE}\" WHERE \"id\" = 1",
                self.virtual_schema
            ));
            assert_eq!(
                length.as_deref(),
                Some("5"),
                "Trino writes the double 1.5 as `1.5E0`, which docs/pushdown.md states"
            );
        }
    }

    /// `pushdown-scalar-function`: `UPPER` over `ß` returns the source's own
    /// case mapping, `expected`, measured once per source.
    pub fn upper_follows_the_source(&self, expected: &str) {
        let statement = self.generated_statement("UPPER(\"s\")", "WHERE \"id\" = 7");
        assert!(statement.contains("UPPER("), "{statement}");
        let upper = self.exasol.query_scalar(&format!(
            "SELECT UPPER(\"s\") FROM {}.\"{TABLE}\" WHERE \"id\" = 7",
            self.virtual_schema
        ));
        assert_eq!(upper.as_deref(), Some(expected));
    }

    /// `pushdown-aggregation-delegation`: grouping by `YEAR` is delegated, an
    /// expression over the key renders as MySQL accepts it, and two keys reach
    /// only the dialect advertising `AGGREGATE_GROUP_BY_TUPLE`.
    pub fn group_by_year(&self) {
        let year = [
            "GROUP BY YEAR(`dt`)",
            "GROUP BY EXTRACT(YEAR FROM \"dt\")",
            "GROUP BY YEAR(\"dt\")",
        ];
        self.assert_delegated(&[
            (
                "YEAR(\"dt\") AS Y, COUNT(*) AS N",
                "GROUP BY YEAR(\"dt\")",
                year,
            ),
            (
                "YEAR(\"dt\") + 1 AS Y, COUNT(*) AS N",
                "GROUP BY YEAR(\"dt\")",
                year,
            ),
        ]);
        let two_keys = "YEAR(\"dt\") AS Y, MONTH(\"dt\") AS M, COUNT(*) AS N";
        let rest = "GROUP BY YEAR(\"dt\"), MONTH(\"dt\")";
        let statement = self.generated_statement(two_keys, rest);
        assert_eq!(
            statement.contains("GROUP BY"),
            self.source == Source::Trino,
            "only Trino advertises AGGREGATE_GROUP_BY_TUPLE: {statement}"
        );
        self.assert_matches_mirror(two_keys, rest);
    }
}

/// The scan specification embedded in one `pushdown` response, which carries
/// it as an Exasol string literal, every single quote doubled.
fn scan_specification(call: &str) -> scan_spec::ScanSpec {
    let prefix = format!("{SCAN_SCRIPT_NAME}('");
    let start = call
        .find(&prefix)
        .unwrap_or_else(|| panic!("a pushdown response calls the scan script: {call}"))
        + prefix.len();
    let end = call
        .rfind("') EMITS (")
        .unwrap_or_else(|| panic!("a pushdown response declares an EMITS clause: {call}"));
    scan_spec::parse(&call[start..end].replace("''", "'"))
        .unwrap_or_else(|error| panic!("parsing the embedded scan specification: {error}\n{call}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::mirror_table;

    fn init_sql(source: &str) -> String {
        std::fs::read_to_string(
            crate::common::exasol::workspace_root().join(format!("docker/{source}/init.sql")),
        )
        .expect("reading init.sql")
    }

    #[test]
    fn every_row_carries_one_value_per_declared_column() {
        mirror_table::assert_one_value_per_column(&[
            ("scalars", COLUMNS.len(), values()),
            ("scalars", trino_columns().len(), trino_values()),
        ]);
    }

    #[test]
    fn postgresql_and_trino_seed_the_same_rows() {
        assert!(
            init_sql("postgres").contains(&values()),
            "docker/postgres/init.sql"
        );
        assert!(
            init_sql("trino").contains(&trino_values()),
            "docker/trino/init.sql"
        );
    }
}
