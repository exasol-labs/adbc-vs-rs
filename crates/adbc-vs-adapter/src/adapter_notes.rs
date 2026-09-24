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

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The ordered name segments that qualify a table at its source.
///
/// A list rather than one string because sources nest their tables to
/// different depths — a MySQL database is one name, a Trino table is named by a
/// catalog and a schema — and a catalog or a schema name may itself contain a
/// dot, so a joined string could never be split back into the segments that
/// built it. Nothing here joins them: which separator and which quoting a
/// qualifier is written with is the dialect's business, and this type is what
/// hands both the dialect and the metadata source one shape for it.
///
/// It carries at least one segment, and no segment of it is blank, so every
/// reader can render it without a fallback for a qualifier that names nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceQualifier {
    segments: Vec<String>,
}

impl SourceQualifier {
    /// The qualifier `segments` spell, or why they spell none.
    ///
    /// Refusing here rather than at each reader is what lets a dialect and a
    /// metadata source treat a qualifier as always renderable: a blank segment
    /// would reach a generated statement as an empty quoted name, which
    /// addresses nothing and fails at the source rather than at the operator.
    pub fn of(segments: Vec<String>) -> Result<Self, String> {
        if segments.is_empty() {
            return Err("a source qualifier names at least one segment".to_string());
        }
        if let Some(position) = segments.iter().position(|segment| segment.is_empty()) {
            return Err(format!(
                "source qualifier segment {} is empty, and an empty name qualifies nothing",
                position + 1
            ));
        }
        Ok(Self { segments })
    }

    /// The qualifier one already-known name spells.
    ///
    /// Infallible because its callers hold a source name rather than operator
    /// text: a technology whose qualifier is one segment derives that segment
    /// from a `CONNECTION` address [`Self::of`] already accepted, or from a
    /// constant of its own.
    pub fn single(segment: impl Into<String>) -> Self {
        Self {
            segments: vec![segment.into()],
        }
    }

    /// The segments, outermost first.
    pub fn segments(&self) -> &[String] {
        &self.segments
    }

    /// The outermost segment, which every qualifier carries.
    pub fn first(&self) -> &str {
        &self.segments[0]
    }
}

/// Writes one segment as the bare string an earlier release wrote, and more as
/// an array.
///
/// Exasol persists `adapterNotes` and returns them rather than having them
/// recreated, so a virtual schema created before the qualifier became a list is
/// read back by this release. Keeping the single-segment spelling identical is
/// what lets those notes be read *and* rewritten unchanged, and it leaves the
/// statements every single-segment technology generates byte-identical too.
impl Serialize for SourceQualifier {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self.segments.as_slice() {
            [only] => serializer.serialize_str(only),
            many => many.serialize(serializer),
        }
    }
}

/// Reads back either spelling, under the one field rather than a second one: a
/// second field would let two records of one fact disagree.
impl<'de> Deserialize<'de> for SourceQualifier {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Recorded {
            Joined(String),
            Segments(Vec<String>),
        }

        let segments = match Recorded::deserialize(deserializer)? {
            Recorded::Joined(segment) => vec![segment],
            Recorded::Segments(segments) => segments,
        };
        Self::of(segments).map_err(serde::de::Error::custom)
    }
}

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
    /// Which source facts it is derived from, and how many segments it takes
    /// to spell, are per technology — a MySQL database, PostgreSQL's `public`
    /// schema, a Trino catalog and schema — and the metadata source answers
    /// both. The JSON name stays `database`, the field an earlier release
    /// wrote, and a single-segment qualifier keeps that release's spelling, so
    /// notes Exasol persisted before this one parse and rewrite unchanged.
    pub database: SourceQualifier,
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
