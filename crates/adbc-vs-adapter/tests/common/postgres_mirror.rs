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
//! exact value and ordering them by code point. Those answers are the ground
//! truth the delegated `postgresql` path has to reproduce over
//! `COLLATE "C"`, which decision-log gate 2.3(d) and (e) measured PostgreSQL's
//! own default `en_US.utf8` ordering and grouping disagreeing with. The
//! mirror is seeded from `docker/postgres/init.sql`'s own literal values
//! rather than from what PostgreSQL returns for them, for the same reason
//! `super::mirror` states: a mirror filled from the source's answers would
//! agree with the source by construction and prove nothing.
//!
//! Only `items`'s and `item_notes`'s mapped columns are mirrored here.
//! `items` also carries `code`, `description`, `details`, `photo`, `uid`, and
//! `seen_at`, which `docker/postgres/init.sql` documents as unmapped — no
//! delegated query can reach them, so no mirror column corresponds to them,
//! the same reasoning `super::mirror` states for MySQL's latin1 `region`
//! column. `numeric_specials` and `all_unmapped` carry no mirror at all:
//! `numeric_specials` exists to exercise the coercion failure task 7.1 states
//! rather than a row comparison, and `all_unmapped` has no mapped column for
//! a delegated query to reach in the first place.
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
use super::mirror_table::{self, Column};

/// The schema holding the PostgreSQL mirror tables, distinct from
/// [`super::mirror::SCHEMA`] so the two sources' `items` tables never
/// collide in one Exasol instance.
pub const SCHEMA: &str = "PG_MIRROR";

/// The mirrored `items` columns, in `docker/postgres/init.sql`'s column
/// order restricted to the mapped ones.
///
/// `integer` maps to Exasol `DECIMAL(10,0)` and `character varying(n)` maps
/// to Exasol `VARCHAR(n) UTF8`, per `postgres_type_mapping::map_data_type`.
const ITEM_COLUMNS: [Column; 4] = [
    Column {
        name: "id",
        declaration: "DECIMAL(10,0) NOT NULL",
    },
    Column {
        name: "category",
        declaration: "VARCHAR(20) UTF8 NOT NULL",
    },
    Column {
        name: "sort_key",
        declaration: "VARCHAR(5) UTF8 NOT NULL",
    },
    Column {
        name: "tag",
        declaration: "VARCHAR(10) UTF8 NOT NULL",
    },
];

/// The `items` rows, `(id, category, sort_key, tag)`, copied verbatim from
/// `docker/postgres/init.sql`'s `INSERT INTO items` statement.
const ITEM_VALUES: &str = "\
    (1, 'widget', 'B', 'ab '), \
    (2, 'Widget', 'a', 'ab'), \
    (3, 'widget', 'C', 'ab!'), \
    (4, 'Widget', 'B', 'ab '), \
    (5, 'cafe', 'a', 'ab'), \
    (6, 'café', 'C', 'ab!')";

/// The mirrored `item_notes` columns, in `docker/postgres/init.sql`'s column
/// order. Both columns are mapped, so both are mirrored.
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

/// Creates the PostgreSQL mirror schema and both tables, seeded from the
/// literals above.
///
/// Replaces whatever an earlier run left behind rather than adding to it: the
/// Compose stack outlives a test run, so a mirror surviving from an older
/// fixture would make every differential comparison report a pushdown defect
/// that is really a stale table.
pub fn install(exasol: &Exasol) {
    exasol.execute(&format!("CREATE SCHEMA IF NOT EXISTS {SCHEMA}"));
    mirror_table::seed(exasol, SCHEMA, "items", &ITEM_COLUMNS, ITEM_VALUES);
    mirror_table::seed(exasol, SCHEMA, "item_notes", &NOTE_COLUMNS, NOTE_VALUES);
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
    fn the_mirrored_items_carry_no_column_for_an_unmapped_source_column() {
        let declarations = item_declarations();
        for unmapped in ["code", "description", "details", "photo", "uid", "seen_at"] {
            assert!(
                !declarations.contains(unmapped),
                "metadata discovery unmaps {unmapped}, so no delegated query reaches it and a \
                 mirror column would be compared against nothing: {declarations}"
            );
        }
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
