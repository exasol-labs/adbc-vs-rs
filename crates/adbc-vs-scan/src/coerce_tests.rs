use std::sync::Arc;

use arrow::array::{
    Array, ArrayRef, BinaryArray, BooleanArray, Date32Array, Date64Array, Decimal128Array,
    Decimal256Array, Decimal64Array, Float32Array, Float64Array, Int32Array, Int64Array,
    LargeStringArray, NullArray, RecordBatch, StringArray, StringViewArray,
    TimestampMicrosecondArray, TimestampMillisecondArray, TimestampNanosecondArray,
    TimestampSecondArray,
};
use arrow::datatypes::{i256, DataType, Field, Schema, TimeUnit};

use super::coerce::*;

/// `2026-01-15 08:30:00Z`, the instant the fixture's `created_at` holds.
const INSTANT_SECONDS: i64 = 1_768_465_800;

/// `99999999.9999` at scale 4 — the largest value a `DECIMAL(12,4)` holds,
/// and the value the MySQL fixture seeds to exercise the precision boundary.
const MAX_SCALE_UNSCALED: i128 = 999_999_999_999;

fn declared(name: &str, declared_type: DeclaredType) -> DeclaredColumn {
    DeclaredColumn {
        name: name.to_string(),
        declared_type,
    }
}

fn batch_of(name: &str, column: ArrayRef) -> RecordBatch {
    let field = Field::new(name, column.data_type().clone(), true);
    RecordBatch::try_new(Arc::new(Schema::new(vec![field])), vec![column])
        .expect("the fixture batch matches the field built from its own column")
}

fn coerce_one(column: ArrayRef, declared_type: DeclaredType) -> Result<ArrayRef, String> {
    let columns = vec![declared("value", declared_type)];
    let coercion = Coercion::for_columns(&columns).map_err(|error| error.to_string())?;
    let coerced = coercion
        .apply(&batch_of("source", column))
        .map_err(|error| error.to_string())?;
    Ok(Arc::clone(coerced.column(0)))
}

fn expect_coerced(column: ArrayRef, declared_type: DeclaredType) -> ArrayRef {
    coerce_one(column, declared_type).expect("the fixture column coerces to its declared type")
}

fn amount() -> DeclaredType {
    DeclaredType::Decimal {
        precision: 12,
        scale: 4,
    }
}

#[test]
fn timestamp_units_normalize() {
    let seconds: ArrayRef = Arc::new(TimestampSecondArray::from(vec![INSTANT_SECONDS]));
    let millis: ArrayRef = Arc::new(TimestampMillisecondArray::from(vec![
        INSTANT_SECONDS * 1_000,
    ]));
    let micros: ArrayRef = Arc::new(TimestampMicrosecondArray::from(vec![
        INSTANT_SECONDS * 1_000_000,
    ]));
    let nanos: ArrayRef = Arc::new(TimestampNanosecondArray::from(vec![
        INSTANT_SECONDS * 1_000_000_000,
    ]));
    let zoned: ArrayRef = Arc::new(
        TimestampMillisecondArray::from(vec![INSTANT_SECONDS * 1_000]).with_timezone("+02:00"),
    );

    let expected: ArrayRef = Arc::new(TimestampMicrosecondArray::from(vec![
        INSTANT_SECONDS * 1_000_000,
    ]));
    for source in [seconds, millis, micros, nanos, zoned] {
        let unit = source.data_type().clone();
        let coerced = expect_coerced(source, DeclaredType::Timestamp);

        assert_eq!(
            coerced.data_type(),
            &DataType::Timestamp(TimeUnit::Microsecond, None),
            "{unit:?} did not normalize to the declared TIMESTAMP representation"
        );
        assert_eq!(&coerced, &expected, "{unit:?} did not preserve the instant");
    }
}

#[test]
fn decimal_widths_normalize() {
    let narrow: ArrayRef = Arc::new(
        Decimal64Array::from(vec![MAX_SCALE_UNSCALED as i64])
            .with_precision_and_scale(18, 4)
            .expect("the fixture value fits a Decimal64 of precision 18"),
    );
    let native: ArrayRef = Arc::new(
        Decimal128Array::from(vec![MAX_SCALE_UNSCALED])
            .with_precision_and_scale(12, 4)
            .expect("the fixture value fits the declared precision"),
    );
    let wide: ArrayRef = Arc::new(
        Decimal256Array::from(vec![i256::from_i128(MAX_SCALE_UNSCALED)])
            .with_precision_and_scale(38, 4)
            .expect("the fixture value fits a Decimal256 of precision 38"),
    );
    let rescaled: ArrayRef = Arc::new(
        Decimal128Array::from(vec![MAX_SCALE_UNSCALED * 100])
            .with_precision_and_scale(20, 6)
            .expect("the rescaled fixture value fits a Decimal128 of precision 20"),
    );
    let text: ArrayRef = Arc::new(StringArray::from(vec!["99999999.9999"]));

    let expected: ArrayRef = Arc::new(
        Decimal128Array::from(vec![MAX_SCALE_UNSCALED])
            .with_precision_and_scale(12, 4)
            .expect("the fixture value fits the declared precision"),
    );
    for source in [narrow, native, wide, rescaled, text] {
        let width = source.data_type().clone();
        let coerced = expect_coerced(source, amount());

        assert_eq!(
            coerced.data_type(),
            &DataType::Decimal128(12, 4),
            "{width:?} did not normalize to the declared DECIMAL(12,4) representation"
        );
        assert_eq!(
            &coerced, &expected,
            "{width:?} did not preserve the source precision and scale"
        );
    }
}

#[test]
fn wider_declared_decimal_widens() {
    let sum_result_type = DeclaredType::Decimal {
        precision: 23,
        scale: 4,
    };

    let narrower_decimal: ArrayRef = Arc::new(
        Decimal128Array::from(vec![MAX_SCALE_UNSCALED])
            .with_precision_and_scale(12, 4)
            .expect("the fixture value fits a Decimal128 of precision 12"),
    );
    let as_integer: ArrayRef = Arc::new(Int64Array::from(vec![42]));
    let as_string: ArrayRef = Arc::new(StringArray::from(vec!["99999999.9999"]));

    let expected_from_decimal: ArrayRef = Arc::new(
        Decimal128Array::from(vec![MAX_SCALE_UNSCALED])
            .with_precision_and_scale(23, 4)
            .expect("the widened value fits a Decimal128 of precision 23"),
    );
    let expected_from_integer: ArrayRef = Arc::new(
        Decimal128Array::from(vec![420_000])
            .with_precision_and_scale(23, 4)
            .expect("the scaled integer fits a Decimal128 of precision 23"),
    );
    let expected_from_string: ArrayRef = Arc::new(
        Decimal128Array::from(vec![MAX_SCALE_UNSCALED])
            .with_precision_and_scale(23, 4)
            .expect("the parsed string fits a Decimal128 of precision 23"),
    );

    for (source, expected) in [
        (narrower_decimal, expected_from_decimal),
        (as_integer, expected_from_integer),
        (as_string, expected_from_string),
    ] {
        let origin = source.data_type().clone();
        let coerced = expect_coerced(source, sum_result_type);

        assert_eq!(
            coerced.data_type(),
            &DataType::Decimal128(23, 4),
            "{origin:?} did not widen to the declared DECIMAL(23,4) representation"
        );
        assert_eq!(
            &coerced, &expected,
            "{origin:?} did not preserve its digits once widened"
        );
    }
}

#[test]
fn a_decimal_source_coerces_to_a_declared_double() {
    let source: ArrayRef = Arc::new(
        Decimal128Array::from(vec![MAX_SCALE_UNSCALED])
            .with_precision_and_scale(12, 4)
            .expect("the fixture value fits the declared precision"),
    );

    let coerced = expect_coerced(source, DeclaredType::Double);

    assert_eq!(coerced.data_type(), &DataType::Float64);
    assert_eq!(
        &coerced,
        &(Arc::new(Float64Array::from(vec![99999999.9999])) as ArrayRef)
    );
}

#[test]
fn declared_boolean_accepts_driver_representations() {
    let expected: ArrayRef = Arc::new(BooleanArray::from(vec![Some(true), Some(false), None]));

    let already_boolean: ArrayRef =
        Arc::new(BooleanArray::from(vec![Some(true), Some(false), None]));
    let as_int64: ArrayRef = Arc::new(Int64Array::from(vec![Some(1), Some(0), None]));
    let as_int32: ArrayRef = Arc::new(Int32Array::from(vec![Some(1), Some(0), None]));
    let as_non_zero_int64: ArrayRef = Arc::new(Int64Array::from(vec![Some(42), Some(0), None]));

    for source in [already_boolean, as_int64, as_int32, as_non_zero_int64] {
        let origin = source.data_type().clone();
        let coerced = expect_coerced(source, DeclaredType::Boolean);

        assert_eq!(
            coerced.data_type(),
            &DataType::Boolean,
            "{origin:?} did not coerce to the declared BOOLEAN representation"
        );
        assert_eq!(
            &coerced, &expected,
            "{origin:?} did not emit a non-zero as true, a zero as false, and a null as a null"
        );
        assert!(
            coerced.is_null(2),
            "{origin:?} turned the null of a comparison against a null operand into a value"
        );
    }
}

#[test]
fn nulls_survive_coercion() {
    let sources: Vec<(ArrayRef, DeclaredType)> = vec![
        (
            Arc::new(
                Decimal128Array::from(vec![Some(MAX_SCALE_UNSCALED), None])
                    .with_precision_and_scale(12, 4)
                    .expect("the fixture value fits the declared precision"),
            ),
            amount(),
        ),
        (
            Arc::new(StringArray::from(vec![Some("1234.5600"), None])),
            amount(),
        ),
        (
            Arc::new(StringArray::from(vec![Some("Widget"), None])),
            DeclaredType::Text,
        ),
        (
            Arc::new(Date64Array::from(vec![Some(INSTANT_SECONDS * 1_000), None])),
            DeclaredType::Date,
        ),
        (
            Arc::new(TimestampSecondArray::from(vec![
                Some(INSTANT_SECONDS),
                None,
            ])),
            DeclaredType::Timestamp,
        ),
        (
            Arc::new(Float32Array::from(vec![Some(19.5f32), None])),
            DeclaredType::Double,
        ),
    ];

    for (source, declared_type) in sources {
        let origin = source.data_type().clone();
        let coerced = expect_coerced(source, declared_type);

        assert_eq!(coerced.len(), 2, "{origin:?} lost a row");
        assert!(!coerced.is_null(0), "{origin:?} turned a value into a null");
        assert!(
            coerced.is_null(1),
            "{origin:?} substituted a value for a null"
        );
        assert_eq!(coerced.null_count(), 1, "{origin:?} changed the null count");
    }
}

#[test]
fn an_all_null_column_coerces_to_nulls_of_the_declared_type() {
    let coerced = expect_coerced(Arc::new(NullArray::new(3)), amount());

    assert_eq!(coerced.data_type(), &DataType::Decimal128(12, 4));
    assert_eq!(coerced.null_count(), 3);
}

#[test]
fn text_widths_normalize() {
    for source in [
        Arc::new(LargeStringArray::from(vec!["Widget"])) as ArrayRef,
        Arc::new(StringViewArray::from(vec!["Widget"])) as ArrayRef,
        Arc::new(StringArray::from(vec!["Widget"])) as ArrayRef,
    ] {
        let width = source.data_type().clone();
        let coerced = expect_coerced(source, DeclaredType::Text);

        assert_eq!(
            coerced.data_type(),
            &DataType::Utf8,
            "{width:?} stayed wide"
        );
        assert_eq!(
            &coerced,
            &(Arc::new(StringArray::from(vec!["Widget"])) as ArrayRef)
        );
    }
}

#[test]
fn date_representations_normalize_to_date32() {
    for source in [
        Arc::new(Date64Array::from(vec![INSTANT_SECONDS * 1_000])) as ArrayRef,
        Arc::new(StringArray::from(vec!["2026-01-15"])) as ArrayRef,
    ] {
        let coerced = expect_coerced(source, DeclaredType::Date);

        assert_eq!(coerced.data_type(), &DataType::Date32);
        assert_eq!(
            &coerced,
            &(Arc::new(Date32Array::from(vec![20_468])) as ArrayRef)
        );
    }
}

/// Arrow's `Date64`->`Date32` cast divides by milliseconds-per-day and
/// truncates toward zero rather than flooring, so a pre-epoch instant
/// carrying a nonzero time-of-day lands one calendar day later than the
/// correct one (`0`, 1970-01-01, instead of `-1`, 1969-12-31). MySQL `DATE`
/// reaches 1000-01-01, so this path is reachable; this pins the actual
/// rounding rather than the exact one the module doc otherwise claims.
#[test]
fn a_pre_epoch_date_with_a_nonzero_time_of_day_rounds_toward_the_epoch() {
    let source: ArrayRef = Arc::new(Date64Array::from(vec![-43_200_000_i64]));

    let coerced = expect_coerced(source, DeclaredType::Date);

    assert_eq!(coerced.data_type(), &DataType::Date32);
    assert_eq!(
        &coerced,
        &(Arc::new(Date32Array::from(vec![0])) as ArrayRef),
        "a Date64 of noon the day before the epoch must round toward the epoch rather than \
         floor to the correct calendar day"
    );
}

#[test]
fn a_column_already_in_the_declared_representation_is_reused_unchanged() {
    let source: ArrayRef = Arc::new(Float64Array::from(vec![19.99]));

    let coerced = expect_coerced(Arc::clone(&source), DeclaredType::Double);

    assert!(
        Arc::ptr_eq(&source, &coerced),
        "a column already in its declared representation was copied"
    );
}

#[test]
fn a_value_beyond_the_declared_precision_fails_rather_than_truncating() {
    let too_large: ArrayRef = Arc::new(
        Decimal128Array::from(vec![MAX_SCALE_UNSCALED * 10])
            .with_precision_and_scale(20, 4)
            .expect("the oversize value fits a Decimal128 of precision 20"),
    );

    let message = coerce_one(too_large, amount())
        .expect_err("a value beyond the declared precision must not be emitted");

    assert!(
        message.contains("9999999999990"),
        "error message {message:?} does not name the offending unscaled value"
    );
    assert!(
        message.contains("DECIMAL(12,4)"),
        "error message {message:?} does not name the declared type"
    );
}

#[test]
fn a_column_the_declared_type_cannot_represent_names_both_types() {
    let opaque: ArrayRef = Arc::new(BinaryArray::from(vec![b"\x00\x01".as_slice()]));

    let message = coerce_one(opaque, DeclaredType::Double)
        .expect_err("a binary column cannot be emitted as a DOUBLE");

    assert!(
        message.contains("Binary"),
        "error message {message:?} does not name the Arrow type the driver returned"
    );
    assert!(
        message.contains("DOUBLE"),
        "error message {message:?} does not name the declared type"
    );
}

#[test]
fn a_declared_decimal_wider_than_arrow_represents_fails_before_any_batch() {
    let columns = vec![declared(
        "amount",
        DeclaredType::Decimal {
            precision: 45,
            scale: 2,
        },
    )];

    let message = Coercion::for_columns(&columns)
        .err()
        .expect("a precision beyond Arrow's decimal width must be refused")
        .to_string();

    assert!(
        message.contains("amount") && message.contains("45"),
        "error message {message:?} does not name the column and its precision"
    );
}

#[test]
fn a_declared_scale_beyond_its_precision_fails_before_any_batch() {
    let columns = vec![declared(
        "amount",
        DeclaredType::Decimal {
            precision: 4,
            scale: 6,
        },
    )];

    let message = Coercion::for_columns(&columns)
        .err()
        .expect("a scale beyond its own precision must be refused")
        .to_string();

    assert!(
        message.contains("amount"),
        "error message {message:?} does not name the column"
    );
}

#[test]
fn a_batch_with_a_different_column_count_names_both_counts() {
    let columns = vec![
        declared("id", DeclaredType::Double),
        declared("name", DeclaredType::Text),
    ];
    let coercion = Coercion::for_columns(&columns).expect("the declaration is representable");
    let batch = batch_of("id", Arc::new(Float64Array::from(vec![1.0])));

    let message = coercion
        .apply(&batch)
        .expect_err("a batch of one column cannot fill a two-column declaration")
        .to_string();

    assert!(
        message.contains('1') && message.contains('2'),
        "error message {message:?} does not name both column counts"
    );
}

#[test]
fn the_coerced_batch_carries_the_declared_names_in_the_declared_order() {
    let columns = vec![
        declared(
            "id",
            DeclaredType::Decimal {
                precision: 10,
                scale: 0,
            },
        ),
        declared("name", DeclaredType::Text),
        declared("created_at", DeclaredType::Timestamp),
    ];
    let coercion = Coercion::for_columns(&columns).expect("the declaration is representable");
    let source = RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("whatever_the_driver_called_it", DataType::Int64, true),
            Field::new("b", DataType::LargeUtf8, true),
            Field::new("c", DataType::Timestamp(TimeUnit::Second, None), true),
        ])),
        vec![
            Arc::new(Int64Array::from(vec![7])),
            Arc::new(LargeStringArray::from(vec!["Widget"])),
            Arc::new(TimestampSecondArray::from(vec![INSTANT_SECONDS])),
        ],
    )
    .expect("the fixture batch matches its schema");

    let coerced = coercion.apply(&source).expect("the fixture batch coerces");

    let fields: Vec<_> = coerced
        .schema()
        .fields()
        .iter()
        .map(|field| (field.name().clone(), field.data_type().clone()))
        .collect();
    assert_eq!(
        fields,
        vec![
            ("id".to_string(), DataType::Int64),
            ("name".to_string(), DataType::Utf8),
            (
                "created_at".to_string(),
                DataType::Timestamp(TimeUnit::Microsecond, None)
            ),
        ]
    );
    assert_eq!(coerced.schema(), coercion.schema());
}

#[test]
fn an_empty_batch_coerces_to_an_empty_batch_of_the_declared_schema() {
    let columns = vec![declared("amount", amount())];
    let coercion = Coercion::for_columns(&columns).expect("the declaration is representable");
    let empty = RecordBatch::new_empty(coercion.schema());

    let coerced = coercion.apply(&empty).expect("an empty batch coerces");

    assert_eq!(coerced.num_rows(), 0);
    assert_eq!(coerced.schema(), coercion.schema());
}

#[test]
fn a_scale_zero_decimal_within_nine_digits_emits_as_a_32_bit_integer() {
    let source: ArrayRef = Arc::new(Int64Array::from(vec![123_456_789]));

    let coerced = expect_coerced(
        source,
        DeclaredType::Decimal {
            precision: 9,
            scale: 0,
        },
    );

    assert_eq!(
        coerced.data_type(),
        &DataType::Int32,
        "Exasol declares DECIMAL(9,0) as a 32-bit integer column"
    );
}

#[test]
fn a_scale_zero_decimal_of_ten_to_eighteen_digits_emits_as_a_64_bit_integer() {
    for precision in [10, 18] {
        let source: ArrayRef = Arc::new(
            Decimal128Array::from(vec![9_000_000_001])
                .with_precision_and_scale(19, 0)
                .expect("the fixture value fits a Decimal128 of precision 19"),
        );

        let coerced = expect_coerced(
            source,
            DeclaredType::Decimal {
                precision,
                scale: 0,
            },
        );

        assert_eq!(
            coerced.data_type(),
            &DataType::Int64,
            "Exasol declares DECIMAL({precision},0) as a 64-bit integer column"
        );
        assert_eq!(
            &coerced,
            &(Arc::new(Int64Array::from(vec![9_000_000_001])) as ArrayRef)
        );
    }
}

/// A scale-0 `DECIMAL` leaves the decimal path for a 32- or 64-bit integer
/// column, so a value too wide for that integer is what this module has to
/// refuse on those two paths — a delegated `COUNT` lands on them, since
/// Exasol declares `COUNT` as `DECIMAL(18,0)`. Emitting a truncated or
/// wrapped integer for a delegated aggregate is the wrong-value failure this
/// release withheld two capabilities to avoid, so the conversion fails
/// naming what did not fit. The `Decimal128` emit path carries its own case
/// in `a_value_beyond_the_declared_precision_fails_rather_than_truncating`.
#[test]
fn aggregate_overflow_fails_rather_than_truncating() {
    let ten_digits: ArrayRef = Arc::new(Int64Array::from(vec![9_999_999_999]));
    let nineteen_digits: ArrayRef = Arc::new(
        Decimal128Array::from(vec![9_999_999_999_999_999_999_i128])
            .with_precision_and_scale(19, 0)
            .expect("a nineteen-digit value fits a Decimal128 of precision 19"),
    );

    for (source, precision, offending) in [
        (ten_digits, 9_u32, "9999999999"),
        (nineteen_digits, 18_u32, "9999999999999999999"),
    ] {
        let message = coerce_one(
            source,
            DeclaredType::Decimal {
                precision,
                scale: 0,
            },
        )
        .expect_err("a value wider than the declared column's integer must not be emitted");

        assert!(
            message.contains("column `value`"),
            "error message {message:?} does not name the column"
        );
        assert!(
            message.contains(&format!("DECIMAL({precision},0)")),
            "error message {message:?} does not name the declared type"
        );
        assert!(
            message.contains(offending),
            "error message {message:?} does not name the value that did not fit"
        );
    }
}

/// The PostgreSQL driver returns `numeric` as an Arrow string rather than a
/// decimal-typed array, and that string form is the primary case this scan
/// must coerce, not a fallback below some other representation. This pins
/// the source precision and scale carrying through unchanged, the same
/// guarantee the decimal-width cases above already hold for a driver's
/// native decimal representations.
#[test]
fn string_form_numeric_coerces_to_declared_decimal() {
    let source: ArrayRef = Arc::new(StringArray::from(vec!["12345.67"]));
    let declared_type = DeclaredType::Decimal {
        precision: 10,
        scale: 2,
    };

    let coerced = expect_coerced(source, declared_type);

    assert_eq!(coerced.data_type(), &DataType::Decimal128(10, 2));
    assert_eq!(
        &coerced,
        &(Arc::new(
            Decimal128Array::from(vec![1_234_567])
                .with_precision_and_scale(10, 2)
                .expect("the fixture value fits the declared DECIMAL(10,2)")
        ) as ArrayRef)
    );
}

/// The PostgreSQL driver spells `numeric`'s non-finite values `nan`, `inf`,
/// and `-inf` on the wire — lowercase, never the PostgreSQL-literal
/// `NaN`/`Infinity`/`-Infinity` spellings. Both sets are covered here: the
/// lowercase ones because they are what a real column can produce (a `NaN`
/// stored in a constrained `numeric(p,s)` column reads back as `nan`), the
/// capitalized ones as defense in depth against a value arriving unexpected.
/// None of Exasol's `DECIMAL` values can hold a non-finite value, so every
/// spelling must fail rather than coerce, and the failure must name the
/// exact value it rejected alongside the declared type.
#[test]
fn non_finite_string_form_numeric_fails_naming_the_value_and_declared_type() {
    let declared_type = DeclaredType::Decimal {
        precision: 12,
        scale: 4,
    };

    for spelling in ["nan", "inf", "-inf", "NaN", "Infinity", "-Infinity"] {
        let source: ArrayRef = Arc::new(StringArray::from(vec![spelling]));

        let message = coerce_one(source, declared_type)
            .expect_err(&format!("{spelling:?} must not coerce to a DECIMAL"));

        assert!(
            message.contains(spelling),
            "error message {message:?} does not cite the offending value {spelling:?}"
        );
        assert!(
            message.contains("DECIMAL(12,4)"),
            "error message {message:?} does not name the declared type"
        );
    }
}

/// A non-finite value must be refused per value, not per column: rejecting
/// one batch's `nan` must not leave the declared column unable to coerce a
/// later batch whose value is finite. The same [`Coercion`] is reused across
/// both calls here so a column-level memoization bug — refusing every future
/// batch once one bad value was seen — would fail this test.
#[test]
fn a_finite_value_coerces_after_a_non_finite_value_in_the_same_column() {
    let declared_type = DeclaredType::Decimal {
        precision: 10,
        scale: 2,
    };
    let columns = vec![declared("value", declared_type)];
    let coercion = Coercion::for_columns(&columns).expect("the declaration is representable");

    let non_finite_batch = batch_of("source", Arc::new(StringArray::from(vec!["nan"])));
    coercion
        .apply(&non_finite_batch)
        .expect_err("a non-finite string must not coerce");

    let finite_batch = batch_of("source", Arc::new(StringArray::from(vec!["1234.56"])));
    let coerced = coercion
        .apply(&finite_batch)
        .expect("a finite value must still coerce after an earlier batch's non-finite value");

    assert_eq!(
        coerced.column(0),
        &(Arc::new(
            Decimal128Array::from(vec![123_456])
                .with_precision_and_scale(10, 2)
                .expect("the fixture value fits the declared DECIMAL(10,2)")
        ) as ArrayRef)
    );
}

#[test]
fn a_scale_zero_decimal_beyond_eighteen_digits_emits_as_a_decimal() {
    let source: ArrayRef = Arc::new(Int64Array::from(vec![9_000_000_001]));

    let coerced = expect_coerced(
        source,
        DeclaredType::Decimal {
            precision: 19,
            scale: 0,
        },
    );

    assert_eq!(
        coerced.data_type(),
        &DataType::Decimal128(19, 0),
        "a DECIMAL(19,0) exceeds a 64-bit integer, so Exasol declares it as a numeric column"
    );
}
