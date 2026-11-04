//! Converts a PostgreSQL column definition into the Exasol data type the
//! virtual table declares, and names how the dialect reads the column so a
//! delegated fragment acts on the value the scan emits.
//!
//! The mapping is a pure function of PostgreSQL catalog metadata — never of
//! the Arrow type the ADBC driver returns at scan time — because Exasol must
//! declare an exact column type at `CREATE VIRTUAL SCHEMA` time and the
//! driver's representation of a value varies with the query that returned it.
//!
//! Every PostgreSQL type maps except an array, as the Exasol PostgreSQL JDBC
//! Virtual Schema maps them. A type with an Exasol counterpart maps to it, with
//! two departures from that dialect: a `numeric(p,s)` of at most 36 digits
//! keeps its own `DECIMAL(p,s)`, and a `timestamp` keeps its own fractional
//! digits. A type with no counterpart maps to `VARCHAR` and is read as
//! PostgreSQL's text for the value, so the scan and every delegated comparison
//! see the same text. Unmapped is a normal outcome, not a system failure: the
//! caller omits the column and records why, per `vs-adapter/type-mapping`.
//!
//! PostgreSQL states its table independently of MySQL's rather than deriving
//! one from the other. The two agree on `DECIMAL(10,0)` for a 32-bit integer
//! and `DECIMAL(19,0)` for a 64-bit one because those digit counts follow from
//! the integer widths, which are not a property of either source; the sources
//! disagree elsewhere, so nothing here reads MySQL's table. What both do read
//! is Exasol's own type bounds, which belong to neither source.

use crate::exa_type::ExasolType;
use crate::type_mapping::{
    MappedColumn, MAX_CHAR_SIZE, MAX_DECIMAL_PRECISION, MAX_VARCHAR_SIZE, MIN_CHARACTER_SIZE,
};

/// The `DECIMAL` precision a PostgreSQL `smallint` column maps to: the digits
/// of a signed 16-bit integer. Stated for the same reason as
/// [`INTEGER_PRECISION`].
const SMALLINT_PRECISION: u32 = 5;

/// The `DECIMAL` precision a PostgreSQL `integer` column maps to: the digits
/// of a signed 32-bit integer.
///
/// Stated rather than read from `numeric_precision`, which PostgreSQL reports
/// in the radix `numeric_precision_radix` names — 2 for the integer types, so
/// an `integer` column reports 32 and a `bigint` column 64.
const INTEGER_PRECISION: u32 = 10;

/// The `DECIMAL` precision a PostgreSQL `bigint` column maps to: the digits of
/// a signed 64-bit integer. Stated for the same reason as
/// [`INTEGER_PRECISION`].
const BIGINT_PRECISION: u32 = 19;

/// The `VARCHAR` size of a `time` column read as text: `12:34:56.123456+01:30`
/// is 21 characters, and the JDBC Virtual Schema declares 100 for the same
/// type.
const TIME_TEXT_SIZE: u32 = 100;

/// The fractional-second precision a `timestamp` carries when the catalog
/// reports none: PostgreSQL's own default.
const DEFAULT_POSTGRES_TIMESTAMP_PRECISION: u32 = 6;

/// A PostgreSQL column as `information_schema.columns` reports it: the fields
/// the mapping reads and nothing else.
///
/// It carries no character set, because PostgreSQL declares an encoding per
/// database rather than per column, and no unsigned flag, because PostgreSQL
/// has no unsigned integer type. Both are MySQL catalog facts with no
/// PostgreSQL counterpart, which is why no column struct is shared between the
/// two sources.
///
/// `character_length` is the declared length of a character or bit-string
/// type, and `datetime_precision` the fractional-second digits of a `time` or
/// `timestamp`.
pub struct PostgresColumn {
    pub data_type: String,
    pub character_length: Option<u32>,
    pub numeric_precision: Option<u32>,
    pub numeric_scale: Option<u32>,
    pub datetime_precision: Option<u32>,
    pub is_nullable: bool,
}

/// Maps a PostgreSQL column to its Exasol column definition, or returns the
/// reason it cannot be represented without loss.
///
/// `Err` here is not a system failure: an unmapped column is a normal,
/// expected outcome that the caller omits from the virtual table and records
/// rather than treating as fatal.
pub fn map_column(column: &PostgresColumn) -> Result<MappedColumn, String> {
    let exasol_type = map_data_type(column)?;
    Ok(MappedColumn {
        exasol_type,
        source_type: column.data_type.clone(),
        is_nullable: column.is_nullable,
    })
}

/// The SQL that reads `column`, already quoted, as the value the scan emits
/// for `declared`, selected by the `source_type` the column's `adapterNotes`
/// recorded.
///
/// The PostgreSQL dialect renders every column reference through this one
/// function, so a delegated filter, grouping key, aggregate operand, and join
/// key act on the value Exasol receives. A column of a type that reads as
/// itself renders bare, which leaves a source index on it usable. A column
/// without a recorded type, as a virtual schema created before columns carried
/// one declares it, renders bare too.
pub fn column_read_sql(source_type: Option<&str>, declared: ExasolType, column: &str) -> String {
    let Some(source_type) = source_type else {
        return column.to_string();
    };

    match (source_type.to_ascii_lowercase().as_str(), declared) {
        ("money", ExasolType::Double) => {
            format!("CAST(CAST({column} AS NUMERIC) AS DOUBLE PRECISION)")
        }
        ("bit", ExasolType::Boolean) => format!("(position(B'1' IN {column}) > 0)"),
        ("character varying" | "text", _) => column.to_string(),
        (_, ExasolType::Varchar { .. }) => format!("CAST({column} AS TEXT)"),
        _ => column.to_string(),
    }
}

/// The Exasol type a PostgreSQL `data_type` spelling maps to.
///
/// `data_type` is matched as `information_schema` spells it — lower case, and
/// the SQL standard's name rather than the internal one, so a `varchar` column
/// reports `character varying` and a `bool` column reports `boolean`. Compared
/// case-insensitively anyway, because the comparison costs nothing and a
/// catalog that spelled one differently would otherwise silently map every
/// column of that type to the text fallback.
///
/// Any spelling this match does not name, including `USER-DEFINED` for an enum
/// and a type a later PostgreSQL adds, is `VARCHAR` read as text: the text
/// form exists for every PostgreSQL type, so the fallback never loses a column.
fn map_data_type(column: &PostgresColumn) -> Result<ExasolType, String> {
    let varchar = |size: u32| ExasolType::Varchar { size };
    let decimal = |precision: u32| ExasolType::Decimal {
        precision,
        scale: 0,
    };

    match column.data_type.to_ascii_lowercase().as_str() {
        "smallint" => Ok(decimal(SMALLINT_PRECISION)),
        "integer" => Ok(decimal(INTEGER_PRECISION)),
        "bigint" => Ok(decimal(BIGINT_PRECISION)),
        "numeric" => Ok(map_numeric(column)),
        "real" | "double precision" | "money" => Ok(ExasolType::Double),
        "boolean" | "bit" => Ok(ExasolType::Boolean),
        "bit varying" => Ok(varchar(bounded_size(column.character_length))),
        "character" => Ok(map_character(column)),
        "character varying" => Ok(varchar(bounded_size(column.character_length))),
        "text" => Ok(varchar(MAX_VARCHAR_SIZE)),
        "time without time zone" | "time with time zone" => Ok(varchar(TIME_TEXT_SIZE)),
        "date" => Ok(ExasolType::Date),
        "timestamp without time zone" | "timestamp with time zone" => Ok(ExasolType::Timestamp {
            precision: column
                .datetime_precision
                .unwrap_or(DEFAULT_POSTGRES_TIMESTAMP_PRECISION),
        }),
        "array" => Err(array_reason(&column.data_type)),
        _ => Ok(varchar(MAX_VARCHAR_SIZE)),
    }
}

/// Maps a `numeric(p,s)` of at most Exasol's maximum precision to Exasol
/// `DECIMAL(p,s)`, keeping the source's own precision and scale.
///
/// An unconstrained `numeric` reports no precision, and one beyond Exasol's
/// maximum has no `DECIMAL` that holds it. Both map to the text PostgreSQL
/// prints for the value, which keeps every digit, the trailing zeros, and the
/// `NaN` and `Infinity` an unconstrained `numeric` can hold. A scale the
/// catalog leaves null on a column that does report a precision is 0, which is
/// what `numeric(p)` means.
fn map_numeric(column: &PostgresColumn) -> ExasolType {
    match column.numeric_precision {
        Some(precision) if precision <= MAX_DECIMAL_PRECISION => ExasolType::Decimal {
            precision,
            scale: column.numeric_scale.unwrap_or(0),
        },
        _ => ExasolType::Varchar {
            size: MAX_VARCHAR_SIZE,
        },
    }
}

/// Maps a `character(n)` to Exasol `CHAR(n)` up to Exasol's maximum `CHAR`
/// size, and to a `VARCHAR` of the same size, capped at Exasol's maximum
/// `VARCHAR`, above it.
///
/// A column without a reported length is `VARCHAR` of the maximum, which holds
/// any value the source could.
fn map_character(column: &PostgresColumn) -> ExasolType {
    match column.character_length {
        Some(length) if length.max(MIN_CHARACTER_SIZE) <= MAX_CHAR_SIZE => ExasolType::Char {
            size: length.max(MIN_CHARACTER_SIZE),
        },
        length => ExasolType::Varchar {
            size: bounded_size(length),
        },
    }
}

/// A declared character length as an Exasol `VARCHAR` size: at least Exasol's
/// minimum, at most its maximum, and the maximum for a length the catalog does
/// not report, which is an unbounded type.
fn bounded_size(length: Option<u32>) -> u32 {
    length
        .map(|length| length.clamp(MIN_CHARACTER_SIZE, MAX_VARCHAR_SIZE))
        .unwrap_or(MAX_VARCHAR_SIZE)
}

fn array_reason(data_type: &str) -> String {
    format!(
        "PostgreSQL type {data_type} is not mapped, because the catalog names no element type and \
         the Exasol PostgreSQL JDBC Virtual Schema leaves arrays out too"
    )
}

#[cfg(test)]
#[path = "postgres_type_mapping_tests.rs"]
pub(crate) mod tests;
