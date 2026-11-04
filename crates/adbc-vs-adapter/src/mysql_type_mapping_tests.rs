use super::*;
use crate::exa_type::ExasolType;

fn column(data_type: &str) -> MySqlColumn {
    MySqlColumn {
        data_type: data_type.to_string(),
        character_length: None,
        numeric_precision: None,
        numeric_scale: None,
        datetime_precision: None,
        is_unsigned: false,
        is_nullable: true,
    }
}

fn unsigned(data_type: &str) -> MySqlColumn {
    MySqlColumn {
        is_unsigned: true,
        ..column(data_type)
    }
}

fn decimal_column(precision: u32, scale: u32) -> MySqlColumn {
    MySqlColumn {
        numeric_precision: Some(precision),
        numeric_scale: Some(scale),
        ..column("decimal")
    }
}

fn character_column(data_type: &str, length: u32) -> MySqlColumn {
    MySqlColumn {
        character_length: Some(length),
        ..column(data_type)
    }
}

fn temporal_column(data_type: &str, precision: u32) -> MySqlColumn {
    MySqlColumn {
        datetime_precision: Some(precision),
        ..column(data_type)
    }
}

fn mapped(column: &MySqlColumn) -> ExasolType {
    map_column(column)
        .unwrap_or_else(|reason| panic!("`{}` must map: {reason}", column.data_type))
        .exasol_type
}

fn unmapped(column: &MySqlColumn) -> String {
    map_column(column).expect_err(&format!("`{}` must be unmapped", column.data_type))
}

fn decimal(precision: u32) -> ExasolType {
    ExasolType::Decimal {
        precision,
        scale: 0,
    }
}

/// One column of each MySQL type this mapping maps, for the rules every
/// source's mapping obeys: every Exasol type the mapping produces appears at
/// least once, and so does every conversion its read expressions apply.
pub(crate) fn one_column_of_every_mapped_type() -> Vec<MySqlColumn> {
    vec![
        column("tinyint"),
        column("smallint"),
        column("mediumint"),
        column("int"),
        column("bigint"),
        unsigned("bigint"),
        decimal_column(12, 4),
        decimal_column(40, 2),
        column("float"),
        column("double"),
        column("bit"),
        character_column("char", 10),
        character_column("varchar", 10),
        column("tinytext"),
        column("text"),
        column("mediumtext"),
        column("longtext"),
        character_column("enum", 5),
        character_column("set", 4),
        column("json"),
        column("date"),
        column("year"),
        temporal_column("datetime", 6),
        temporal_column("timestamp", 6),
        temporal_column("time", 3),
    ]
}

#[test]
fn narrow_signed_integers_map_to_decimal_with_matching_digits() {
    for (data_type, precision) in [
        ("tinyint", 3),
        ("smallint", 5),
        ("mediumint", 7),
        ("int", 10),
        ("bigint", 19),
    ] {
        assert_eq!(
            mapped(&column(data_type)),
            decimal(precision),
            "{data_type}"
        );
        assert_eq!(
            mapped(&column(&data_type.to_ascii_uppercase())),
            decimal(precision),
            "{data_type} spelled in upper case"
        );
    }
}

#[test]
fn unsigned_integers_map_to_decimal_with_unsigned_digits() {
    for (data_type, precision) in [
        ("tinyint", 3),
        ("smallint", 5),
        ("mediumint", 8),
        ("int", 10),
        ("bigint", 20),
    ] {
        assert_eq!(
            mapped(&unsigned(data_type)),
            decimal(precision),
            "{data_type}"
        );
    }
}

#[test]
fn decimal_preserves_precision_and_scale() {
    assert_eq!(
        mapped(&decimal_column(12, 4)),
        ExasolType::Decimal {
            precision: 12,
            scale: 4
        }
    );
    assert_eq!(
        mapped(&decimal_column(36, 2)),
        ExasolType::Decimal {
            precision: 36,
            scale: 2
        },
        "Exasol's maximum precision itself must map to DECIMAL"
    );
}

#[test]
fn decimal_beyond_precision_36_maps_to_varchar_read_as_text() {
    let wide = decimal_column(40, 2);

    let declared = mapped(&wide);

    assert_eq!(declared, ExasolType::Varchar { size: 2_000_000 });
    assert_eq!(
        column_read_sql(Some("decimal"), declared, "`c`"),
        "CAST(`c` AS CHAR)"
    );
}

#[test]
fn a_decimal_without_a_declared_precision_is_unmapped() {
    let reason = unmapped(&column("decimal"));

    assert!(
        reason.contains("precision"),
        "the reason must name the missing precision: {reason}"
    );
}

#[test]
fn character_types_preserve_the_declared_length() {
    assert_eq!(
        mapped(&character_column("varchar", 100)),
        ExasolType::Varchar { size: 100 }
    );
    assert_eq!(
        mapped(&character_column("char", 10)),
        ExasolType::Char { size: 10 },
        "MySQL CHAR maps to Exasol CHAR, as the JDBC Virtual Schema maps it"
    );
    assert_eq!(
        mapped(&character_column("varchar", 0)),
        ExasolType::Varchar { size: 1 },
        "Exasol's minimum character-type size is 1"
    );
    assert_eq!(
        mapped(&character_column("char", 0)),
        ExasolType::Char { size: 1 },
        "Exasol rejects CHAR(0), so a length-0 CHAR maps to size 1"
    );
}

#[test]
fn a_character_column_without_a_declared_length_is_unmapped() {
    for data_type in ["varchar", "char", "enum", "set"] {
        let reason = unmapped(&column(data_type));

        assert!(
            reason.contains("CHARACTER_MAXIMUM_LENGTH"),
            "{data_type}: the reason must name the missing CHARACTER_MAXIMUM_LENGTH: {reason}"
        );
    }
}

/// The catalog row carries no character set, so a `latin1` column and a
/// `utf8mb4` one are the same input to the mapping and declare the same type.
#[test]
fn character_columns_map_whatever_their_character_set() {
    assert_eq!(
        mapped(&character_column("varchar", 50)),
        ExasolType::Varchar { size: 50 }
    );
    assert_eq!(
        mapped(&character_column("char", 50)),
        ExasolType::Char { size: 50 }
    );
    assert_eq!(
        mapped(&character_column("text", 65_535)),
        ExasolType::Varchar { size: 65_535 }
    );
}

#[test]
fn text_types_map_to_varchar_bounded_by_their_storage_limit() {
    for (data_type, catalog_length, size) in [
        ("tinytext", 255, 255),
        ("text", 65_535, 65_535),
        ("mediumtext", 16_777_215, 2_000_000),
        ("longtext", 4_294_967_295, 2_000_000),
    ] {
        assert_eq!(
            mapped(&character_column(data_type, catalog_length)),
            ExasolType::Varchar { size },
            "{data_type}"
        );
        assert_eq!(
            mapped(&column(data_type)),
            ExasolType::Varchar { size },
            "{data_type} with no catalog length"
        );
    }
}

#[test]
fn bit_of_every_width_maps_to_boolean() {
    for width in [1, 8, 64] {
        let bit = character_column("bit", width);
        assert_eq!(mapped(&bit), ExasolType::Boolean, "BIT({width})");
        assert_eq!(
            column_read_sql(Some("bit"), ExasolType::Boolean, "`c`"),
            "(`c` <> 0)"
        );
    }
}

#[test]
fn enum_and_set_map_to_char_up_to_2000() {
    assert_eq!(
        mapped(&character_column("enum", 5)),
        ExasolType::Char { size: 5 }
    );
    assert_eq!(
        mapped(&character_column("set", 4)),
        ExasolType::Char { size: 4 }
    );
    assert_eq!(
        mapped(&character_column("set", 2000)),
        ExasolType::Char { size: 2000 },
        "2000 is Exasol's maximum CHAR size"
    );
    assert_eq!(
        mapped(&character_column("set", 2001)),
        ExasolType::Varchar { size: 2001 },
        "above 2000 a SET maps to VARCHAR of the same length"
    );
}

#[test]
fn json_maps_to_varchar_read_as_text() {
    let declared = mapped(&column("json"));

    assert_eq!(declared, ExasolType::Varchar { size: 2_000_000 });
    assert_eq!(
        column_read_sql(Some("json"), declared, "`c`"),
        "CAST(`c` AS CHAR)"
    );
}

#[test]
fn temporal_and_floating_point_types_map_to_their_exasol_counterparts() {
    assert_eq!(mapped(&column("date")), ExasolType::Date);
    assert_eq!(
        mapped(&temporal_column("datetime", 0)),
        ExasolType::Timestamp { precision: 0 }
    );
    assert_eq!(
        mapped(&temporal_column("datetime", 6)),
        ExasolType::Timestamp { precision: 6 }
    );
    assert_eq!(mapped(&column("double")), ExasolType::Double);
    assert_eq!(mapped(&column("float")), ExasolType::Double);
}

#[test]
fn a_datetime_without_a_reported_precision_declares_the_most_mysql_holds() {
    assert_eq!(
        mapped(&column("datetime")),
        ExasolType::Timestamp { precision: 6 }
    );
}

#[test]
fn timestamp_maps_to_timestamp_of_its_precision() {
    for precision in [0, 3, 6] {
        let timestamp = temporal_column("timestamp", precision);

        assert_eq!(mapped(&timestamp), ExasolType::Timestamp { precision });
        assert_eq!(
            column_read_sql(
                Some("timestamp"),
                ExasolType::Timestamp { precision },
                "`c`"
            ),
            "`c`",
            "a TIMESTAMP reads bare, in the session time zone"
        );
    }
}

#[test]
fn time_maps_to_a_timestamp_of_its_precision_and_year_to_a_date() {
    assert_eq!(
        mapped(&temporal_column("time", 3)),
        ExasolType::Timestamp { precision: 3 }
    );
    assert_eq!(mapped(&column("year")), ExasolType::Date);
}

#[test]
fn column_read_sql_follows_the_source_type() {
    let varchar = ExasolType::Varchar { size: 2_000_000 };
    let timestamp = ExasolType::Timestamp { precision: 3 };

    for (source_type, declared, expected) in [
        ("time", timestamp, "TIMESTAMP(DATE '1970-01-01', `c`)"),
        ("year", ExasolType::Date, "MAKEDATE(`c`, 1)"),
        ("bit", ExasolType::Boolean, "(`c` <> 0)"),
        ("json", varchar, "CAST(`c` AS CHAR)"),
        ("decimal", varchar, "CAST(`c` AS CHAR)"),
        ("TIME", timestamp, "TIMESTAMP(DATE '1970-01-01', `c`)"),
        ("decimal", decimal(12), "`c`"),
        ("varchar", ExasolType::Varchar { size: 10 }, "`c`"),
        ("char", ExasolType::Char { size: 10 }, "`c`"),
        ("enum", ExasolType::Char { size: 5 }, "`c`"),
        ("text", varchar, "`c`"),
        ("datetime", timestamp, "`c`"),
        ("timestamp", timestamp, "`c`"),
        ("float", ExasolType::Double, "`c`"),
        ("bigint", decimal(20), "`c`"),
    ] {
        assert_eq!(
            column_read_sql(Some(source_type), declared, "`c`"),
            expected,
            "for {source_type} declared {declared:?}"
        );
    }
    assert_eq!(column_read_sql(None, varchar, "`c`"), "`c`");
}

#[test]
fn non_core_types_unmapped() {
    for mysql_type in [
        "BINARY",
        "VARBINARY",
        "TINYBLOB",
        "BLOB",
        "MEDIUMBLOB",
        "LONGBLOB",
        "GEOMETRY",
        "POINT",
    ] {
        let reason = unmapped(&column(mysql_type));

        assert!(
            reason.contains(mysql_type),
            "reason for {mysql_type} must name the rejected type, got: {reason}"
        );
    }
}

#[test]
fn nullability_carries_through() {
    let mut not_null_column = column("INT");
    not_null_column.is_nullable = false;
    let mapped = map_column(&not_null_column).expect("INT must map");
    assert!(!mapped.is_nullable);

    let mut nullable_column = column("INT");
    nullable_column.is_nullable = true;
    let mapped = map_column(&nullable_column).expect("INT must map");
    assert!(mapped.is_nullable);
}
