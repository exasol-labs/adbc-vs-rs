//! What every source's type mapping produces, and the Exasol bounds every one
//! of them maps within.
//!
//! Nothing here names a technology. Which of a source's types map, and what
//! its catalog calls them, is that technology's own module — `mysql_type_
//! mapping`, `postgres_type_mapping` — and each one answers in the vocabulary
//! this module defines, so `create_schema` assembles the same response
//! whichever source produced it.
//!
//! The bounds live here for the same reason: they are Exasol's, not any
//! source's. A source that declared a type wider than Exasol accepts would
//! fail the whole `CREATE VIRTUAL SCHEMA` instead of omitting one column, so
//! every mapping reads the same three constants rather than restating them.

use crate::exa_type;

/// Exasol's maximum `DECIMAL` precision.
pub(crate) const MAX_DECIMAL_PRECISION: u32 = 36;

/// Exasol's maximum `VARCHAR` size.
pub(crate) const MAX_VARCHAR_SIZE: u32 = 2_000_000;

/// Exasol's minimum character-type size. Exasol rejects `VARCHAR(0)`, so a
/// source length of 0 is raised to this rather than declared as reported.
pub(crate) const MIN_CHARACTER_SIZE: u32 = 1;

/// The Exasol type a mapped source column declares in the virtual table.
///
/// Exactly five, and no more: no source this release maps produces Exasol
/// `BOOLEAN` or `CHAR`, though both are declarable in an `EMITS` clause. That
/// is why the declarable set is its own type rather than this one widened — a
/// `MappedType` no source can produce would be a state every match over a
/// mapped column had to carry an unreachable arm for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MappedType {
    Decimal { precision: u32, scale: u32 },
    Varchar { size: u32 },
    Date,
    Timestamp,
    Double,
}

/// Widens a mapped type into the set an `EMITS` column may declare.
///
/// One-way by construction. Every type a source column maps to is declarable,
/// so this direction is total and lossless; the reverse would have to name the
/// source column an Exasol `BOOLEAN` or `CHAR` came from, and there is none.
impl From<MappedType> for exa_type::ExasolType {
    fn from(mapped: MappedType) -> Self {
        match mapped {
            MappedType::Decimal { precision, scale } => {
                exa_type::ExasolType::Decimal { precision, scale }
            }
            MappedType::Varchar { size } => exa_type::ExasolType::Varchar { size },
            MappedType::Date => exa_type::ExasolType::Date,
            MappedType::Timestamp => exa_type::ExasolType::Timestamp,
            MappedType::Double => exa_type::ExasolType::Double,
        }
    }
}

/// A source column successfully mapped to an Exasol column definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MappedColumn {
    pub exasol_type: MappedType,
    pub is_nullable: bool,
}
