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
//! `'ab '`, `'ab'`, `'ab!'` is `'ab'`. Those answers are the ground truth the
//! delegated path has to reproduce — MySQL's own defaults return two groups
//! and a different `MIN`, which is the silent wrong-value failure the rendered
//! collations exist to prevent. The mirror is therefore seeded from the
//! fixture's own values rather than from what MySQL returns for them: a mirror
//! filled from the source's answers would agree with the source by
//! construction and prove nothing.
//!
//! For the same reason the column types below are stated here rather than
//! computed through `adbc_vs_adapter::type_mapping`. A mirror deriving its
//! types from the code under test would inherit that code's mistakes; an
//! independently stated type set, asserted equal to the virtual table's by an
//! end-to-end test, catches a mapping change instead of absorbing it.

use super::exasol::Exasol;
use super::items;

/// The schema holding the mirror tables. A differential test substitutes this
/// for the virtual schema's name in one query template.
pub const SCHEMA: &str = "MIRROR";

/// A mirror column: the name the virtual table gives it, and the Exasol type
/// declaration the virtual table declares for it.
struct Column {
    name: &'static str,
    declaration: &'static str,
}

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
    seed(exasol, "items", &ITEM_COLUMNS, &items::mirror_values());
    seed(exasol, "item_notes", &NOTE_COLUMNS, &items::note_values());
}

/// Creates one table and fills it, in that order and never one without the
/// other: a mirror table that exists but holds nothing looks to a differential
/// test exactly like a delegated query returning no row.
fn seed(exasol: &Exasol, table: &str, columns: &[Column], values: &str) {
    exasol.execute(&create_statement(table, columns));
    exasol.execute(&insert_statement(table, columns, values));
}

fn create_statement(table: &str, columns: &[Column]) -> String {
    let definitions: Vec<String> = columns
        .iter()
        .map(|column| format!("\"{}\" {}", column.name, column.declaration))
        .collect();
    format!(
        "CREATE OR REPLACE TABLE {SCHEMA}.\"{table}\" ({})",
        definitions.join(", ")
    )
}

/// Renders the seeding statement, naming every column rather than relying on
/// declaration order, and taking the names from the same list the table was
/// created from so the two cannot drift apart.
fn insert_statement(table: &str, columns: &[Column], values: &str) -> String {
    let names: Vec<String> = columns
        .iter()
        .map(|column| format!("\"{}\"", column.name))
        .collect();
    format!(
        "INSERT INTO {SCHEMA}.\"{table}\" ({}) VALUES {values}",
        names.join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item_declarations() -> String {
        create_statement("items", &ITEM_COLUMNS)
    }

    #[test]
    fn a_table_is_named_exactly_as_the_virtual_schema_names_it() {
        let one_column = [Column {
            name: "id",
            declaration: "DECIMAL(10,0)",
        }];

        assert_eq!(
            create_statement("items", &one_column),
            "CREATE OR REPLACE TABLE MIRROR.\"items\" (\"id\" DECIMAL(10,0))",
            "schema, table, and column names must match the virtual schema's, quoted and lower \
             case, so one query template serves both schemas"
        );
    }

    #[test]
    fn a_table_replaces_the_one_an_earlier_run_left_behind() {
        assert!(
            item_declarations().starts_with("CREATE OR REPLACE TABLE"),
            "the Compose stack outlives a run, so a stale mirror must be replaced rather than \
             appended to: {}",
            item_declarations()
        );
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

    #[test]
    fn a_seeding_statement_names_every_column_the_table_was_created_with() {
        let statement = insert_statement("items", &ITEM_COLUMNS, "(1, 'Widget')");

        assert!(
            statement.starts_with(
                "INSERT INTO MIRROR.\"items\" (\"id\", \"category\", \"code\", \"batch\", \
                 \"quantity\", \"rank_key\", \"tie_break\") VALUES "
            ),
            "the column list must come from the same list the table was created from, in the \
             order the values are rendered in: {statement}"
        );
    }

    #[test]
    fn every_column_is_seeded_with_a_value() {
        for (columns, values) in [
            (&ITEM_COLUMNS[..], items::mirror_values()),
            (&NOTE_COLUMNS[..], items::note_values()),
        ] {
            let first_tuple = values
                .split_once(')')
                .expect("a rendered value list holds at least one tuple")
                .0;
            assert_eq!(
                first_tuple.matches(", ").count() + 1,
                columns.len(),
                "the rendered tuple {first_tuple}) must carry one value per declared column, \
                 or Exasol assigns the values to the wrong columns"
            );
        }
    }
}
