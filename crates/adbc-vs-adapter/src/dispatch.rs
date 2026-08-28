//! Routes each Virtual Schema request Exasol sends to its handler, and
//! turns every handler failure into a diagnosable `UdfError` instead of a
//! malformed response.
//!
//! The adapter holds no state between calls. Requests are parsed as
//! untyped JSON because the wire format carries fields this adapter
//! ignores, and a strict typed deserialization would reject a request over
//! an unrelated field.
//!
//! This is also the one place that names a concrete
//! [`SqlDialect`](crate::sql_dialect::SqlDialect): the capability response
//! and the pushdown response both read the same implementation, so the two
//! can never disagree about what the adapter delegates.

use std::fmt;

use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;
use serde_json::Value;

use crate::capabilities;
use crate::create_schema::{self, SchemaError};
use crate::mysql_dialect::MySqlDialect;
use crate::pushdown::{self, PushdownError};

/// Everything that stops a Virtual Schema request from producing its
/// response.
#[derive(Debug)]
pub enum AdapterError {
    MalformedRequest(String),
    MissingRequestType,
    UnsupportedRequestType(String),
    Schema(SchemaError),
    Pushdown(PushdownError),
}

impl fmt::Display for AdapterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MalformedRequest(cause) => write!(
                f,
                "failed to parse the virtual schema request as JSON: {cause}"
            ),
            Self::MissingRequestType => {
                write!(f, "the virtual schema request has no `type` field")
            }
            Self::UnsupportedRequestType(value) => write!(
                f,
                "virtual schema request type {value:?} is not one of the six adbc-vs supports: \
                 createVirtualSchema, refresh, setProperties, dropVirtualSchema, \
                 getCapabilities, pushdown"
            ),
            Self::Schema(error) => write!(f, "{error}"),
            Self::Pushdown(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for AdapterError {}

impl From<SchemaError> for AdapterError {
    fn from(error: SchemaError) -> Self {
        Self::Schema(error)
    }
}

impl From<PushdownError> for AdapterError {
    fn from(error: PushdownError) -> Self {
        Self::Pushdown(error)
    }
}

/// Parses `request_json`, dispatches on its `type` field, and returns the
/// handler's JSON response as text — the signature
/// `exasol-udf-macros`' `vs_adapter(...)` form requires.
pub fn adapter_call(ctx: &mut dyn UdfContext, request_json: &str) -> Result<String, UdfError> {
    handle(ctx, request_json)
        .map(|response| response.to_string())
        .map_err(|error| UdfError::User(error.to_string()))
}

fn handle(ctx: &mut dyn UdfContext, request_json: &str) -> Result<Value, AdapterError> {
    let request: Value = serde_json::from_str(request_json)
        .map_err(|error| AdapterError::MalformedRequest(error.to_string()))?;

    match request.get("type").and_then(Value::as_str) {
        Some("createVirtualSchema") => Ok(create_schema::create_virtual_schema(ctx, &request)?),
        Some("refresh") => Ok(create_schema::refresh(ctx, &request)?),
        Some("setProperties") => Ok(create_schema::set_properties(ctx, &request)?),
        Some("dropVirtualSchema") => Ok(drop_virtual_schema()),
        Some("getCapabilities") => Ok(capabilities::capabilities_response(&MySqlDialect)),
        Some("pushdown") => Ok(pushdown::pushdown_response(&request, &MySqlDialect)?),
        Some(other) => Err(AdapterError::UnsupportedRequestType(other.to_string())),
        None => Err(AdapterError::MissingRequestType),
    }
}

/// `dropVirtualSchema` releases no external resource: other virtual schemas
/// may still reference the same installed driver, so the response is bare.
fn drop_virtual_schema() -> Value {
    serde_json::json!({"type": "dropVirtualSchema"})
}
