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
//! every mapping reads the same constants rather than restating them.
//!
//! A mapped column declares an [`ExasolType`], the same set an `EMITS` column
//! declares, so the pushdown closure argument rests on one type set.

use crate::exa_type::ExasolType;

/// Exasol's maximum `DECIMAL` precision.
pub(crate) const MAX_DECIMAL_PRECISION: u32 = 36;

/// Exasol's maximum `VARCHAR` size.
pub(crate) const MAX_VARCHAR_SIZE: u32 = 2_000_000;

/// Exasol's maximum `CHAR` size.
pub(crate) const MAX_CHAR_SIZE: u32 = 2000;

/// Exasol's minimum character-type size. Exasol rejects `VARCHAR(0)`, so a
/// source length of 0 is raised to this rather than declared as reported.
pub(crate) const MIN_CHARACTER_SIZE: u32 = 1;

/// A source column successfully mapped to an Exasol column definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MappedColumn {
    pub exasol_type: ExasolType,
    /// The source type as the catalog spelled it, which the column records so
    /// its technology's read expression can be selected at `pushdown` time.
    pub source_type: String,
    pub is_nullable: bool,
}
