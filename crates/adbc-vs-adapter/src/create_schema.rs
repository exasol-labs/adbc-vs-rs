//! Handles `createVirtualSchema`, `refresh`, and `setProperties`: all three
//! validate the same properties, resolve the driver identity, make that
//! driver's library readable, refuse a source its dialect cannot render
//! against, discover the source's already-mapped tables, and record the state
//! `pushdown` needs in `schemaMetadata.adapterNotes`.
//!
//! Every property the statement carries is validated before any `CONNECTION` is
//! resolved, so a statement that could never provision is refused without
//! reaching anything outside the process.
//!
//! Nothing here names a technology. Which catalog is read, which of its types
//! map, and what the source must supply are all answered behind
//! [`MetadataSource`], so this module assembles the same response whatever the
//! `DRIVER` property said.
//!
//! `dropVirtualSchema` needs none of this — it lives in [`crate::dispatch`]
//! as the bare response it always is.

use std::fmt;
use std::path::{Path, PathBuf};

use adbc_vs_scan::driver::{Driver, UnsupportedDriver};
use exasol_udf_sdk::connect_back::ConnectionObject;
use exasol_udf_sdk::context::UdfContext;
use serde_json::Value;

use crate::adapter_notes::{AdapterNotes, Omission, SourceQualifier};
use crate::exa_type;
use crate::metadata::{self, DiscoveredTable, MetadataError, MetadataSource};
use crate::provisioning::{
    self, Bucket, Credentials, InstalledDriver, ProvisioningError, Registry, DRIVER_PATH_PROPERTY,
};
use crate::type_mapping::MappedColumn;

/// The bucket-relative mount every UDF sees BucketFS's `default` bucket at.
///
/// [`open`] is the only reader of it. Everything downstream takes the mount as
/// an argument, so the install path and the default driver location are always
/// computed under one value — which is what makes their disjointness a property
/// of the layout rather than a coincidence of two callers agreeing — and so
/// both branches of the last-resort route are assertable against a directory
/// inside the workspace.
const BUCKETFS_MOUNT: &str = "/buckets/bfsdefault/default";

/// The property naming the Exasol `CONNECTION` object holding the BucketFS
/// write credentials driver self-install needs.
///
/// Required only while self-install is the route: a statement that supplies its
/// own driver library through `DRIVER_PATH` writes nothing, and reads it back
/// through the read-only mount every UDF already sees.
const BUCKETFS_CONNECTION_PROPERTY: &str = "BUCKETFS_CONNECTION";

/// The `CONNECTION` self-install writes the fetched driver through when the
/// statement names no `BUCKETFS_CONNECTION` of its own.
///
/// The name is reserved rather than configurable: a value an operator can set
/// is a decision the module declined to make, and every cluster this adapter
/// runs on needs exactly one BucketFS write credential, never a choice among
/// several. It lives here rather than in `provisioning` because this module
/// owns every connection-name decision the adapter makes; `provisioning` is
/// handed credentials and resolves no name of its own.
pub const DEFAULT_BUCKETFS_CONNECTION: &str = "ADBC_VS_BUCKETFS";

/// The property naming the Exasol `CONNECTION` object that addresses the
/// source.
///
/// It cannot be spelled `CONNECTION`: Exasol's parser reserves that word
/// inside a `CREATE VIRTUAL SCHEMA ... WITH` clause and rejects a quoted
/// property name outright, so a virtual schema carrying it can never be
/// created.
const CONNECTION_PROPERTY: &str = "CONNECTION_NAME";

/// Why self-install through the reserved connection could not run at all,
/// which is the only thing that reaches the default driver location.
///
/// Two variants rather than two `Option` fields on the error, so a last-resort
/// failure cannot be built naming both causes or neither: exactly one of them
/// happened. The registry arm carries the two facts
/// [`ProvisioningError::registry_unreachable`] hands back, never a rendered
/// [`ProvisioningError`], because that module's prose names `DRIVER_PATH` as
/// the remedy and this route's remedy is the location its own message names.
///
/// Public only because it is a field of the public [`SchemaError`]. Nothing
/// outside this module builds one: the route rule that decides which variant a
/// failure produces is [`self_install_fallback`], and it is private.
#[derive(Debug)]
pub enum FallbackCause {
    ConnectionUnresolved(String),
    RegistryUnreachable { url: String, cause: String },
}

/// The sentence that reports the reserved connection itself as the failure.
///
/// Named rather than inlined because its absence is the assertable difference
/// between the two [`FallbackCause`] arms: on the registry arm that connection
/// resolved, so the message names it as context and must not report it as what
/// went wrong.
const UNRESOLVED_CONNECTION_SENTENCE: &str = "That reserved connection did not resolve";

/// Everything that stops `createVirtualSchema`, `refresh`, or
/// `setProperties` from producing a `schemaMetadata` response.
#[derive(Debug)]
pub enum SchemaError {
    MissingProperty(&'static str),
    UnsupportedDriver(UnsupportedDriver),
    ConnectionResolution {
        name: String,
        cause: String,
    },
    ConnectionAddressQualifierArity {
        driver: &'static str,
        address: String,
        needed: usize,
        carried: usize,
    },
    /// An address that spells the right number of path elements but names
    /// nothing with one of them. Reachable only for a driver qualifying with
    /// more than one segment: a single non-empty path element cannot be
    /// blank, so a one-segment driver fails the arity check instead.
    ConnectionAddressQualifierSegment {
        driver: &'static str,
        address: String,
        cause: String,
    },
    ConnectionAddressMissingScheme {
        driver: &'static str,
        address: String,
    },
    ConnectionSchemeMismatch {
        driver: &'static str,
        scheme: String,
    },
    NoDriverRoute {
        driver: &'static str,
        path: PathBuf,
        fallback: FallbackCause,
        unreadable: String,
    },
    Provisioning(ProvisioningError),
    ScanConnection(adbc_vs_scan::connection::ConnectError),
    Metadata(MetadataError),
}

impl fmt::Display for SchemaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingProperty(name) => write!(
                f,
                "the CREATE VIRTUAL SCHEMA statement is missing required property `{name}`"
            ),
            Self::UnsupportedDriver(error) => write!(f, "{error}"),
            Self::ConnectionResolution { name, cause } => {
                write!(f, "resolving CONNECTION `{name}`: {cause}")
            }
            Self::ConnectionAddressQualifierArity {
                driver,
                address,
                needed,
                carried,
            } => write!(
                f,
                "the {CONNECTION_PROPERTY} connection's address `{address}` spells {carried} \
                 qualifier segments, but DRIVER `{driver}` qualifies its source tables with \
                 {needed}; adbc-vs reads one path element per segment, so an address carrying \
                 another number names a different object than the statement asked for"
            ),
            Self::ConnectionAddressQualifierSegment {
                driver,
                address,
                cause,
            } => write!(
                f,
                "the {CONNECTION_PROPERTY} connection's address `{address}` cannot name the \
                 `{driver}` source qualifier: {cause}"
            ),
            Self::ConnectionAddressMissingScheme { driver, address } => write!(
                f,
                "the {CONNECTION_PROPERTY} connection's address `{address}` carries no URI \
                 scheme; DRIVER `{driver}` requires an address of the form \
                 {driver}://host:port/database"
            ),
            Self::ConnectionSchemeMismatch { driver, scheme } => write!(
                f,
                "the {CONNECTION_PROPERTY} connection's address carries the `{scheme}` scheme, \
                 but the statement names DRIVER `{driver}`; adbc-vs hands the address to that \
                 driver unchanged, so the mismatch would surface as a source connection failure \
                 rather than as the property mismatch it is"
            ),
            Self::NoDriverRoute {
                driver,
                path,
                fallback,
                unreadable,
            } => write_no_driver_route(f, driver, path, fallback, unreadable),
            Self::Provisioning(error) => write!(f, "{error}"),
            Self::ScanConnection(error) => write!(f, "{error}"),
            Self::Metadata(error) => write!(f, "{error}"),
        }
    }
}

/// The refusal an operator gets when every route to a driver library is
/// exhausted, which is the whole diagnosis they have to act on.
///
/// It names the three property and connection names from this module's own
/// constants rather than from fields, so the reserved connection is context on
/// both arms and the reported failure on one. It quotes no text from
/// [`ProvisioningError`]: that prose offers `DRIVER_PATH` as the remedy, and
/// this route's remedy is the location this message already names.
///
/// `unreadable` and `fallback` report two different failures and never one
/// twice: `fallback` is why self-install could not run, and `unreadable` is why
/// the location it fell back to holds nothing loadable. Stating the second is
/// what keeps the message true for an operator who *did* pre-place a library
/// and got its kind or its mode bits wrong — telling them nothing is there
/// would send them to look in the wrong place.
fn write_no_driver_route(
    f: &mut fmt::Formatter<'_>,
    driver: &str,
    path: &Path,
    fallback: &FallbackCause,
    unreadable: &str,
) -> fmt::Result {
    write!(
        f,
        "no `{driver}` driver library is available: the statement named neither \
         {BUCKETFS_CONNECTION_PROPERTY} nor {DRIVER_PATH_PROPERTY}, so the reserved \
         CONNECTION `{DEFAULT_BUCKETFS_CONNECTION}` was the route to a self-installed one. "
    )?;
    match fallback {
        FallbackCause::ConnectionUnresolved(cause) => {
            write!(f, "{UNRESOLVED_CONNECTION_SENTENCE} ({cause}). ")
        }
        FallbackCause::RegistryUnreachable { url, cause } => write!(
            f,
            "It resolved, but the driver registry index at {url} reached no host, so no \
             version could be resolved to install: {cause}. "
        ),
    }?;
    write!(
        f,
        "The default driver location {} holds no library this adapter can load either \
         ({unreadable}), so no route is left. Pre-place the `{driver}` library there, name \
         one with {DRIVER_PATH_PROPERTY}, or name a BucketFS write credential with \
         {BUCKETFS_CONNECTION_PROPERTY}.",
        path.display()
    )
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
) -> Result<(AdapterNotes, Box<dyn MetadataSource>), SchemaError> {
    let properties = read_properties(request);
    let driver = resolve_driver(&properties)?;
    let connection_name = required_property(&properties, CONNECTION_PROPERTY)?.to_string();
    let source = driver_source(&properties);

    let source_connection_object = resolve_connection(ctx, &connection_name)?;
    check_connection_scheme(&source_connection_object.address, driver)?;
    let qualifier = parse_qualifier(&source_connection_object.address, driver)?;

    let library = open_driver_library(ctx, Path::new(BUCKETFS_MOUNT), &source, driver)?;

    let source_connection = adbc_vs_scan::connection::SourceConnection {
        uri: source_connection_object.address,
        username: source_connection_object.user,
        password: source_connection_object.password,
    };
    let adbc_connection = adbc_vs_scan::connection::open(driver, &library.path, &source_connection)
        .map_err(SchemaError::ScanConnection)?;
    let metadata_source =
        metadata::source_for(driver, adbc_connection, &qualifier).map_err(SchemaError::Metadata)?;

    let notes = AdapterNotes {
        driver: driver.as_str().to_string(),
        driver_version: library.version,
        driver_path: library.path,
        connection: connection_name,
        database: metadata_source.namespace(),
        omitted: vec![],
    };
    Ok((notes, metadata_source))
}

/// Where this statement's driver library comes from.
///
/// The two routes are read off the properties once and acted on later, so a
/// statement that cannot provision is refused before any `CONNECTION` is
/// resolved, and so nothing downstream has to re-derive which route it is on
/// from a pair of optional properties.
#[derive(Debug)]
enum DriverSource {
    /// A library the operator placed themselves, named by `DRIVER_PATH`.
    Supplied(String),
    /// The `BUCKETFS_CONNECTION` self-install writes the fetched driver
    /// through.
    SelfInstall(String),
    /// Neither property is set: self-install writes through
    /// [`DEFAULT_BUCKETFS_CONNECTION`], the reserved connection.
    DefaultBucketFs,
}

/// The driver library this virtual schema loads, in the spellings its notes
/// record.
///
/// The version is what separates the two routes: resolution selected one for a
/// provisioned driver, and there is none to report for a supplied one.
#[derive(Debug)]
struct DriverLibrary {
    path: String,
    version: Option<String>,
}

impl DriverLibrary {
    fn provisioned(installed: InstalledDriver) -> Self {
        Self {
            path: installed.path.to_string_lossy().into_owned(),
            version: Some(installed.version.to_string()),
        }
    }

    fn supplied(path: PathBuf) -> Self {
        Self {
            path: path.to_string_lossy().into_owned(),
            version: None,
        }
    }
}

/// Reads which of the three driver routes the statement asked for.
///
/// `DRIVER_PATH` decides the first split: an operator who names a library has
/// supplied it, and `BUCKETFS_CONNECTION` — whose only purpose is the write
/// credential self-install needs — is then not a property the statement has
/// to carry. Absent that, an explicit `BUCKETFS_CONNECTION` is used exactly as
/// named; without either property, self-install still has a route, through the
/// reserved connection. `DRIVER_PATH` no longer decides the route alone.
///
/// Stays pure — no `ctx` parameter — because it runs before any `CONNECTION`
/// is resolved, and that ordering is what keeps an unprovisionable statement
/// refusable without reaching outside the process.
fn driver_source(properties: &serde_json::Map<String, Value>) -> DriverSource {
    if let Some(path) = optional_property(properties, DRIVER_PATH_PROPERTY) {
        return DriverSource::Supplied(path.to_string());
    }
    match optional_property(properties, BUCKETFS_CONNECTION_PROPERTY) {
        Some(name) => DriverSource::SelfInstall(name.to_string()),
        None => DriverSource::DefaultBucketFs,
    }
}

/// Makes the driver library readable, by whichever route the statement chose,
/// under the BucketFS mount it was handed.
///
/// The two credential routes share one `ensure_driver` call and one
/// [`self_install_fallback`] call on its failure, so the route rule lives in a
/// function rather than in an arm: an edit that let an explicit
/// `BUCKETFS_CONNECTION` fall back has to change the function one test owns.
/// Only resolving the credential still branches by route, because a failed
/// resolution is a `UdfError` and never reaches the provisioning taxonomy the
/// rule reads.
fn open_driver_library(
    ctx: &mut dyn UdfContext,
    mount: &Path,
    source: &DriverSource,
    driver: Driver,
) -> Result<DriverLibrary, SchemaError> {
    let bucketfs_connection_name = match source {
        DriverSource::Supplied(path) => {
            return provisioning::supplied_driver(path, driver)
                .map(DriverLibrary::supplied)
                .map_err(SchemaError::Provisioning)
        }
        DriverSource::SelfInstall(name) => name.as_str(),
        DriverSource::DefaultBucketFs => DEFAULT_BUCKETFS_CONNECTION,
    };

    let credential = match resolve_connection(ctx, bucketfs_connection_name) {
        Ok(credential) => credential,
        Err(unresolved) if matches!(source, DriverSource::DefaultBucketFs) => {
            return default_location_library(
                mount,
                driver,
                FallbackCause::ConnectionUnresolved(unresolved.to_string()),
            )
        }
        Err(unresolved) => return Err(unresolved),
    };

    self_install_driver(mount, &credential, driver).or_else(|error| {
        match self_install_fallback(source, &error) {
            Some(cause) => default_location_library(mount, driver, cause),
            None => Err(SchemaError::Provisioning(error)),
        }
    })
}

/// Whether an `ensure_driver` failure leaves the statement a last resort, and
/// why.
///
/// The rule this owns is one sentence: only the reserved-connection route has a
/// last resort, and only an index request that reached no host reaches it.
/// Every other pairing is `None`, including every failure of a registry that
/// answered something — a status, a truncated body, a package host that went
/// unanswered — because none of those says anything about the cluster's egress.
///
/// Both credential routes call it, which is what makes `source` a value rather
/// than a constant: `SelfInstall` reaches `None` through the same call
/// `DefaultBucketFs` reaches `Some` through, so the explicit route's refusal to
/// fall back is a live answer rather than an arm nothing exercises.
fn self_install_fallback(
    source: &DriverSource,
    error: &ProvisioningError,
) -> Option<FallbackCause> {
    if !matches!(source, DriverSource::DefaultBucketFs) {
        return None;
    }
    let (url, cause) = error.registry_unreachable()?;
    Some(FallbackCause::RegistryUnreachable {
        url: url.to_owned(),
        cause: cause.to_owned(),
    })
}

/// The last resort both triggers reach: the driver library an operator
/// pre-placed at the default location, or the refusal that no route is left.
///
/// It is reached only after self-install could not run at all, never before an
/// install would have been attempted. Checking it first would restore the
/// defect the versioned install path exists to remove, where an unversioned
/// path answers for whichever version arrived first.
///
/// The refusal carries [`provisioning::readable_library`]'s own cause rather
/// than discarding it, which is the whole reason that function reports one:
/// absence and a file of the wrong kind or the wrong mode bits are different
/// mistakes with different corrections, and only the cause tells them apart.
fn default_location_library(
    mount: &Path,
    driver: Driver,
    cause: FallbackCause,
) -> Result<DriverLibrary, SchemaError> {
    let path = provisioning::default_driver_path(mount, driver);
    match provisioning::readable_library(&path) {
        Ok(()) => Ok(DriverLibrary::supplied(path)),
        Err(unreadable) => Err(SchemaError::NoDriverRoute {
            driver: driver.as_str(),
            path,
            fallback: cause,
            unreadable,
        }),
    }
}

/// Provisions the driver from the registry into the bucket `credential`
/// addresses, under `mount`.
fn self_install_driver(
    mount: &Path,
    credential: &ConnectionObject,
    driver: Driver,
) -> Result<DriverLibrary, ProvisioningError> {
    let bucket = Bucket::new(
        &credential.address,
        mount,
        Credentials::new(&credential.user, &credential.password),
    );
    provisioning::ensure_driver(&Registry::public(driver), &bucket).map(DriverLibrary::provisioned)
}

/// Everything the response needs from outside this process: [`open`]'s
/// connection and notes, plus the source's own tables discovered over it.
///
/// All three handlers answer through here, so all three check the source
/// before reading it. `createVirtualSchema` is not the only path that
/// resolves the `CONNECTION`: `setProperties` can point an existing virtual
/// schema at a different server, and `refresh` re-discovers against whatever
/// its `CONNECTION` resolves to now, so a schema created against a supported
/// server can end up reading one that supports nothing its dialect renders.
fn discover(
    ctx: &mut dyn UdfContext,
    request: &Value,
) -> Result<(AdapterNotes, Vec<DiscoveredTable>), SchemaError> {
    let (notes, mut metadata_source) = open(ctx, request)?;
    let tables = discover_supported_source(metadata_source.as_mut())?;
    Ok((notes, tables))
}

/// Discovers the tables of `source`'s own namespace, unless `source` reports
/// that it does not meet the precondition its own dialect states.
///
/// The check runs first and returns rather than discovering anything, so a
/// source this release cannot render against is refused at the moment an
/// operator can act on it rather than after a table list has been built from
/// it. Composing the two steps in one function is what leaves no path that
/// discovers tables from an unchecked source.
fn discover_supported_source(
    source: &mut dyn MetadataSource,
) -> Result<Vec<DiscoveredTable>, SchemaError> {
    source.check_source().map_err(SchemaError::Metadata)?;
    source.discover_tables().map_err(SchemaError::Metadata)
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
    let (table_values, omitted) = exposed_tables(tables);
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

/// A property the statement may carry, absent when it does not.
///
/// A property set to the empty string reads as absent: Exasol carries no way to
/// distinguish "set to nothing" from "not set", so treating the two alike is
/// what keeps an optional property optional and a required one named as
/// missing.
fn optional_property<'a>(
    properties: &'a serde_json::Map<String, Value>,
    name: &str,
) -> Option<&'a str> {
    properties
        .get(name)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
}

fn required_property<'a>(
    properties: &'a serde_json::Map<String, Value>,
    name: &'static str,
) -> Result<&'a str, SchemaError> {
    optional_property(properties, name).ok_or(SchemaError::MissingProperty(name))
}

/// The technology this virtual schema's `DRIVER` property names.
///
/// Every later decision follows from it — which package to provision, which
/// library to load, which dialect renders its pushdowns — so it is resolved to
/// an identity once, here, rather than re-read as a string at each of them.
fn resolve_driver(properties: &serde_json::Map<String, Value>) -> Result<Driver, SchemaError> {
    Driver::parse(required_property(properties, "DRIVER")?).map_err(SchemaError::UnsupportedDriver)
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

/// Refuses a `CONNECTION` address that is not addressed at the driver the
/// statement named.
///
/// The address is handed to the driver as its connection target unchanged, so
/// without this the mismatch is reported from inside the wrong driver, as a
/// source connection failure rather than as the property mismatch an operator
/// can act on. The scheme compared against is the driver's own canonical
/// name — the one spelling `DRIVER`, the registry, and the source's own URI
/// all use for a technology — so this stays a reading of the identity rather
/// than a fourth per-driver table beside the three that are keyed on it.
fn check_connection_scheme(address: &str, driver: Driver) -> Result<(), SchemaError> {
    let Some((scheme, _)) = address.split_once("://") else {
        return Err(SchemaError::ConnectionAddressMissingScheme {
            driver: driver.as_str(),
            address: address_without_userinfo(address),
        });
    };
    if scheme.eq_ignore_ascii_case(driver.as_str()) {
        return Ok(());
    }
    Err(SchemaError::ConnectionSchemeMismatch {
        driver: driver.as_str(),
        scheme: scheme.to_string(),
    })
}

/// The source qualifier the `CONNECTION` address's path component spells, one
/// path element per segment.
///
/// The address must already be a full driver URI, so the qualifier can only
/// come from here and never from a separate property. How many segments it
/// takes is read off the driver's own declaration rather than decided here, and
/// both too few and too many fail: taking everything after the first `/` as one
/// name was safe only while every technology qualified a table with one name,
/// and under a deeper qualifier that leniency becomes a way to address a
/// different object than the statement asked for.
fn parse_qualifier(address: &str, driver: Driver) -> Result<SourceQualifier, SchemaError> {
    let carried = address_path_elements(address);
    let needed = driver.qualifier_segment_count();
    if carried.len() != needed {
        return Err(SchemaError::ConnectionAddressQualifierArity {
            driver: driver.as_str(),
            address: address_without_userinfo(address),
            needed,
            carried: carried.len(),
        });
    }
    SourceQualifier::of(carried).map_err(|cause| SchemaError::ConnectionAddressQualifierSegment {
        driver: driver.as_str(),
        address: address_without_userinfo(address),
        cause,
    })
}

/// The address with its authority's userinfo component removed, for the errors
/// that name the address back to the querying session.
///
/// A user and password written into the address are not the credential —
/// Exasol resolves that beside the address — but an operator can still write
/// one there, and `dispatch` hands a `SchemaError`'s text to the session
/// unredacted, so an address stored verbatim would return that secret in clear
/// text. The scan crate strips the same component from the address it opens
/// with, for the same reason.
///
/// The authority runs from the scheme separator to the first `/`, `?`, or `#`,
/// and its userinfo ends at the last `@` inside it, which is how the driver's
/// own URL reader delimits both. An address with no scheme separator is
/// stripped from its first byte, because the errors that name one are reached
/// precisely when the scheme is absent.
fn address_without_userinfo(address: &str) -> String {
    const SCHEME_SEPARATOR: &str = "://";

    let authority_start = address
        .find(SCHEME_SEPARATOR)
        .map_or(0, |index| index + SCHEME_SEPARATOR.len());
    let authority_end = address[authority_start..]
        .find(['/', '?', '#'])
        .map_or(address.len(), |offset| authority_start + offset);

    match address[authority_start..authority_end].rsplit_once('@') {
        None => address.to_string(),
        Some((_, host)) => format!(
            "{}{host}{}",
            &address[..authority_start],
            &address[authority_end..]
        ),
    }
}

/// The elements of a driver URI's path component, in order, with the query
/// string and the fragment stripped.
///
/// An address with no path at all and one whose path is empty both yield no
/// element, so a missing qualifier is counted rather than treated as a shape of
/// its own. An empty element is kept rather than dropped, because dropping one
/// would let `//` inside a path pass a count it does not really spell.
fn address_path_elements(address: &str) -> Vec<String> {
    let after_scheme = address.split_once("://").map_or(address, |(_, rest)| rest);
    let path = after_scheme
        .split_once('/')
        .map_or("", |(_, path)| path)
        .split(['?', '#'])
        .next()
        .unwrap_or("");
    match path {
        "" => Vec::new(),
        path => path.split('/').map(str::to_owned).collect(),
    }
}

/// Splits the discovered tables into the ones the virtual schema exposes and
/// the full list of omissions.
///
/// The source already decided which columns map and why the rest did not.
/// What is left is the one rule that belongs to the schema rather than to any
/// technology: a table no column of which maps is not exposed at all, and is
/// recorded as an omission of its own beside its columns'.
fn exposed_tables(tables: Vec<DiscoveredTable>) -> (Vec<Value>, Vec<Omission>) {
    let mut table_values = Vec::new();
    let mut omitted = Vec::new();

    for table in tables {
        omitted.extend(table.omissions);
        if table.columns.is_empty() {
            omitted.push(Omission {
                table: table.name,
                column: None,
                source_type: None,
                reason: "no column of this table maps to a supported Exasol type".to_string(),
            });
            continue;
        }

        let columns: Vec<Value> = table
            .columns
            .iter()
            .map(|column| column_json(&column.name, column.mapped))
            .collect();
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
