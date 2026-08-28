//! Handles `createVirtualSchema`, `refresh`, and `setProperties`: all three
//! validate the same three properties, provision the driver, refuse a source
//! server missing a collation pushdown rendering emits, discover MySQL
//! metadata, map it to Exasol column definitions, and record the state
//! `pushdown` needs in `schemaMetadata.adapterNotes`.
//!
//! `dropVirtualSchema` needs none of this — it lives in [`crate::dispatch`]
//! as the bare response it always is.

use std::fmt;

use exasol_udf_sdk::connect_back::ConnectionObject;
use exasol_udf_sdk::context::UdfContext;
use serde_json::Value;

use crate::adapter_notes::{AdapterNotes, Omission};
use crate::exa_type;
use crate::metadata::{
    AdbcMetadataSource, DiscoveredTable, MetadataError, MetadataSource, SourceCollations,
};
use crate::mysql_dialect;
use crate::provisioning::{self, Bucket, Credentials, ProvisioningError, Registry};
use crate::type_mapping::{self, MappedColumn};

/// The bucket-relative mount every UDF sees BucketFS's `default` bucket at.
const BUCKETFS_MOUNT: &str = "/buckets/bfsdefault/default";

/// The only `DRIVER` property value this release supports.
const SUPPORTED_DRIVER: &str = "mysql";

/// The property naming the Exasol `CONNECTION` object that addresses MySQL.
///
/// It cannot be spelled `CONNECTION`: Exasol's parser reserves that word
/// inside a `CREATE VIRTUAL SCHEMA ... WITH` clause and rejects a quoted
/// property name outright, so a virtual schema carrying it can never be
/// created.
const CONNECTION_PROPERTY: &str = "CONNECTION_NAME";

/// The collations pushdown rendering requires at the source: an explicit
/// comparison collation for every character predicate, and a `NO PAD`
/// collation for a character operand of `MIN` or `MAX`.
///
/// Both names come from the dialect that emits them rather than being spelled
/// again here. The check and the renderer MUST agree: validating a source
/// against a collation the renderer no longer emits would pass a server that
/// then fails every delegated character predicate with MySQL error 1273 at
/// query time — the outcome this check exists to replace with a rejection an
/// operator can act on.
const REQUIRED_COLLATIONS: [&str; 2] = [
    mysql_dialect::COMPARISON_COLLATION,
    mysql_dialect::ORDERING_COLLATION,
];

/// The oldest MySQL version that reports both [`REQUIRED_COLLATIONS`], named
/// in the rejection error rather than compared against: a fork could report
/// an unrelated version string while still supplying the collations.
const MINIMUM_MYSQL_VERSION: &str = "8.0.17";

/// Everything that stops `createVirtualSchema`, `refresh`, or
/// `setProperties` from producing a `schemaMetadata` response.
#[derive(Debug)]
pub enum SchemaError {
    MissingProperty(&'static str),
    UnsupportedDriver(String),
    ConnectionResolution {
        name: String,
        cause: String,
    },
    ConnectionAddressMissingDatabase(String),
    Provisioning(ProvisioningError),
    ScanConnection(adbc_vs_scan::connection::ConnectError),
    Metadata(MetadataError),
    MissingCollation {
        collation: String,
        server_version: String,
    },
    UnsafeDatabaseName {
        database: String,
        character: char,
    },
}

impl fmt::Display for SchemaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingProperty(name) => write!(
                f,
                "the CREATE VIRTUAL SCHEMA statement is missing required property `{name}`"
            ),
            Self::UnsupportedDriver(value) => write!(
                f,
                "DRIVER `{value}` is not supported by adbc-vs 0.1; the only supported driver is \
                 `{SUPPORTED_DRIVER}`"
            ),
            Self::ConnectionResolution { name, cause } => {
                write!(f, "resolving CONNECTION `{name}`: {cause}")
            }
            Self::ConnectionAddressMissingDatabase(address) => write!(
                f,
                "connection address `{address}` has no database path component; expected \
                 mysql://host:port/database"
            ),
            Self::Provisioning(error) => write!(f, "{error}"),
            Self::ScanConnection(error) => write!(f, "{error}"),
            Self::Metadata(error) => write!(f, "{error}"),
            Self::MissingCollation {
                collation,
                server_version,
            } => write!(
                f,
                "the MySQL source server (reported version {server_version}) is missing \
                 collation `{collation}`, which adbc-vs 0.1's pushdown rendering requires; the \
                 minimum supported MySQL version is {MINIMUM_MYSQL_VERSION}"
            ),
            Self::UnsafeDatabaseName {
                database,
                character,
            } => write!(
                f,
                "the database name `{database}`, resolved from the {CONNECTION_PROPERTY} \
                 connection's address, contains `{character}`, which the information_schema \
                 queries built from it cannot safely quote"
            ),
        }
    }
}

impl std::error::Error for SchemaError {}

/// Builds the response to a `createVirtualSchema` request.
pub fn create_virtual_schema(
    ctx: &mut dyn UdfContext,
    request: &Value,
) -> Result<Value, SchemaError> {
    let (notes, tables) = discover(ctx, request)?;
    Ok(schema_response("createVirtualSchema", notes, tables))
}

/// Builds the response to a `refresh` request: the same discovery pipeline
/// as `createVirtualSchema`, echoing `requestedTables` back when the
/// request carries it.
pub fn refresh(ctx: &mut dyn UdfContext, request: &Value) -> Result<Value, SchemaError> {
    let (notes, tables) = discover(ctx, request)?;
    Ok(echo_requested_tables(
        request,
        schema_response("refresh", notes, tables),
    ))
}

/// Builds the response to a `setProperties` request: the same discovery
/// pipeline as `createVirtualSchema`, over the properties the request now
/// carries.
pub fn set_properties(ctx: &mut dyn UdfContext, request: &Value) -> Result<Value, SchemaError> {
    let (notes, tables) = discover(ctx, request)?;
    Ok(schema_response("setProperties", notes, tables))
}

/// The connection, driver, and in-progress notes every handler needs before
/// it can discover anything from the source.
///
/// [`discover`] is its only caller, which is what keeps the source-collation
/// check on every path that resolves a `CONNECTION`: opening the source and
/// discovering from it are one step to every handler.
///
/// The notes returned record no omission — nothing has mapped a column yet.
fn open(
    ctx: &mut dyn UdfContext,
    request: &Value,
) -> Result<(AdapterNotes, AdbcMetadataSource), SchemaError> {
    let properties = read_properties(request);
    let driver = required_property(&properties, "DRIVER")?;
    validate_driver(driver)?;
    let connection_name = required_property(&properties, CONNECTION_PROPERTY)?.to_string();
    let bucketfs_connection_name = required_property(&properties, "BUCKETFS_CONNECTION")?;

    let mysql_connection_object = resolve_connection(ctx, &connection_name)?;
    let database = parse_database(&mysql_connection_object.address)?;
    reject_unsafe_database_name(&database)?;

    let bucketfs_connection_object = resolve_connection(ctx, bucketfs_connection_name)?;
    let bucket = Bucket::new(
        &bucketfs_connection_object.address,
        BUCKETFS_MOUNT,
        Credentials::new(
            &bucketfs_connection_object.user,
            &bucketfs_connection_object.password,
        ),
    );
    let driver_path = provisioning::ensure_driver(&Registry::pinned(), &bucket)
        .map_err(SchemaError::Provisioning)?;
    let driver_path = driver_path.to_string_lossy().into_owned();

    let mysql_connection = adbc_vs_scan::connection::MysqlConnection {
        uri: mysql_connection_object.address,
        username: mysql_connection_object.user,
        password: mysql_connection_object.password,
    };
    let adbc_connection =
        adbc_vs_scan::connection::open_mysql_connection(&driver_path, &mysql_connection)
            .map_err(SchemaError::ScanConnection)?;
    let metadata_source = AdbcMetadataSource::new(adbc_connection);

    let notes = AdapterNotes {
        driver: SUPPORTED_DRIVER.to_string(),
        driver_version: provisioning::DRIVER_VERSION.to_string(),
        driver_path,
        connection: connection_name,
        database,
        omitted: vec![],
    };
    Ok((notes, metadata_source))
}

/// Everything the response needs from outside this process: [`open`]'s
/// connection and notes, plus the source's own tables discovered over it.
///
/// All three handlers answer through here, so all three check the source
/// server's collations. `createVirtualSchema` is not the only path that
/// resolves the `CONNECTION`: `setProperties` can point an existing virtual
/// schema at a different MySQL server, and `refresh` re-discovers against
/// whatever its `CONNECTION` resolves to now, so a schema created against a
/// supported server can end up reading one that supports nothing the dialect
/// renders.
fn discover(
    ctx: &mut dyn UdfContext,
    request: &Value,
) -> Result<(AdapterNotes, Vec<DiscoveredTable>), SchemaError> {
    let (notes, mut metadata_source) = open(ctx, request)?;
    let collations = metadata_source
        .discover_collations()
        .map_err(SchemaError::Metadata)?;
    let tables = discover_supported_source(collations, &mut metadata_source, &notes.database)?;
    Ok((notes, tables))
}

/// Discovers `database`'s tables over `source`, unless `collations` — what
/// that same source reported — is missing a collation pushdown rendering
/// emits.
///
/// The check runs first and returns rather than discovering anything, so a
/// source this release cannot render against is refused at the moment an
/// operator can act on it rather than after a table list has been built from
/// it. Composing the two steps in one function is what leaves no path that
/// discovers tables from an unchecked source.
fn discover_supported_source(
    collations: SourceCollations,
    source: &mut dyn MetadataSource,
    database: &str,
) -> Result<Vec<DiscoveredTable>, SchemaError> {
    check_required_collations(&collations)?;
    source
        .discover_tables(database)
        .map_err(SchemaError::Metadata)
}

/// Fails with [`SchemaError::MissingCollation`] naming the first of the
/// collations pushdown rendering requires that `collations` did not report,
/// and the server version `collations` reported alongside them.
fn check_required_collations(collations: &SourceCollations) -> Result<(), SchemaError> {
    for required in REQUIRED_COLLATIONS {
        if !collations.names.iter().any(|name| name == required) {
            return Err(SchemaError::MissingCollation {
                collation: required.to_string(),
                server_version: collations.server_version.clone(),
            });
        }
    }
    Ok(())
}

/// Assembles the `schemaMetadata` response every discovery answers with.
///
/// Pure: it reaches nothing outside its arguments, which is what lets the
/// shape Exasol persists be asserted without a source database. `notes`
/// arrives from [`discover`] carrying no omission, because only mapping the
/// columns produces one; the list this mapping yields replaces whatever
/// `omitted` held.
fn schema_response(
    response_type: &str,
    mut notes: AdapterNotes,
    tables: Vec<DiscoveredTable>,
) -> Value {
    let (table_values, omitted) = map_tables(tables);
    notes.omitted = omitted;

    serde_json::json!({
        "type": response_type,
        "schemaMetadata": {
            "tables": table_values,
            "adapterNotes": notes.to_json_string(),
        },
    })
}

/// Puts a `refresh` request's own `requestedTables` back on its response,
/// which the protocol expects echoed rather than answered.
fn echo_requested_tables(request: &Value, mut response: Value) -> Value {
    if let Some(requested_tables) = request.get("requestedTables") {
        response["requestedTables"] = requested_tables.clone();
    }
    response
}

/// The properties this request is to be answered under.
///
/// A `setProperties` request carries the schema's *current* properties under
/// `schemaMetadataInfo` and only the *changed* ones at the top level, so the
/// two are merged with the request-level entry winning — reading either alone
/// would re-discover against the configuration the statement just replaced. A
/// `null` at the top level is how the protocol removes a property, so it drops
/// the key rather than storing a null. `createVirtualSchema` carries no
/// top-level `properties` object at all and passes through unchanged.
fn read_properties(request: &Value) -> serde_json::Map<String, Value> {
    let mut properties = request
        .pointer("/schemaMetadataInfo/properties")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();

    let Some(changed) = request.get("properties").and_then(Value::as_object) else {
        return properties;
    };
    for (name, value) in changed {
        if value.is_null() {
            properties.remove(name);
        } else {
            properties.insert(name.clone(), value.clone());
        }
    }
    properties
}

fn required_property<'a>(
    properties: &'a serde_json::Map<String, Value>,
    name: &'static str,
) -> Result<&'a str, SchemaError> {
    properties
        .get(name)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or(SchemaError::MissingProperty(name))
}

fn validate_driver(driver: &str) -> Result<(), SchemaError> {
    if driver.eq_ignore_ascii_case(SUPPORTED_DRIVER) {
        Ok(())
    } else {
        Err(SchemaError::UnsupportedDriver(driver.to_string()))
    }
}

fn resolve_connection(
    ctx: &mut dyn UdfContext,
    name: &str,
) -> Result<ConnectionObject, SchemaError> {
    ctx.connection(name)
        .map_err(|error| SchemaError::ConnectionResolution {
            name: name.to_string(),
            cause: error.to_string(),
        })
}

/// The database name is the path component of a MySQL `CONNECTION`
/// address (`mysql://host:port/database`) — the live spike found the
/// address must already be a full driver URI, so the database can only
/// come from here, never from a separate property.
fn parse_database(address: &str) -> Result<String, SchemaError> {
    let missing = || SchemaError::ConnectionAddressMissingDatabase(address.to_string());

    let after_scheme = address.split_once("://").map_or(address, |(_, rest)| rest);
    let (_, path) = after_scheme.split_once('/').ok_or_else(missing)?;
    let database = path.split(['?', '#']).next().unwrap_or("");
    if database.is_empty() {
        return Err(missing());
    }
    Ok(database.to_string())
}

/// Refuses a resolved database name carrying a single quote or a backslash.
///
/// `metadata::quote_string_literal` escapes both for the
/// `information_schema` queries built from `database`, so a name carrying
/// either is not itself unsafe to query; this rejects it anyway, because
/// `crate::metadata`'s own escaper uses the backslash-escaping rule
/// `mysql_dialect.rs` documents as unsafe under `NO_BACKSLASH_ESCAPES` — the
/// same rule this dialect refuses to rely on for every other string it
/// renders.
fn reject_unsafe_database_name(database: &str) -> Result<(), SchemaError> {
    match database
        .chars()
        .find(|character| matches!(character, '\'' | '\\'))
    {
        Some(character) => Err(SchemaError::UnsafeDatabaseName {
            database: database.to_string(),
            character,
        }),
        None => Ok(()),
    }
}

/// Maps every discovered table's columns, returning the tables that kept at
/// least one mappable column and the full list of omissions — a column
/// whose type does not map, and a table left with no mappable column at
/// all.
fn map_tables(tables: Vec<DiscoveredTable>) -> (Vec<Value>, Vec<Omission>) {
    let mut table_values = Vec::new();
    let mut omitted = Vec::new();

    for table in tables {
        let mut columns = Vec::new();
        for column in table.columns {
            match type_mapping::map_column(&column.mysql) {
                Ok(mapped) => columns.push(column_json(&column.name, mapped)),
                Err(reason) => omitted.push(Omission {
                    table: table.name.clone(),
                    column: Some(column.name),
                    mysql_type: Some(column.mysql.data_type),
                    reason,
                }),
            }
        }

        if columns.is_empty() {
            omitted.push(Omission {
                table: table.name,
                column: None,
                mysql_type: None,
                reason: "no column of this table maps to a supported Exasol type".to_string(),
            });
            continue;
        }
        table_values.push(serde_json::json!({"name": table.name, "columns": columns}));
    }

    (table_values, omitted)
}

fn column_json(name: &str, mapped: MappedColumn) -> Value {
    serde_json::json!({
        "name": name,
        "dataType": exa_type::to_json(mapped.exasol_type.into()),
        "isNullable": mapped.is_nullable,
    })
}

#[cfg(test)]
#[path = "create_schema_tests.rs"]
mod tests;
