//! Reports the query fragments the adapter delegates to the source.
//!
//! The set itself is not stated here. Exasol never re-applies a delegated
//! fragment once its capability is advertised, so a capability the renderer
//! cannot express returns wrong rows rather than an error — which makes the
//! advertised set and the renderer one decision. It is owned by the
//! [`SqlDialect`](crate::sql_dialect::SqlDialect) implementation that renders
//! it, and this module only shapes that set into the protocol's response.

use serde_json::Value;

use crate::sql_dialect::SqlDialect;

/// Builds the response to a `getCapabilities` request.
pub fn capabilities_response(dialect: &dyn SqlDialect) -> Value {
    serde_json::json!({
        "type": "getCapabilities",
        "capabilities": dialect.capabilities(),
    })
}

#[cfg(test)]
#[path = "capabilities_tests.rs"]
mod tests;
