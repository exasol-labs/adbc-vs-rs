//! The closed set of Exasol types a virtual column or an `EMITS` column may
//! declare, and the two encodings Exasol's Virtual Schema wire protocol
//! carries it in: the DDL fragment `CREATE VIRTUAL SCHEMA` and the `EMITS`
//! clause both need, and the JSON `dataType` object `schemaMetadata`,
//! `involvedTables[].columns[].dataType`, and `selectListDataTypes` all use.
//!
//! A mapped column and a select-list entry draw their declared type from this
//! one set, so the parser, every dialect, and the scan share one vocabulary. A
//! `TIMESTAMP` carries its fractional-second precision, because a declaration
//! without one holds only Exasol's default of 3 digits.

use serde_json::{json, Value};

/// The fractional-second precision Exasol gives a `TIMESTAMP` declared without
/// one.
pub(crate) const DEFAULT_TIMESTAMP_PRECISION: u32 = 3;

/// The `type` field spelling [`from_json`] accepts, in the order its `match`
/// tries them. Declaring the error message from this list rather than
/// spelling the count in prose keeps the message honest when a variant is
/// added or removed.
const DECLARABLE_TYPE_NAMES: &[&str] = &[
    "DECIMAL",
    "VARCHAR",
    "CHAR",
    "DATE",
    "TIMESTAMP",
    "DOUBLE",
    "BOOLEAN",
];

/// The Exasol type an `EMITS` column declares and a scan specification
/// carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExasolType {
    Decimal { precision: u32, scale: u32 },
    Varchar { size: u32 },
    Char { size: u32 },
    Date,
    Timestamp { precision: u32 },
    Double,
    Boolean,
}

/// Renders `exasol_type` as the DDL fragment Exasol expects after a column
/// name, in an `EMITS` clause or a `DESCRIBE` output.
pub fn to_ddl(exasol_type: ExasolType) -> String {
    match exasol_type {
        ExasolType::Decimal { precision, scale } => format!("DECIMAL({precision},{scale})"),
        ExasolType::Varchar { size } => format!("VARCHAR({size}) UTF8"),
        ExasolType::Char { size } => format!("CHAR({size}) UTF8"),
        ExasolType::Date => "DATE".to_string(),
        ExasolType::Timestamp { precision } => format!("TIMESTAMP({precision})"),
        ExasolType::Double => "DOUBLE".to_string(),
        ExasolType::Boolean => "BOOLEAN".to_string(),
    }
}

/// Renders `exasol_type` as the `dataType` JSON object Exasol's Virtual
/// Schema wire protocol carries for a column.
pub fn to_json(exasol_type: ExasolType) -> Value {
    match exasol_type {
        ExasolType::Decimal { precision, scale } => {
            json!({"type": "DECIMAL", "precision": precision, "scale": scale})
        }
        ExasolType::Varchar { size } => {
            json!({"type": "VARCHAR", "size": size, "characterSet": "UTF8"})
        }
        ExasolType::Char { size } => json!({"type": "CHAR", "size": size, "characterSet": "UTF8"}),
        ExasolType::Date => json!({"type": "DATE"}),
        ExasolType::Timestamp { precision } => json!({
            "type": "TIMESTAMP",
            "withLocalTimeZone": false,
            "fractionalSecondsPrecision": precision
        }),
        ExasolType::Double => json!({"type": "DOUBLE"}),
        ExasolType::Boolean => json!({"type": "BOOLEAN"}),
    }
}

/// Parses a `dataType` JSON object into an [`ExasolType`], the direction
/// every pushdown needs: `involvedTables[].columns[].dataType` carries back
/// exactly the shape [`to_json`] produced at `createVirtualSchema` time, and
/// `selectListDataTypes` carries Exasol's own answer for a select-list entry
/// the adapter never mapped.
///
/// Those two sources spell the same type differently, and only the fields
/// this function reads are load-bearing. A `TIMESTAMP` without
/// `fractionalSecondsPrecision`, as a column declared as a bare `TIMESTAMP`
/// echoes it, reads as Exasol's default precision of 3. A `TIMESTAMP` carrying
/// `withLocalTimeZone: true` fails rather than reading as `TIMESTAMP`, because
/// Exasol compares it in the session time zone. A `VARCHAR` from
/// `selectListDataTypes` carries no `characterSet`, so it is not required.
pub fn from_json(value: &Value) -> Result<ExasolType, String> {
    let type_name = value
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("dataType object {value} is missing its `type` field"))?;

    match type_name {
        "DECIMAL" => Ok(ExasolType::Decimal {
            precision: required_u32(value, "precision")?,
            scale: required_u32(value, "scale")?,
        }),
        "VARCHAR" => Ok(ExasolType::Varchar {
            size: required_u32(value, "size")?,
        }),
        "CHAR" => Ok(ExasolType::Char {
            size: required_u32(value, "size")?,
        }),
        "DATE" => Ok(ExasolType::Date),
        "TIMESTAMP" if has_local_time_zone(value) => Err(format!(
            "dataType `TIMESTAMP WITH LOCAL TIME ZONE` ({value}) is not one of the Exasol types \
             an EMITS column may declare: {}",
            DECLARABLE_TYPE_NAMES.join(", ")
        )),
        "TIMESTAMP" => Ok(ExasolType::Timestamp {
            precision: timestamp_precision(value)?,
        }),
        "DOUBLE" => Ok(ExasolType::Double),
        "BOOLEAN" => Ok(ExasolType::Boolean),
        other => Err(format!(
            "dataType `{other}` is not one of the Exasol types an EMITS column may declare: {}",
            DECLARABLE_TYPE_NAMES.join(", ")
        )),
    }
}

fn has_local_time_zone(value: &Value) -> bool {
    value.get("withLocalTimeZone").and_then(Value::as_bool) == Some(true)
}

fn timestamp_precision(value: &Value) -> Result<u32, String> {
    const FIELD: &str = "fractionalSecondsPrecision";
    match value.get(FIELD) {
        None => Ok(DEFAULT_TIMESTAMP_PRECISION),
        Some(_) => required_u32(value, FIELD),
    }
}

fn required_u32(value: &Value, field: &str) -> Result<u32, String> {
    value
        .get(field)
        .and_then(Value::as_u64)
        .and_then(|number| u32::try_from(number).ok())
        .ok_or_else(|| format!("dataType object {value} is missing a numeric `{field}` field"))
}

#[cfg(test)]
#[path = "exa_type_tests.rs"]
mod tests;
