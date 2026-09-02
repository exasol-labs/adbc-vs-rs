//! What a virtual schema needs to know about its source, behind an interface
//! the adapter defines in its own vocabulary.
//!
//! A source answers two questions and no others: whether it meets the
//! precondition the dialect that renders for it states, and which tables and
//! columns it exposes — each column already mapped to the Exasol type its
//! virtual column declares. Both answers are per technology, and neither
//! leaks the shape it was read from: no source-catalog column struct crosses
//! this boundary, so `createVirtualSchema` assembles a response without
//! knowing which catalog it came from or which types that catalog spells.
//!
//! Mapping belongs behind the interface rather than in front of it because it
//! is a pure function of one source's own catalog metadata. A shared column
//! struct wide enough to carry every source's catalog would put every
//! technology's fields in front of every other technology's mapper.
//!
//! Every implementation queries the source's own `information_schema` rather
//! than ADBC's catalog calls (`GetTableSchema`, `GetObjects`), because Exasol
//! must declare an exact column type at `CREATE VIRTUAL SCHEMA` time:
//! `GetTableSchema` reports `VARCHAR` as Arrow `Utf8` with no length, and
//! whether a driver populates `GetObjects`'s `xdbc_*` fields is undocumented.
//! Which catalog columns to read and what their values mean is per technology
//! and stays in that technology's own source; what is shared is running a
//! statement, reading an Arrow cell out of its result, and grouping the rows
//! that came back into tables, which is why those live here. Row assembly is
//! pure and tested without a database; only the queries and their Arrow result
//! parsing touch ADBC.
//!
//! This module holds no technology's source. MySQL's is `mysql_source` and
//! PostgreSQL's is `postgres_source`; each is reached only through
//! [`source_for`].

use std::collections::BTreeMap;
use std::fmt;

use adbc_core::{Connection, Statement};
use adbc_driver_manager::ManagedConnection;
use adbc_vs_scan::driver::Driver;
use arrow::array::{
    Array, Decimal128Array, Int16Array, Int32Array, Int64Array, Int8Array, LargeStringArray,
    StringArray, UInt16Array, UInt32Array, UInt64Array, UInt8Array,
};
use arrow::datatypes::DataType;
use arrow::record_batch::RecordBatch;

use crate::adapter_notes::Omission;
use crate::mysql_source::MySqlSource;
use crate::postgres_source::PostgresSource;
use crate::type_mapping::MappedColumn;

/// A source column that maps, and the Exasol column definition it maps to.
pub struct DiscoveredColumn {
    pub name: String,
    pub mapped: MappedColumn,
}

/// A source table or view discovered for a virtual schema: the columns that
/// map, in the source's own ordinal order, and the ones that do not, each
/// already carrying the source type and the reason it was left out.
pub struct DiscoveredTable {
    pub name: String,
    pub columns: Vec<DiscoveredColumn>,
    pub omissions: Vec<Omission>,
}

/// Why a source could not serve a virtual schema.
///
/// The three are different outcomes for an operator, so they are different
/// variants: a catalog read that failed may succeed on the next attempt, a
/// source the rendering cannot be used against is answered by changing the
/// server, and a namespace this source cannot query for is answered by
/// changing the `CONNECTION` address.
#[derive(Debug)]
pub enum MetadataError {
    Catalog(String),
    UnsupportedSource(String),
    UnsafeDatabaseName { database: String, character: char },
}

impl MetadataError {
    pub(crate) fn catalog(cause: impl Into<String>) -> Self {
        Self::Catalog(cause.into())
    }
}

impl fmt::Display for MetadataError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Catalog(cause) => write!(f, "discovering source metadata: {cause}"),
            Self::UnsupportedSource(reason) => write!(f, "{reason}"),
            Self::UnsafeDatabaseName {
                database,
                character,
            } => write!(
                f,
                "the MySQL database name `{database}`, taken from the path component of the \
                 source connection's address, contains `{character}`, which the \
                 information_schema queries built from it cannot safely quote"
            ),
        }
    }
}

impl std::error::Error for MetadataError {}

/// Everything a virtual schema learns from its source.
pub trait MetadataSource {
    /// The one source namespace this virtual schema exposes, and the value
    /// `adapterNotes` records as it.
    ///
    /// Derived rather than configured, and what it is derived from is the
    /// source's own business: MySQL's namespace is the database the connection
    /// address names, because a MySQL database *is* its SQL qualifier, while
    /// PostgreSQL's is the `public` schema, because a PostgreSQL database is
    /// not one. Answering it here rather than at the call site is what keeps
    /// that difference inside the technology that has it — the schema
    /// assembles the same response whichever answer it gets, and the dialect
    /// qualifies its tables with whatever was recorded.
    fn namespace(&self) -> &str;

    /// Refuses a source this release's rendering cannot be used against.
    ///
    /// The precondition is stated by the dialect that renders for the source
    /// rather than by the source itself: MySQL's rendering needs two named
    /// collations at the server, and another technology's rendering needs
    /// something else entirely, so each implementation answers only for its
    /// own.
    fn check_source(&mut self) -> Result<(), MetadataError>;

    /// Discovers the tables of [`Self::namespace`], every column already
    /// mapped to the Exasol type its virtual column declares and every
    /// unmappable column recorded as an omission.
    ///
    /// An unmapped column is a normal outcome rather than a failure: the
    /// virtual table is built from what maps, and what did not is recorded
    /// for an operator to read back.
    fn discover_tables(&mut self) -> Result<Vec<DiscoveredTable>, MetadataError>;
}

/// The metadata source that reads `driver`'s catalog, over an already-open
/// ADBC connection to it.
///
/// `address_database` is the database component of the `CONNECTION` object's
/// address. Whether that is also the namespace the virtual schema exposes is
/// the source's own answer, not this function's: MySQL keeps it, PostgreSQL
/// exposes `public` and uses the database only as the connection target.
///
/// One table, matched exhaustively, so a technology added to the identity has
/// to bring a source that answers its own catalog — no other implementation
/// can, and a fallback would answer one catalog's questions against another.
pub fn source_for(
    driver: Driver,
    connection: ManagedConnection,
    address_database: &str,
) -> Box<dyn MetadataSource> {
    match driver {
        Driver::MySql => Box::new(MySqlSource::new(connection, address_database)),
        Driver::Postgres => Box::new(PostgresSource::new(connection)),
    }
}

/// The four things [`assemble_tables`] reads off a catalog row, whichever
/// catalog it came from.
///
/// Deliberately not the row itself: everything else a row carries is what its
/// own mapping reads, and a trait wide enough to expose that would put every
/// technology's catalog fields in front of every other technology's.
pub(crate) trait CatalogRow {
    fn table(&self) -> &str;
    fn column(&self) -> &str;
    fn ordinal_position(&self) -> i64;
    /// The source's own name for this column's type, recorded on an omission
    /// so an operator reads back the spelling their own catalog uses.
    fn source_type(&self) -> &str;
}

/// Groups a source's catalog rows under their table, in the table-name order
/// `table_names` gives and the ordinal-position order each row already
/// carries, mapping each row through `map` as it goes.
///
/// The rule is the framework's rather than any technology's: which columns a
/// virtual table declares and in which order, what an omission records, and
/// what a table with no rows becomes are answers `createVirtualSchema` depends
/// on being the same whichever source produced them. Only `map` is per
/// technology, and it is a function rather than a second trait method because
/// it consumes the row — the mapping reads fields no source shares.
///
/// A table with no column rows — nothing this release should encounter against
/// a real source, but not a failure this function itself judges — is reported
/// with an empty column list; the caller treats that the same way it treats a
/// fully unmapped table.
pub(crate) fn assemble_tables<R: CatalogRow>(
    table_names: Vec<String>,
    mut rows: Vec<R>,
    map: impl Fn(R) -> Result<MappedColumn, String>,
) -> Vec<DiscoveredTable> {
    rows.sort_by(|one, other| {
        one.table()
            .cmp(other.table())
            .then(one.ordinal_position().cmp(&other.ordinal_position()))
    });

    let mut mapped_by_table: BTreeMap<String, Vec<DiscoveredColumn>> = BTreeMap::new();
    let mut omissions_by_table: BTreeMap<String, Vec<Omission>> = BTreeMap::new();
    for row in rows {
        let table = row.table().to_owned();
        let name = row.column().to_owned();
        let source_type = row.source_type().to_owned();
        match map(row) {
            Ok(mapped) => mapped_by_table
                .entry(table)
                .or_default()
                .push(DiscoveredColumn { name, mapped }),
            Err(reason) => omissions_by_table
                .entry(table.clone())
                .or_default()
                .push(Omission {
                    table,
                    column: Some(name),
                    source_type: Some(source_type),
                    reason,
                }),
        }
    }

    table_names
        .into_iter()
        .map(|name| DiscoveredTable {
            columns: mapped_by_table.remove(&name).unwrap_or_default(),
            omissions: omissions_by_table.remove(&name).unwrap_or_default(),
            name,
        })
        .collect()
}

pub(crate) fn run_query(
    connection: &mut ManagedConnection,
    sql: &str,
) -> Result<Vec<RecordBatch>, MetadataError> {
    let mut statement = connection
        .new_statement()
        .map_err(|error| MetadataError::catalog(format!("preparing `{sql}`: {error}")))?;
    statement
        .set_sql_query(sql)
        .map_err(|error| MetadataError::catalog(format!("setting query `{sql}`: {error}")))?;
    let reader = statement
        .execute()
        .map_err(|error| MetadataError::catalog(format!("executing `{sql}`: {error}")))?;

    let mut batches = Vec::new();
    for batch in reader {
        batches.push(batch.map_err(|error| {
            MetadataError::catalog(format!("reading a result batch of `{sql}`: {error}"))
        })?);
    }
    Ok(batches)
}

pub(crate) fn batch_column<'a>(
    batch: &'a RecordBatch,
    name: &str,
    sql: &str,
) -> Result<&'a dyn Array, MetadataError> {
    batch
        .column_by_name(name)
        .map(|array| array.as_ref())
        .ok_or_else(|| MetadataError::catalog(format!("`{sql}` returned no `{name}` column")))
}

pub(crate) fn string_cell(array: &dyn Array, row: usize) -> Result<Option<String>, MetadataError> {
    if array.is_null(row) {
        return Ok(None);
    }
    match array.data_type() {
        DataType::Utf8 => Ok(Some(downcast::<StringArray>(array)?.value(row).to_string())),
        DataType::LargeUtf8 => Ok(Some(
            downcast::<LargeStringArray>(array)?.value(row).to_string(),
        )),
        other => Err(MetadataError::catalog(format!(
            "expected a string information_schema column, got arrow type {other:?}"
        ))),
    }
}

pub(crate) fn required_string_cell(
    array: &dyn Array,
    row: usize,
    column: &str,
) -> Result<String, MetadataError> {
    string_cell(array, row)?
        .ok_or_else(|| MetadataError::catalog(format!("{column} is unexpectedly NULL")))
}

pub(crate) fn i64_cell(array: &dyn Array, row: usize) -> Result<Option<i64>, MetadataError> {
    if array.is_null(row) {
        return Ok(None);
    }
    let value = match array.data_type() {
        DataType::Int8 => downcast::<Int8Array>(array)?.value(row) as i64,
        DataType::Int16 => downcast::<Int16Array>(array)?.value(row) as i64,
        DataType::Int32 => downcast::<Int32Array>(array)?.value(row) as i64,
        DataType::Int64 => downcast::<Int64Array>(array)?.value(row),
        DataType::UInt8 => downcast::<UInt8Array>(array)?.value(row) as i64,
        DataType::UInt16 => downcast::<UInt16Array>(array)?.value(row) as i64,
        DataType::UInt32 => downcast::<UInt32Array>(array)?.value(row) as i64,
        DataType::UInt64 => {
            let value = downcast::<UInt64Array>(array)?.value(row);
            i64::try_from(value).map_err(|_| {
                MetadataError::catalog(format!(
                    "{value} overflows i64 in an information_schema column"
                ))
            })?
        }
        DataType::Decimal128(_, scale) if *scale == 0 => {
            downcast::<Decimal128Array>(array)?.value(row) as i64
        }
        other => {
            return Err(MetadataError::catalog(format!(
                "expected an integer information_schema column, got arrow type {other:?}"
            )))
        }
    };
    Ok(Some(value))
}

pub(crate) fn required_i64_cell(
    array: &dyn Array,
    row: usize,
    column: &str,
) -> Result<i64, MetadataError> {
    i64_cell(array, row)?
        .ok_or_else(|| MetadataError::catalog(format!("{column} is unexpectedly NULL")))
}

fn downcast<T: Array + 'static>(array: &dyn Array) -> Result<&T, MetadataError> {
    array.as_any().downcast_ref::<T>().ok_or_else(|| {
        MetadataError::catalog(format!(
            "an information_schema column reported type {:?} but did not downcast to it",
            array.data_type()
        ))
    })
}

#[cfg(test)]
#[path = "metadata_tests.rs"]
mod tests;

/// The rules that bind every source rather than one of them. Attached here
/// because each source's own catalog shape is what they are driven through,
/// and this module owns the interface those shapes are reached behind.
#[cfg(test)]
#[path = "metadata_source_tests.rs"]
mod source_tests;
