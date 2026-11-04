use super::*;

/// A column of `data_type` carrying none of the catalog's optional fields:
/// what `information_schema` reports for a type that declares no length, no
/// precision.
fn column(data_type: &str) -> PostgresColumn {
    PostgresColumn {
        data_type: data_type.to_string(),
        character_length: None,
        numeric_precision: None,
        numeric_scale: None,
        datetime_precision: None,
        is_nullable: true,
    }
}

fn numeric_column(precision: Option<u32>, scale: Option<u32>) -> PostgresColumn {
    PostgresColumn {
        numeric_precision: precision,
        numeric_scale: scale,
        ..column("numeric")
    }
}

fn character_varying_column(length: Option<u32>) -> PostgresColumn {
    PostgresColumn {
        character_length: length,
        ..column("character varying")
    }
}

fn character_column(data_type: &str, length: Option<u32>) -> PostgresColumn {
    PostgresColumn {
        character_length: length,
        ..column(data_type)
    }
}

fn timestamp_column(data_type: &str, precision: u32) -> PostgresColumn {
    PostgresColumn {
        datetime_precision: Some(precision),
        ..column(data_type)
    }
}

/// One column of each PostgreSQL type this mapping maps, for the rules every
/// source's mapping obeys: every Exasol type the mapping produces appears at
/// least once, and so does every conversion its read expressions apply.
pub(crate) fn one_column_of_every_mapped_type() -> Vec<PostgresColumn> {
    vec![
        column("smallint"),
        column("integer"),
        column("bigint"),
        numeric_column(Some(12), Some(4)),
        numeric_column(None, None),
        column("real"),
        column("double precision"),
        column("money"),
        column("boolean"),
        character_column("bit", Some(8)),
        character_column("bit varying", Some(16)),
        character_column("character", Some(5)),
        character_column("character", Some(3000)),
        character_varying_column(Some(100)),
        column("text"),
        column("uuid"),
        column("jsonb"),
        column("time without time zone"),
        column("date"),
        timestamp_column("timestamp without time zone", 6),
        timestamp_column("timestamp with time zone", 6),
    ]
}

fn mapped(column: &PostgresColumn) -> ExasolType {
    map_column(column)
        .unwrap_or_else(|reason| panic!("`{}` must map: {reason}", column.data_type))
        .exasol_type
}

/// How the dialect reads `column`, named `"c"`, once it is mapped.
fn read(column: &PostgresColumn) -> String {
    let mapped = map_column(column)
        .unwrap_or_else(|reason| panic!("`{}` must map: {reason}", column.data_type));

    column_read_sql(Some(&mapped.source_type), mapped.exasol_type, r#""c""#)
}

fn unmapped(column: &PostgresColumn) -> String {
    match map_column(column) {
        Ok(mapped) => panic!(
            "`{}` must not map, but mapped to {:?}",
            column.data_type, mapped.exasol_type
        ),
        Err(reason) => reason,
    }
}

/// `vs-adapter/postgresql-type-mapping`: the digit counts follow from the
/// integer widths rather than from the source, so they are the same values
/// MySQL's mapping produces for `SMALLINT`, `INT`, and `BIGINT` — stated here
/// rather than read from MySQL's table, which the two mappings do not share.
#[test]
fn integer_types_map_to_decimal() {
    for (data_type, precision) in [("smallint", 5), ("integer", 10), ("bigint", 19)] {
        assert_eq!(
            mapped(&column(data_type)),
            ExasolType::Decimal {
                precision,
                scale: 0
            },
            "for {data_type}"
        );
        assert_eq!(read(&column(data_type)), r#""c""#);
    }
}

/// PostgreSQL reports an integer column's `numeric_precision` in the radix
/// `numeric_precision_radix` names, which is 2 for the integer types: an
/// `integer` column reports 32 and a `bigint` column 64, measured against
/// `postgres:18.6-trixie`. Reading those as digit counts would declare
/// `DECIMAL(32,0)` and `DECIMAL(64,0)`, the second of which Exasol rejects
/// outright, so the mapping states the digit counts instead of reading them.
#[test]
fn an_integer_width_is_not_read_from_the_catalogs_precision() {
    let reported_by_the_catalog = PostgresColumn {
        numeric_precision: Some(32),
        numeric_scale: Some(0),
        ..column("integer")
    };

    assert_eq!(
        mapped(&reported_by_the_catalog),
        ExasolType::Decimal {
            precision: 10,
            scale: 0
        }
    );
}

/// `vs-adapter/postgresql-type-mapping`: a `numeric(p,s)` keeps the source's
/// own precision and scale and reads bare, and Exasol's maximum precision
/// itself still maps.
#[test]
fn numeric_preserves_the_source_precision_and_scale() {
    for (column, expected) in [
        (
            numeric_column(Some(12), Some(4)),
            ExasolType::Decimal {
                precision: 12,
                scale: 4,
            },
        ),
        (
            numeric_column(Some(30), None),
            ExasolType::Decimal {
                precision: 30,
                scale: 0,
            },
        ),
        (
            numeric_column(Some(36), Some(2)),
            ExasolType::Decimal {
                precision: 36,
                scale: 2,
            },
        ),
    ] {
        assert_eq!(mapped(&column), expected);
        assert_eq!(read(&column), r#""c""#);
    }
}

/// `vs-adapter/type-mapping`, PostgreSQL's half: a source precision Exasol's
/// `DECIMAL` cannot hold maps to `VARCHAR(2000000)` read as PostgreSQL's text
/// for the value, which keeps every digit, rather than being reduced.
#[test]
fn numeric_beyond_precision_36_maps_to_varchar_read_as_text() {
    let beyond = numeric_column(Some(40), Some(2));

    assert_eq!(mapped(&beyond), ExasolType::Varchar { size: 2_000_000 });
    assert_eq!(read(&beyond), r#"CAST("c" AS TEXT)"#);
}

/// An unconstrained `numeric` reports no precision. Its values need not fit
/// any `DECIMAL`, and `NaN`, `Infinity`, and trailing zeros are values of its
/// own, so it maps to the text PostgreSQL prints for each.
#[test]
fn unconstrained_numeric_maps_to_varchar_read_as_text() {
    let unconstrained = numeric_column(None, None);

    assert_eq!(
        mapped(&unconstrained),
        ExasolType::Varchar { size: 2_000_000 }
    );
    assert_eq!(read(&unconstrained), r#"CAST("c" AS TEXT)"#);
}

/// `vs-adapter/postgresql-type-mapping`: the declared length carries through
/// to Exasol `VARCHAR`, a declared length of 0 — which PostgreSQL rejects but
/// the mapping must not depend on it rejecting — is raised to Exasol's minimum
/// size of 1, and an absent or oversize length, like `text`, is the maximum.
/// All of them read bare.
#[test]
fn character_varying_and_text_map_to_varchar_capped_at_the_maximum() {
    for (column, size) in [
        (character_varying_column(Some(100)), 100),
        (character_varying_column(Some(0)), 1),
        (character_varying_column(Some(2_000_000)), 2_000_000),
        (character_varying_column(Some(10_485_760)), 2_000_000),
        (character_varying_column(None), 2_000_000),
        (column("text"), 2_000_000),
    ] {
        assert_eq!(
            mapped(&column),
            ExasolType::Varchar { size },
            "for {}",
            column.data_type
        );
        assert_eq!(read(&column), r#""c""#);
    }
}

/// `vs-adapter/postgresql-type-mapping`: `character(n)` is Exasol `CHAR(n)` up
/// to Exasol's maximum `CHAR` size of 2000 and reads bare. Above it, no `CHAR`
/// holds the column, so it is a `VARCHAR` of the same size read as text, which
/// drops the blanks PostgreSQL pads with.
#[test]
fn character_maps_to_char_up_to_2000_and_varchar_above() {
    for (length, expected, expected_read) in [
        (5, ExasolType::Char { size: 5 }, r#""c""#),
        (2000, ExasolType::Char { size: 2000 }, r#""c""#),
        (
            2001,
            ExasolType::Varchar { size: 2001 },
            r#"CAST("c" AS TEXT)"#,
        ),
        (
            3000,
            ExasolType::Varchar { size: 3000 },
            r#"CAST("c" AS TEXT)"#,
        ),
        (
            10_485_760,
            ExasolType::Varchar { size: 2_000_000 },
            r#"CAST("c" AS TEXT)"#,
        ),
    ] {
        let character = character_column("character", Some(length));

        assert_eq!(mapped(&character), expected, "for length {length}");
        assert_eq!(read(&character), expected_read, "for length {length}");
    }
}

/// `vs-adapter/postgresql-type-mapping`: a `bpchar` without a length reports
/// `character` with no length and is the maximum `VARCHAR` read as text, and a
/// reported length of 0 is raised to Exasol's minimum `CHAR(1)`.
#[test]
fn unbounded_and_zero_length_character_columns_map() {
    let unbounded = character_column("character", None);
    assert_eq!(mapped(&unbounded), ExasolType::Varchar { size: 2_000_000 });
    assert_eq!(read(&unbounded), r#"CAST("c" AS TEXT)"#);

    let zero_length = character_column("character", Some(0));
    assert_eq!(mapped(&zero_length), ExasolType::Char { size: 1 });
    assert_eq!(read(&zero_length), r#""c""#);
}

/// `vs-adapter/postgresql-type-mapping`: the temporal and floating-point
/// types, each with its own fractional-second precision where it has one, all
/// read bare.
#[test]
fn temporal_and_floating_point_types_map_to_their_exasol_counterparts() {
    assert_eq!(mapped(&column("date")), ExasolType::Date);
    for (precision, column) in [
        (6, timestamp_column("timestamp without time zone", 6)),
        (0, timestamp_column("timestamp without time zone", 0)),
    ] {
        assert_eq!(mapped(&column), ExasolType::Timestamp { precision });
    }
    assert_eq!(mapped(&column("double precision")), ExasolType::Double);
    assert_eq!(mapped(&column("real")), ExasolType::Double);

    for data_type in ["date", "double precision", "real"] {
        assert_eq!(read(&column(data_type)), r#""c""#);
    }
    assert_eq!(
        read(&timestamp_column("timestamp without time zone", 6)),
        r#""c""#
    );
}

/// `vs-adapter/postgresql-type-mapping`: `boolean` and `bit` of every width
/// are `BOOLEAN`, and a `bit` column reads as whether any bit is set, as the
/// JDBC Virtual Schema reads it.
#[test]
fn boolean_and_bit_of_every_width_map_to_boolean() {
    assert_eq!(mapped(&column("boolean")), ExasolType::Boolean);
    assert_eq!(read(&column("boolean")), r#""c""#);

    for length in [1, 8, 64] {
        let bit = character_column("bit", Some(length));

        assert_eq!(mapped(&bit), ExasolType::Boolean, "for bit({length})");
        assert_eq!(
            read(&bit),
            r#"(position(B'1' IN "c") > 0)"#,
            "for bit({length})"
        );
    }
}

/// `vs-adapter/postgresql-type-mapping`: PostgreSQL defines no direct
/// conversion from `money` to `double precision`, so the read goes through
/// `numeric`.
#[test]
fn money_maps_to_double_read_through_numeric() {
    assert_eq!(mapped(&column("money")), ExasolType::Double);
    assert_eq!(
        read(&column("money")),
        r#"CAST(CAST("c" AS NUMERIC) AS DOUBLE PRECISION)"#
    );
}

/// `vs-adapter/postgresql-type-mapping`: a type Exasol has no counterpart for
/// is `VARCHAR` read as PostgreSQL's text for the value, sized to what that
/// text can hold: a bit string's length, 100 for a `time`, and the maximum
/// for the rest.
#[test]
fn types_without_a_counterpart_map_to_varchar_read_as_text() {
    for data_type in [
        "uuid",
        "json",
        "jsonb",
        "xml",
        "bytea",
        "interval",
        "inet",
        "cidr",
        "macaddr",
        "point",
        "tsvector",
        "USER-DEFINED",
    ] {
        let column = column(data_type);

        assert_eq!(
            mapped(&column),
            ExasolType::Varchar { size: 2_000_000 },
            "for {data_type}"
        );
        assert_eq!(read(&column), r#"CAST("c" AS TEXT)"#, "for {data_type}");
    }

    for data_type in ["time without time zone", "time with time zone"] {
        let column = column(data_type);

        assert_eq!(mapped(&column), ExasolType::Varchar { size: 100 });
        assert_eq!(read(&column), r#"CAST("c" AS TEXT)"#);
    }

    for (length, size) in [(Some(16), 16), (None, 2_000_000)] {
        let bit_varying = character_column("bit varying", length);

        assert_eq!(mapped(&bit_varying), ExasolType::Varchar { size });
        assert_eq!(read(&bit_varying), r#"CAST("c" AS TEXT)"#);
    }
}

/// `vs-adapter/postgresql-type-mapping`: the read expression is selected by the
/// source type the column recorded and the type it declared, so a column of a
/// type that reads as itself renders bare, a column without notes renders bare
/// as a schema created before notes existed does, and a `character varying`
/// declared `VARCHAR` is not wrapped in a conversion that would change nothing.
#[test]
fn column_read_sql_follows_the_source_type() {
    let varchar = ExasolType::Varchar { size: 2_000_000 };
    let double = ExasolType::Double;

    for (source_type, declared, expected) in [
        ("uuid", varchar, r#"CAST("c" AS TEXT)"#),
        ("USER-DEFINED", varchar, r#"CAST("c" AS TEXT)"#),
        ("numeric", varchar, r#"CAST("c" AS TEXT)"#),
        ("character", varchar, r#"CAST("c" AS TEXT)"#),
        ("character varying", varchar, r#""c""#),
        ("text", varchar, r#""c""#),
        (
            "money",
            double,
            r#"CAST(CAST("c" AS NUMERIC) AS DOUBLE PRECISION)"#,
        ),
        (
            "MONEY",
            double,
            r#"CAST(CAST("c" AS NUMERIC) AS DOUBLE PRECISION)"#,
        ),
        ("bit", ExasolType::Boolean, r#"(position(B'1' IN "c") > 0)"#),
        ("boolean", ExasolType::Boolean, r#""c""#),
        ("real", double, r#""c""#),
        ("character", ExasolType::Char { size: 5 }, r#""c""#),
    ] {
        assert_eq!(
            column_read_sql(Some(source_type), declared, r#""c""#),
            expected,
            "for {source_type} declared {declared:?}"
        );
    }
    assert_eq!(column_read_sql(None, varchar, r#""c""#), r#""c""#);
}

/// `vs-adapter/postgresql-type-mapping`: a `timestamp with time zone` is
/// `TIMESTAMP` of its own precision rather than a time-zone type, and reads
/// bare, because the scan emits its UTC wall-clock value.
#[test]
fn timestamp_with_time_zone_maps_to_timestamp_of_its_precision() {
    for precision in [0, 3, 6] {
        let timestamptz = timestamp_column("timestamp with time zone", precision);

        assert_eq!(mapped(&timestamptz), ExasolType::Timestamp { precision });
        assert_eq!(read(&timestamptz), r#""c""#);
    }
}

/// `vs-adapter/postgresql-type-mapping`: a timestamp whose catalog row reports
/// no precision declares PostgreSQL's default of 6 fractional digits.
#[test]
fn a_timestamp_without_a_reported_precision_declares_postgresqls_default_of_six() {
    for data_type in ["timestamp without time zone", "timestamp with time zone"] {
        assert_eq!(
            mapped(&column(data_type)),
            ExasolType::Timestamp { precision: 6 },
            "for {data_type}"
        );
    }
}

/// `vs-adapter/postgresql-type-mapping`: an array is omitted, as the JDBC
/// Virtual Schema leaves it out, and the reason names `ARRAY` as the
/// catalog spelled it, which names no element type.
#[test]
fn array_columns_are_unmapped_naming_array() {
    let reason = unmapped(&column("ARRAY"));

    assert_eq!(
        reason,
        "PostgreSQL type ARRAY is not mapped, because the catalog names no element type and the \
         Exasol PostgreSQL JDBC Virtual Schema leaves arrays out too"
    );
}

/// `vs-adapter/type-mapping`: whether a column may hold a null is the
/// catalog's answer, carried through untouched, because Exasol declares the
/// virtual column from it and a column declared `NOT NULL` over a nullable
/// source would fail on the first null row.
#[test]
fn nullability_carries_through() {
    for is_nullable in [true, false] {
        let mapped = map_column(&PostgresColumn {
            is_nullable,
            ..column("integer")
        })
        .expect("an `integer` column must map");

        assert_eq!(mapped.is_nullable, is_nullable);
    }
}

/// `information_schema` spells `data_type` in lower case, so the mapping is
/// never asked another case in practice; matching case-insensitively means a
/// catalog that spelled one differently unmaps nothing silently.
#[test]
fn a_data_type_is_matched_whatever_its_case() {
    assert_eq!(mapped(&column("INTEGER")), mapped(&column("integer")));
    assert_eq!(
        mapped(&PostgresColumn {
            character_length: Some(20),
            ..column("CHARACTER VARYING")
        }),
        ExasolType::Varchar { size: 20 }
    );
}
