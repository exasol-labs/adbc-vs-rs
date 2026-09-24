use super::*;

fn column(data_type: &str) -> TrinoColumn {
    TrinoColumn {
        data_type: data_type.to_string(),
        is_nullable: true,
    }
}

fn mapped(data_type: &str) -> MappedType {
    map_column(&column(data_type))
        .unwrap_or_else(|reason| panic!("`{data_type}` must map: {reason}"))
        .exasol_type
}

fn unmapped(data_type: &str) -> String {
    match map_column(&column(data_type)) {
        Ok(mapped) => panic!(
            "`{data_type}` must not map, but mapped to {:?}",
            mapped.exasol_type
        ),
        Err(reason) => reason,
    }
}

/// A `data_type` spelling the parser does not recognize is unmapped with the
/// reason naming the value exactly as the catalog spelled it, rather than
/// being read as the bare name that precedes its parameters.
#[test]
fn an_unrecognized_data_type_is_named_in_full_rather_than_truncated_to_its_bare_name() {
    let reason = unmapped("foo(3)");
    assert!(
        reason.contains("foo(3)"),
        "the reason must name the full spelling, not just `foo`: {reason}"
    );
}

/// A `decimal`'s comma carries no space, while a `map`'s carries one — both
/// spellings were measured on a live Trino for `vs-adapter/trino-type-mapping`,
/// and the parser must not treat one rule as covering both. That `map` names
/// itself in its omission reason is the non-core-type loop's claim, not this
/// one's: here it only must not parse as a decimal's precision and scale.
#[test]
fn decimal_and_map_disagree_on_whether_the_comma_carries_a_space() {
    assert_eq!(
        mapped("decimal(30,2)"),
        MappedType::Decimal {
            precision: 30,
            scale: 2
        }
    );

    unmapped("map(varchar, integer)");
}

/// `vs-adapter/trino-type-mapping`, scenario "Integer types map to DECIMAL
/// with the matching digit count": the digit counts equal the ones MySQL's
/// and PostgreSQL's mappings produce for a 32-bit and a 64-bit integer.
#[test]
fn integer_types_map_to_decimal_with_the_matching_digit_count() {
    assert_eq!(
        mapped("integer"),
        MappedType::Decimal {
            precision: 10,
            scale: 0
        }
    );
    assert_eq!(
        mapped("bigint"),
        MappedType::Decimal {
            precision: 19,
            scale: 0
        }
    );
}

/// `vs-adapter/trino-type-mapping`, scenario "decimal preserves the source
/// precision and scale".
#[test]
fn decimal_preserves_the_source_precision_and_scale() {
    assert_eq!(
        mapped("decimal(12,4)"),
        MappedType::Decimal {
            precision: 12,
            scale: 4
        }
    );

    let reason = unmapped("decimal(38,2)");
    assert!(
        reason.contains("36"),
        "the reason must name Exasol's maximum DECIMAL precision of 36, not Trino's own maximum \
         of 38: {reason}"
    );
}

/// `vs-adapter/trino-type-mapping`, scenario "varchar preserves the declared
/// length and uses UTF8".
#[test]
fn varchar_preserves_the_declared_length() {
    assert_eq!(mapped("varchar(100)"), MappedType::Varchar { size: 100 });
    assert_eq!(mapped("varchar(0)"), MappedType::Varchar { size: 1 });

    let unbounded = unmapped("varchar");
    assert!(
        unbounded.contains("varchar"),
        "an unparenthesized varchar is Trino's unbounded varchar, not a zero-length one: \
         {unbounded}"
    );

    let too_long = unmapped("varchar(2000001)");
    assert!(
        too_long.contains("2000000") || too_long.contains("2,000,000"),
        "the reason must name Exasol's maximum VARCHAR size: {too_long}"
    );
}

/// `vs-adapter/trino-type-mapping`, scenario "char is unmapped because it is
/// blank-padded" — gate 2.2 confirmed the LIKE/`=` disagreement is real, so no
/// spec correction was needed.
#[test]
fn char_is_unmapped_naming_the_blank_padding() {
    let reason = unmapped("char(5)");
    assert!(
        reason.to_ascii_lowercase().contains("pad"),
        "the reason must name the blank padding as the cause: {reason}"
    );
}

/// `vs-adapter/trino-type-mapping`, scenario "Temporal and floating-point
/// types map to their Exasol counterparts".
#[test]
fn temporal_and_floating_point_types_map_to_their_exasol_counterparts() {
    assert_eq!(mapped("date"), MappedType::Date);
    assert_eq!(mapped("timestamp(6)"), MappedType::Timestamp);
    for precision in ["timestamp(0)", "timestamp(3)", "timestamp(9)"] {
        assert_eq!(
            mapped(precision),
            MappedType::Timestamp,
            "every timestamp precision Trino admits must map to Exasol TIMESTAMP"
        );
    }
    assert_eq!(mapped("double"), MappedType::Double);
}

/// `vs-adapter/trino-type-mapping`, scenario "Non-core types are unmapped" —
/// the exact spellings gate 2.2 measured against Trino 483.
#[test]
fn non_core_types_are_unmapped_naming_the_rejected_type() {
    for data_type in [
        "real",
        "smallint",
        "tinyint",
        "time(3)",
        "time(3) with time zone",
        "varbinary",
        "json",
        "uuid",
        "ipaddress",
        "array(integer)",
        "map(varchar, integer)",
        r#"row("a" integer)"#,
        "interval day to second",
        "interval year to month",
    ] {
        let reason = unmapped(data_type);
        assert!(
            reason.contains(data_type),
            "the reason for `{data_type}` must name the rejected type: {reason}"
        );
    }
}

/// `boolean` is unmapped despite Exasol having a `BOOLEAN` type, because a
/// mapped column would widen the set of Exasol types a source column
/// produces.
#[test]
fn boolean_is_unmapped_despite_exasol_having_a_boolean_type() {
    let reason = unmapped("boolean");
    assert!(
        reason.contains("boolean"),
        "the reason must name the rejected type: {reason}"
    );
    assert!(
        reason.to_ascii_lowercase().contains("widen"),
        "the reason must explain that a mapped BOOLEAN would widen the declarable set: {reason}"
    );
}

/// `timestamp(p) with time zone` is unmapped rather than mapped to Exasol
/// `TIMESTAMP`, because the two carry different instants for one wall-clock
/// value — every precision gate 2.2 measured.
#[test]
fn timestamp_with_time_zone_is_unmapped_naming_the_instant_disagreement() {
    for data_type in ["timestamp(3) with time zone", "timestamp(0) with time zone"] {
        let reason = unmapped(data_type);
        assert!(
            reason.to_ascii_lowercase().contains("instant"),
            "the reason for `{data_type}` must name the differing instants: {reason}"
        );
    }
}
