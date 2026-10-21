//! Discovers MySQL metadata over an open ADBC connection.
//!
//! The exposed namespace is the database the `CONNECTION` address names,
//! because a MySQL database *is* its SQL qualifier: the same value filters
//! both `information_schema` queries, is what `adapterNotes` records, and is
//! what `mysql_dialect` renders in front of every table name.
//!
//! That value is resolved from an operator-typed address, so it is the one
//! thing this source interpolates into a query. Both halves of handling it
//! live here — [`reject_unsafe_database_name`] refuses the two characters
//! [`quote_string_literal`]'s escaping cannot be trusted with, and no other
//! technology inherits either, because a source that filters on a compile-time
//! namespace has no literal to escape.

use adbc_driver_manager::ManagedConnection;

use crate::adapter_notes::SourceQualifier;
use crate::metadata::{
    assemble_tables, batch_column, i64_cell, required_i64_cell, required_string_cell, run_query,
    string_cell, CatalogRow, DiscoveredTable, MetadataError, MetadataSource,
};
use crate::mysql_type_mapping::{self, MySqlColumn};

/// Discovers MySQL metadata over an open ADBC connection.
pub struct MySqlSource {
    connection: ManagedConnection,
    /// The MySQL database this schema exposes, which is both the
    /// `information_schema` filter value and the SQL qualifier a rendered
    /// statement carries.
    database: String,
}

impl MySqlSource {
    /// Wraps an already-open ADBC connection to `database` on the source MySQL
    /// server.
    pub fn new(connection: ManagedConnection, database: &str) -> Self {
        Self {
            connection,
            database: database.to_owned(),
        }
    }
}

impl MetadataSource for MySqlSource {
    fn namespace(&self) -> SourceQualifier {
        SourceQualifier::single(&self.database)
    }

    /// Refuses a database name this source cannot build a catalog query for.
    ///
    /// The check reaches nothing outside the process, so an address an
    /// operator can fix is answered without a round trip. It queries no
    /// server collation and no server version: the MySQL dialect renders no
    /// collation, so no server is refused for its collation or version. The check is
    /// MySQL's own and lives nowhere else, and
    /// `create_schema::discover_supported_source` — which checks before it
    /// discovers, on every path — is what leaves no route to
    /// [`quote_string_literal`] that skipped it.
    fn check_source(&mut self) -> Result<(), MetadataError> {
        reject_unsafe_database_name(&self.database)
    }

    fn discover_tables(&mut self) -> Result<Vec<DiscoveredTable>, MetadataError> {
        let table_names = query_table_names(&mut self.connection, &self.database)?;
        let column_rows = query_columns(&mut self.connection, &self.database)?;
        Ok(assemble_tables(table_names, column_rows, |row| {
            mysql_type_mapping::map_column(&mysql_column(row))
        }))
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

impl CatalogRow for ColumnRow {
    fn table(&self) -> &str {
        &self.table
    }

    fn column(&self) -> &str {
        &self.column
    }

    fn ordinal_position(&self) -> i64 {
        self.ordinal_position
    }

    /// `DATA_TYPE` rather than `COLUMN_TYPE`: an omission records the type
    /// name an operator reads in their own catalog, not the declaration text
    /// MySQL renders around it.
    fn source_type(&self) -> &str {
        &self.data_type
    }
}

/// The fields MySQL's type mapping reads, off one `information_schema` row.
fn mysql_column(row: ColumnRow) -> MySqlColumn {
    MySqlColumn {
        is_unsigned: row.column_type.to_ascii_lowercase().contains("unsigned"),
        data_type: row.data_type,
        character_length: row
            .character_maximum_length
            .and_then(|n| u32::try_from(n).ok()),
        character_set: row.character_set,
        numeric_precision: row.numeric_precision.and_then(|n| u32::try_from(n).ok()),
        numeric_scale: row.numeric_scale.and_then(|n| u32::try_from(n).ok()),
        is_nullable: row.is_nullable,
    }
}

/// Refuses a database name carrying a single quote or a backslash.
///
/// [`quote_string_literal`] escapes both, so a name carrying either is not
/// itself unsafe to query; this refuses it anyway, because that escaper uses
/// the backslash-escaping rule `mysql_dialect.rs` documents as unsafe under
/// `NO_BACKSLASH_ESCAPES` — the same rule this dialect refuses to rely on for
/// every other string it renders.
fn reject_unsafe_database_name(database: &str) -> Result<(), MetadataError> {
    match database
        .chars()
        .find(|character| matches!(character, '\'' | '\\'))
    {
        Some(character) => Err(MetadataError::UnsafeDatabaseName {
            database: database.to_owned(),
            character,
        }),
        None => Ok(()),
    }
}

/// Quotes a MySQL string literal, escaping a backslash and an embedded quote
/// so a database name containing either cannot change the shape of a
/// generated `information_schema` query.
///
/// A database name is validated before it ever reaches here:
/// [`reject_unsafe_database_name`] runs as this source's first precondition,
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

/// Visible to the crate's own tests: `metadata_source_tests` drives the rules
/// that bind every source through each source's own catalog shape, and this
/// module owns MySQL's.
#[cfg(test)]
#[path = "mysql_source_tests.rs"]
pub(crate) mod tests;
