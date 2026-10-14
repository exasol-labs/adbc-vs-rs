use super::*;
use crate::type_mapping::MappedType;

/// One `information_schema.columns` row, with the fields a test does not care
/// about set to what the first column of a table reports.
fn column_row(column: &str, data_type: &str) -> ColumnRow {
    ColumnRow {
        table: "orders".to_string(),
        column: column.to_string(),
        ordinal_position: 1,
        data_type: data_type.to_string(),
        character_maximum_length: None,
        numeric_precision: None,
        numeric_scale: None,
        is_nullable: true,
    }
}

fn assembled(rows: Vec<ColumnRow>) -> DiscoveredTable {
    postgres_tables(vec!["orders".to_string()], rows)
        .pop()
        .expect("the named table is always reported")
}

/// The framework assembler driven through PostgreSQL's own row shape and
/// mapping, exactly as [`PostgresSource::discover_tables`] drives it.
fn postgres_tables(table_names: Vec<String>, rows: Vec<ColumnRow>) -> Vec<DiscoveredTable> {
    assemble_tables(table_names, rows, |row| {
        postgres_type_mapping::map_column(&postgres_column(row))
    })
}

/// The PostgreSQL catalog fixture the rules binding every source are driven
/// through: one `orders` table holding a column that maps and a column of
/// PostgreSQL's own that does not.
///
/// It is assembled here, out of this source's own row shape and by this
/// source's own assembler, because that is exactly what
/// `metadata_source_tests` asserts of every source — a fixture built anywhere
/// else would need a row struct shared between two catalogs, which is the
/// thing the interface exists to keep out.
pub(crate) fn one_mapped_and_one_unmapped_column() -> Vec<DiscoveredTable> {
    postgres_tables(
        vec!["orders".to_string()],
        vec![
            ColumnRow {
                is_nullable: false,
                ..column_row("id", "integer")
            },
            ColumnRow {
                ordinal_position: 2,
                ..column_row("payload", "jsonb")
            },
        ],
    )
}

/// `vs-adapter/create-virtual-schema`: a table's columns are exposed in the
/// source's own ordinal order, whatever order the catalog rows arrived in,
/// because Exasol declares the virtual table from that order.
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
/// the ones that do not, so neither half is lost to the other, and the
/// omission carries the source's own spelling of the type it could not map.
#[test]
fn a_table_reports_its_mapped_and_its_omitted_columns_together() {
    let table = assembled(vec![
        column_row("id", "integer"),
        ColumnRow {
            ordinal_position: 2,
            ..column_row("payload", "jsonb")
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
    assert_eq!(table.omissions[0].source_type.as_deref(), Some("jsonb"));
}

/// A table every column of which is unmapped is still reported, with no
/// columns: whether such a table is exposed at all is `create_schema`'s rule,
/// not this source's, and answering it here would decide it twice.
#[test]
fn a_table_with_no_mappable_column_is_reported_with_no_columns() {
    let table = assembled(vec![column_row("payload", "jsonb")]);

    assert!(table.columns.is_empty());
    assert_eq!(table.omissions.len(), 1);
}

/// A table the catalog reported no column row for is still reported, so the
/// table list and the column list cannot disagree about which tables exist.
#[test]
fn a_table_with_no_column_rows_is_reported_with_no_columns() {
    let tables = postgres_tables(vec!["empty".to_string()], vec![]);

    assert_eq!(tables.len(), 1);
    assert_eq!(tables[0].name, "empty");
    assert!(tables[0].columns.is_empty());
    assert!(tables[0].omissions.is_empty());
}

/// `vs-adapter/postgresql-type-mapping`: `information_schema` reports `YES`
/// and `NO` rather than a boolean, and reading anything but `YES` as
/// not-nullable is what makes a column Exasol declares `NOT NULL` actually
/// non-null at the source.
#[test]
fn nullability_is_read_from_the_catalogs_yes_no_spelling() {
    for (spelling, expected) in [("YES", true), ("NO", false)] {
        let table = assembled(vec![ColumnRow {
            is_nullable: spelling == NULLABLE,
            ..column_row("id", "integer")
        }]);

        assert_eq!(table.columns[0].mapped.is_nullable, expected);
    }
}

/// `vs-adapter/postgresql-type-mapping`: the namespace is `public` and is not
/// derived from the connection address, because a PostgreSQL database is not a
/// SQL qualifier. It is what `adapterNotes` records and what the dialect
/// qualifies every rendered table with.
///
/// Both catalog queries filter on `public` and neither interpolates a value
/// resolved from a `CONNECTION` object, so no source-supplied name can change
/// their shape and neither query needs a string escaper.
#[test]
fn both_catalog_queries_filter_on_the_exposed_namespace() {
    let table_names = table_names_query();
    let columns = columns_query();

    for sql in [&table_names, &columns] {
        assert!(
            sql.contains("table_schema = 'public'"),
            "the query must filter to the exposed namespace: {sql}"
        );
    }
    assert!(
        table_names.contains("'BASE TABLE', 'VIEW'"),
        "a view is a queryable relation and is exposed beside a base table: {table_names}"
    );
    assert!(
        !columns.contains("column_type"),
        "PostgreSQL has no `COLUMN_TYPE` column; reusing MySQL's query text would fail: {columns}"
    );
}

/// Every field `postgres_type_mapping` reads has to be selected, or a column
/// that maps is silently omitted for want of a length, a precision, or a scale
/// the query never asked for.
#[test]
fn the_columns_query_selects_every_field_the_mapping_reads() {
    let columns = columns_query();

    for field in [
        "character_maximum_length",
        "numeric_precision",
        "numeric_scale",
        "is_nullable",
    ] {
        assert!(
            columns.contains(field),
            "the query must select `{field}`: {columns}"
        );
    }
}

/// `vs-adapter/postgresql-type-mapping`: the catalog's precision and scale
/// reach the mapping, so a `numeric(12,4)` is declared `DECIMAL(12,4)` rather
/// than omitted for want of a precision the query never read.
#[test]
fn a_numeric_columns_precision_and_scale_reach_the_mapping() {
    let table = assembled(vec![ColumnRow {
        numeric_precision: Some(12),
        numeric_scale: Some(4),
        ..column_row("amount", "numeric")
    }]);

    assert_eq!(
        table.columns[0].mapped.exasol_type,
        MappedType::Decimal {
            precision: 12,
            scale: 4
        }
    );
}

/// The declared length reaches the mapping, so a `character varying(100)` is
/// declared `VARCHAR(100)`.
#[test]
fn a_character_columns_length_reaches_the_mapping() {
    let table = assembled(vec![ColumnRow {
        character_maximum_length: Some(100),
        ..column_row("name", "character varying")
    }]);

    assert_eq!(
        table.columns[0].mapped.exasol_type,
        MappedType::Varchar { size: 100 }
    );
}

/// `vs-adapter/postgresql-type-mapping`: the mapping declares every character
/// column `UTF8`, so a database of another encoding is refused rather than
/// exposed. The error names both encodings, because an operator can act on
/// neither alone.
#[test]
fn a_database_of_another_encoding_is_refused_naming_it_and_utf8() {
    let Err(error) = check_server_encoding("LATIN1") else {
        panic!("a non-UTF8 database must be refused");
    };

    let message = error.to_string();
    assert!(
        message.contains("LATIN1") && message.contains(REQUIRED_ENCODING),
        "the error must name the reported encoding and the required one: {message}"
    );
    assert!(
        matches!(error, MetadataError::UnsupportedSource(_)),
        "a source the rendering cannot be used against is answered by changing the server, \
         never by retrying: {error:?}"
    );
}

/// `UTF8` is the one encoding accepted, whatever case the server spelled it
/// in: the value is a server-reported name rather than an operator-typed one,
/// so a differing case is a spelling difference and not a different encoding.
#[test]
fn a_utf8_database_is_accepted() {
    for spelling in ["UTF8", "utf8"] {
        check_server_encoding(spelling)
            .unwrap_or_else(|error| panic!("`{spelling}` must be accepted: {error}"));
    }
}

/// A catalog value that cannot be a length, a precision, or a scale is read as
/// absent rather than wrapped into a plausible one, so the column is omitted
/// with a reason instead of declared at a size the source never reported.
#[test]
fn a_length_or_precision_outside_u32_is_read_as_absent() {
    let table = assembled(vec![ColumnRow {
        character_maximum_length: Some(-1),
        ..column_row("name", "character varying")
    }]);

    assert!(table.columns.is_empty());
    assert!(table.omissions[0].reason.contains("length"));
}
