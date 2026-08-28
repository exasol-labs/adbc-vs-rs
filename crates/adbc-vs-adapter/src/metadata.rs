//! Discovers MySQL tables and columns through `information_schema`, behind
//! an interface the adapter defines in its own vocabulary.
//!
//! `information_schema` is queried rather than ADBC's own catalog calls
//! (`GetTableSchema`, `GetObjects`) because Exasol must declare an exact
//! column type at `CREATE VIRTUAL SCHEMA` time: `GetTableSchema` reports
//! `VARCHAR` as Arrow `Utf8` with no length, and whether this driver
//! populates `GetObjects`'s `xdbc_*` fields is undocumented.
//!
//! Row assembly ([`assemble_tables`]) is pure and tested without a
//! database; only the two `information_schema` queries and their Arrow
//! result parsing touch ADBC.

use std::collections::BTreeMap;
use std::fmt;

use adbc_core::{Connection, Statement};
use adbc_driver_manager::ManagedConnection;
use arrow::array::{
    Array, Decimal128Array, Int16Array, Int32Array, Int64Array, Int8Array, LargeStringArray,
    StringArray, UInt16Array, UInt32Array, UInt64Array, UInt8Array,
};
use arrow::datatypes::DataType;
use arrow::record_batch::RecordBatch;

use crate::type_mapping::MySqlColumn;

/// A MySQL column discovered by [`MetadataSource::discover_tables`], paired
/// with its name and the fields [`crate::type_mapping::map_column`] needs.
pub struct DiscoveredColumn {
    pub name: String,
    pub mysql: MySqlColumn,
}

/// A MySQL base table or view discovered by [`MetadataSource::discover_tables`],
/// with its columns in `information_schema`'s ordinal position order.
pub struct DiscoveredTable {
    pub name: String,
    pub columns: Vec<DiscoveredColumn>,
}

/// The collations a MySQL source server reports, and the version string it
/// reports alongside them.
///
/// `crate::create_schema` checks this at `CREATE VIRTUAL SCHEMA` time: the
/// two collations pushdown rendering requires arrived at different points in
/// MySQL's own release history, so the check reads the collation listing
/// itself rather than trusting a parsed version number.
pub struct SourceCollations {
    pub names: Vec<String>,
    pub server_version: String,
}

/// Metadata discovery failed: the query it ran and why.
#[derive(Debug)]
pub struct MetadataError(String);

impl fmt::Display for MetadataError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "discovering MySQL metadata: {}", self.0)
    }
}

impl std::error::Error for MetadataError {}

/// Discovers the tables and columns of a MySQL database.
///
/// The adapter defines this interface in its own vocabulary — a list of
/// tables, each with its columns in ordinal order — rather than an ADBC or
/// `information_schema` shape, so a later backend supplies its own
/// implementation without the adapter changing.
pub trait MetadataSource {
    fn discover_tables(&mut self, database: &str) -> Result<Vec<DiscoveredTable>, MetadataError>;
}

/// Discovers MySQL metadata over an open ADBC connection.
pub struct AdbcMetadataSource {
    connection: ManagedConnection,
}

impl AdbcMetadataSource {
    /// Wraps an already-open ADBC connection to the source MySQL database.
    pub fn new(connection: ManagedConnection) -> Self {
        Self { connection }
    }

    /// Reads every collation name the source server knows, and the server's
    /// own version string.
    ///
    /// This is not part of [`MetadataSource`]: it answers a question specific
    /// to this release's MySQL pushdown rendering, not the driver-agnostic
    /// table discovery the trait exists to abstract over.
    pub fn discover_collations(&mut self) -> Result<SourceCollations, MetadataError> {
        let names = query_collation_names(&mut self.connection)?;
        let server_version = query_server_version(&mut self.connection)?;
        Ok(SourceCollations {
            names,
            server_version,
        })
    }
}

impl MetadataSource for AdbcMetadataSource {
    fn discover_tables(&mut self, database: &str) -> Result<Vec<DiscoveredTable>, MetadataError> {
        let table_names = query_table_names(&mut self.connection, database)?;
        let column_rows = query_columns(&mut self.connection, database)?;
        Ok(assemble_tables(table_names, column_rows))
    }
}

/// One row of the `information_schema.columns` query, before it is grouped
/// into a [`DiscoveredTable`].
struct ColumnRow {
    table: String,
    column: String,
    ordinal_position: i64,
    data_type: String,
    column_type: String,
    character_maximum_length: Option<i64>,
    character_set: Option<String>,
    numeric_precision: Option<i64>,
    numeric_scale: Option<i64>,
    is_nullable: bool,
}

/// Groups `information_schema.columns` rows under their table, in the
/// table-name order `table_names` gives and the ordinal-position order each
/// row already carries.
///
/// A table with no column rows — nothing this release should encounter
/// against a real MySQL database, but not a failure this function itself
/// judges — is reported with an empty column list; the caller treats that
/// the same way it treats a fully unmapped table.
fn assemble_tables(
    table_names: Vec<String>,
    mut column_rows: Vec<ColumnRow>,
) -> Vec<DiscoveredTable> {
    column_rows.sort_by_key(|row| (row.table.clone(), row.ordinal_position));

    let mut columns_by_table: BTreeMap<String, Vec<DiscoveredColumn>> = BTreeMap::new();
    for row in column_rows {
        let is_unsigned = row.column_type.to_ascii_lowercase().contains("unsigned");
        let column = DiscoveredColumn {
            name: row.column,
            mysql: MySqlColumn {
                data_type: row.data_type,
                character_length: row
                    .character_maximum_length
                    .and_then(|n| u32::try_from(n).ok()),
                character_set: row.character_set,
                numeric_precision: row.numeric_precision.and_then(|n| u32::try_from(n).ok()),
                numeric_scale: row.numeric_scale.and_then(|n| u32::try_from(n).ok()),
                is_unsigned,
                is_nullable: row.is_nullable,
            },
        };
        columns_by_table.entry(row.table).or_default().push(column);
    }

    table_names
        .into_iter()
        .map(|name| {
            let columns = columns_by_table.remove(&name).unwrap_or_default();
            DiscoveredTable { name, columns }
        })
        .collect()
}

/// Quotes a MySQL string literal, escaping a backslash and an embedded quote
/// so a database name containing either cannot change the shape of a
/// generated `information_schema` query.
///
/// A database name is validated before it ever reaches here:
/// `create_schema::reject_unsafe_database_name` refuses one carrying a single
/// quote or a backslash at the point `CONNECTION_NAME`'s address resolves it,
/// so this escaper's input is never one of those two characters in practice.
fn quote_string_literal(value: &str) -> String {
    format!("'{}'", value.replace('\\', "\\\\").replace('\'', "\\'"))
}

fn query_table_names(
    connection: &mut ManagedConnection,
    database: &str,
) -> Result<Vec<String>, MetadataError> {
    let sql = format!(
        "SELECT TABLE_NAME FROM information_schema.tables \
         WHERE TABLE_SCHEMA = {} AND TABLE_TYPE IN ('BASE TABLE', 'VIEW') \
         ORDER BY TABLE_NAME",
        quote_string_literal(database)
    );

    let mut names = Vec::new();
    for batch in run_query(connection, &sql)? {
        let column = batch_column(&batch, "TABLE_NAME", &sql)?;
        for row in 0..batch.num_rows() {
            names.push(required_string_cell(column, row, "TABLE_NAME")?);
        }
    }
    Ok(names)
}

fn query_columns(
    connection: &mut ManagedConnection,
    database: &str,
) -> Result<Vec<ColumnRow>, MetadataError> {
    let sql = format!(
        "SELECT TABLE_NAME, COLUMN_NAME, ORDINAL_POSITION, DATA_TYPE, COLUMN_TYPE, \
         CHARACTER_MAXIMUM_LENGTH, CHARACTER_SET_NAME, NUMERIC_PRECISION, NUMERIC_SCALE, \
         IS_NULLABLE \
         FROM information_schema.columns WHERE TABLE_SCHEMA = {} \
         ORDER BY TABLE_NAME, ORDINAL_POSITION",
        quote_string_literal(database)
    );

    let mut rows = Vec::new();
    for batch in run_query(connection, &sql)? {
        let table = batch_column(&batch, "TABLE_NAME", &sql)?;
        let column = batch_column(&batch, "COLUMN_NAME", &sql)?;
        let ordinal_position = batch_column(&batch, "ORDINAL_POSITION", &sql)?;
        let data_type = batch_column(&batch, "DATA_TYPE", &sql)?;
        let column_type = batch_column(&batch, "COLUMN_TYPE", &sql)?;
        let character_maximum_length = batch_column(&batch, "CHARACTER_MAXIMUM_LENGTH", &sql)?;
        let character_set = batch_column(&batch, "CHARACTER_SET_NAME", &sql)?;
        let numeric_precision = batch_column(&batch, "NUMERIC_PRECISION", &sql)?;
        let numeric_scale = batch_column(&batch, "NUMERIC_SCALE", &sql)?;
        let is_nullable = batch_column(&batch, "IS_NULLABLE", &sql)?;

        for row in 0..batch.num_rows() {
            rows.push(ColumnRow {
                table: required_string_cell(table, row, "TABLE_NAME")?,
                column: required_string_cell(column, row, "COLUMN_NAME")?,
                ordinal_position: required_i64_cell(ordinal_position, row, "ORDINAL_POSITION")?,
                data_type: required_string_cell(data_type, row, "DATA_TYPE")?,
                column_type: required_string_cell(column_type, row, "COLUMN_TYPE")?,
                character_maximum_length: i64_cell(character_maximum_length, row)?,
                character_set: string_cell(character_set, row)?,
                numeric_precision: i64_cell(numeric_precision, row)?,
                numeric_scale: i64_cell(numeric_scale, row)?,
                is_nullable: required_string_cell(is_nullable, row, "IS_NULLABLE")? == "YES",
            });
        }
    }
    Ok(rows)
}

fn query_collation_names(connection: &mut ManagedConnection) -> Result<Vec<String>, MetadataError> {
    let sql = "SELECT COLLATION_NAME FROM information_schema.COLLATIONS";

    let mut names = Vec::new();
    for batch in run_query(connection, sql)? {
        let column = batch_column(&batch, "COLLATION_NAME", sql)?;
        for row in 0..batch.num_rows() {
            names.push(required_string_cell(column, row, "COLLATION_NAME")?);
        }
    }
    Ok(names)
}

fn query_server_version(connection: &mut ManagedConnection) -> Result<String, MetadataError> {
    let sql = "SELECT VERSION() AS SERVER_VERSION";

    for batch in run_query(connection, sql)? {
        let column = batch_column(&batch, "SERVER_VERSION", sql)?;
        if batch.num_rows() > 0 {
            return required_string_cell(column, 0, "SERVER_VERSION");
        }
    }
    Err(MetadataError(format!("`{sql}` returned no row")))
}

fn run_query(
    connection: &mut ManagedConnection,
    sql: &str,
) -> Result<Vec<RecordBatch>, MetadataError> {
    let mut statement = connection
        .new_statement()
        .map_err(|error| MetadataError(format!("preparing `{sql}`: {error}")))?;
    statement
        .set_sql_query(sql)
        .map_err(|error| MetadataError(format!("setting query `{sql}`: {error}")))?;
    let reader = statement
        .execute()
        .map_err(|error| MetadataError(format!("executing `{sql}`: {error}")))?;

    let mut batches = Vec::new();
    for batch in reader {
        batches.push(batch.map_err(|error| {
            MetadataError(format!("reading a result batch of `{sql}`: {error}"))
        })?);
    }
    Ok(batches)
}

fn batch_column<'a>(
    batch: &'a RecordBatch,
    name: &str,
    sql: &str,
) -> Result<&'a dyn Array, MetadataError> {
    batch
        .column_by_name(name)
        .map(|array| array.as_ref())
        .ok_or_else(|| MetadataError(format!("`{sql}` returned no `{name}` column")))
}

fn string_cell(array: &dyn Array, row: usize) -> Result<Option<String>, MetadataError> {
    if array.is_null(row) {
        return Ok(None);
    }
    match array.data_type() {
        DataType::Utf8 => Ok(Some(downcast::<StringArray>(array)?.value(row).to_string())),
        DataType::LargeUtf8 => Ok(Some(
            downcast::<LargeStringArray>(array)?.value(row).to_string(),
        )),
        other => Err(MetadataError(format!(
            "expected a string information_schema column, got arrow type {other:?}"
        ))),
    }
}

fn required_string_cell(
    array: &dyn Array,
    row: usize,
    column: &str,
) -> Result<String, MetadataError> {
    string_cell(array, row)?.ok_or_else(|| MetadataError(format!("{column} is unexpectedly NULL")))
}

fn i64_cell(array: &dyn Array, row: usize) -> Result<Option<i64>, MetadataError> {
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
                MetadataError(format!(
                    "{value} overflows i64 in an information_schema column"
                ))
            })?
        }
        DataType::Decimal128(_, scale) if *scale == 0 => {
            downcast::<Decimal128Array>(array)?.value(row) as i64
        }
        other => {
            return Err(MetadataError(format!(
                "expected an integer information_schema column, got arrow type {other:?}"
            )))
        }
    };
    Ok(Some(value))
}

fn required_i64_cell(array: &dyn Array, row: usize, column: &str) -> Result<i64, MetadataError> {
    i64_cell(array, row)?.ok_or_else(|| MetadataError(format!("{column} is unexpectedly NULL")))
}

fn downcast<T: Array + 'static>(array: &dyn Array) -> Result<&T, MetadataError> {
    array.as_any().downcast_ref::<T>().ok_or_else(|| {
        MetadataError(format!(
            "an information_schema column reported type {:?} but did not downcast to it",
            array.data_type()
        ))
    })
}

#[cfg(test)]
#[path = "metadata_tests.rs"]
mod tests;
