//! Converts a MySQL column definition into the Exasol data type the virtual
//! table declares, and names how the dialect reads the column so a delegated
//! fragment acts on the value the scan emits.
//!
//! The mapping is a pure function of MySQL catalog metadata, never of the
//! Arrow type the ADBC driver returns at scan time, because the driver's
//! `DATETIME` representation varies with the query. Every MySQL type maps
//! except the binary, blob, and spatial ones, as the Exasol MySQL JDBC Virtual
//! Schema maps them, with these departures from that dialect: an unsigned
//! integer keeps the digits of its unsigned range, a `DECIMAL` of at most 36
//! digits keeps its own precision, a `TIMESTAMP` keeps its own fractional
//! digits, and a `TINYINT(1)` is a number rather than a boolean. Unmapped is a
//! normal outcome, not a system failure: the caller omits the column and
//! records why.
//!
//! The Exasol bounds this mapping stays inside belong to no technology, so
//! they live in [`crate::type_mapping`] and are read from there.

use crate::exa_type::ExasolType;
use crate::type_mapping::{
    MappedColumn, MAX_CHAR_SIZE, MAX_DECIMAL_PRECISION, MAX_VARCHAR_SIZE, MIN_CHARACTER_SIZE,
};

/// The `DECIMAL` precision of a MySQL `TINYINT`: the digits of a signed or
/// unsigned 8-bit integer, which are 3 either way.
const TINYINT_PRECISION: u32 = 3;

/// The `DECIMAL` precision of a MySQL `SMALLINT`, signed or unsigned.
const SMALLINT_PRECISION: u32 = 5;

/// The `DECIMAL` precision of a signed MySQL `MEDIUMINT`: the digits of
/// 8388607.
const MEDIUMINT_PRECISION: u32 = 7;

/// The `DECIMAL` precision of an unsigned MySQL `MEDIUMINT`: the digits of
/// 16777215.
const UNSIGNED_MEDIUMINT_PRECISION: u32 = 8;

/// The `DECIMAL` precision of a MySQL `INT`, signed or unsigned.
const INT_PRECISION: u32 = 10;

/// The `DECIMAL` precision of a signed MySQL `BIGINT`.
const BIGINT_PRECISION: u32 = 19;

/// The `DECIMAL` precision of an unsigned MySQL `BIGINT`: the digits of
/// 18446744073709551615, which exceed [`BIGINT_PRECISION`].
const UNSIGNED_BIGINT_PRECISION: u32 = 20;

/// The `VARCHAR` size of a MySQL `TINYTEXT`: its storage limit in characters.
const TINYTEXT_SIZE: u32 = 255;

/// The `VARCHAR` size of a MySQL `TEXT`: its storage limit in characters.
const TEXT_SIZE: u32 = 65_535;

/// The fractional-second precision a `DATETIME`, `TIMESTAMP`, or `TIME`
/// declares when the catalog reports none: the most MySQL holds, so the
/// declaration loses no digit.
const MAX_DATETIME_PRECISION: u32 = 6;

/// A MySQL column as reported by `information_schema`: the fields the
/// mapping needs and nothing else.
///
/// `character_length` is the declared length of a character type, `ENUM`, or
/// `SET`, and `datetime_precision` the fractional-second digits of a
/// `DATETIME`, `TIMESTAMP`, or `TIME`. It carries no character set, because
/// every character column maps to `UTF8` whatever MySQL stores it in.
pub struct MySqlColumn {
    pub data_type: String,
    pub character_length: Option<u32>,
    pub numeric_precision: Option<u32>,
    pub numeric_scale: Option<u32>,
    pub datetime_precision: Option<u32>,
    pub is_unsigned: bool,
    pub is_nullable: bool,
}

/// Maps a MySQL column to its Exasol column definition, or returns the
/// reason it cannot be represented without loss.
///
/// `Err` here is not a system failure: an unmapped column is a normal,
/// expected outcome that the caller omits from the virtual table and
/// records rather than treating as fatal.
pub fn map_column(column: &MySqlColumn) -> Result<MappedColumn, String> {
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
/// The MySQL dialect renders every column reference through this one
/// function, so a delegated filter, grouping key, aggregate operand, and join
/// key act on the value Exasol receives. A column of a type that reads as
/// itself renders bare, which leaves a source index on it usable, and so does
/// a column without a recorded type, as a virtual schema created before
/// columns carried one declares it.
pub fn column_read_sql(source_type: Option<&str>, declared: ExasolType, column: &str) -> String {
    let Some(source_type) = source_type else {
        return column.to_string();
    };

    match (source_type.to_ascii_lowercase().as_str(), declared) {
        ("time", _) => format!("TIMESTAMP(DATE '1970-01-01', {column})"),
        ("year", _) => format!("MAKEDATE({column}, 1)"),
        ("bit", ExasolType::Boolean) => format!("({column} <> 0)"),
        ("json", _) | ("decimal", ExasolType::Varchar { .. }) => {
            format!("CAST({column} AS CHAR)")
        }
        _ => column.to_string(),
    }
}

/// The Exasol type a MySQL `DATA_TYPE` spelling maps to.
///
/// `DATA_TYPE` arrives lower case and is compared case-insensitively anyway,
/// because the comparison costs nothing and a catalog that spelled one
/// differently would otherwise omit every column of that type.
fn map_data_type(column: &MySqlColumn) -> Result<ExasolType, String> {
    let mysql_type = column.data_type.to_ascii_lowercase();
    match mysql_type.as_str() {
        "tinyint" => Ok(decimal(TINYINT_PRECISION)),
        "smallint" => Ok(decimal(SMALLINT_PRECISION)),
        "mediumint" => Ok(decimal(if column.is_unsigned {
            UNSIGNED_MEDIUMINT_PRECISION
        } else {
            MEDIUMINT_PRECISION
        })),
        "int" => Ok(decimal(INT_PRECISION)),
        "bigint" => Ok(decimal(if column.is_unsigned {
            UNSIGNED_BIGINT_PRECISION
        } else {
            BIGINT_PRECISION
        })),
        "decimal" => map_decimal(column.numeric_precision, column.numeric_scale),
        "float" | "double" => Ok(ExasolType::Double),
        "bit" => Ok(ExasolType::Boolean),
        "char" => Ok(ExasolType::Char {
            size: character_size(column.character_length, &mysql_type)?,
        }),
        "varchar" => Ok(ExasolType::Varchar {
            size: character_size(column.character_length, &mysql_type)?,
        }),
        "tinytext" => Ok(varchar(TINYTEXT_SIZE)),
        "text" => Ok(varchar(TEXT_SIZE)),
        "mediumtext" | "longtext" | "json" => Ok(varchar(MAX_VARCHAR_SIZE)),
        "enum" | "set" => map_enumeration(column.character_length, &mysql_type),
        "date" | "year" => Ok(ExasolType::Date),
        "datetime" | "timestamp" | "time" => Ok(ExasolType::Timestamp {
            precision: column.datetime_precision.unwrap_or(MAX_DATETIME_PRECISION),
        }),
        _ => Err(unmapped_reason(&mysql_type)),
    }
}

fn decimal(precision: u32) -> ExasolType {
    ExasolType::Decimal {
        precision,
        scale: 0,
    }
}

fn varchar(size: u32) -> ExasolType {
    ExasolType::Varchar { size }
}

/// Maps a `DECIMAL` of at most Exasol's maximum precision to Exasol
/// `DECIMAL(p,s)`, keeping the source's own precision and scale.
///
/// A wider one has no `DECIMAL` that holds it, so it maps to the text MySQL
/// prints for the value, which keeps every digit.
fn map_decimal(
    numeric_precision: Option<u32>,
    numeric_scale: Option<u32>,
) -> Result<ExasolType, String> {
    let precision = numeric_precision
        .ok_or_else(|| "MySQL DECIMAL column is missing a declared precision".to_string())?;
    if precision > MAX_DECIMAL_PRECISION {
        return Ok(varchar(MAX_VARCHAR_SIZE));
    }
    Ok(ExasolType::Decimal {
        precision,
        scale: numeric_scale.unwrap_or(0),
    })
}

/// Maps an `ENUM` or `SET` to Exasol `CHAR` of its catalog length up to
/// Exasol's maximum `CHAR` size, and to a `VARCHAR` of that length above it.
fn map_enumeration(declared_length: Option<u32>, mysql_type: &str) -> Result<ExasolType, String> {
    let size = character_size(declared_length, mysql_type)?;
    if size <= MAX_CHAR_SIZE {
        Ok(ExasolType::Char { size })
    } else {
        Ok(varchar(size))
    }
}

fn character_size(declared_length: Option<u32>, mysql_type: &str) -> Result<u32, String> {
    match declared_length {
        None => Err(format!(
            "MySQL {mysql_type} column reports no CHARACTER_MAXIMUM_LENGTH, so adbc-vs cannot declare its Exasol length"
        )),
        // Exasol rejects a zero-length declaration, so a reported 0 becomes 1
        // rather than failing CREATE VIRTUAL SCHEMA for a column MySQL accepts.
        Some(length) => Ok(length.clamp(MIN_CHARACTER_SIZE, MAX_VARCHAR_SIZE)),
    }
}

fn unmapped_reason(mysql_type: &str) -> String {
    let mysql_type = mysql_type.to_ascii_uppercase();
    format!(
        "MySQL type {mysql_type} is not mapped, because the Exasol MySQL JDBC Virtual Schema leaves binary, blob, and spatial columns out"
    )
}

#[cfg(test)]
#[path = "mysql_type_mapping_tests.rs"]
pub(crate) mod tests;
