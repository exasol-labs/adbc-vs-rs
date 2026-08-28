use super::*;
use crate::exa_type::ExasolType;

fn column(data_type: &str) -> MySqlColumn {
    MySqlColumn {
        data_type: data_type.to_string(),
        character_length: None,
        character_set: None,
        numeric_precision: None,
        numeric_scale: None,
        is_unsigned: false,
        is_nullable: true,
    }
}

#[test]
fn signed_integers_map_to_decimal() {
    let int_column = column("INT");
    let mapped = map_column(&int_column).expect("INT must map");
    assert_eq!(
        mapped.exasol_type,
        MappedType::Decimal {
            precision: 10,
            scale: 0
        }
    );

    let bigint_column = column("BIGINT");
    let mapped = map_column(&bigint_column).expect("BIGINT must map");
    assert_eq!(
        mapped.exasol_type,
        MappedType::Decimal {
            precision: 19,
            scale: 0
        }
    );
}

#[test]
fn decimal_preserves_precision_and_scale() {
    let mut decimal_column = column("DECIMAL");
    decimal_column.numeric_precision = Some(12);
    decimal_column.numeric_scale = Some(4);

    let mapped = map_column(&decimal_column).expect("DECIMAL(12,4) must map");
    assert_eq!(
        mapped.exasol_type,
        MappedType::Decimal {
            precision: 12,
            scale: 4
        }
    );
}

#[test]
fn oversize_decimal_unmapped() {
    let mut decimal_column = column("DECIMAL");
    decimal_column.numeric_precision = Some(40);
    decimal_column.numeric_scale = Some(2);

    let reason = map_column(&decimal_column).expect_err("DECIMAL(40,2) must be unmapped");
    assert!(
        reason.contains("36"),
        "reason must name Exasol's maximum DECIMAL precision of 36, got: {reason}"
    );
}

fn utf8mb4_column(data_type: &str) -> MySqlColumn {
    let mut column = column(data_type);
    column.character_set = Some("utf8mb4".to_string());
    column
}

#[test]
fn character_types_preserve_length() {
    let mut varchar_column = utf8mb4_column("VARCHAR");
    varchar_column.character_length = Some(100);
    let mapped = map_column(&varchar_column).expect("VARCHAR(100) must map");
    assert_eq!(mapped.exasol_type, MappedType::Varchar { size: 100 });

    let mut char_column = utf8mb4_column("CHAR");
    char_column.character_length = Some(10);
    let mapped = map_column(&char_column).expect("CHAR(10) must map");
    assert_eq!(
        mapped.exasol_type,
        MappedType::Varchar { size: 10 },
        "MySQL CHAR must map to Exasol VARCHAR, never Exasol CHAR, because Exasol blank-pads \
         CHAR and MySQL does not pad a CHAR operand under the collations pushdown renders"
    );

    let mut zero_length_column = utf8mb4_column("VARCHAR");
    zero_length_column.character_length = Some(0);
    let mapped = map_column(&zero_length_column).expect("VARCHAR(0) must map to size 1");
    assert_eq!(mapped.exasol_type, MappedType::Varchar { size: 1 });
}

#[test]
fn a_character_column_without_a_declared_length_is_unmapped() {
    let varchar_column = utf8mb4_column("VARCHAR");

    let reason =
        map_column(&varchar_column).expect_err("VARCHAR with no declared length must be unmapped");
    assert!(
        reason.contains("CHARACTER_MAXIMUM_LENGTH"),
        "reason must name the missing CHARACTER_MAXIMUM_LENGTH, got: {reason}"
    );
}

#[test]
fn a_character_column_reporting_no_character_set_is_unmapped() {
    let mut column = column("VARCHAR");
    column.character_length = Some(50);
    column.character_set = None;

    let reason = map_column(&column)
        .expect_err("a VARCHAR reporting no CHARACTER_SET_NAME must be unmapped");
    assert!(
        reason.contains("CHARACTER_SET_NAME"),
        "reason must name the missing CHARACTER_SET_NAME field, got: {reason}"
    );
}

#[test]
fn non_utf8mb4_character_columns_unmapped() {
    for mysql_type in ["VARCHAR", "CHAR"] {
        let mut latin1_column = column(mysql_type);
        latin1_column.character_length = Some(50);
        latin1_column.character_set = Some("latin1".to_string());

        let reason = map_column(&latin1_column).expect_err(&format!(
            "{mysql_type} of character set latin1 must be unmapped"
        ));
        assert!(
            reason.contains("latin1"),
            "reason must name the column's character set latin1, got: {reason}"
        );
        assert!(
            reason.contains("utf8mb4"),
            "reason must name utf8mb4 as the only character set this release maps, got: {reason}"
        );
    }
}

#[test]
fn temporal_and_double_map() {
    assert_eq!(
        map_column(&column("DATE"))
            .expect("DATE must map")
            .exasol_type,
        MappedType::Date
    );
    assert_eq!(
        map_column(&column("DATETIME"))
            .expect("DATETIME must map")
            .exasol_type,
        MappedType::Timestamp
    );
    assert_eq!(
        map_column(&column("DOUBLE"))
            .expect("DOUBLE must map")
            .exasol_type,
        MappedType::Double
    );
}

#[test]
fn unsigned_integers_unmapped() {
    let mut unsigned_bigint = column("BIGINT");
    unsigned_bigint.is_unsigned = true;

    let reason = map_column(&unsigned_bigint).expect_err("BIGINT UNSIGNED must be unmapped");
    assert!(
        reason.to_lowercase().contains("unsigned"),
        "reason must identify the unsigned modifier as unsupported, got: {reason}"
    );
}

#[test]
fn non_core_types_unmapped() {
    let unmapped_types = [
        "JSON",
        "TEXT",
        "BLOB",
        "ENUM",
        "SET",
        "TINYINT",
        "SMALLINT",
        "MEDIUMINT",
        "FLOAT",
        "TIME",
        "YEAR",
        "TIMESTAMP",
        "GEOMETRY",
    ];

    for mysql_type in unmapped_types {
        let reason =
            map_column(&column(mysql_type)).expect_err(&format!("{mysql_type} must be unmapped"));
        assert!(
            reason.contains(mysql_type),
            "reason for {mysql_type} must name the rejected type, got: {reason}"
        );
    }
}

#[test]
fn every_mapped_type_widens_into_its_declarable_counterpart() {
    let cases = [
        (
            MappedType::Decimal {
                precision: 12,
                scale: 4,
            },
            ExasolType::Decimal {
                precision: 12,
                scale: 4,
            },
        ),
        (
            MappedType::Varchar { size: 100 },
            ExasolType::Varchar { size: 100 },
        ),
        (MappedType::Date, ExasolType::Date),
        (MappedType::Timestamp, ExasolType::Timestamp),
        (MappedType::Double, ExasolType::Double),
    ];

    for (mapped, expected) in cases {
        assert_eq!(
            ExasolType::from(mapped),
            expected,
            "{mapped:?} must widen into the declarable set unchanged"
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
