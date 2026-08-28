//! The boundary between the query fragment Exasol delegated and the target
//! SQL text that reads it at the source.
//!
//! One implementation owns both halves of the delegation contract: the set of
//! capabilities the adapter advertises, and the rendering of a
//! [`Query`](crate::pushdown_query::Query) into that target's SQL. Exasol
//! never re-applies a fragment once its capability is advertised, so an
//! advertised capability the renderer cannot express returns wrong rows
//! instead of an error. Keeping both in one implementation makes that a
//! single decision with a single owner; nothing in the type system enforces
//! the agreement, so an invariant test that renders every advertised
//! capability is what establishes it.
//!
//! The trait exists for the second driver rather than for this one:
//! mission.md's v1 driver set names PostgreSQL, Snowflake, Databricks, and
//! DuckDB, and each adds an implementation rather than editing a renderer.

use std::fmt;

use crate::pushdown_query::Query;

/// A query fragment the dialect cannot express in its target SQL.
///
/// Returning it is mandatory rather than optional: a dialect that dropped
/// the fragment and rendered the rest would return rows Exasol never asked
/// for, and Exasol would not re-apply the fragment to correct them.
#[derive(Debug)]
pub struct RenderError(String);

impl RenderError {
    /// States what could not be rendered. The message MUST name the
    /// protocol element or model node responsible, because it reaches the
    /// operator as the failing statement's only diagnosis.
    pub fn new(cause: impl Into<String>) -> Self {
        Self(cause.into())
    }
}

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "rendering the pushed-down query: {}", self.0)
    }
}

impl std::error::Error for RenderError {}

/// Renders the adapter's query model into one target SQL dialect, and
/// advertises exactly what that rendering covers.
pub trait SqlDialect {
    /// The capabilities the adapter advertises to Exasol, in a fixed order
    /// so the `getCapabilities` response is byte-identical between calls.
    ///
    /// Every entry MUST have a rendering path in [`Self::render`].
    fn capabilities(&self) -> &'static [&'static str];

    /// Renders `query` as a single statement the target executes at the
    /// source, or fails naming the part it cannot express.
    fn render(&self, query: &Query) -> Result<String, RenderError>;
}
