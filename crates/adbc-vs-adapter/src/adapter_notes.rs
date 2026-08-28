//! The state `createVirtualSchema` records for `pushdown`, `refresh`, and
//! `setProperties` to read back, because Exasol persists and returns
//! `schemaMetadata.adapterNotes` but discards everything else the adapter
//! might otherwise have kept.
//!
//! Defined once so the writer (`createVirtualSchema`) and every reader
//! (`pushdown`, `refresh`, `setProperties`) agree on field names without
//! duplicating the JSON shape between them. It never carries a credential:
//! the MySQL user and password, and the BucketFS write credentials, are
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
    #[serde(rename = "mysqlType", skip_serializing_if = "Option::is_none")]
    pub mysql_type: Option<String>,
    pub reason: String,
}

/// The state `pushdown`, `refresh`, and `setProperties` need back, recorded
/// by `createVirtualSchema` into `schemaMetadata.adapterNotes`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdapterNotes {
    pub driver: String,
    #[serde(rename = "driverVersion")]
    pub driver_version: String,
    #[serde(rename = "driverPath")]
    pub driver_path: String,
    pub connection: String,
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
}

#[cfg(test)]
#[path = "adapter_notes_tests.rs"]
mod tests;
