//! Routes each Virtual Schema request Exasol sends to its handler, and
//! turns every handler failure into a diagnosable `UdfError` instead of a
//! malformed response.
//!
//! The adapter holds no state between calls. Requests are parsed as
//! untyped JSON because the wire format carries fields this adapter
//! ignores, and a strict typed deserialization would reject a request over
//! an unrelated field.
//!
//! Neither dialect-reading arm names a concrete
//! [`SqlDialect`](crate::sql_dialect::SqlDialect). Both resolve one from the
//! driver identity `createVirtualSchema` recorded in the request's own
//! `adapterNotes`, so the set the adapter advertises and the statement it
//! renders are always the same virtual schema's — and a request that records
//! no identity is refused rather than answered for a guessed one.
//!
//! That refusal is safe because of a measured property of Exasol 2025.2.1:
//! `getCapabilities` is sent only at query-compile time. `CREATE VIRTUAL
//! SCHEMA`, `ALTER VIRTUAL SCHEMA ... REFRESH`, and `ALTER VIRTUAL SCHEMA
//! ... SET` were each driven against an adapter whose `getCapabilities`
//! always failed, and all three succeeded, so no `getCapabilities` can
//! precede the `createVirtualSchema` that writes the notes it reads.

use std::fmt;

use adbc_vs_scan::driver::{Driver, UnsupportedDriver};
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;
use serde_json::Value;

use crate::adapter_notes::AdapterNotes;
use crate::capabilities;
use crate::create_schema::{self, SchemaError};
use crate::pushdown::{self, PushdownError};
use crate::sql_dialect::{dialect_for, SqlDialect};

/// Everything that stops a Virtual Schema request from producing its
/// response.
#[derive(Debug)]
pub enum AdapterError {
    MalformedRequest(String),
    MissingRequestType,
    UnsupportedRequestType(String),
    UnidentifiedSchema(String),
    UnsupportedDriver(UnsupportedDriver),
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
            Self::UnidentifiedSchema(cause) => write!(
                f,
                "cannot tell which driver this virtual schema was created for: {cause}"
            ),
            Self::UnsupportedDriver(error) => write!(f, "{error}"),
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
        Some("getCapabilities") => Ok(capabilities::capabilities_response(dialect_of(&request)?)),
        Some("pushdown") => Ok(pushdown::pushdown_response(
            &request,
            dialect_of(&request)?,
        )?),
        Some(other) => Err(AdapterError::UnsupportedRequestType(other.to_string())),
        None => Err(AdapterError::MissingRequestType),
    }
}

/// The dialect the virtual schema this request is about renders through.
///
/// Read from `adapterNotes.driver` rather than from the request's properties:
/// the notes are what `createVirtualSchema` recorded for this schema, and
/// resolving both dialect-reading arms from the same field is what keeps the
/// advertised set and the rendered statement talking about one source.
fn dialect_of(request: &Value) -> Result<&'static dyn SqlDialect, AdapterError> {
    let notes = AdapterNotes::read(request).map_err(AdapterError::UnidentifiedSchema)?;
    let driver = Driver::parse(&notes.driver).map_err(AdapterError::UnsupportedDriver)?;
    Ok(dialect_for(driver))
}

/// `dropVirtualSchema` releases no external resource: other virtual schemas
/// may still reference the same installed driver, so the response is bare.
fn drop_virtual_schema() -> Value {
    serde_json::json!({"type": "dropVirtualSchema"})
}
