//! The local Exasol tables the PostgreSQL differential end-to-end tests
//! compare a delegated query against.
//!
//! Structural twin of [`super::mirror`], the MySQL mirror: same reasoning,
//! same shape, a different schema so the two never collide in one Exasol
//! instance. A mirror table holds the same values as its PostgreSQL
//! counterpart in `docker/postgres/init.sql`, declares the Exasol types the
//! virtual table declares for the same columns, and names its columns the
//! same way — quoted and lower case — so one query template runs against
//! either schema with only the schema name substituted.
//!
//! Exasol evaluates these tables under its own rules, which is the whole
//! point: it offers no configurable collation, comparing character values by
//! exact value and ordering them by code point. The delegated `postgresql`
//! path renders no collation, so it reproduces those answers only over a
//! source column that itself declares `COLLATE "C"`, which is why
//! `docker/postgres/init.sql` seeds `items_c` beside `items`: a query over
//! `items_c` compares with this module's `items`, and a query over `items`,
//! whose database default `en_US.utf8` orders differently, asserts
//! PostgreSQL's own answer instead. The mirror is seeded from
//! `docker/postgres/init.sql`'s own literal values rather than from what
//! PostgreSQL returns for them, for the same reason `super::mirror` states: a
//! mirror filled from the source's answers would agree with the source by
//! construction and prove nothing.
//!
//! `items`, `items_c`, `item_notes`, `type_coverage`, and `measures` are mirrored, each
//! with the values Exasol reads rather than the values PostgreSQL stores: a
//! column read as text holds PostgreSQL's text for the value, `money` and
//! `real` hold a `DOUBLE`, a `bit` column holds a `BOOLEAN`, a `character(n)`
//! holds a `CHAR(n)`, and a time-zone-aware timestamp holds its UTC wall-clock
//! value. `items` also carries `labels`, an array, which no mapping maps, so
//! no mirror column corresponds to it. `numeric_specials` and `all_unmapped`
//! carry no mirror at all: `numeric_specials` exists to exercise the coercion
//! failure of a constrained `numeric` rather than a row comparison, and
//! `all_unmapped` has no mapped column for a delegated query to reach.
//! `text_overflow` and `collated_names` are read for a failure and for
//! PostgreSQL's own answer, and `nan_measures` for a `NaN` Exasol cannot hold.
//! `refresh_target`, the `order_names` view, and `"MixedCase"` are likewise
//! unmirrored: none of the three is read differentially.
//!
//! There is no Rust module shared between `docker/postgres/init.sql` and this
//! one the way `super::items` is shared between `super::mysql` and
//! `super::mirror`: the PostgreSQL source fixture is authored directly as SQL
//! rather than templated from Rust, per the task that grew it. The values
//! below are therefore copied by hand from that file rather than computed
//! from a shared constant, and the two must be kept in sync by hand when
//! either changes — the tests below assert this module's own generated SQL
//! text, which is what a reviewer diffs against `docker/postgres/init.sql`.

use super::exasol::Exasol;
use super::mirror_table::{self, column, Column};

/// The schema holding the PostgreSQL mirror tables, distinct from
/// [`super::mirror::SCHEMA`] so the two sources' `items` tables never
/// collide in one Exasol instance.
pub const SCHEMA: &str = "PG_MIRROR";

/// The mirrored `items` columns, in `docker/postgres/init.sql`'s column
/// order restricted to the mapped ones.
///
/// `integer` maps to Exasol `DECIMAL(10,0)`, `character varying(n)` to
/// `VARCHAR(n) UTF8`, `character(8)` to `CHAR(8) UTF8`, `timestamp with time
/// zone` to `TIMESTAMP(6)`, and `text`, `jsonb`, `bytea`, and `uuid` to
/// `VARCHAR(2000000) UTF8`, per `postgres_type_mapping::map_data_type`.
const ITEM_COLUMNS: [Column; 10] = [
    column("id", "DECIMAL(10,0) NOT NULL"),
    column("category", "VARCHAR(20) UTF8 NOT NULL"),
    column("sort_key", "VARCHAR(5) UTF8 NOT NULL"),
    column("tag", "VARCHAR(10) UTF8 NOT NULL"),
    column("code", "CHAR(8) UTF8 NOT NULL"),
    column("description", "VARCHAR(2000000) UTF8 NOT NULL"),
    column("details", "VARCHAR(2000000) UTF8 NULL"),
    column("photo", "VARCHAR(2000000) UTF8 NULL"),
    column("uid", "VARCHAR(2000000) UTF8 NOT NULL"),
    column("seen_at", "TIMESTAMP(6) NOT NULL"),
];

/// The `items` rows, `(id, category, sort_key, tag, code, description,
/// details, photo, uid, seen_at)`, copied from `docker/postgres/init.sql`'s
/// `INSERT INTO items` statement. `details` is the text PostgreSQL prints for
/// the `jsonb` value, `photo` the hex text it prints for the `bytea` value, and
/// `seen_at` the UTC wall-clock value of the `timestamptz`.
const ITEM_VALUES: &str = "\
    (1, 'widget', 'B', 'ab ', 'ITEM0001', 'First widget in the fixture.', '{\"tier\": \"gold\"}', '\\xdeadbeef', '00000000-0000-0000-0000-000000000001', '2026-01-01 10:00:00'), \
    (2, 'Widget', 'a', 'ab', 'ITEM0002', 'Second widget, capitalized.', NULL, '\\xc0ffee', '00000000-0000-0000-0000-000000000002', '2026-01-02 10:00:00'), \
    (3, 'widget', 'C', 'ab!', 'ITEM0003', 'Third widget, lower case again.', '{\"tier\": \"silver\"}', NULL, '00000000-0000-0000-0000-000000000003', '2026-01-03 10:00:00'), \
    (4, 'Widget', 'B', 'ab ', 'ITEM0004', 'Fourth widget, capitalized again.', '{\"tier\": \"silver\"}', '\\xfeed', '00000000-0000-0000-0000-000000000004', '2026-01-04 10:00:00'), \
    (5, 'cafe', 'a', 'ab', 'ITEM0005', 'A plain cafe, no accent.', NULL, '\\xbeef', '00000000-0000-0000-0000-000000000005', '2026-01-05 10:00:00'), \
    (6, 'café', 'C', 'ab!', 'ITEM0006', 'The accented cafe.', '{\"tier\": \"gold\"}', '\\xab', '00000000-0000-0000-0000-000000000006', '2026-01-06 10:00:00')";

/// The mirrored `items_c` columns: `items`'s first four, which are the ones
/// `items_c` carries.
const ITEM_C_COLUMNS: [Column; 4] = [
    column("id", "DECIMAL(10,0) NOT NULL"),
    column("category", "VARCHAR(20) UTF8 NOT NULL"),
    column("sort_key", "VARCHAR(5) UTF8 NOT NULL"),
    column("tag", "VARCHAR(10) UTF8 NOT NULL"),
];

/// The `items_c` rows, `(id, category, sort_key, tag)`: the first four columns
/// of the `items` rows, then the two rows `docker/postgres/init.sql` adds whose
/// tags are `5%x` and `5\yx`.
const ITEM_C_VALUES: &str = "\
    (1, 'widget', 'B', 'ab '), \
    (2, 'Widget', 'a', 'ab'), \
    (3, 'widget', 'C', 'ab!'), \
    (4, 'Widget', 'B', 'ab '), \
    (5, 'cafe', 'a', 'ab'), \
    (6, 'café', 'C', 'ab!'), \
    (7, 'widget', 'B', '5%x'), \
    (8, 'widget', 'B', '5\\yx')";

/// The `item_notes` rows, `(item_id, note)`, copied verbatim from
/// `docker/postgres/init.sql`'s `INSERT INTO item_notes` statement.
/// `item_id` 1 is absent, so that `items` row is unmatched on the `items`
/// side; `item_id` 99 matches no `items` row, so it is unmatched on the
/// `item_notes` side.
const NOTE_VALUES: &str = "\
    (2, 'restocked'), \
    (3, 'discontinued'), \
    (4, 'seasonal'), \
    (5, 'restocked'), \
    (6, 'discontinued'), \
    (99, 'orphan')";

/// The mirrored `type_coverage` columns, in `docker/postgres/init.sql`'s column
/// order: the Exasol type the virtual table declares for each.
///
/// A column of a type without an Exasol counterpart is `VARCHAR(2000000)`
/// holding PostgreSQL's text for the value, `money` and `real` are `DOUBLE`,
/// both `bit` columns of one and eight bits are `BOOLEAN`, a `character(5)` is
/// `CHAR(5)`, a `character(3000)` is `VARCHAR(3000)` because Exasol's `CHAR`
/// holds 2000 characters at most, and every timestamp keeps its own precision.
const TYPE_COVERAGE_COLUMNS: [Column; 28] = [
    column("id", "DECIMAL(10,0) NOT NULL"),
    column("small_id", "DECIMAL(5,0) NOT NULL"),
    column("unbounded_amount", "VARCHAR(2000000) UTF8 NULL"),
    column("wide_amount", "VARCHAR(2000000) UTF8 NULL"),
    column("uid", "VARCHAR(2000000) UTF8 NULL"),
    column("doc_json", "VARCHAR(2000000) UTF8 NULL"),
    column("doc_jsonb", "VARCHAR(2000000) UTF8 NULL"),
    column("doc_xml", "VARCHAR(2000000) UTF8 NULL"),
    column("raw", "VARCHAR(2000000) UTF8 NULL"),
    column("span", "VARCHAR(2000000) UTF8 NULL"),
    column("host", "VARCHAR(2000000) UTF8 NULL"),
    column("price", "DOUBLE NULL"),
    column("approx", "DOUBLE NULL"),
    column("flag", "BOOLEAN NULL"),
    column("other_flag", "BOOLEAN NULL"),
    column("bit_flag", "BOOLEAN NULL"),
    column("bits", "BOOLEAN NULL"),
    column("varbits", "VARCHAR(16) UTF8 NULL"),
    column("fixed_code", "CHAR(5) UTF8 NULL"),
    column("label", "VARCHAR(5) UTF8 NULL"),
    column("wide_code", "VARCHAR(3000) UTF8 NULL"),
    column("feeling", "VARCHAR(2000000) UTF8 NULL"),
    column("clock", "VARCHAR(100) UTF8 NULL"),
    column("clock_tz", "VARCHAR(100) UTF8 NULL"),
    column("precise_at", "TIMESTAMP(6) NULL"),
    column("whole_at", "TIMESTAMP(0) NULL"),
    column("happened_at", "TIMESTAMP(6) NULL"),
    column("local_at", "TIMESTAMP(6) NULL"),
];

/// The `type_coverage` rows, in the column order above, as the values Exasol
/// reads. [`HUGE_NUMERIC`] stands for the text of `1e400`, which is 401 digits.
///
/// Row 1's `host` is `192.168.0.1/32` because PostgreSQL prints the netmask of
/// an `inet` host address when it casts one to text. The `real` `0.1` is the
/// single-precision value widened to double precision.
const TYPE_COVERAGE_VALUES: &str = "\
    (1, 1, '1.0', '12345678901234567890123456789012345678.90', 'a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11', \
     '{\"a\":   1,  \"b\": [1,2]}', '{\"a\": 2, \"b\": 1}', '<a>1</a>', '\\xdeadbeef', '1 day 02:03:04', '192.168.0.1/32', \
     12.34, 0.100000001490116119384765625, TRUE, TRUE, TRUE, TRUE, '1010', 'ab', 'ab', 'ab', 'ok', \
     '16:59:59.123', '16:59:59+02', '2024-01-01 12:00:00.123456', '2024-01-01 12:00:01', \
     '2024-01-01 12:00:00.123456', '2024-01-01 12:00:00.123456'), \
    (2, 2, '1.00', NULL, 'b0eebc99-9c0b-4ef8-bb6d-6bb9bd380a22', \
     '{\"x\": null}', '{\"a\": 2, \"b\": 1}', '<b/>', '\\xc0ffee', '02:00:00', '10.0.0.1/8', \
     0.1, 0.5, FALSE, TRUE, FALSE, TRUE, '1', 'abc', 'ab', NULL, 'happy', \
     '00:00:01', '08:00:00-05', '2024-01-01 12:00:00.123457', '2024-06-01 00:00:00', \
     '2024-01-02 12:00:00', '2024-01-03 12:00:00'), \
    (3, 3, 'NaN', NULL, NULL, \
     NULL, NULL, NULL, NULL, NULL, NULL, \
     NULL, NULL, NULL, NULL, NULL, FALSE, NULL, NULL, NULL, NULL, NULL, \
     NULL, NULL, NULL, NULL, NULL, NULL), \
    (4, 4, 'Infinity', NULL, 'c0eebc99-9c0b-4ef8-bb6d-6bb9bd380a33', \
     NULL, NULL, NULL, NULL, NULL, NULL, \
     NULL, NULL, TRUE, FALSE, TRUE, TRUE, NULL, 'ab', 'ab', 'ab', 'sad', \
     NULL, NULL, '2024-01-01 12:00:00.123456', NULL, '2024-01-04 12:00:00', '2024-01-04 12:00:00'), \
    (5, 5, 'HUGE_NUMERIC', NULL, NULL, \
     NULL, NULL, NULL, NULL, NULL, NULL, \
     NULL, NULL, FALSE, FALSE, FALSE, NULL, NULL, NULL, NULL, NULL, NULL, \
     NULL, NULL, NULL, NULL, '2024-01-05 12:00:00', '2024-01-06 12:00:00')";

/// The marker in [`TYPE_COVERAGE_VALUES`] that [`type_coverage_values`] replaces
/// with the text of `1e400`.
const HUGE_NUMERIC: &str = "HUGE_NUMERIC";

/// The digits PostgreSQL prints for the `numeric` value `1e400`.
const HUGE_NUMERIC_DIGITS: usize = 400;

fn type_coverage_values() -> String {
    let digits = format!("1{}", "0".repeat(HUGE_NUMERIC_DIGITS));
    TYPE_COVERAGE_VALUES.replace(HUGE_NUMERIC, &digits)
}

/// The `measures` rows as Exasol reads them, copied by hand from
/// `docker/postgres/init.sql`.
const MEASURES_VALUES: &str = "\
    (1, 'a', 10, 1.25, 0.5, 0.25, TRUE, '5'), \
    (2, 'a', 20, 2.50, 1.5, 0.5, FALSE, '5.0'), \
    (3, 'a', 20, 3.75, 1.5, 0.5, TRUE, ''), \
    (4, 'b', 1, -0.25, 2.25, 16777216, TRUE, '7'), \
    (5, 'b', 9223372036854775807, 0.25, 4.25, 1, NULL, '7.5'), \
    (6, 'b', NULL, NULL, NULL, NULL, FALSE, NULL), \
    (7, 'c', NULL, NULL, NULL, NULL, NULL, NULL)";

/// Creates the PostgreSQL mirror schema and every table, seeded from the
/// literals above.
///
/// Replaces whatever an earlier run left behind rather than adding to it: the
/// Compose stack outlives a test run, so a mirror surviving from an older
/// fixture would make every differential comparison report a pushdown defect
/// that is really a stale table.
pub fn install(exasol: &Exasol) {
    let seed = mirror_table::seeder(exasol, SCHEMA);
    seed("items", &ITEM_COLUMNS, ITEM_VALUES);
    seed("items_c", &ITEM_C_COLUMNS, ITEM_C_VALUES);
    seed("item_notes", &mirror_table::NOTE_COLUMNS, NOTE_VALUES);
    seed(
        "type_coverage",
        &TYPE_COVERAGE_COLUMNS,
        &type_coverage_values(),
    );
    seed("measures", &mirror_table::MEASURES_COLUMNS, MEASURES_VALUES);
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
    fn the_pg_mirror_schema_is_distinct_from_the_mysql_mirror_schema() {
        assert_ne!(
            SCHEMA,
            super::super::mirror::SCHEMA,
            "the two sources' `items` tables would collide in one Exasol instance if both \
             mirrors used the same schema"
        );
    }

    #[test]
    fn the_mirrored_items_declare_the_types_the_virtual_table_declares() {
        let declarations = item_declarations();

        assert!(
            declarations.contains("\"id\" DECIMAL(10,0)"),
            "PostgreSQL integer maps to Exasol DECIMAL(10,0): {declarations}"
        );
        assert!(
            declarations.contains("\"category\" VARCHAR(20) UTF8")
                && declarations.contains("\"sort_key\" VARCHAR(5) UTF8")
                && declarations.contains("\"tag\" VARCHAR(10) UTF8"),
            "a PostgreSQL character varying(n) maps to Exasol VARCHAR(n) UTF8: {declarations}"
        );
    }

    #[test]
    fn every_mirrored_row_carries_one_value_per_declared_column() {
        mirror_table::assert_one_value_per_column(&[
            ("items", ITEM_COLUMNS.len(), ITEM_VALUES.to_string()),
            ("items_c", ITEM_C_COLUMNS.len(), ITEM_C_VALUES.to_string()),
            (
                "type_coverage",
                TYPE_COVERAGE_COLUMNS.len(),
                type_coverage_values(),
            ),
            (
                "measures",
                mirror_table::MEASURES_COLUMNS.len(),
                MEASURES_VALUES.to_string(),
            ),
        ]);
    }

    #[test]
    fn fixtures_seed_case_and_accent_variants_in_the_grouped_column() {
        for value in ["'widget'", "'Widget'", "'cafe'", "'café'"] {
            assert!(
                ITEM_VALUES.contains(value),
                "category must hold {value}, the set gate 2.3(e) measured Exasol's comparison \
                 collation grouping differently from PostgreSQL's default: {ITEM_VALUES}"
            );
        }
    }

    #[test]
    fn fixtures_seed_the_measured_ordering_triple() {
        for value in ["'B'", "'a'", "'C'"] {
            assert!(
                ITEM_VALUES.contains(value),
                "sort_key must hold {value}, the triple gate 2.3(d) measured PostgreSQL's bare \
                 ordering disagreeing with Exasol's over: {ITEM_VALUES}"
            );
        }
    }

    #[test]
    fn fixtures_seed_a_trailing_space_variant_beside_its_bare_counterpart() {
        assert!(
            ITEM_VALUES.contains("'ab '")
                && ITEM_VALUES.contains("'ab'")
                && ITEM_VALUES.contains("'ab!'"),
            "tag must hold a value differing from another only by a trailing space: {ITEM_VALUES}"
        );
    }

    #[test]
    fn fixtures_leave_an_unmatched_row_on_each_side_of_the_join() {
        assert!(
            !NOTE_VALUES.contains("(1, "),
            "item_notes must carry no row for items.id 1, so that row is unmatched on the \
             items side: {NOTE_VALUES}"
        );
        assert!(
            NOTE_VALUES.contains("(99, "),
            "item_notes must carry a row no items row matches, so that row is unmatched on the \
             item_notes side: {NOTE_VALUES}"
        );
    }
}
