use super::*;
use crate::metadata::DiscoveredTable;
use crate::type_mapping::MappedType;

/// One `information_schema.columns` row, with the fields a test does not care
/// about set to what the first column of a table reports.
fn column_row(column: &str, data_type: &str) -> ColumnRow {
    ColumnRow {
        table: "orders".to_string(),
        column: column.to_string(),
        ordinal_position: 1,
        data_type: data_type.to_string(),
        is_nullable: true,
    }
}

fn assembled(rows: Vec<ColumnRow>) -> DiscoveredTable {
    trino_tables(vec!["orders".to_string()], rows)
        .pop()
        .expect("the named table is always reported")
}

/// The framework assembler driven through Trino's own row shape and mapping,
/// exactly as [`TrinoSource::discover_tables`] drives it.
fn trino_tables(table_names: Vec<String>, rows: Vec<ColumnRow>) -> Vec<DiscoveredTable> {
    assemble_tables(table_names, rows, |row| {
        trino_type_mapping::map_column(&trino_column(row))
    })
}

/// The Trino catalog fixture the rules binding every source are driven
/// through: one `orders` table holding a column that maps and a column of
/// Trino's own that does not.
///
/// It is assembled here, out of this source's own row shape and through this
/// source's own mapping, because that is exactly what `metadata_source_tests`
/// asserts of every source — a fixture built anywhere else would need a row
/// struct shared between two catalogs, which is the thing the interface exists
/// to keep out.
pub(crate) fn one_mapped_and_one_unmapped_column() -> Vec<DiscoveredTable> {
    trino_tables(
        vec!["orders".to_string()],
        vec![
            ColumnRow {
                is_nullable: false,
                ..column_row("id", "integer")
            },
            ColumnRow {
                ordinal_position: 2,
                ..column_row("payload", "json")
            },
        ],
    )
}

/// `vs-adapter/create-virtual-schema`: a table's columns are exposed in the
/// source's own ordinal order, whatever order the catalog rows arrived in.
#[test]
fn columns_are_assembled_in_ordinal_order_within_their_table() {
    let table = assembled(vec![
        ColumnRow {
            ordinal_position: 2,
            ..column_row("quantity", "bigint")
        },
        column_row("id", "integer"),
    ]);

    let columns: Vec<&str> = table.columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(columns, vec!["id", "quantity"]);
}

/// `vs-adapter/type-mapping`: a table keeps the columns that map and records
/// the ones that do not, and the omission carries the source's own spelling
/// of the type it could not map.
#[test]
fn a_table_reports_its_mapped_and_its_omitted_columns_together() {
    let table = assembled(vec![
        column_row("id", "integer"),
        ColumnRow {
            ordinal_position: 2,
            ..column_row("payload", "json")
        },
    ]);

    assert_eq!(table.columns.len(), 1);
    assert_eq!(table.columns[0].name, "id");
    assert_eq!(
        table.columns[0].mapped.exasol_type,
        MappedType::Decimal {
            precision: 10,
            scale: 0
        }
    );

    assert_eq!(table.omissions.len(), 1);
    assert_eq!(table.omissions[0].table, "orders");
    assert_eq!(table.omissions[0].column.as_deref(), Some("payload"));
    assert_eq!(table.omissions[0].source_type.as_deref(), Some("json"));
}

/// A table every column of which is unmapped is still reported, with no
/// columns: whether such a table is exposed at all is `create_schema`'s rule.
#[test]
fn a_table_with_no_mappable_column_is_reported_with_no_columns() {
    let table = assembled(vec![column_row("payload", "json")]);

    assert!(table.columns.is_empty());
    assert_eq!(table.omissions.len(), 1);
}

/// A table the catalog reported no column row for is still reported, so the
/// table list and the column list cannot disagree about which tables exist.
#[test]
fn a_table_with_no_column_rows_is_reported_with_no_columns() {
    let tables = trino_tables(vec!["empty".to_string()], vec![]);

    assert_eq!(tables.len(), 1);
    assert_eq!(tables[0].name, "empty");
    assert!(tables[0].columns.is_empty());
    assert!(tables[0].omissions.is_empty());
}

/// `information_schema.is_nullable` is the string `YES`, not a boolean, as
/// measured against a live Trino for `vs-adapter/trino-type-mapping`. Only
/// that one spelling means nullable; anything else is read as not nullable.
#[test]
fn nullability_is_read_from_the_catalogs_yes_no_spelling() {
    for (spelling, expected) in [("YES", true), ("NO", false), ("UNKNOWN", false)] {
        assert_eq!(
            is_nullable_cell(spelling),
            expected,
            "the catalog's `{spelling}` must be read as nullable = {expected}"
        );
    }
}

/// Both queries are scoped to the named catalog — Trino scopes
/// `information_schema` to one catalog — and filtered to the named schema,
/// and both base tables and views are exposed.
#[test]
fn both_catalog_queries_qualify_the_named_catalog_and_filter_the_named_schema() {
    let table_names = table_names_query("memory", "adbc_vs");
    let columns = columns_query("memory", "adbc_vs");

    for sql in [&table_names, &columns] {
        assert!(
            sql.contains("\"memory\".information_schema"),
            "the query must be qualified by the named catalog: {sql}"
        );
        assert!(
            sql.contains("table_schema = 'adbc_vs'"),
            "the query must filter to the named schema: {sql}"
        );
    }
    assert!(
        table_names.contains("'BASE TABLE', 'VIEW'"),
        "a view is a queryable relation and is exposed beside a base table: {table_names}"
    );
}

/// Trino's `information_schema.columns` carries no `character_maximum_length`,
/// `numeric_precision`, `numeric_scale`, or `datetime_precision` column at
/// all, and selecting one fails the query outright — gate 2.2 measured this
/// against a live Trino.
#[test]
fn the_columns_query_selects_none_of_the_columns_trinos_catalog_does_not_carry() {
    let columns = columns_query("memory", "adbc_vs");

    for absent in [
        "character_maximum_length",
        "numeric_precision",
        "numeric_scale",
        "datetime_precision",
    ] {
        assert!(
            !columns.contains(absent),
            "Trino's information_schema.columns has no `{absent}` column: {columns}"
        );
    }
    assert!(columns.contains("data_type"));
}

#[test]
fn an_identifier_is_double_quoted_doubling_an_embedded_quote() {
    assert_eq!(quote_identifier("memory"), "\"memory\"");
    assert_eq!(quote_identifier(r#"has"quote"#), "\"has\"\"quote\"");
}

#[test]
fn a_string_literal_is_single_quoted_doubling_an_embedded_quote() {
    assert_eq!(quote_string_literal("adbc_vs"), "'adbc_vs'");
    assert_eq!(quote_string_literal("o'reilly"), "'o''reilly'");
}
