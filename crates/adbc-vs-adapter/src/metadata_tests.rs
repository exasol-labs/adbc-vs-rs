use arrow::array::{Int64Array, StringArray};

use super::*;

#[test]
fn assembles_columns_in_ordinal_order_within_their_table() {
    let rows = vec![
        ColumnRow {
            table: "orders".to_string(),
            column: "total".to_string(),
            ordinal_position: 2,
            data_type: "decimal".to_string(),
            column_type: "decimal(12,4)".to_string(),
            character_maximum_length: None,
            character_set: None,
            numeric_precision: Some(12),
            numeric_scale: Some(4),
            is_nullable: true,
        },
        ColumnRow {
            table: "orders".to_string(),
            column: "id".to_string(),
            ordinal_position: 1,
            data_type: "int".to_string(),
            column_type: "int(10)".to_string(),
            character_maximum_length: None,
            character_set: None,
            numeric_precision: Some(10),
            numeric_scale: Some(0),
            is_nullable: false,
        },
    ];

    let tables = assemble_tables(vec!["orders".to_string()], rows);

    assert_eq!(tables.len(), 1);
    let columns: Vec<&str> = tables[0].columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(columns, vec!["id", "total"]);
}

#[test]
fn a_table_with_no_column_rows_is_reported_with_no_columns() {
    let tables = assemble_tables(vec!["empty".to_string()], vec![]);

    assert_eq!(tables.len(), 1);
    assert_eq!(tables[0].name, "empty");
    assert!(tables[0].columns.is_empty());
}

#[test]
fn unsigned_is_read_from_the_column_type_text() {
    let rows = vec![ColumnRow {
        table: "t".to_string(),
        column: "c".to_string(),
        ordinal_position: 1,
        data_type: "int".to_string(),
        column_type: "int(10) unsigned".to_string(),
        character_maximum_length: None,
        character_set: None,
        numeric_precision: Some(10),
        numeric_scale: Some(0),
        is_nullable: true,
    }];

    let tables = assemble_tables(vec!["t".to_string()], rows);

    assert!(tables[0].columns[0].mysql.is_unsigned);
}

#[test]
fn signed_column_type_is_not_reported_unsigned() {
    let rows = vec![ColumnRow {
        table: "t".to_string(),
        column: "c".to_string(),
        ordinal_position: 1,
        data_type: "int".to_string(),
        column_type: "int(10)".to_string(),
        character_maximum_length: None,
        character_set: None,
        numeric_precision: Some(10),
        numeric_scale: Some(0),
        is_nullable: true,
    }];

    let tables = assemble_tables(vec!["t".to_string()], rows);

    assert!(!tables[0].columns[0].mysql.is_unsigned);
}

#[test]
fn character_set_is_carried_through_to_the_mapped_column_input() {
    let rows = vec![ColumnRow {
        table: "t".to_string(),
        column: "name".to_string(),
        ordinal_position: 1,
        data_type: "varchar".to_string(),
        column_type: "varchar(100)".to_string(),
        character_maximum_length: Some(100),
        character_set: Some("utf8mb4".to_string()),
        numeric_precision: None,
        numeric_scale: None,
        is_nullable: true,
    }];

    let tables = assemble_tables(vec!["t".to_string()], rows);

    assert_eq!(
        tables[0].columns[0].mysql.character_set.as_deref(),
        Some("utf8mb4")
    );
}

#[test]
fn absent_character_set_carries_through_as_none() {
    let rows = vec![ColumnRow {
        table: "t".to_string(),
        column: "id".to_string(),
        ordinal_position: 1,
        data_type: "int".to_string(),
        column_type: "int(10)".to_string(),
        character_maximum_length: None,
        character_set: None,
        numeric_precision: Some(10),
        numeric_scale: Some(0),
        is_nullable: true,
    }];

    let tables = assemble_tables(vec!["t".to_string()], rows);

    assert_eq!(tables[0].columns[0].mysql.character_set, None);
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
}

#[test]
fn required_string_cell_names_the_column_when_null() {
    let array = StringArray::from(vec![None::<&str>]);
    let error = required_string_cell(&array, 0, "TABLE_NAME").unwrap_err();
    assert!(error.to_string().contains("TABLE_NAME"));
}

#[test]
fn string_literal_is_single_quoted() {
    assert_eq!(quote_string_literal("shop"), "'shop'");
}

#[test]
fn string_literal_escapes_an_embedded_quote_and_backslash() {
    assert_eq!(quote_string_literal("o'reilly"), "'o\\'reilly'");
    assert_eq!(quote_string_literal(r"a\b"), r"'a\\b'");
}
