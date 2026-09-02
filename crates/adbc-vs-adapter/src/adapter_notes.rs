//! The state `createVirtualSchema` records for `pushdown`, `refresh`, and
//! `setProperties` to read back, because Exasol persists and returns
//! `schemaMetadata.adapterNotes` but discards everything else the adapter
//! might otherwise have kept.
//!
//! Defined once so the writer (`createVirtualSchema`) and every reader
//! (`pushdown`, `refresh`, `setProperties`) agree on field names without
//! duplicating the JSON shape between them. It never carries a credential:
//! the source user and password, and the BucketFS write credentials, are
//! resolved fresh from their `CONNECTION` objects at the moment they are
//! needed, never persisted here.

use serde::{Deserialize, Serialize};

/// One column or table `createVirtualSchema` left out of the virtual
/// schema, recorded for an operator to read back rather than silently
/// dropped.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Omission {
    pub table: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<String>,
    /// The source's own name for the type that could not be mapped.
    ///
    /// `mysqlType` is accepted on the way in but never written: notes Exasol
    /// persisted before this release spell it that way, and a `refresh` or a
    /// `setProperties` over such a schema reads them back before it writes
    /// the new spelling.
    #[serde(
        rename = "sourceType",
        alias = "mysqlType",
        skip_serializing_if = "Option::is_none"
    )]
    pub source_type: Option<String>,
    pub reason: String,
}

/// The state `pushdown`, `refresh`, and `setProperties` need back, recorded
/// by `createVirtualSchema` into `schemaMetadata.adapterNotes`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdapterNotes {
    pub driver: String,
    /// The driver version resolution selected for this virtual schema, and the
    /// only place an operator can read it back.
    ///
    /// Absent when the operator supplied the library themselves through
    /// `DRIVER_PATH`: adbc-vs never learned that library's version, and a
    /// compiled-in one recorded in its place would report a version this
    /// schema is not running. Notes Exasol persisted before the version range
    /// carry the pinned version and parse unchanged.
    #[serde(
        rename = "driverVersion",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub driver_version: Option<String>,
    #[serde(rename = "driverPath")]
    pub driver_path: String,
    pub connection: String,
    /// The one source namespace this virtual schema exposes, and the qualifier
    /// every rendered statement puts in front of a table name.
    ///
    /// Which source fact it is derived from is per technology — a MySQL
    /// database, PostgreSQL's `public` schema — and the metadata source
    /// answers it. The JSON name stays `database`, the field an earlier
    /// release wrote, so notes Exasol persisted before this one parse
    /// unchanged.
    pub database: String,
    #[serde(default)]
    pub omitted: Vec<Omission>,
}

impl AdapterNotes {
    /// Renders these notes as the stringified JSON object
    /// `schemaMetadata.adapterNotes` carries.
    pub fn to_json_string(&self) -> String {
        serde_json::to_string(self).expect("AdapterNotes always serializes to JSON")
    }

    /// Parses `schemaMetadataInfo.adapterNotes` back into [`AdapterNotes`].
    pub fn parse(text: &str) -> Result<Self, String> {
        serde_json::from_str(text).map_err(|error| format!("parsing adapter notes JSON: {error}"))
    }

    /// Reads the notes off a request Exasol returned them on.
    ///
    /// Where they sit on the request is decided once, here, because every
    /// reader of them — the dialect resolution, the pushdown builder — would
    /// otherwise carry the same pointer and the same absence handling. Absence
    /// is an error rather than a default: the notes are the only record of
    /// which source a request is about, and answering one source's request
    /// with another's configuration returns wrong rows rather than failing.
    pub fn read(request: &serde_json::Value) -> Result<Self, String> {
        let text = request
            .pointer("/schemaMetadataInfo/adapterNotes")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                "the request carries no schemaMetadataInfo.adapterNotes, so the virtual schema \
                 it is about cannot be identified; createVirtualSchema records the notes and \
                 Exasol returns them on every later request"
                    .to_string()
            })?;
        Self::parse(text)
    }
}

#[cfg(test)]
#[path = "adapter_notes_tests.rs"]
mod tests;
