//! Discovers Trino metadata over an open ADBC connection.
//!
//! The exposed namespace is the catalog and the schema the `CONNECTION`
//! address's path names, in that order: a Trino table's fully qualified name
//! is `catalog.schema.table`, so both are needed and neither is derivable
//! from the other. Both queries below are qualified by the catalog —
//! `information_schema` is scoped to one catalog at a time, so a row a query
//! returns always belongs to it — and filtered to the schema.
//!
//! Trino's `information_schema.columns` reports a column's full type only as
//! the string in its `data_type` column. It has no `character_maximum_length`,
//! `numeric_precision`, `numeric_scale`, or `datetime_precision` column at
//! all — those names are absent rather than null, so selecting one fails the
//! query — which is why `trino_type_mapping` parses `data_type` rather than
//! reading a field: Trino's catalog reports a column's full type only as a
//! string, so `data_type` is the only place the declared type exists.
//!
//! The catalog and the schema are operator-typed, carried in from the
//! `CONNECTION` address rather than a constant, so both are quoted before
//! they reach either query: the catalog as a double-quoted identifier and the
//! schema as a single-quoted string literal, doubling an embedded quote
//! either way — the same rule `vs-adapter/trino-pushdown` states for every
//! identifier and literal its own rendering emits.
//!
//! Both base tables and views are exposed, because a view is a queryable
//! relation with the same catalog shape and Exasol has no reason to see one
//! and not the other.

use adbc_driver_manager::ManagedConnection;
use adbc_vs_scan::driver::Driver;

use crate::adapter_notes::SourceQualifier;
use crate::metadata::{
    assemble_tables, batch_column, required_i64_cell, required_string_cell, run_query, CatalogRow,
    DiscoveredTable, MetadataError, MetadataSource,
};
use crate::trino_type_mapping::{self, TrinoColumn};

/// The value `information_schema` reports in `is_nullable` for a column that
/// admits a null.
const NULLABLE: &str = "YES";

/// Discovers Trino metadata over an open ADBC connection.
pub struct TrinoSource {
    connection: ManagedConnection,
    /// The catalog and the schema the `CONNECTION` address's path names, in
    /// that order: both the `information_schema` scope and filter, and the
    /// namespace this source exposes.
    qualifier: SourceQualifier,
}

impl TrinoSource {
    /// Wraps an already-open ADBC connection to the catalog and schema
    /// `qualifier` names on the source Trino cluster.
    ///
    /// Taking the qualifier rather than two strings is what makes the
    /// namespace answerable without a fallible step: `SourceQualifier` already
    /// promises every segment is non-blank, so this source cannot be built
    /// around a name that would break that promise for a later reader.
    pub fn new(connection: ManagedConnection, qualifier: SourceQualifier) -> Self {
        Self {
            connection,
            qualifier,
        }
    }

    /// The two names the queries below are scoped and filtered by, read off
    /// the qualifier rather than stored beside it.
    ///
    /// How many segments a Trino qualifier carries is declared in
    /// `drivers/trino.toml` and read back here, so this is not a second owner
    /// of that count. Reading the pair fails rather than indexing past the end
    /// because a panic inside a UDF surfaces as a crashed VM naming nothing,
    /// while `source_for` has already refused the same shape with a message.
    fn catalog_and_schema(&self) -> Result<(&str, &str), MetadataError> {
        let declared = Driver::Trino.qualifier_segment_count();
        let segments = self.qualifier.segments();
        let [catalog, schema] = segments else {
            return Err(MetadataError::NamespaceArity {
                driver: Driver::Trino.as_str(),
                declared,
                produced: segments.len(),
            });
        };
        Ok((catalog, schema))
    }
}

impl MetadataSource for TrinoSource {
    fn namespace(&self) -> SourceQualifier {
        self.qualifier.clone()
    }

    /// Succeeds without querying the server.
    ///
    /// Nothing Trino's rendering depends on is configurable: code-point
    /// comparison, the double-quote identifier form, and the typed-literal
    /// requirement are all fixed in Trino's SQL engine on every server this
    /// release supports, so there is nothing here for a query to read. The
    /// check still runs, at the same point every other driver's does, so
    /// `vs-adapter/create-virtual-schema`'s contract holds with no driver
    /// excepted.
    fn check_source(&mut self) -> Result<(), MetadataError> {
        Ok(())
    }

    fn discover_tables(&mut self) -> Result<Vec<DiscoveredTable>, MetadataError> {
        let (catalog, schema) = self.catalog_and_schema()?;
        let (catalog, schema) = (catalog.to_owned(), schema.to_owned());
        let table_names = query_table_names(&mut self.connection, &catalog, &schema)?;
        let column_rows = query_columns(&mut self.connection, &catalog, &schema)?;
        Ok(assemble_tables(table_names, column_rows, |row| {
            trino_type_mapping::map_column(&trino_column(row))
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

/// The one field Trino's type mapping reads, off one `information_schema`
/// row: the type string it parses for the length, precision, and scale no
/// column of this catalog carries.
fn trino_column(row: ColumnRow) -> TrinoColumn {
    TrinoColumn {
        data_type: row.data_type,
        is_nullable: row.is_nullable,
    }
}

/// Quotes a Trino identifier, doubling an embedded double quote.
///
/// Every identifier is quoted unconditionally, matching the rule
/// `vs-adapter/trino-pushdown` states for the statements it renders: Trino
/// requires quoting for a reserved word and for any name outside letters,
/// digits, and underscores, and a rule that quoted only those names would
/// have to carry Trino's reserved-word list. The catalog reaches this query
/// from an operator-typed `CONNECTION` address rather than from a constant,
/// so it is quoted here for the same reason a rendered statement quotes it.
fn quote_identifier(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// Quotes a Trino string literal, doubling an embedded single quote.
///
/// Matches the literal grammar `vs-adapter/trino-pushdown` renders: a plain
/// quoted constant with every single quote doubled and nothing else escaped.
fn quote_string_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

/// The statement listing the named catalog and schema's base tables and
/// views.
///
/// Separate from the call that runs it so the text itself can be asserted
/// without a source cluster — the catalog qualification, the schema filter,
/// and the exposed relation kinds are the whole of what this source decides
/// about the query.
fn table_names_query(catalog: &str, schema: &str) -> String {
    format!(
        "SELECT table_name FROM {}.information_schema.tables \
         WHERE table_schema = {} AND table_type IN ('BASE TABLE', 'VIEW') \
         ORDER BY table_name",
        quote_identifier(catalog),
        quote_string_literal(schema)
    )
}

/// The statement listing the named catalog and schema's columns, in the
/// order the virtual tables declare them.
///
/// Selects only what `information_schema.columns` actually carries for
/// Trino — `table_name`, `column_name`, `ordinal_position`, `data_type`, and
/// `is_nullable` — because a length, a precision, a scale, or a datetime
/// precision column does not exist on this catalog, and selecting one fails
/// the query rather than returning null for it.
fn columns_query(catalog: &str, schema: &str) -> String {
    format!(
        "SELECT table_name, column_name, ordinal_position, data_type, is_nullable \
         FROM {}.information_schema.columns \
         WHERE table_schema = {} \
         ORDER BY table_name, ordinal_position",
        quote_identifier(catalog),
        quote_string_literal(schema)
    )
}

fn query_table_names(
    connection: &mut ManagedConnection,
    catalog: &str,
    schema: &str,
) -> Result<Vec<String>, MetadataError> {
    let sql = table_names_query(catalog, schema);

    let mut names = Vec::new();
    for batch in run_query(connection, &sql)? {
        let column = batch_column(&batch, "table_name", &sql)?;
        for row in 0..batch.num_rows() {
            names.push(required_string_cell(column, row, "table_name")?);
        }
    }
    Ok(names)
}

fn query_columns(
    connection: &mut ManagedConnection,
    catalog: &str,
    schema: &str,
) -> Result<Vec<ColumnRow>, MetadataError> {
    let sql = columns_query(catalog, schema);

    let mut rows = Vec::new();
    for batch in run_query(connection, &sql)? {
        let table = batch_column(&batch, "table_name", &sql)?;
        let column = batch_column(&batch, "column_name", &sql)?;
        let ordinal_position = batch_column(&batch, "ordinal_position", &sql)?;
        let data_type = batch_column(&batch, "data_type", &sql)?;
        let is_nullable = batch_column(&batch, "is_nullable", &sql)?;

        for row in 0..batch.num_rows() {
            rows.push(ColumnRow {
                table: required_string_cell(table, row, "table_name")?,
                column: required_string_cell(column, row, "column_name")?,
                ordinal_position: required_i64_cell(ordinal_position, row, "ordinal_position")?,
                data_type: required_string_cell(data_type, row, "data_type")?,
                is_nullable: is_nullable_cell(&required_string_cell(
                    is_nullable,
                    row,
                    "is_nullable",
                )?),
            });
        }
    }
    Ok(rows)
}

/// Whether `information_schema` reported a column as admitting a null.
///
/// The catalog spells this as a string rather than a boolean, and only `YES`
/// means nullable — every other spelling, including one a future Trino release
/// might introduce, is read as not nullable, because a column wrongly exposed
/// as nullable loses a constraint Exasol would otherwise enforce.
fn is_nullable_cell(value: &str) -> bool {
    value == NULLABLE
}

/// Visible to the crate's own tests: `metadata_source_tests` drives the rules
/// that bind every source through each source's own catalog shape, and this
/// module owns Trino's.
#[cfg(test)]
#[path = "trino_source_tests.rs"]
pub(crate) mod tests;
