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
fn date_and_timestamp_and_double_render_with_no_arguments() {
    assert_eq!(to_ddl(ExasolType::Date), "DATE");
    assert_eq!(to_ddl(ExasolType::Timestamp), "TIMESTAMP");
    assert_eq!(to_ddl(ExasolType::Double), "DOUBLE");
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
        ExasolType::Timestamp,
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
        ExasolType::Timestamp,
        "selectListDataTypes sends fractionalSecondsPrecision on a TIMESTAMP entry, and the \
         declarable type carries no precision to hold it"
    );
    assert_eq!(
        to_ddl(parsed),
        "TIMESTAMP",
        "the fractional precision is never rendered back: Exasol's own default is 3, which is \
         the only value a virtual table this adapter declared can echo"
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
fn json_with_an_unsupported_type_names_it() {
    let error = from_json(&serde_json::json!({"type": "GEOMETRY"})).unwrap_err();
    assert!(
        error.contains("GEOMETRY"),
        "the error must name the unsupported type it received: {error}"
    );
}
