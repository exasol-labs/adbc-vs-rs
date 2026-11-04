use super::*;

#[test]
fn decimal_renders_as_precision_and_scale() {
    assert_eq!(
        to_ddl(ExasolType::Decimal {
            precision: 19,
            scale: 0
        }),
        "DECIMAL(19,0)"
    );
}

#[test]
fn varchar_renders_with_utf8_charset() {
    assert_eq!(
        to_ddl(ExasolType::Varchar { size: 100 }),
        "VARCHAR(100) UTF8"
    );
}

#[test]
fn char_renders_with_utf8_charset() {
    assert_eq!(to_ddl(ExasolType::Char { size: 8 }), "CHAR(8) UTF8");
}

#[test]
fn date_and_double_render_with_no_arguments() {
    assert_eq!(to_ddl(ExasolType::Date), "DATE");
    assert_eq!(to_ddl(ExasolType::Double), "DOUBLE");
}

#[test]
fn timestamp_precision_round_trips_through_ddl_and_json() {
    for precision in [0, 3, 6, 9] {
        let timestamp = ExasolType::Timestamp { precision };

        assert_eq!(to_ddl(timestamp), format!("TIMESTAMP({precision})"));
        let json = to_json(timestamp);
        assert_eq!(
            json,
            serde_json::json!({
                "type": "TIMESTAMP",
                "withLocalTimeZone": false,
                "fractionalSecondsPrecision": precision
            }),
            "a TIMESTAMP declares its precision and that it carries no local time zone"
        );
        assert_eq!(
            from_json(&json),
            Ok(timestamp),
            "precision {precision} must survive the round trip through {json}"
        );
    }
}

#[test]
fn timestamp_without_precision_reads_as_three() {
    let parsed = from_json(&serde_json::json!({"type": "TIMESTAMP"}))
        .expect("a TIMESTAMP without fractionalSecondsPrecision parses");

    assert_eq!(
        parsed,
        ExasolType::Timestamp { precision: 3 },
        "a dataType without fractionalSecondsPrecision, as a virtual schema declared with a \
         bare TIMESTAMP echoes it, holds Exasol's default precision of 3"
    );
    assert_eq!(to_ddl(parsed), "TIMESTAMP(3)");
}

#[test]
fn timestamp_with_a_non_numeric_precision_fails_naming_the_field() {
    for precision in [
        serde_json::json!("6"),
        serde_json::json!(-1),
        serde_json::Value::Null,
    ] {
        let data_type =
            serde_json::json!({"type": "TIMESTAMP", "fractionalSecondsPrecision": precision});

        let error = from_json(&data_type).expect_err(&format!(
            "{data_type} must fail rather than read as the default precision"
        ));

        assert!(
            error.contains("fractionalSecondsPrecision"),
            "the error must name the unreadable field: {error}"
        );
    }
}

#[test]
fn char_boolean_and_precise_timestamp_entries_read() {
    let cases = [
        (
            serde_json::json!({"type": "CHAR", "size": 8, "characterSet": "UTF8"}),
            ExasolType::Char { size: 8 },
        ),
        (serde_json::json!({"type": "BOOLEAN"}), ExasolType::Boolean),
        (
            serde_json::json!({
                "type": "TIMESTAMP",
                "withLocalTimeZone": false,
                "fractionalSecondsPrecision": 6
            }),
            ExasolType::Timestamp { precision: 6 },
        ),
    ];

    for (data_type, expected) in cases {
        assert_eq!(
            from_json(&data_type),
            Ok(expected),
            "{data_type} is a virtual column's dataType and must read back as {expected:?}"
        );
    }
}

#[test]
fn boolean_renders_with_no_size_and_no_precision() {
    assert_eq!(
        to_ddl(ExasolType::Boolean),
        "BOOLEAN",
        "Exasol's BOOLEAN takes neither a size nor a precision"
    );
    assert_eq!(
        to_json(ExasolType::Boolean),
        serde_json::json!({"type": "BOOLEAN"})
    );
    assert_eq!(
        from_json(&serde_json::json!({"type": "BOOLEAN"})),
        Ok(ExasolType::Boolean),
        "a delegated predicate arrives in selectListDataTypes as {{\"type\":\"BOOLEAN\"}}"
    );
}

#[test]
fn every_declarable_type_round_trips_through_json() {
    let types = [
        ExasolType::Decimal {
            precision: 12,
            scale: 4,
        },
        ExasolType::Varchar { size: 100 },
        ExasolType::Char { size: 8 },
        ExasolType::Date,
        ExasolType::Timestamp { precision: 6 },
        ExasolType::Double,
        ExasolType::Boolean,
    ];

    for exasol_type in types {
        let json = to_json(exasol_type);
        let parsed = from_json(&json).unwrap_or_else(|error| {
            panic!("{exasol_type:?} failed to round-trip through {json}: {error}")
        });
        assert_eq!(parsed, exasol_type, "round trip through {json}");
    }
}

#[test]
fn timestamp_precision_and_absent_character_set_are_read() {
    let timestamp = serde_json::json!({"type": "TIMESTAMP", "fractionalSecondsPrecision": 3});
    let parsed = from_json(&timestamp).expect("a TIMESTAMP carrying a fractional precision parses");

    assert_eq!(
        parsed,
        ExasolType::Timestamp { precision: 3 },
        "selectListDataTypes sends fractionalSecondsPrecision on a TIMESTAMP entry, and the \
         declarable type holds it"
    );
    assert_eq!(
        to_ddl(parsed),
        "TIMESTAMP(3)",
        "the fractional precision is rendered back into the EMITS clause"
    );

    let sized_varchar = serde_json::json!({"type": "VARCHAR", "size": 100});
    assert_eq!(
        from_json(&sized_varchar),
        Ok(ExasolType::Varchar { size: 100 }),
        "selectListDataTypes omits characterSet on a VARCHAR entry, unlike \
         involvedTables[].columns[].dataType, so it MUST NOT be required"
    );
}

#[test]
fn json_without_a_type_field_names_the_missing_field() {
    let error = from_json(&serde_json::json!({"precision": 10})).unwrap_err();
    assert!(
        error.contains("type"),
        "the error must name the missing `type` field: {error}"
    );
}

#[test]
fn every_declarable_type_name_parses() {
    for name in DECLARABLE_TYPE_NAMES {
        let json = match *name {
            "DECIMAL" => serde_json::json!({"type": "DECIMAL", "precision": 10, "scale": 0}),
            "VARCHAR" => serde_json::json!({"type": "VARCHAR", "size": 10}),
            "CHAR" => serde_json::json!({"type": "CHAR", "size": 10}),
            _ => serde_json::json!({"type": name}),
        };
        from_json(&json).unwrap_or_else(|error| {
            panic!("declarable type name `{name}` failed to parse: {error}")
        });
    }
}

#[test]
fn local_time_zone_timestamp_fails_rather_than_reading_as_timestamp() {
    let data_type = serde_json::json!({
        "type": "TIMESTAMP",
        "withLocalTimeZone": true,
        "fractionalSecondsPrecision": 3
    });

    let error = from_json(&data_type).expect_err(
        "a TIMESTAMP WITH LOCAL TIME ZONE compares in the Exasol session zone, so reading it as \
         TIMESTAMP would substitute a type the adapter can render",
    );

    assert!(
        error.contains("TIMESTAMP WITH LOCAL TIME ZONE"),
        "the error must name the type it received: {error}"
    );
}

#[test]
fn timestamp_declaring_no_local_time_zone_reads_as_timestamp() {
    let data_type = serde_json::json!({"type": "TIMESTAMP", "withLocalTimeZone": false});

    assert_eq!(
        from_json(&data_type),
        Ok(ExasolType::Timestamp { precision: 3 })
    );
}

#[test]
fn json_with_an_unsupported_type_names_it() {
    let error = from_json(&serde_json::json!({"type": "GEOMETRY"})).unwrap_err();
    assert!(
        error.contains("GEOMETRY"),
        "the error must name the unsupported type it received: {error}"
    );
}
