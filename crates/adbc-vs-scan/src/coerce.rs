//! Drives every Arrow representation the MySQL driver may return into the
//! one representation each declared Exasol column requires.
//!
//! Exasol fixes a column's type at `CREATE VIRTUAL SCHEMA` time, but the
//! driver's Arrow type for the same MySQL column is not fixed: `DATETIME`
//! arrives as `timestamp[s]`, `[ms]`, `[us]`, or `[ns]` depending on how the
//! statement was written, and `DECIMAL` arrives at whatever decimal width —
//! or string or floating-point form — suits the driver. Every one of those
//! shapes carries the same value, so this module treats the declared type as
//! the authority and the driver's Arrow type as an input to convert, never
//! as something to trust or to derive a type from.
//!
//! Conversion is exact or it fails. A value that will not fit the declared
//! precision is reported, never truncated into a wrong number, and a null
//! stays a null of the declared type rather than becoming a zero, an empty
//! string, or an epoch instant.
//!
//! The one path that does not hold this exactly is `DATE`: Arrow's
//! `Date64`->`Date32` cast divides by milliseconds-per-day and truncates
//! toward zero rather than flooring, so a pre-epoch `Date64` carrying a
//! nonzero time-of-day lands one calendar day later than the correct one.
//! MySQL `DATE` never carries a time-of-day, so no value this adapter maps
//! reaches the divergence in practice, but the driver's own `Date64`
//! representation could.

use std::fmt;
use std::sync::Arc;

use arrow::array::{Array, ArrayRef, Decimal128Array, RecordBatch};
use arrow::compute::kernels::cast::{cast_with_options, CastOptions};
use arrow::datatypes::{DataType, Field, Schema, SchemaRef, TimeUnit};

use crate::error::ScanError;

/// Exasol's `TIMESTAMP` holds whole microseconds at most, and a microsecond
/// is also the finest unit MySQL's `DATETIME(6)` carries, so normalizing
/// every unit the driver may return to microseconds converts without
/// discarding anything either side can hold.
const TIMESTAMP_UNIT: TimeUnit = TimeUnit::Microsecond;

/// The widest decimal Arrow's 128-bit decimal holds. Exasol's own maximum is
/// lower, but that is the adapter's constraint to enforce when it declares a
/// column; this module's constraint is what it can carry.
const MAX_DECIMAL_PRECISION: u32 = 38;

/// How many digits of a scale-0 `DECIMAL` Exasol still represents as a 32-bit
/// and as a 64-bit integer column.
///
/// A declared `DECIMAL(p,0)` is not a decimal column inside Exasol: the
/// database declares it as `INT32` while it fits 32 bits and as `INT64` while
/// it fits 64, and only beyond that as a numeric column. The script language
/// container accepts exactly the Arrow type matching that decision, so a
/// `Decimal128` offered for a `DECIMAL(10,0)` column is a hard emit failure
/// rather than a silent widening. These two bounds are that decision
/// restated, and the reason the scale-0 cases leave the decimal path.
const MAX_INT32_DIGITS: u32 = 9;
const MAX_INT64_DIGITS: u32 = 18;

/// The Exasol type a scan output column was declared as.
///
/// `Text` covers Exasol's `VARCHAR` and `CHAR` alike: both emit as Arrow
/// `Utf8`, and the declared size is enforced by Exasol when it accepts the
/// row, so the scan needs no finer distinction than "this column is text".
///
/// `Boolean` corresponds to no MySQL column type: it is what Exasol declares
/// for a predicate it delegated as a select-list expression, which MySQL
/// returns as a number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeclaredType {
    Decimal { precision: u32, scale: u32 },
    Text,
    Date,
    Timestamp,
    Double,
    Boolean,
}

/// One declared output column of a scan: the name Exasol's `EMITS` clause
/// gave it and the Exasol type it was declared as.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclaredColumn {
    pub name: String,
    pub declared_type: DeclaredType,
}

impl fmt::Display for DeclaredType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DeclaredType::Decimal { precision, scale } => {
                write!(f, "DECIMAL({precision},{scale})")
            }
            DeclaredType::Text => write!(f, "text"),
            DeclaredType::Date => write!(f, "DATE"),
            DeclaredType::Timestamp => write!(f, "TIMESTAMP"),
            DeclaredType::Double => write!(f, "DOUBLE"),
            DeclaredType::Boolean => write!(f, "BOOLEAN"),
        }
    }
}

/// A scan's declared output columns, resolved once to the Arrow schema every
/// emitted batch must carry.
///
/// Resolving up front is what lets an impossible declaration fail before the
/// statement runs rather than on the first batch, and it means the emitted
/// schema is known before any row exists — which is exactly what an empty
/// result needs.
pub struct Coercion {
    schema: SchemaRef,
    declared: Vec<DeclaredColumn>,
}

impl Coercion {
    /// Resolves `columns` to the Arrow schema they require, or reports the
    /// column whose declared type Arrow cannot carry.
    pub fn for_columns(columns: &[DeclaredColumn]) -> Result<Coercion, ScanError> {
        let fields = columns
            .iter()
            .map(|column| {
                arrow_type_of(column).map(|data_type| Field::new(&column.name, data_type, true))
            })
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Coercion {
            schema: Arc::new(Schema::new(fields)),
            declared: columns.to_vec(),
        })
    }

    /// The Arrow schema every batch [`apply`](Self::apply) produces: the
    /// declared names, in the declared order, each carrying the Arrow
    /// representation its declared Exasol type requires.
    pub fn schema(&self) -> SchemaRef {
        Arc::clone(&self.schema)
    }

    /// Converts `batch` into the declared schema, column by column.
    ///
    /// A column already in its declared representation is passed through
    /// untouched, so the common case where the driver already agrees costs
    /// no copy.
    pub fn apply(&self, batch: &RecordBatch) -> Result<RecordBatch, ScanError> {
        if batch.num_columns() != self.declared.len() {
            return Err(ScanError::new(format!(
                "the driver returned a batch of {} columns but the scan declared {} output columns",
                batch.num_columns(),
                self.declared.len()
            )));
        }

        let columns = self
            .declared
            .iter()
            .zip(self.schema.fields())
            .enumerate()
            .map(|(position, (declared, field))| {
                convert(batch.column(position), field.data_type(), declared)
            })
            .collect::<Result<Vec<_>, _>>()?;

        RecordBatch::try_new(self.schema(), columns).map_err(|error| {
            ScanError::new(format!(
                "assembling the coerced batch for the scan's declared output columns: {error}"
            ))
        })
    }
}

fn convert(
    column: &ArrayRef,
    target: &DataType,
    declared: &DeclaredColumn,
) -> Result<ArrayRef, ScanError> {
    if column.data_type() == target {
        return Ok(Arc::clone(column));
    }

    cast_with_options(
        column,
        target,
        &CastOptions {
            safe: false,
            ..CastOptions::default()
        },
    )
    .map_err(|error| {
        ScanError::new(format!(
            "coercing column `{}` from the {:?} the driver returned to its declared {}: \
             {error}{}",
            declared.name,
            column.data_type(),
            declared.declared_type,
            unscaled_decimal_values(column)
        ))
    })
}

/// The raw unscaled digits `column` holds, if it is a `Decimal128` array.
///
/// A decimal cast failure's own message reports the value scaled — `999999999.9990`
/// rather than the `9999999999990` the source array actually stores — so the
/// digits an operator needs to find the offending row are appended here
/// instead of relying on the cast error to carry them.
fn unscaled_decimal_values(column: &ArrayRef) -> String {
    let Some(decimal) = column.as_any().downcast_ref::<Decimal128Array>() else {
        return String::new();
    };

    let values: Vec<String> = (0..decimal.len())
        .filter(|&row| !decimal.is_null(row))
        .map(|row| decimal.value(row).to_string())
        .collect();
    format!(" (unscaled source value(s): {})", values.join(", "))
}

fn arrow_type_of(column: &DeclaredColumn) -> Result<DataType, ScanError> {
    match column.declared_type {
        DeclaredType::Decimal { precision, scale } => decimal_type(column, precision, scale),
        DeclaredType::Text => Ok(DataType::Utf8),
        DeclaredType::Date => Ok(DataType::Date32),
        DeclaredType::Timestamp => Ok(DataType::Timestamp(TIMESTAMP_UNIT, None)),
        DeclaredType::Double => Ok(DataType::Float64),
        DeclaredType::Boolean => Ok(DataType::Boolean),
    }
}

fn decimal_type(
    column: &DeclaredColumn,
    precision: u32,
    scale: u32,
) -> Result<DataType, ScanError> {
    if precision == 0 || precision > MAX_DECIMAL_PRECISION {
        return Err(ScanError::new(format!(
            "column `{}` is declared {} but a scan carries a decimal of precision 1 to {MAX_DECIMAL_PRECISION}",
            column.name, column.declared_type
        )));
    }
    if scale > precision {
        return Err(ScanError::new(format!(
            "column `{}` is declared {}, whose scale exceeds its own precision",
            column.name, column.declared_type
        )));
    }
    if scale > 0 || precision > MAX_INT64_DIGITS {
        return Ok(DataType::Decimal128(precision as u8, scale as i8));
    }
    if precision > MAX_INT32_DIGITS {
        return Ok(DataType::Int64);
    }
    Ok(DataType::Int32)
}
