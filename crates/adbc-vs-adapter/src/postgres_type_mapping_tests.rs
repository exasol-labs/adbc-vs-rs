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

fn mapped(column: &PostgresColumn) -> MappedType {
    map_column(column)
        .unwrap_or_else(|reason| panic!("`{}` must map: {reason}", column.data_type))
        .exasol_type
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
/// MySQL's mapping produces for `INT` and `BIGINT` — stated here rather than
/// read from MySQL's table, which the two mappings do not share.
#[test]
fn integer_types_map_to_decimal() {
    assert_eq!(
        mapped(&column("integer")),
        MappedType::Decimal {
            precision: 10,
            scale: 0
        }
    );
    assert_eq!(
        mapped(&column("bigint")),
        MappedType::Decimal {
            precision: 19,
            scale: 0
        }
    );
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
        MappedType::Decimal {
            precision: 10,
            scale: 0
        }
    );
}

/// `vs-adapter/postgresql-type-mapping`: a `numeric(p,s)` keeps the source's
/// own precision and scale, because Exasol compares a declared type for exact
/// equality and a reduced precision would truncate values.
#[test]
fn numeric_preserves_the_source_precision_and_scale() {
    assert_eq!(
        mapped(&numeric_column(Some(12), Some(4))),
        MappedType::Decimal {
            precision: 12,
            scale: 4
        }
    );
    assert_eq!(
        mapped(&numeric_column(Some(30), None)),
        MappedType::Decimal {
            precision: 30,
            scale: 0
        }
    );
}

/// `vs-adapter/type-mapping`, PostgreSQL's half: a source precision Exasol's
/// `DECIMAL` cannot hold is omitted rather than silently reduced, and the
/// reason names Exasol's maximum so an operator can see why.
#[test]
fn oversize_numeric_is_unmapped() {
    let reason = unmapped(&numeric_column(Some(40), Some(2)));

    assert!(
        reason.contains("40") && reason.contains("36"),
        "the reason must name the source precision and Exasol's maximum: {reason}"
    );
    assert!(
        reason.contains("numeric"),
        "the reason must name the type it rejected: {reason}"
    );

    assert_eq!(
        mapped(&numeric_column(Some(36), Some(2))),
        MappedType::Decimal {
            precision: 36,
            scale: 2
        },
        "Exasol's maximum precision itself must still map"
    );
}

/// An unconstrained `numeric` reports no precision. Substituting Exasol's
/// maximum would declare a type an arbitrary-precision source value need not
/// fit, so the column is omitted and the reason names the absent precision.
#[test]
fn numeric_without_a_declared_precision_is_unmapped() {
    let reason = unmapped(&numeric_column(None, None));

    assert!(
        reason.contains("numeric") && reason.contains("precision"),
        "the reason must name the type and its absent precision: {reason}"
    );
}

/// `vs-adapter/postgresql-type-mapping`: the declared length carries through
/// to Exasol `VARCHAR`, and a declared length of 0 — which PostgreSQL rejects
/// but the mapping must not depend on it rejecting — is raised to Exasol's
/// minimum character-type size of 1 rather than declared as `VARCHAR(0)`,
/// which Exasol rejects.
#[test]
fn character_varying_preserves_its_declared_length() {
    assert_eq!(
        mapped(&character_varying_column(Some(100))),
        MappedType::Varchar { size: 100 }
    );
    assert_eq!(
        mapped(&character_varying_column(Some(0))),
        MappedType::Varchar { size: 1 }
    );
}

/// A `character varying` with no declared length is `varchar` without a
/// length specifier, which is unbounded. Exasol needs an exact size for a
/// type it compares for equality, so the column is omitted.
#[test]
fn character_varying_without_a_declared_length_is_unmapped() {
    let reason = unmapped(&character_varying_column(None));

    assert!(
        reason.contains("character varying") && reason.contains("length"),
        "the reason must name the type and its absent length: {reason}"
    );
}

/// PostgreSQL admits a `character varying` far longer than Exasol's maximum
/// `VARCHAR` size. Declaring it anyway would fail the whole `CREATE VIRTUAL
/// SCHEMA` rather than omit one column, which is the outcome the omission
/// contract exists to avoid.
#[test]
fn oversize_character_varying_is_unmapped() {
    let reason = unmapped(&character_varying_column(Some(10_485_760)));

    assert!(
        reason.contains("10485760") && reason.contains("2000000"),
        "the reason must name the source length and Exasol's maximum: {reason}"
    );

    assert_eq!(
        mapped(&character_varying_column(Some(2_000_000))),
        MappedType::Varchar { size: 2_000_000 },
        "Exasol's maximum size itself must still map"
    );
}

/// `vs-adapter/postgresql-type-mapping`: the three remaining mapped types.
#[test]
fn temporal_and_floating_point_types_map_to_their_exasol_counterparts() {
    assert_eq!(mapped(&column("date")), MappedType::Date);
    assert_eq!(
        mapped(&column("timestamp without time zone")),
        MappedType::Timestamp
    );
    assert_eq!(mapped(&column("double precision")), MappedType::Double);
}

/// `vs-adapter/postgresql-type-mapping`: every type outside this release's
/// scope is omitted, and each reason names the type the catalog reported so
/// the omission reaches an operator as the whole explanation.
///
/// `ARRAY` is spelled as `information_schema` reports it — upper case, and
/// naming no element type — which is why the reason has to echo the catalog's
/// own spelling rather than a normalized one.
#[test]
fn non_core_types_are_unmapped_with_the_reason_naming_the_type() {
    for data_type in [
        "boolean",
        "text",
        "smallint",
        "real",
        "time without time zone",
        "time with time zone",
        "timestamp with time zone",
        "uuid",
        "json",
        "jsonb",
        "bytea",
        "interval",
        "money",
        "ARRAY",
    ] {
        let reason = unmapped(&column(data_type));

        assert!(
            reason.contains(data_type),
            "`{data_type}`'s reason must name it: {reason}"
        );
    }
}

/// `character` is blank-padded: PostgreSQL returns the value padded to its
/// declared length and compares two of them ignoring trailing spaces, which
/// Exasol `VARCHAR` does neither of. Exasol `CHAR` would match both, but no
/// column this release maps produces Exasol `CHAR`.
#[test]
fn character_is_unmapped_because_it_is_blank_padded() {
    let reason = unmapped(&PostgresColumn {
        character_length: Some(5),
        ..column("character")
    });

    assert!(
        reason.contains("character"),
        "the reason must name the type it rejected: {reason}"
    );
    assert!(
        reason.contains("pad"),
        "the reason must name the blank padding as the cause: {reason}"
    );
}

/// `boolean` is omitted despite Exasol having a `BOOLEAN` type: a mapped
/// boolean column would widen the set of Exasol types a source column
/// produces and bring a boolean operand into every predicate path.
#[test]
fn boolean_is_unmapped_despite_exasol_having_one() {
    let reason = unmapped(&column("boolean"));

    assert!(
        reason.contains("boolean") && reason.contains("BOOLEAN"),
        "the reason must name the source type and the Exasol type it is not mapped to: {reason}"
    );
}

/// `timestamp with time zone` is omitted rather than mapped to Exasol
/// `TIMESTAMP`: the two carry different instants for one wall-clock value.
/// The internal spelling is refused for the same reason, so a catalog that
/// reported it would not fall through to the unspecific reason.
#[test]
fn a_time_zone_aware_timestamp_is_unmapped_rather_than_mapped_to_timestamp() {
    for spelling in ["timestamp with time zone", "timestamptz"] {
        let reason = unmapped(&column(spelling));

        assert!(
            reason.contains(spelling),
            "the reason must name the type it rejected: {reason}"
        );
        assert!(
            reason.contains("instant"),
            "the reason must name the differing instants as the cause: {reason}"
        );
    }
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
        MappedType::Varchar { size: 20 }
    );
}
