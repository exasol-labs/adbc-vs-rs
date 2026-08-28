//! The scan specification the adapter's `pushdown` response embeds and the
//! scan UDF entry point parses back.
//!
//! It is the one JSON shape both sides of the adapter/scan boundary agree
//! on, owned by neither in particular — defining it once here means the
//! pushdown response builder and the scan entry point cannot drift apart on
//! field names or types. It carries no credential: the scan UDF resolves
//! `connection` itself, at execution time, through the UDF SDK's connection
//! accessor.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::exa_type::{self, ExasolType};

/// One declared output column of the scan: its name and the Exasol type the
/// virtual table declared for it.
#[derive(Debug, Clone, PartialEq)]
pub struct ScanColumn {
    pub name: String,
    pub exasol_type: ExasolType,
}

/// Everything the scan UDF needs to run the pushed-down statement and emit
/// its result in the shape Exasol's dynamic `EMITS` clause declared.
#[derive(Debug, Clone, PartialEq)]
pub struct ScanSpec {
    pub driver_path: String,
    pub connection: String,
    pub sql: String,
    pub columns: Vec<ScanColumn>,
}

impl ScanSpec {
    /// Renders this specification as the JSON text the pushdown response
    /// embeds as the scan UDF's single string argument.
    pub fn to_json_string(&self) -> String {
        Wire::from(self).render()
    }
}

/// Parses the JSON text a pushdown response embedded back into a
/// [`ScanSpec`].
pub fn parse(text: &str) -> Result<ScanSpec, String> {
    let wire: Wire =
        serde_json::from_str(text).map_err(|error| format!("parsing scan spec JSON: {error}"))?;
    wire.into_scan_spec()
}

/// The wire encoding of [`ScanSpec`]: `columns[].dataType` uses the same
/// JSON shape [`exa_type`] renders for `schemaMetadata`, so a mapped column
/// is written once and read back through one shared encoder.
#[derive(Serialize, Deserialize)]
struct Wire {
    #[serde(rename = "driverPath")]
    driver_path: String,
    connection: String,
    sql: String,
    columns: Vec<WireColumn>,
}

#[derive(Serialize, Deserialize)]
struct WireColumn {
    name: String,
    #[serde(rename = "dataType")]
    data_type: Value,
}

impl From<&ScanSpec> for Wire {
    fn from(spec: &ScanSpec) -> Self {
        Wire {
            driver_path: spec.driver_path.clone(),
            connection: spec.connection.clone(),
            sql: spec.sql.clone(),
            columns: spec
                .columns
                .iter()
                .map(|column| WireColumn {
                    name: column.name.clone(),
                    data_type: exa_type::to_json(column.exasol_type),
                })
                .collect(),
        }
    }
}

impl Wire {
    fn into_scan_spec(self) -> Result<ScanSpec, String> {
        let columns = self
            .columns
            .into_iter()
            .map(|column| {
                exa_type::from_json(&column.data_type).map(|exasol_type| ScanColumn {
                    name: column.name,
                    exasol_type,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ScanSpec {
            driver_path: self.driver_path,
            connection: self.connection,
            sql: self.sql,
            columns,
        })
    }

    fn render(&self) -> String {
        serde_json::to_string(self).expect("a ScanSpec always serializes to JSON")
    }
}

#[cfg(test)]
#[path = "scan_spec_tests.rs"]
mod tests;
