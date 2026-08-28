//! The Exasol data type an `EMITS` column may declare, and the two encodings
//! Exasol's Virtual Schema wire protocol carries it in: the DDL fragment
//! `CREATE VIRTUAL SCHEMA` and the `EMITS` clause both need, and the JSON
//! `dataType` object `schemaMetadata`, `involvedTables[].columns[].dataType`,
//! and `selectListDataTypes` all use.
//!
//! This is the declarable set, and it is deliberately wider in both
//! directions than [`MappedType`](crate::type_mapping::MappedType), the set a
//! MySQL column may map to. It adds `Boolean`, which Exasol declares for a
//! predicate it delegated into the select list and which no MySQL column this
//! adapter maps can be. It keeps `Char`, which no mapping produces since
//! MySQL `CHAR` began mapping to Exasol `VARCHAR`, because a virtual schema
//! release 0.1 created still declares Exasol `CHAR` in the
//! `involvedTables[].columns[].dataType` every pushdown echoes back until an
//! operator refreshes it.
//!
//! Nothing here touches MySQL. The widening from the mapped set lives with
//! the mapping that produces it, and there is no conversion back: the two
//! extra variants answer to no MySQL column.

use serde_json::{json, Value};

/// The `type` field spelling [`from_json`] accepts, in the order its `match`
/// tries them. Declaring the error message from this list rather than
/// spelling the count in prose keeps the message honest when a variant is
/// added or removed, as `MappedType` just was.
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
    Timestamp,
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
        ExasolType::Timestamp => "TIMESTAMP".to_string(),
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
        ExasolType::Timestamp => json!({"type": "TIMESTAMP"}),
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
/// this function reads are load-bearing. A `TIMESTAMP` from
/// `selectListDataTypes` also carries `fractionalSecondsPrecision`, which is
/// read past: [`ExasolType`] holds no precision, so [`to_ddl`] renders a bare
/// `TIMESTAMP` and Exasol's own default of 3 applies — the only precision a
/// virtual table this adapter declared can echo back. A `VARCHAR` from the
/// same source carries no `characterSet`, so it is not required.
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
        "TIMESTAMP" => Ok(ExasolType::Timestamp),
        "DOUBLE" => Ok(ExasolType::Double),
        "BOOLEAN" => Ok(ExasolType::Boolean),
        other => Err(format!(
            "dataType `{other}` is not one of the Exasol types an EMITS column may declare: {}",
            DECLARABLE_TYPE_NAMES.join(", ")
        )),
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
