//! Converts a PostgreSQL column definition into the Exasol data type the
//! virtual table declares.
//!
//! The mapping is a pure function of PostgreSQL catalog metadata — never of
//! the Arrow type the ADBC driver returns at scan time — because Exasol must
//! declare an exact column type at `CREATE VIRTUAL SCHEMA` time and the
//! driver's representation of a value varies with the query that returned it.
//!
//! Seven PostgreSQL types map: `integer`, `bigint`, `numeric(p,s)`,
//! `character varying(n)`, `date`, `timestamp without time zone`, and `double
//! precision`. Every other type is unmapped, with the reason naming the type
//! the catalog reported. Unmapped is a normal outcome, not a system failure:
//! the caller omits the column and records why, per `vs-adapter/type-mapping`.
//!
//! PostgreSQL states its table independently of MySQL's rather than deriving
//! one from the other. The two agree on `DECIMAL(10,0)` for a 32-bit integer
//! and `DECIMAL(19,0)` for a 64-bit one because those digit counts follow from
//! the integer widths, which are not a property of either source; the sources
//! disagree elsewhere, so nothing here reads MySQL's table. What both do read
//! is Exasol's own type bounds, which belong to neither source.

use crate::type_mapping::{
    MappedColumn, MappedType, MAX_DECIMAL_PRECISION, MAX_VARCHAR_SIZE, MIN_CHARACTER_SIZE,
};

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

/// A PostgreSQL column as `information_schema.columns` reports it: the fields
/// the mapping reads and nothing else.
///
/// It carries no character set, because PostgreSQL declares an encoding per
/// database rather than per column, and no unsigned flag, because PostgreSQL
/// has no unsigned integer type. Both are MySQL catalog facts with no
/// PostgreSQL counterpart, which is why no column struct is shared between the
/// two sources.
pub struct PostgresColumn {
    pub data_type: String,
    pub character_length: Option<u32>,
    pub numeric_precision: Option<u32>,
    pub numeric_scale: Option<u32>,
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
        is_nullable: column.is_nullable,
    })
}

/// The Exasol type a PostgreSQL `data_type` spelling maps to.
///
/// `data_type` is matched as `information_schema` spells it — lower case, and
/// the SQL standard's name rather than the internal one, so a `varchar` column
/// reports `character varying` and a `bool` column reports `boolean`. Compared
/// case-insensitively anyway, because the comparison costs nothing and a
/// catalog that spelled one differently would otherwise silently unmap every
/// column of that type.
///
/// Four unmapped types answer with their own reason rather than the general
/// one, because each is a type an operator has a specific question about: the
/// three Exasol has a counterpart for and would expect mapped, and `text`,
/// whose only obstacle is a length the catalog does not report.
fn map_data_type(column: &PostgresColumn) -> Result<MappedType, String> {
    match column.data_type.to_ascii_lowercase().as_str() {
        "integer" => Ok(MappedType::Decimal {
            precision: INTEGER_PRECISION,
            scale: 0,
        }),
        "bigint" => Ok(MappedType::Decimal {
            precision: BIGINT_PRECISION,
            scale: 0,
        }),
        "numeric" => map_numeric(column),
        "character varying" => map_character_varying(column),
        "date" => Ok(MappedType::Date),
        "timestamp without time zone" => Ok(MappedType::Timestamp),
        "double precision" => Ok(MappedType::Double),
        "character" => Err(blank_padded_reason(&column.data_type)),
        "boolean" => Err(boolean_reason(&column.data_type)),
        "text" => Err(unbounded_length_reason(&column.data_type)),
        "timestamp with time zone" | "timestamptz" => Err(time_zone_reason(&column.data_type)),
        _ => Err(format!(
            "PostgreSQL type {} is not one of the types adbc-vs maps",
            column.data_type
        )),
    }
}

/// Maps a `numeric(p,s)` to Exasol `DECIMAL(p,s)`, keeping the source's own
/// precision and scale.
///
/// An unconstrained `numeric` reports no precision and is refused rather than
/// given Exasol's maximum: Exasol compares a declared type for exact equality,
/// and an arbitrary-precision source value need not fit whatever was
/// substituted. A scale the catalog leaves null on a column that does report a
/// precision is 0, which is what `numeric(p)` means.
fn map_numeric(column: &PostgresColumn) -> Result<MappedType, String> {
    let precision = column.numeric_precision.ok_or_else(|| {
        "PostgreSQL numeric column declares no precision, as an unconstrained `numeric` reports \
         none, so adbc-vs cannot declare the exact Exasol DECIMAL precision Exasol compares for \
         equality"
            .to_string()
    })?;
    if precision > MAX_DECIMAL_PRECISION {
        return Err(format!(
            "PostgreSQL numeric precision {precision} exceeds Exasol's maximum DECIMAL precision \
             of {MAX_DECIMAL_PRECISION}"
        ));
    }
    Ok(MappedType::Decimal {
        precision,
        scale: column.numeric_scale.unwrap_or(0),
    })
}

/// Maps a `character varying(n)` to Exasol `VARCHAR(n)`, refusing the two
/// lengths Exasol has no type for.
fn map_character_varying(column: &PostgresColumn) -> Result<MappedType, String> {
    let length = column
        .character_length
        .ok_or_else(|| unbounded_length_reason(&column.data_type))?;
    let size = length.max(MIN_CHARACTER_SIZE);
    if size > MAX_VARCHAR_SIZE {
        return Err(format!(
            "PostgreSQL {} length {length} exceeds Exasol's maximum VARCHAR size of \
             {MAX_VARCHAR_SIZE}",
            column.data_type
        ));
    }
    Ok(MappedType::Varchar { size })
}

fn blank_padded_reason(data_type: &str) -> String {
    format!(
        "PostgreSQL type {data_type} is blank-padded to its declared length: PostgreSQL returns \
         the value padded and compares two of them ignoring trailing spaces, while Exasol VARCHAR \
         does neither, and no column adbc-vs maps produces Exasol CHAR"
    )
}

fn boolean_reason(data_type: &str) -> String {
    format!(
        "PostgreSQL type {data_type} is not mapped to Exasol BOOLEAN, because a mapped boolean \
         column would widen the set of Exasol types a source column produces and bring a boolean \
         operand into every predicate path"
    )
}

fn unbounded_length_reason(data_type: &str) -> String {
    format!(
        "PostgreSQL type {data_type} declares no character length, so adbc-vs cannot declare the \
         exact Exasol VARCHAR length Exasol compares for equality"
    )
}

fn time_zone_reason(data_type: &str) -> String {
    format!(
        "PostgreSQL type {data_type} is not mapped to Exasol TIMESTAMP, because the two carry \
         different instants for one wall-clock value"
    )
}

#[cfg(test)]
#[path = "postgres_type_mapping_tests.rs"]
mod tests;
