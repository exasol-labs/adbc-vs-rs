use super::*;
use crate::metadata::DiscoveredTable;
use crate::type_mapping::MappedType;

/// One `information_schema.columns` row, with the fields a test does not care
/// about set to what an integer column reports.
fn column_row(column: &str, data_type: &str, column_type: &str) -> ColumnRow {
    ColumnRow {
        table: "t".to_string(),
        column: column.to_string(),
        ordinal_position: 1,
        data_type: data_type.to_string(),
        column_type: column_type.to_string(),
        character_maximum_length: None,
        character_set: None,
        numeric_precision: Some(10),
        numeric_scale: Some(0),
        is_nullable: true,
    }
}

fn assembled(rows: Vec<ColumnRow>) -> DiscoveredTable {
    mysql_tables(vec!["t".to_string()], rows)
        .pop()
        .expect("the named table is always reported")
}

/// The framework assembler driven through MySQL's own row shape and mapping,
/// exactly as [`MySqlSource::discover_tables`] drives it.
fn mysql_tables(table_names: Vec<String>, rows: Vec<ColumnRow>) -> Vec<DiscoveredTable> {
    assemble_tables(table_names, rows, |row| {
        mysql_type_mapping::map_column(&mysql_column(row))
    })
}

/// The MySQL catalog fixture the rules binding every source are driven
/// through: one `orders` table holding a column that maps and a column of
/// MySQL's own that does not.
///
/// It is assembled here, out of this source's own row shape and through this
/// source's own mapping, because that is exactly what `metadata_source_tests`
/// asserts of every source — a fixture built anywhere else would need a row
/// struct shared between two catalogs, which is the thing the interface exists
/// to keep out.
pub(crate) fn one_mapped_and_one_unmapped_column() -> Vec<DiscoveredTable> {
    mysql_tables(
        vec!["orders".to_string()],
        vec![
            ColumnRow {
                table: "orders".to_string(),
                is_nullable: false,
                ..column_row("id", "int", "int(10)")
            },
            ColumnRow {
                table: "orders".to_string(),
                ordinal_position: 2,
                ..column_row("payload", "json", "json")
            },
        ],
    )
}

/// `UNSIGNED` is spelled only in `COLUMN_TYPE`'s free text, and an unsigned
/// integer is unmapped, so reading it wrongly would expose a column whose
/// range the declared `DECIMAL` cannot hold.
#[test]
fn unsigned_is_read_from_the_column_type_text() {
    let table = assembled(vec![column_row("c", "int", "int(10) unsigned")]);

    assert!(table.columns.is_empty(), "an unsigned INT does not map");
    assert_eq!(table.omissions.len(), 1);
    assert_eq!(table.omissions[0].column.as_deref(), Some("c"));
}

#[test]
fn signed_column_type_is_not_reported_unsigned() {
    let table = assembled(vec![column_row("c", "int", "int(10)")]);

    assert!(table.omissions.is_empty(), "a signed INT maps");
    assert_eq!(
        table.columns[0].mapped.exasol_type,
        MappedType::Decimal {
            precision: 10,
            scale: 0
        }
    );
}

#[test]
fn character_set_is_read_from_the_row_the_column_maps_from() {
    let mut row = column_row("name", "varchar", "varchar(100)");
    row.character_maximum_length = Some(100);
    row.character_set = Some("utf8mb4".to_string());
    row.numeric_precision = None;
    row.numeric_scale = None;

    let table = assembled(vec![row]);

    assert_eq!(
        table.columns[0].mapped.exasol_type,
        MappedType::Varchar { size: 100 }
    );
}

/// An omission records `DATA_TYPE`, the name an operator reads in their own
/// catalog, rather than `COLUMN_TYPE`'s declaration text.
#[test]
fn a_character_column_of_another_character_set_is_omitted_naming_its_type() {
    let mut row = column_row("name", "varchar", "varchar(100)");
    row.character_maximum_length = Some(100);
    row.character_set = Some("latin1".to_string());

    let table = assembled(vec![row]);

    assert!(table.columns.is_empty());
    assert_eq!(table.omissions[0].source_type.as_deref(), Some("varchar"));
    assert!(
        table.omissions[0].reason.contains("latin1"),
        "the recorded reason must name what disqualified the column: {}",
        table.omissions[0].reason
    );
}

/// The guard belongs to MySQL rather than to the schema that resolved the
/// name: it exists only because [`quote_string_literal`] interpolates the
/// database into MySQL's own `information_schema` queries, and a source that
/// interpolates nothing has no such name to refuse.
#[test]
fn a_database_name_containing_a_single_quote_is_refused() {
    let error = reject_unsafe_database_name("sho'p")
        .expect_err("a name MySQL's escaper cannot safely quote must be refused");

    let message = error.to_string();
    assert!(
        matches!(
            error,
            MetadataError::UnsafeDatabaseName {
                character: '\'',
                ..
            }
        ),
        "expected the unsafe-name refusal naming the quote, got: {message}"
    );
    assert!(
        message.contains("sho'p"),
        "the error must name the database it was given: {message}"
    );
}

#[test]
fn a_database_name_containing_a_backslash_is_refused() {
    let error = reject_unsafe_database_name("sho\\p")
        .expect_err("a name MySQL's escaper cannot safely quote must be refused");

    assert!(
        matches!(
            error,
            MetadataError::UnsafeDatabaseName {
                character: '\\',
                ..
            }
        ),
        "expected the unsafe-name refusal naming the backslash, got: {error}"
    );
}

#[test]
fn an_ordinary_database_name_is_accepted() {
    reject_unsafe_database_name("shop").expect("a name carrying neither character must be used");
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
