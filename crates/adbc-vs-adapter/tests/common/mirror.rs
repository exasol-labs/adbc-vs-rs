//! The local Exasol tables the differential end-to-end tests compare a
//! delegated query against.
//!
//! A mirror table holds the same values as its MySQL counterpart, declares the
//! Exasol types the virtual table declares for the same columns, and names its
//! columns the same way — quoted and lower case. One query template therefore
//! runs against either schema with only the schema name substituted, so a
//! difference between the two answers is a difference in how the query was
//! evaluated and not in what it was evaluated over.
//!
//! Exasol evaluates these tables under its own rules, which is the whole
//! point. It offers no configurable collation: it compares character values by
//! exact value, so `Widget`, `widget`, `Gizmo`, and `Gîzmo` are four distinct
//! values, and it orders them by code point and pads nothing, so `MIN` over
//! `'ab '`, `'ab'`, `'ab!'` is `'ab'`. The delegated path renders no
//! collation, so it reproduces those answers only over a source column that
//! itself declares a binary collation, which is why a mirror comparison over a
//! character column runs against `items_bin`. A test over MySQL's default
//! collation asserts the source's own answer instead. The mirror is seeded
//! from the fixture's own values rather than from what MySQL returns for
//! them: a mirror filled from the source's answers would agree with the source
//! by construction and prove nothing.
//!
//! For the same reason the column types below are stated here rather than
//! computed through `adbc_vs_adapter::type_mapping`. A mirror deriving its
//! types from the code under test would inherit that code's mistakes; an
//! independently stated type set, asserted equal to the virtual table's by an
//! end-to-end test, catches a mapping change instead of absorbing it.
//!
//! `items`, `items_bin`, `item_notes`, `type_coverage`, and `measures` are mirrored, each
//! with the values Exasol reads rather than the values MySQL stores: a `BIT`
//! holds a `BOOLEAN`, a `FLOAT` its double-precision widening, a `CHAR`, `ENUM`,
//! and `SET` a `CHAR`, a `TIME` a timestamp on 1970-01-01, and a `YEAR` a date.
//! `items_bin` holds the two rows only it carries. The `latin1` `region` column
//! maps, so it is mirrored. `mixed_types`, `unmapped_only`, `text_overflow`,
//! `year_edges`, and the views are read for an omission, a failure, or MySQL's
//! own answer and carry no mirror.

use super::exasol::Exasol;
use super::items;
use super::mirror_table::{self, Column};
use super::mysql;

/// The schema holding the mirror tables. A differential test substitutes this
/// for the virtual schema's name in one query template.
pub const SCHEMA: &str = "MIRROR";

/// The mirrored `items` columns, in the order
/// [`items::mirror_values`](super::items::mirror_values) renders them.
///
/// Each type is what `vs-adapter/type-mapping` declares for the source column:
/// MySQL `INT` becomes `DECIMAL(10,0)`, a `utf8mb4` or `latin1` `VARCHAR(n)`
/// becomes `VARCHAR(n) UTF8`, and MySQL `CHAR(8)` becomes `CHAR(8) UTF8`, the
/// fixed-width type Exasol pads.
const ITEM_COLUMNS: [Column; 8] = [
    Column {
        name: "id",
        declaration: "DECIMAL(10,0) NOT NULL",
    },
    Column {
        name: "category",
        declaration: "VARCHAR(50) UTF8 NOT NULL",
    },
    Column {
        name: "code",
        declaration: "CHAR(8) UTF8 NOT NULL",
    },
    Column {
        name: "batch",
        declaration: "VARCHAR(20) UTF8 NOT NULL",
    },
    Column {
        name: "quantity",
        declaration: "DECIMAL(9,0)",
    },
    Column {
        name: "rank_key",
        declaration: "DECIMAL(10,0) NOT NULL",
    },
    Column {
        name: "tie_break",
        declaration: "DECIMAL(10,0) NOT NULL",
    },
    Column {
        name: "region",
        declaration: "VARCHAR(30) UTF8 NOT NULL",
    },
];

/// The mirrored `item_notes` columns, in the order
/// [`items::note_values`](super::items::note_values) renders them.
const NOTE_COLUMNS: [Column; 2] = [
    Column {
        name: "item_id",
        declaration: "DECIMAL(10,0) NOT NULL",
    },
    Column {
        name: "note",
        declaration: "VARCHAR(50) UTF8 NOT NULL",
    },
];

/// The mirrored `type_coverage` columns, in the order of the fixture's own
/// `CREATE TABLE`: the
/// Exasol type the virtual table declares for each, stated here rather than
/// computed from the mapping.
///
/// An unsigned `BIGINT` is `DECIMAL(20,0)`, a `FLOAT` and a `DOUBLE` are
/// `DOUBLE`, every `BIT` is `BOOLEAN`, a `CHAR`, `ENUM`, and `SET` are `CHAR`, a
/// `DECIMAL` beyond 36 digits and a `JSON` are `VARCHAR(2000000)` holding
/// MySQL's text, a `TIME` is a `TIMESTAMP` on 1970-01-01, and a `YEAR` is a
/// `DATE`.
const TYPE_COVERAGE_COLUMNS: [Column; 34] = [
    Column {
        name: "id",
        declaration: "DECIMAL(10,0) NOT NULL",
    },
    Column {
        name: "tiny_value",
        declaration: "DECIMAL(3,0)",
    },
    Column {
        name: "small_value",
        declaration: "DECIMAL(5,0)",
    },
    Column {
        name: "medium_value",
        declaration: "DECIMAL(7,0)",
    },
    Column {
        name: "int_value",
        declaration: "DECIMAL(10,0)",
    },
    Column {
        name: "big_value",
        declaration: "DECIMAL(19,0)",
    },
    Column {
        name: "utiny_value",
        declaration: "DECIMAL(3,0)",
    },
    Column {
        name: "usmall_value",
        declaration: "DECIMAL(5,0)",
    },
    Column {
        name: "umedium_value",
        declaration: "DECIMAL(8,0)",
    },
    Column {
        name: "uint_value",
        declaration: "DECIMAL(10,0)",
    },
    Column {
        name: "ubig_value",
        declaration: "DECIMAL(20,0)",
    },
    Column {
        name: "flag_value",
        declaration: "DECIMAL(3,0)",
    },
    Column {
        name: "approx",
        declaration: "DOUBLE",
    },
    Column {
        name: "wide_amount",
        declaration: "VARCHAR(2000000) UTF8",
    },
    Column {
        name: "bit_flag",
        declaration: "BOOLEAN",
    },
    Column {
        name: "bits",
        declaration: "BOOLEAN",
    },
    Column {
        name: "wide_bits",
        declaration: "BOOLEAN",
    },
    Column {
        name: "fixed",
        declaration: "CHAR(10) UTF8",
    },
    Column {
        name: "tiny_text",
        declaration: "VARCHAR(255) UTF8",
    },
    Column {
        name: "plain_text",
        declaration: "VARCHAR(65535) UTF8",
    },
    Column {
        name: "medium_text",
        declaration: "VARCHAR(2000000) UTF8",
    },
    Column {
        name: "long_text",
        declaration: "VARCHAR(2000000) UTF8",
    },
    Column {
        name: "latin_varchar",
        declaration: "VARCHAR(20) UTF8",
    },
    Column {
        name: "latin_char",
        declaration: "CHAR(10) UTF8",
    },
    Column {
        name: "latin_text",
        declaration: "VARCHAR(65535) UTF8",
    },
    Column {
        name: "kind",
        declaration: "CHAR(5) UTF8",
    },
    Column {
        name: "tags",
        declaration: "CHAR(4) UTF8",
    },
    Column {
        name: "doc",
        declaration: "VARCHAR(2000000) UTF8",
    },
    Column {
        name: "elapsed",
        declaration: "TIMESTAMP(3)",
    },
    Column {
        name: "y",
        declaration: "DATE",
    },
    Column {
        name: "at_second",
        declaration: "TIMESTAMP(0)",
    },
    Column {
        name: "at_micro",
        declaration: "TIMESTAMP(6)",
    },
    Column {
        name: "happened_at",
        declaration: "TIMESTAMP(6)",
    },
    Column {
        name: "local_at",
        declaration: "TIMESTAMP(6)",
    },
];

/// The `type_coverage` rows as Exasol reads them, copied by hand from
/// `TYPE_COVERAGE_SCRIPT` in [`super::mysql`]: the `FLOAT` `0.1` is the
/// double-precision value of its single-precision form, every `BIT` column is
/// whether any bit is set, the `TIME` values sit on 1970-01-01 and a `25:00:00`
/// on 1970-01-02, and the `YEAR` is the first of its year.
const TYPE_COVERAGE_VALUES: &str = "\
    (1, 127, 32767, 8388607, 2147483647, 9223372036854775807, \
     255, 65535, 16777215, 4294967295, 18446744073709551615, 5, \
     0.100000001490116119384765625, '12345678901234567890123456789012345678.90', \
     TRUE, TRUE, TRUE, \
     'ab', 'ab', 'plain', 'medium', 'long', 'café', 'café', 'café', \
     'ab', 'x', '{\"a\": 1}', \
     '1970-01-01 16:59:59.123', '2024-01-01', '2024-01-01 12:00:00', \
     '2024-01-01 12:00:00.123456', '2024-01-01 12:00:00.123456', '2024-01-01 12:00:00.123456'), \
    (2, -128, -32768, -8388608, -2147483648, -9223372036854775808, \
     0, 0, 0, 0, 0, 0, \
     1.5, '-0.05', \
     FALSE, TRUE, TRUE, \
     'abcdefghij', 'a', 'b', 'c', 'd', 'tea', 'tea', 'tea', \
     'abcde', 'yz', '{\"a\": 2}', \
     '1970-01-02 01:00:00.000', '2023-01-01', '2024-06-15 08:30:00', \
     '2024-06-15 08:30:00.500000', '2024-06-15 08:30:00.500000', '2024-06-15 09:30:00.500000'), \
    (3, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, \
     NULL, NULL, NULL, FALSE, NULL, \
     NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, \
     NULL, NULL, NULL, \
     NULL, NULL, NULL, NULL, NULL, NULL)";

/// The mirrored `measures` columns, in the order of the fixture's own
/// `CREATE TABLE`: a `BIGINT` is `DECIMAL(19,0)`, a `FLOAT` and a `DOUBLE` are
/// `DOUBLE`, a `BIT(1)` is `BOOLEAN`, and a `VARCHAR` is `VARCHAR(n) UTF8`.
const MEASURES_COLUMNS: [Column; 8] = [
    Column {
        name: "id",
        declaration: "DECIMAL(10,0) NOT NULL",
    },
    Column {
        name: "grp",
        declaration: "VARCHAR(10) UTF8 NOT NULL",
    },
    Column {
        name: "big_value",
        declaration: "DECIMAL(19,0)",
    },
    Column {
        name: "decimal_value",
        declaration: "DECIMAL(30,2)",
    },
    Column {
        name: "double_value",
        declaration: "DOUBLE",
    },
    Column {
        name: "float_value",
        declaration: "DOUBLE",
    },
    Column {
        name: "flag",
        declaration: "BOOLEAN",
    },
    Column {
        name: "label",
        declaration: "VARCHAR(20) UTF8",
    },
];

/// The `measures` rows as Exasol reads them, copied by hand from
/// `MEASURES_SCRIPT` in [`super::mysql`], every `BIT(1)` as a `BOOLEAN`.
const MEASURES_VALUES: &str = "\
    (1, 'a', 10, 1.25, 0.5, 0.25, TRUE, '5'), \
    (2, 'a', 20, 2.50, 1.5, 0.5, FALSE, '5.0'), \
    (3, 'a', 20, 3.75, 1.5, 0.5, TRUE, ''), \
    (4, 'b', 1, -0.25, 2.25, 1.0, TRUE, '7'), \
    (5, 'b', 9223372036854775807, 0.25, 4.25, 2.0, NULL, '7.5'), \
    (6, 'b', NULL, NULL, NULL, NULL, FALSE, NULL), \
    (7, 'c', NULL, NULL, NULL, NULL, NULL, NULL)";

/// Creates the mirror schema and its tables, seeded from [`items`](super::items)
/// and the literals above.
///
/// Replaces whatever an earlier run left behind rather than adding to it: the
/// Compose stack outlives a test run, so a mirror surviving from an older
/// fixture would make every differential comparison report a pushdown defect
/// that is really a stale table.
pub fn install(exasol: &Exasol) {
    exasol.execute(&format!("CREATE SCHEMA IF NOT EXISTS {SCHEMA}"));
    mirror_table::seed(
        exasol,
        SCHEMA,
        "items",
        &ITEM_COLUMNS,
        &items::mirror_values(),
    );
    mirror_table::seed(
        exasol,
        SCHEMA,
        mysql::CODE_POINT_ITEMS,
        &ITEM_COLUMNS,
        &items::code_point_mirror_values(),
    );
    mirror_table::seed(
        exasol,
        SCHEMA,
        "item_notes",
        &NOTE_COLUMNS,
        &items::note_values(),
    );
    mirror_table::seed(
        exasol,
        SCHEMA,
        "type_coverage",
        &TYPE_COVERAGE_COLUMNS,
        TYPE_COVERAGE_VALUES,
    );
    mirror_table::seed(
        exasol,
        SCHEMA,
        "measures",
        &MEASURES_COLUMNS,
        MEASURES_VALUES,
    );
}

fn create_statement(table: &str, columns: &[Column]) -> String {
    mirror_table::create_statement(SCHEMA, table, columns)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item_declarations() -> String {
        create_statement("items", &ITEM_COLUMNS)
    }

    #[test]
    fn the_mirrored_items_declare_the_types_the_virtual_table_declares() {
        let declarations = item_declarations();

        assert!(
            declarations.contains("\"code\" CHAR(8) UTF8"),
            "MySQL CHAR(8) maps to Exasol CHAR(8): {declarations}"
        );
        assert!(
            declarations.contains("\"id\" DECIMAL(10,0)")
                && declarations.contains("\"rank_key\" DECIMAL(10,0)")
                && declarations.contains("\"tie_break\" DECIMAL(10,0)"),
            "MySQL INT maps to Exasol DECIMAL(10,0): {declarations}"
        );
        assert!(
            declarations.contains("\"category\" VARCHAR(50) UTF8")
                && declarations.contains("\"batch\" VARCHAR(20) UTF8"),
            "a utf8mb4 VARCHAR(n) maps to Exasol VARCHAR(n) UTF8: {declarations}"
        );
        assert!(
            declarations.contains("\"quantity\" DECIMAL(9,0),"),
            "the nullable numeric column must stay nullable, so a projected predicate over it \
             can produce a null: {declarations}"
        );
        assert!(
            declarations.contains("\"region\" VARCHAR(30) UTF8"),
            "a latin1 VARCHAR maps to Exasol VARCHAR UTF8, so the mirror carries it: \
             {declarations}"
        );
    }

    #[test]
    fn every_mirrored_row_carries_one_value_per_declared_column() {
        for (table, columns, values) in [
            ("items", ITEM_COLUMNS.len(), items::mirror_values()),
            (
                "items_bin",
                ITEM_COLUMNS.len(),
                items::code_point_mirror_values(),
            ),
            (
                "type_coverage",
                TYPE_COVERAGE_COLUMNS.len(),
                TYPE_COVERAGE_VALUES.to_string(),
            ),
            (
                "measures",
                MEASURES_COLUMNS.len(),
                MEASURES_VALUES.to_string(),
            ),
        ] {
            let counts = mirror_table::values_per_tuple(&values);

            assert!(!counts.is_empty(), "`{table}` must hold rows");
            assert!(
                counts.iter().all(|count| *count == columns),
                "every `{table}` row must carry {columns} values, one per declared column, or \
                 Exasol assigns the values to the wrong columns: {counts:?}"
            );
        }
    }
}
