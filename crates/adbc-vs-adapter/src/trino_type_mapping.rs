//! Converts a Trino column definition into the Exasol data type the virtual
//! table declares.
//!
//! Trino's `information_schema.columns` carries no `character_maximum_length`,
//! `numeric_precision`, `numeric_scale`, or `datetime_precision` column at
//! all — those names are absent rather than null, so selecting one fails the
//! catalog query. Every declared length, precision, and scale therefore lives
//! inside the `data_type` string, and this mapping parses where MySQL's and
//! PostgreSQL's read a field of their own.
//!
//! `data_type` is Trino's own type display form, and this release's parser
//! reads exactly three shapes: a bare name (`integer`), a name carrying one
//! parameter (`varchar(100)`), or a name carrying a precision and a scale
//! separated by a comma with no space (`decimal(12,4)`). A spelling outside
//! those three shapes — a trailing clause like `with time zone`, a
//! parenthesized list separated by a comma *with* a space such as
//! `map(varchar, integer)`, or a parameter that is not a whole number — is
//! read as unparsed rather than forced into one of them, so it is never
//! mistaken for the bare name that precedes its parameters: the omission
//! names the value exactly as the catalog spelled it.
//!
//! Seven Trino types map: `integer`, `bigint`, `decimal(p,s)`, `varchar(n)`,
//! `date`, `timestamp(p)` at any precision, and `double`. Every other type is
//! unmapped, with the reason naming the type the catalog reported. Unmapped
//! is a normal outcome, not a system failure: the caller omits the column and
//! records why, per `vs-adapter/type-mapping`.

use crate::type_mapping::{
    MappedColumn, MappedType, MAX_DECIMAL_PRECISION, MAX_VARCHAR_SIZE, MIN_CHARACTER_SIZE,
};

/// The `DECIMAL` precision a Trino `integer` column maps to: the digits of a
/// signed 32-bit integer. Stated rather than read from the catalog, which
/// reports no precision column at all.
const INTEGER_PRECISION: u32 = 10;

/// The `DECIMAL` precision a Trino `bigint` column maps to: the digits of a
/// signed 64-bit integer. Stated for the same reason as [`INTEGER_PRECISION`].
const BIGINT_PRECISION: u32 = 19;

/// The suffix that turns a parsed `timestamp(p)` spelling into `timestamp(p)
/// with time zone`, which carries a different instant for one wall-clock
/// value than Exasol `TIMESTAMP` does and so is never mapped to it.
const TIME_ZONE_SUFFIX: &str = ") with time zone";

/// A Trino column as `information_schema.columns` reports it: the type
/// string the mapping parses, and nothing else. Trino's catalog carries no
/// length, precision, or scale column of its own, so `data_type` is the only
/// field this mapping reads.
pub struct TrinoColumn {
    pub data_type: String,
    pub is_nullable: bool,
}

/// Maps a Trino column to its Exasol column definition, or returns the
/// reason it cannot be represented without loss.
///
/// `Err` here is not a system failure: an unmapped column is a normal,
/// expected outcome that the caller omits from the virtual table and records
/// rather than treating as fatal.
pub fn map_column(column: &TrinoColumn) -> Result<MappedColumn, String> {
    let exasol_type = map_data_type(column)?;
    Ok(MappedColumn {
        exasol_type,
        is_nullable: column.is_nullable,
    })
}

/// One of the three `data_type` shapes this release's parser reads.
///
/// `name` and any parameter are borrowed out of the original string rather
/// than copied, so a shape that parses but names a type this release does not
/// map still carries the exact original spelling forward into its reason.
enum ParsedType<'a> {
    Bare(&'a str),
    OneParam {
        name: &'a str,
        param: u32,
    },
    PrecisionScale {
        name: &'a str,
        precision: u32,
        scale: u32,
    },
}

/// Reads `data_type` as a bare name, a name with one parameter, or a name
/// with a precision and a scale separated by a comma with no space.
///
/// Returns `None` for anything outside those three shapes — including a name
/// followed by `(...)` whose content is not purely digits, or a comma
/// followed by a space, which is how Trino spells `map(varchar, integer)`
/// rather than a precision and a scale. The caller falls back to the full
/// original string in that case, never to `name` alone.
fn parse_data_type(data_type: &str) -> Option<ParsedType<'_>> {
    let Some((name, rest)) = data_type.split_once('(') else {
        return Some(ParsedType::Bare(data_type));
    };
    let inner = rest.strip_suffix(')')?;
    match inner.split_once(',') {
        Some((precision, scale)) => {
            if precision.contains(' ') || scale.contains(' ') {
                return None;
            }
            Some(ParsedType::PrecisionScale {
                name,
                precision: precision.parse().ok()?,
                scale: scale.parse().ok()?,
            })
        }
        None => Some(ParsedType::OneParam {
            name,
            param: inner.parse().ok()?,
        }),
    }
}

/// Whether `data_type` is a `timestamp(p) with time zone` of any precision.
///
/// Checked ahead of [`parse_data_type`] rather than folded into its three
/// grammars: the trailing `with time zone` clause is not one of them, and
/// this spelling needs its own reason naming the instant disagreement rather
/// than the generic "not one of the types adbc-vs maps".
fn is_timestamp_with_time_zone(data_type: &str) -> bool {
    data_type.starts_with("timestamp(") && data_type.ends_with(TIME_ZONE_SUFFIX)
}

fn map_data_type(column: &TrinoColumn) -> Result<MappedType, String> {
    let data_type = column.data_type.as_str();

    if is_timestamp_with_time_zone(data_type) {
        return Err(time_zone_reason(data_type));
    }

    match parse_data_type(data_type) {
        Some(ParsedType::Bare("integer")) => Ok(MappedType::Decimal {
            precision: INTEGER_PRECISION,
            scale: 0,
        }),
        Some(ParsedType::Bare("bigint")) => Ok(MappedType::Decimal {
            precision: BIGINT_PRECISION,
            scale: 0,
        }),
        Some(ParsedType::Bare("date")) => Ok(MappedType::Date),
        Some(ParsedType::Bare("double")) => Ok(MappedType::Double),
        Some(ParsedType::Bare("varchar")) => Err(unbounded_varchar_reason(data_type)),
        Some(ParsedType::Bare("boolean")) => Err(boolean_reason(data_type)),
        Some(ParsedType::OneParam {
            name: "varchar",
            param,
        }) => map_varchar(param, data_type),
        Some(ParsedType::OneParam { name: "char", .. }) => Err(char_reason(data_type)),
        Some(ParsedType::OneParam {
            name: "timestamp", ..
        }) => Ok(MappedType::Timestamp),
        Some(ParsedType::PrecisionScale {
            name: "decimal",
            precision,
            scale,
        }) => map_decimal(precision, scale, data_type),
        _ => Err(not_mapped_reason(data_type)),
    }
}

/// Maps `varchar(n)` to Exasol `VARCHAR(n)`, raising a declared length of 0 to
/// Exasol's minimum and refusing a length beyond Exasol's maximum.
fn map_varchar(param: u32, data_type: &str) -> Result<MappedType, String> {
    let size = param.max(MIN_CHARACTER_SIZE);
    if size > MAX_VARCHAR_SIZE {
        return Err(format!(
            "Trino type {data_type} declares length {param}, which exceeds Exasol's maximum \
             VARCHAR size of {MAX_VARCHAR_SIZE}"
        ));
    }
    Ok(MappedType::Varchar { size })
}

/// Maps `decimal(p,s)` to Exasol `DECIMAL(p,s)`, refusing a precision beyond
/// Exasol's maximum rather than Trino's own — Trino admits up to 38, Exasol
/// only 36.
fn map_decimal(precision: u32, scale: u32, data_type: &str) -> Result<MappedType, String> {
    if precision > MAX_DECIMAL_PRECISION {
        return Err(format!(
            "Trino type {data_type} declares precision {precision}, which exceeds Exasol's \
             maximum DECIMAL precision of {MAX_DECIMAL_PRECISION}"
        ));
    }
    Ok(MappedType::Decimal { precision, scale })
}

fn char_reason(data_type: &str) -> String {
    format!(
        "Trino type {data_type} is blank-padded to its declared length: Trino returns the value \
         padded and compares two of them ignoring trailing spaces while a pattern match over them \
         does not, so a delegated LIKE over a mapped column would disagree with a delegated \
         equality over the same column; Exasol VARCHAR does neither and no column adbc-vs maps \
         produces Exasol CHAR"
    )
}

fn boolean_reason(data_type: &str) -> String {
    format!(
        "Trino type {data_type} is not mapped to Exasol BOOLEAN, because a mapped boolean column \
         would widen the set of Exasol types a source column produces and bring a boolean operand \
         into every predicate path"
    )
}

fn unbounded_varchar_reason(data_type: &str) -> String {
    format!(
        "Trino type {data_type} declares no length, because this spelling is Trino's unbounded \
         varchar rather than a zero-length one, and adbc-vs cannot declare the exact Exasol \
         VARCHAR length Exasol compares for equality"
    )
}

fn time_zone_reason(data_type: &str) -> String {
    format!(
        "Trino type {data_type} is not mapped to Exasol TIMESTAMP, because the two carry \
         different instants for one wall-clock value"
    )
}

fn not_mapped_reason(data_type: &str) -> String {
    format!("Trino type {data_type} is not one of the types adbc-vs maps")
}

#[cfg(test)]
#[path = "trino_type_mapping_tests.rs"]
mod tests;
