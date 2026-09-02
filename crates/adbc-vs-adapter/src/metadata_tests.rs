use arrow::array::{Int64Array, StringArray, UInt64Array};

use super::*;
use crate::type_mapping::MappedType;

/// A catalog row belonging to no technology: the four fields the assembler
/// reads and nothing else, so the rules it enforces are asserted without any
/// source's own catalog shape standing in for them.
struct Row {
    table: &'static str,
    column: &'static str,
    ordinal_position: i64,
    source_type: &'static str,
}

impl CatalogRow for Row {
    fn table(&self) -> &str {
        self.table
    }

    fn column(&self) -> &str {
        self.column
    }

    fn ordinal_position(&self) -> i64 {
        self.ordinal_position
    }

    fn source_type(&self) -> &str {
        self.source_type
    }
}

/// A mapping that maps one named source type and refuses every other, which is
/// the shape every real one has: unmapped is an outcome the assembler records
/// rather than a failure it propagates.
fn map_only_dates(row: Row) -> Result<MappedColumn, String> {
    if row.source_type == "date" {
        return Ok(MappedColumn {
            exasol_type: MappedType::Date,
            is_nullable: true,
        });
    }
    Err(format!("{} is not mapped", row.source_type))
}

fn row(table: &'static str, column: &'static str, ordinal_position: i64) -> Row {
    Row {
        table,
        column,
        ordinal_position,
        source_type: "date",
    }
}

#[test]
fn assembles_columns_in_ordinal_order_within_their_table() {
    let tables = assemble_tables(
        vec!["orders".to_string()],
        vec![row("orders", "total", 2), row("orders", "id", 1)],
        map_only_dates,
    );

    assert_eq!(tables.len(), 1);
    let columns: Vec<&str> = tables[0].columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(columns, vec!["id", "total"]);
}

#[test]
fn a_table_with_no_column_rows_is_reported_with_no_columns() {
    let tables = assemble_tables(vec!["empty".to_string()], Vec::<Row>::new(), map_only_dates);

    assert_eq!(tables.len(), 1);
    assert_eq!(tables[0].name, "empty");
    assert!(tables[0].columns.is_empty());
}

/// A table keeps the columns that map and records the ones that do not, so
/// neither half is lost to the other, and the omission carries the source's
/// own spelling of the type it could not map.
#[test]
fn a_table_reports_its_mapped_and_its_omitted_columns_together() {
    let tables = assemble_tables(
        vec!["orders".to_string()],
        vec![
            row("orders", "created", 1),
            Row {
                source_type: "json",
                ..row("orders", "payload", 2)
            },
        ],
        map_only_dates,
    );
    let table = &tables[0];

    assert_eq!(table.columns.len(), 1);
    assert_eq!(table.columns[0].name, "created");
    assert_eq!(table.omissions.len(), 1);
    assert_eq!(table.omissions[0].table, "orders");
    assert_eq!(table.omissions[0].column.as_deref(), Some("payload"));
    assert_eq!(table.omissions[0].source_type.as_deref(), Some("json"));
    assert!(
        table.omissions[0].reason.contains("json"),
        "the recorded reason is the mapping's own: {}",
        table.omissions[0].reason
    );
}

/// Each table is assembled from its own rows alone, so two tables discovered
/// together cannot borrow each other's columns or omissions.
#[test]
fn rows_are_grouped_under_the_table_they_name() {
    let tables = assemble_tables(
        vec!["items".to_string(), "orders".to_string()],
        vec![
            Row {
                source_type: "json",
                ..row("items", "payload", 1)
            },
            row("orders", "created", 1),
        ],
        map_only_dates,
    );

    assert_eq!(tables[0].name, "items");
    assert!(tables[0].columns.is_empty());
    assert_eq!(tables[0].omissions.len(), 1);
    assert_eq!(tables[1].name, "orders");
    assert_eq!(tables[1].columns.len(), 1);
    assert!(tables[1].omissions.is_empty());
}

#[test]
fn string_cell_reads_a_utf8_array_value() {
    let array = StringArray::from(vec![Some("orders"), None]);
    assert_eq!(string_cell(&array, 0).unwrap(), Some("orders".to_string()));
    assert_eq!(string_cell(&array, 1).unwrap(), None);
}

#[test]
fn string_cell_rejects_a_non_string_array() {
    let array = Int64Array::from(vec![1]);
    let error = string_cell(&array, 0).unwrap_err();
    assert!(error.to_string().contains("Int64"));
}

#[test]
fn i64_cell_reads_signed_and_unsigned_integer_arrays() {
    let signed = Int64Array::from(vec![Some(7), None]);
    assert_eq!(i64_cell(&signed, 0).unwrap(), Some(7));
    assert_eq!(i64_cell(&signed, 1).unwrap(), None);

    let unsigned = UInt64Array::from(vec![7u64]);
    assert_eq!(i64_cell(&unsigned, 0).unwrap(), Some(7));
}

#[test]
fn i64_cell_reports_a_uint64_that_overflows_i64() {
    let unsigned = UInt64Array::from(vec![u64::MAX]);
    let error = i64_cell(&unsigned, 0).unwrap_err();
    assert!(
        error.to_string().contains("overflows i64"),
        "the error must name the failure the value hit: {error}"
    );
}

#[test]
fn required_string_cell_names_the_column_when_null() {
    let array = StringArray::from(vec![None::<&str>]);
    let error = required_string_cell(&array, 0, "TABLE_NAME").unwrap_err();
    assert!(error.to_string().contains("TABLE_NAME"));
}
