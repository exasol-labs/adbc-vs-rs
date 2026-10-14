//! Converts a MySQL column definition into the Exasol data type the virtual
//! table declares.
//!
//! The mapping is a pure function of MySQL catalog metadata — never of the
//! Arrow type the ADBC driver returns at scan time, because the driver's
//! `DATETIME` representation varies with the query. Only eight MySQL types
//! map; every other type, and every unsigned integer, is unmapped. Unmapped
//! is a normal outcome, not a system failure: the caller omits the column
//! and records why.
//!
//! [`MappedType`] answers only "what Exasol type can a source column become",
//! and it and the Exasol bounds this mapping stays inside belong to no
//! technology, so they live in [`crate::type_mapping`] and are read from
//! there.

use crate::type_mapping::{MappedColumn, MappedType, MAX_DECIMAL_PRECISION, MIN_CHARACTER_SIZE};

/// The `DECIMAL` precision a MySQL `INT` column maps to.
const INT_PRECISION: u32 = 10;

/// The `DECIMAL` precision a MySQL `BIGINT` column maps to.
const BIGINT_PRECISION: u32 = 19;

/// The only MySQL character set this release maps a character column from.
///
/// The mapping declares every character column `UTF8`, and the MySQL dialect
/// renders every string literal in this character set, so a column of
/// another character set would be compared against a literal of a different
/// one. How MySQL converts such a literal is not measured for this release,
/// so a column of another character set is unmapped rather than declared
/// `UTF8`.
pub(crate) const SUPPORTED_CHARACTER_SET: &str = "utf8mb4";

/// A MySQL column as reported by `information_schema`: the fields the
/// mapping needs and nothing else.
pub struct MySqlColumn {
    pub data_type: String,
    pub character_length: Option<u32>,
    pub character_set: Option<String>,
    pub numeric_precision: Option<u32>,
    pub numeric_scale: Option<u32>,
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
        is_nullable: column.is_nullable,
    })
}

fn map_data_type(column: &MySqlColumn) -> Result<MappedType, String> {
    let mysql_type = column.data_type.to_ascii_uppercase();
    match mysql_type.as_str() {
        "INT" => {
            if column.is_unsigned {
                return Err(unsigned_reason(&mysql_type));
            }
            Ok(MappedType::Decimal {
                precision: INT_PRECISION,
                scale: 0,
            })
        }
        "BIGINT" => {
            if column.is_unsigned {
                return Err(unsigned_reason(&mysql_type));
            }
            Ok(MappedType::Decimal {
                precision: BIGINT_PRECISION,
                scale: 0,
            })
        }
        "DECIMAL" => map_decimal(column.numeric_precision, column.numeric_scale),
        "VARCHAR" | "CHAR" => map_character(column, &mysql_type),
        "DATE" => Ok(MappedType::Date),
        "DATETIME" => Ok(MappedType::Timestamp),
        "DOUBLE" => Ok(MappedType::Double),
        other => Err(format!(
            "MySQL type {other} is not one of the eight types this release maps"
        )),
    }
}

fn unsigned_reason(mysql_type: &str) -> String {
    format!(
        "MySQL {mysql_type} UNSIGNED is not supported in this release; the unsigned range exceeds the signed DECIMAL precision this type would otherwise map to"
    )
}

fn map_decimal(
    numeric_precision: Option<u32>,
    numeric_scale: Option<u32>,
) -> Result<MappedType, String> {
    let precision = numeric_precision
        .ok_or_else(|| "MySQL DECIMAL column is missing a declared precision".to_string())?;
    if precision > MAX_DECIMAL_PRECISION {
        return Err(format!(
            "MySQL DECIMAL precision {precision} exceeds Exasol's maximum DECIMAL precision of {MAX_DECIMAL_PRECISION}"
        ));
    }
    Ok(MappedType::Decimal {
        precision,
        scale: numeric_scale.unwrap_or(0),
    })
}

/// Maps a MySQL `VARCHAR` or `CHAR` column to Exasol `VARCHAR`.
///
/// Both MySQL types map to the same Exasol type: Exasol blank-pads a `CHAR`
/// value, and MySQL returns a `CHAR` value with its trailing spaces
/// stripped, so a mapped Exasol `CHAR` would declare a blank-padded type for
/// a value MySQL returns unpadded. Mapping to `VARCHAR` instead declares the
/// value MySQL actually returns.
fn map_character(column: &MySqlColumn, mysql_type: &str) -> Result<MappedType, String> {
    require_utf8mb4(column.character_set.as_deref(), mysql_type)?;
    Ok(MappedType::Varchar {
        size: character_size(column.character_length, mysql_type)?,
    })
}

fn require_utf8mb4(character_set: Option<&str>, mysql_type: &str) -> Result<(), String> {
    match character_set {
        Some(character_set) if character_set.eq_ignore_ascii_case(SUPPORTED_CHARACTER_SET) => {
            Ok(())
        }
        Some(character_set) => Err(format!(
            "MySQL {mysql_type} column has character set {character_set}, but this release only \
             maps character columns whose character set is {SUPPORTED_CHARACTER_SET}"
        )),
        None => Err(format!(
            "MySQL {mysql_type} column reports no CHARACTER_SET_NAME, so adbc-vs cannot confirm \
             it is {SUPPORTED_CHARACTER_SET}"
        )),
    }
}

fn character_size(declared_length: Option<u32>, mysql_type: &str) -> Result<u32, String> {
    match declared_length {
        None => Err(format!(
            "MySQL {mysql_type} column reports no CHARACTER_MAXIMUM_LENGTH, so adbc-vs cannot declare its Exasol length"
        )),
        // A reported length of 0 is clamped up to 1 rather than rejected:
        // Exasol rejects `VARCHAR(0)` outright, so declaring the column at its
        // reported length would fail `CREATE VIRTUAL SCHEMA` for a column
        // MySQL itself accepts.
        Some(length) => Ok(length.max(MIN_CHARACTER_SIZE)),
    }
}

#[cfg(test)]
#[path = "mysql_type_mapping_tests.rs"]
mod tests;
