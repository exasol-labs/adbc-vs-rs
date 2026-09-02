//! The mirror-table mechanics [`super::mirror`] and [`super::postgres_mirror`]
//! share: a mirror column, and the statements that create and seed one.
//!
//! Each source's own mirror module states its own schema constant, its own
//! column declarations, and its own row values — those are source-specific
//! facts. How a declared column becomes a `CREATE TABLE`, and a value list
//! becomes an `INSERT`, is not: it is the same rendering rule for every
//! source, and a second copy of it would let the two mirrors' shapes drift
//! apart from each other with nothing to notice.

use super::exasol::Exasol;

/// A mirror column: the name the virtual table gives it, and the Exasol type
/// declaration the virtual table declares for it.
pub struct Column {
    pub name: &'static str,
    pub declaration: &'static str,
}

/// Creates one table and fills it, in that order and never one without the
/// other: a mirror table that exists but holds nothing looks to a
/// differential test exactly like a delegated query returning no row.
pub fn seed(exasol: &Exasol, schema: &str, table: &str, columns: &[Column], values: &str) {
    exasol.execute(&create_statement(schema, table, columns));
    exasol.execute(&insert_statement(schema, table, columns, values));
}

pub fn create_statement(schema: &str, table: &str, columns: &[Column]) -> String {
    let definitions: Vec<String> = columns
        .iter()
        .map(|column| format!("\"{}\" {}", column.name, column.declaration))
        .collect();
    format!(
        "CREATE OR REPLACE TABLE {schema}.\"{table}\" ({})",
        definitions.join(", ")
    )
}

/// Renders the seeding statement, naming every column rather than relying on
/// declaration order, and taking the names from the same list the table was
/// created from so the two cannot drift apart.
pub fn insert_statement(schema: &str, table: &str, columns: &[Column], values: &str) -> String {
    let names: Vec<String> = columns
        .iter()
        .map(|column| format!("\"{}\"", column.name))
        .collect();
    format!(
        "INSERT INTO {schema}.\"{table}\" ({}) VALUES {values}",
        names.join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCHEMA: &str = "TEST_SCHEMA";

    #[test]
    fn a_table_is_named_exactly_as_the_virtual_schema_names_it() {
        let one_column = [Column {
            name: "id",
            declaration: "DECIMAL(10,0)",
        }];

        assert_eq!(
            create_statement(SCHEMA, "items", &one_column),
            "CREATE OR REPLACE TABLE TEST_SCHEMA.\"items\" (\"id\" DECIMAL(10,0))",
            "schema, table, and column names must match the virtual schema's, quoted and lower \
             case, so one query template serves both schemas"
        );
    }

    #[test]
    fn a_table_replaces_the_one_an_earlier_run_left_behind() {
        let one_column = [Column {
            name: "id",
            declaration: "DECIMAL(10,0)",
        }];
        let statement = create_statement(SCHEMA, "items", &one_column);

        assert!(
            statement.starts_with("CREATE OR REPLACE TABLE"),
            "the Compose stack outlives a run, so a stale mirror must be replaced rather than \
             appended to: {statement}"
        );
    }

    #[test]
    fn a_seeding_statement_names_every_column_the_table_was_created_with() {
        let columns = [
            Column {
                name: "id",
                declaration: "DECIMAL(10,0)",
            },
            Column {
                name: "category",
                declaration: "VARCHAR(50) UTF8",
            },
        ];
        let statement = insert_statement(SCHEMA, "items", &columns, "(1, 'Widget')");

        assert!(
            statement
                .starts_with("INSERT INTO TEST_SCHEMA.\"items\" (\"id\", \"category\") VALUES "),
            "the column list must come from the same list the table was created from, in the \
             order the values are rendered in: {statement}"
        );
    }

    #[test]
    fn every_column_is_seeded_with_a_value() {
        let columns = [
            Column {
                name: "id",
                declaration: "DECIMAL(10,0)",
            },
            Column {
                name: "category",
                declaration: "VARCHAR(50) UTF8",
            },
        ];
        let values = "(1, 'Widget'), (2, 'Gizmo')";

        let first_tuple = values
            .split_once(')')
            .expect("a rendered value list holds at least one tuple")
            .0;
        assert_eq!(
            first_tuple.matches(", ").count() + 1,
            columns.len(),
            "the rendered tuple {first_tuple}) must carry one value per declared column, or \
             Exasol assigns the values to the wrong columns"
        );
    }
}
