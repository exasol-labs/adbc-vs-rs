//! Discovers PostgreSQL metadata over an open ADBC connection.
//!
//! The exposed namespace is the `public` schema and nothing else. A PostgreSQL
//! database is not a SQL qualifier — it is what the connection selects — so
//! the connection address's database component says nothing about which schema
//! a statement should qualify a table with. `public` is both PostgreSQL's
//! default qualifier and the `information_schema` filter value, it is what
//! `adapterNotes` records as the namespace, and it is what
//! `postgres_dialect` then renders in front of every table name.
//!
//! Because the namespace is a compile-time constant, no value resolved from a
//! `CONNECTION` object or typed by an operator reaches either query below:
//! there is no string literal to escape and no name a source could carry that
//! changes their shape. MySQL's own catalog queries escape a database name for
//! exactly the reason this source has none to escape.
//!
//! Every selected column is cast to a base type and given an explicit alias.
//! `information_schema` reports its columns as SQL-standard domains —
//! `sql_identifier`, `character_data`, `cardinal_number`, `yes_or_no` — and
//! which Arrow type a driver produces for a domain is a decision belonging to
//! the driver rather than to this catalog. Casting settles it here instead of
//! depending on an answer nothing in this repo controls.
//!
//! Both base tables and views are exposed, because a view is a queryable
//! relation with the same catalog shape and Exasol has no reason to see one
//! and not the other.

use adbc_driver_manager::ManagedConnection;

use crate::adapter_notes::SourceQualifier;
use crate::metadata::{
    assemble_tables, batch_column, i64_cell, required_i64_cell, required_string_cell, run_query,
    string_cell, CatalogRow, DiscoveredTable, MetadataError, MetadataSource,
};
use crate::postgres_type_mapping::{self, ColumnCollation, PostgresColumn};

/// The one PostgreSQL schema a virtual schema exposes, and the namespace
/// `adapterNotes` records.
///
/// Exposing `public` alone is this release's boundary: a virtual schema per
/// source schema needs no new property, while a property naming a schema would
/// be a decision the adapter declined to make and could not later narrow.
const EXPOSED_NAMESPACE: &str = "public";

/// The value `information_schema` reports in `is_nullable` for a column that
/// admits a null.
const NULLABLE: &str = "YES";

/// The one database encoding this release maps a PostgreSQL source from.
///
/// The mapping declares every character column Exasol `VARCHAR` with character
/// set `UTF8`, and the `COLLATE "C"` suffix `postgres_dialect` renders for an
/// ordering comparison reproduces Exasol's code-point order only while the
/// server compares UTF-8 bytes. Under another encoding both are wrong in ways
/// that surface as wrong rows rather than as an error, which is why the
/// database is refused instead.
const REQUIRED_ENCODING: &str = "UTF8";

/// The two spellings a `boolean` takes once cast to text, which is how the
/// columns query returns `pg_collation.collisdeterministic` — cast rather than
/// read as a boolean, so no Arrow representation of one has to be assumed of
/// the driver.
const DETERMINISTIC: &str = "true";
const NONDETERMINISTIC: &str = "false";

/// Discovers PostgreSQL metadata over an open ADBC connection.
pub struct PostgresSource {
    connection: ManagedConnection,
}

impl PostgresSource {
    /// Wraps an already-open ADBC connection to the source PostgreSQL
    /// database.
    ///
    /// It takes no namespace: which schema this source exposes is
    /// [`EXPOSED_NAMESPACE`] and not a caller's decision.
    pub fn new(connection: ManagedConnection) -> Self {
        Self { connection }
    }
}

impl MetadataSource for PostgresSource {
    fn namespace(&self) -> SourceQualifier {
        SourceQualifier::single(EXPOSED_NAMESPACE)
    }

    /// Refuses a database whose encoding is not [`REQUIRED_ENCODING`].
    ///
    /// This is the precondition PostgreSQL's rendering states, and it is the
    /// whole of it: MySQL's collation-and-version check belongs to MySQL's own
    /// source and does not run here, just as this one does not run there.
    fn check_source(&mut self) -> Result<(), MetadataError> {
        let encoding = query_server_encoding(&mut self.connection)?;
        check_server_encoding(&encoding)
    }

    fn discover_tables(&mut self) -> Result<Vec<DiscoveredTable>, MetadataError> {
        let table_names = query_table_names(&mut self.connection)?;
        let column_rows = query_columns(&mut self.connection)?;
        Ok(assemble_tables(table_names, column_rows, |row| {
            postgres_type_mapping::map_column(&postgres_column(row))
        }))
    }
}

/// One row of the `information_schema.columns` query, before it is grouped
/// into a [`DiscoveredTable`].
///
/// `collation_is_deterministic` is the one field no `information_schema` view
/// reports: the standard names a column's collation but says nothing about its
/// determinism, so the query resolves it out of `pg_catalog` and it is absent
/// for a column declaring no collation of its own.
struct ColumnRow {
    table: String,
    column: String,
    ordinal_position: i64,
    data_type: String,
    character_maximum_length: Option<i64>,
    numeric_precision: Option<i64>,
    numeric_scale: Option<i64>,
    collation_name: Option<String>,
    collation_is_deterministic: Option<bool>,
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

    fn source_type(&self) -> &str {
        &self.data_type
    }
}

/// Reads the determinism flag the columns query returns as text.
///
/// Absent means deterministic — the column declares no collation of its own,
/// or the server predates the flag and so has no nondeterministic collation to
/// distinguish — and so does anything but the two spellings a `boolean` casts
/// to, because refusing every character column of a source whose catalog
/// answered unexpectedly is a worse outcome than trusting the release-wide
/// fact that every collation PostgreSQL ships is deterministic.
fn determinism_flag(reported: Option<String>) -> Option<bool> {
    match reported.as_deref() {
        Some(DETERMINISTIC) => Some(true),
        Some(NONDETERMINISTIC) => Some(false),
        _ => None,
    }
}

/// The fields PostgreSQL's type mapping reads, off one `information_schema`
/// row.
///
/// A length, precision, or scale outside `u32` is read as absent rather than
/// clamped: the mapping refuses a column whose size it does not know, which is
/// the right outcome for a value that cannot be one.
fn postgres_column(row: ColumnRow) -> PostgresColumn {
    PostgresColumn {
        data_type: row.data_type,
        character_length: row
            .character_maximum_length
            .and_then(|n| u32::try_from(n).ok()),
        numeric_precision: row.numeric_precision.and_then(|n| u32::try_from(n).ok()),
        numeric_scale: row.numeric_scale.and_then(|n| u32::try_from(n).ok()),
        collation: row.collation_name.map(|name| ColumnCollation {
            name,
            is_deterministic: row.collation_is_deterministic,
        }),
        is_nullable: row.is_nullable,
    }
}

/// The statement listing the exposed namespace's base tables and views.
///
/// Separate from the call that runs it so the text itself can be asserted
/// without a source database — the filter, the exposed relation kinds, and the
/// casts are the whole of what this source decides about the query.
fn table_names_query() -> String {
    format!(
        "SELECT table_name::text AS table_name FROM information_schema.tables \
         WHERE table_schema = '{EXPOSED_NAMESPACE}' \
         AND table_type IN ('BASE TABLE', 'VIEW') \
         ORDER BY table_name"
    )
}

/// The statement listing the exposed namespace's columns, in the order the
/// virtual tables declare them.
///
/// `information_schema.columns` answers everything the mapping reads except
/// one thing: it names a column's collation but not whether that collation is
/// deterministic, which only `pg_catalog.pg_collation` reports. The outer join
/// resolves it there, matched on the collation's schema as well as its name
/// because two schemas may hold collations sharing one, and left so a column
/// declaring no collation of its own still produces a row.
fn columns_query() -> String {
    format!(
        "SELECT c.table_name::text AS table_name, c.column_name::text AS column_name, \
         c.ordinal_position::bigint AS ordinal_position, c.data_type::text AS data_type, \
         c.character_maximum_length::bigint AS character_maximum_length, \
         c.numeric_precision::bigint AS numeric_precision, \
         c.numeric_scale::bigint AS numeric_scale, \
         c.is_nullable::text AS is_nullable, \
         c.collation_name::text AS collation_name, \
         pgc.collisdeterministic::text AS collation_is_deterministic \
         FROM information_schema.columns AS c \
         LEFT JOIN (pg_catalog.pg_collation AS pgc \
         JOIN pg_catalog.pg_namespace AS pgn ON pgn.oid = pgc.collnamespace) \
         ON pgc.collname = c.collation_name::text \
         AND pgn.nspname = c.collation_schema::text \
         WHERE c.table_schema = '{EXPOSED_NAMESPACE}' \
         ORDER BY c.table_name, c.ordinal_position"
    )
}

/// The statement reading the connected database's own encoding.
///
/// `current_setting` rather than `SHOW`, so the value arrives under an alias
/// this module named; PostgreSQL 16 removed the `lc_collate` and `lc_ctype`
/// GUCs but `server_encoding` is unaffected.
///
/// Visible to the crate's own tests: `metadata_source_tests` asserts that each
/// source's precondition reads its own catalog and no other's, which is a
/// claim about this statement.
pub(crate) fn server_encoding_query() -> &'static str {
    "SELECT current_setting('server_encoding')::text AS server_encoding"
}

/// Fails naming the encoding the database reported and the one this release
/// requires, because an operator can act on neither alone.
///
/// [`MetadataError::UnsupportedSource`] rather than `Catalog`: a database's
/// encoding is fixed at creation, so the answer is a different database and
/// never a retry.
fn check_server_encoding(encoding: &str) -> Result<(), MetadataError> {
    if encoding.eq_ignore_ascii_case(REQUIRED_ENCODING) {
        return Ok(());
    }
    Err(MetadataError::UnsupportedSource(format!(
        "the PostgreSQL source database reports encoding {encoding}, but adbc-vs maps a \
         PostgreSQL source only from a database whose encoding is {REQUIRED_ENCODING}; every \
         character column is declared UTF8 and the ordering collation the dialect renders \
         reproduces Exasol's order only over UTF-8 bytes"
    )))
}

fn query_server_encoding(connection: &mut ManagedConnection) -> Result<String, MetadataError> {
    let sql = server_encoding_query();

    for batch in run_query(connection, sql)? {
        let column = batch_column(&batch, "server_encoding", sql)?;
        if batch.num_rows() > 0 {
            return required_string_cell(column, 0, "server_encoding");
        }
    }
    Err(MetadataError::Catalog(format!("`{sql}` returned no row")))
}

fn query_table_names(connection: &mut ManagedConnection) -> Result<Vec<String>, MetadataError> {
    let sql = table_names_query();

    let mut names = Vec::new();
    for batch in run_query(connection, &sql)? {
        let column = batch_column(&batch, "table_name", &sql)?;
        for row in 0..batch.num_rows() {
            names.push(required_string_cell(column, row, "table_name")?);
        }
    }
    Ok(names)
}

fn query_columns(connection: &mut ManagedConnection) -> Result<Vec<ColumnRow>, MetadataError> {
    let sql = columns_query();

    let mut rows = Vec::new();
    for batch in run_query(connection, &sql)? {
        let table = batch_column(&batch, "table_name", &sql)?;
        let column = batch_column(&batch, "column_name", &sql)?;
        let ordinal_position = batch_column(&batch, "ordinal_position", &sql)?;
        let data_type = batch_column(&batch, "data_type", &sql)?;
        let character_maximum_length = batch_column(&batch, "character_maximum_length", &sql)?;
        let numeric_precision = batch_column(&batch, "numeric_precision", &sql)?;
        let numeric_scale = batch_column(&batch, "numeric_scale", &sql)?;
        let is_nullable = batch_column(&batch, "is_nullable", &sql)?;
        let collation_name = batch_column(&batch, "collation_name", &sql)?;
        let collation_is_deterministic = batch_column(&batch, "collation_is_deterministic", &sql)?;

        for row in 0..batch.num_rows() {
            rows.push(ColumnRow {
                table: required_string_cell(table, row, "table_name")?,
                column: required_string_cell(column, row, "column_name")?,
                ordinal_position: required_i64_cell(ordinal_position, row, "ordinal_position")?,
                data_type: required_string_cell(data_type, row, "data_type")?,
                character_maximum_length: i64_cell(character_maximum_length, row)?,
                numeric_precision: i64_cell(numeric_precision, row)?,
                numeric_scale: i64_cell(numeric_scale, row)?,
                collation_name: string_cell(collation_name, row)?,
                collation_is_deterministic: determinism_flag(string_cell(
                    collation_is_deterministic,
                    row,
                )?),
                is_nullable: required_string_cell(is_nullable, row, "is_nullable")? == NULLABLE,
            });
        }
    }
    Ok(rows)
}

/// Visible to the crate's own tests: `metadata_source_tests` drives the rules
/// that bind every source through each source's own catalog shape, and this
/// module owns PostgreSQL's.
#[cfg(test)]
#[path = "postgres_source_tests.rs"]
pub(crate) mod tests;
