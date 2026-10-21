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

use super::exasol::Exasol;
use super::items;
use super::mirror_table::{self, Column};

/// The schema holding the mirror tables. A differential test substitutes this
/// for the virtual schema's name in one query template.
pub const SCHEMA: &str = "MIRROR";

/// The mirrored `items` columns, in the order
/// [`items::mirror_values`](super::items::mirror_values) renders them.
///
/// Each type is what `vs-adapter/type-mapping` declares for the source column:
/// MySQL `INT` becomes `DECIMAL(10,0)`, a `utf8mb4` `VARCHAR(n)` becomes
/// `VARCHAR(n) UTF8`, and MySQL `CHAR(8)` becomes `VARCHAR(8) UTF8` rather
/// than Exasol `CHAR(8)` — decision-log `[15]`, so no blank-padded Exasol type
/// reaches either side of a comparison. The `latin1` `region` column has no
/// entry: metadata discovery unmaps it, so no delegated query can reach it and
/// a mirror column for it would be compared against nothing.
const ITEM_COLUMNS: [Column; 7] = [
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
        declaration: "VARCHAR(8) UTF8 NOT NULL",
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

/// Creates the mirror schema and both tables, seeded from
/// [`items`](super::items).
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
        "item_notes",
        &NOTE_COLUMNS,
        &items::note_values(),
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
            declarations.contains("\"code\" VARCHAR(8) UTF8"),
            "MySQL CHAR(8) maps to Exasol VARCHAR(8), not CHAR(8), so the mirror must declare \
             no blank-padded type: {declarations}"
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
    }

    #[test]
    fn the_mirrored_items_carry_no_column_for_the_unmapped_latin1_source_column() {
        assert!(
            !item_declarations().contains("region"),
            "metadata discovery unmaps the latin1 column, so no delegated query reaches it and \
             a mirror column would be compared against nothing: {}",
            item_declarations()
        );
    }
}
