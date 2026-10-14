use semver::Version;
use serde_json::json;
use tempfile::TempDir;

use super::*;
use crate::metadata::{DiscoveredColumn, DiscoveredTable, MetadataError};
use crate::type_mapping::MappedType;

/// A version no constant in this crate states, so a note reporting it can only
/// have got it from the resolution that selected it.
const RESOLVED_VERSION: &str = "0.6.1";

/// A table of already-mapped columns, in the shape a source hands back.
fn table(name: &str, columns: &[&str]) -> DiscoveredTable {
    DiscoveredTable {
        name: name.to_string(),
        columns: columns
            .iter()
            .map(|column| DiscoveredColumn {
                name: (*column).to_string(),
                mapped: MappedColumn {
                    exasol_type: MappedType::Decimal {
                        precision: 10,
                        scale: 0,
                    },
                    is_nullable: true,
                },
            })
            .collect(),
        omissions: vec![],
    }
}

/// A table whose named columns mapped and whose named omissions did not, the
/// shape a source hands back for a partly mappable table.
fn table_omitting(name: &str, columns: &[&str], omitted: &[(&str, &str)]) -> DiscoveredTable {
    DiscoveredTable {
        omissions: omitted
            .iter()
            .map(|(column, source_type)| Omission {
                table: name.to_string(),
                column: Some((*column).to_string()),
                source_type: Some((*source_type).to_string()),
                reason: format!("{source_type} does not map"),
            })
            .collect(),
        ..table(name, columns)
    }
}

/// A [`MetadataSource`] that answers with tables it was handed, so the
/// response assembly is driven in the adapter's own vocabulary without a
/// source connection anywhere — which is what the seam exists for. It records
/// whether it was asked for them, which is how a test tells a source refused
/// before discovery from one refused after it.
struct FakeMetadataSource {
    tables: Vec<DiscoveredTable>,
    refusal: Option<String>,
    tables_discovered: bool,
}

impl FakeMetadataSource {
    fn holding(tables: Vec<DiscoveredTable>) -> Self {
        Self {
            tables,
            refusal: None,
            tables_discovered: false,
        }
    }

    fn refusing(reason: &str) -> Self {
        Self {
            refusal: Some(reason.to_string()),
            ..Self::holding(vec![table("orders", &["id"])])
        }
    }
}

impl MetadataSource for FakeMetadataSource {
    fn namespace(&self) -> SourceQualifier {
        SourceQualifier::single("shop")
    }

    fn check_source(&mut self) -> Result<(), MetadataError> {
        match &self.refusal {
            Some(reason) => Err(MetadataError::UnsupportedSource(reason.clone())),
            None => Ok(()),
        }
    }

    fn discover_tables(&mut self) -> Result<Vec<DiscoveredTable>, MetadataError> {
        self.tables_discovered = true;
        Ok(std::mem::take(&mut self.tables))
    }
}

fn discovered(tables: Vec<DiscoveredTable>) -> Vec<DiscoveredTable> {
    FakeMetadataSource::holding(tables)
        .discover_tables()
        .expect("the fake source answers with the tables it was handed")
}

/// The notes [`discover`] hands on: everything but the omission list, which
/// only mapping the columns can produce.
fn discovered_notes() -> AdapterNotes {
    AdapterNotes {
        driver: Driver::MySql.as_str().to_string(),
        driver_version: Some(RESOLVED_VERSION.to_string()),
        driver_path: "/buckets/bfsdefault/default/adbc-drivers/libadbc_driver_mysql.so".to_string(),
        connection: "MY_MYSQL_CONNECTION".to_string(),
        database: SourceQualifier::single("shop"),
        omitted: vec![],
    }
}

fn recorded_notes(response: &Value) -> AdapterNotes {
    let rendered = response["schemaMetadata"]["adapterNotes"]
        .as_str()
        .expect("adapterNotes is a JSON string");
    AdapterNotes::parse(rendered).expect("the recorded notes parse back")
}

#[test]
fn the_response_type_is_the_request_type_it_answers() {
    for response_type in ["createVirtualSchema", "refresh", "setProperties"] {
        let response = schema_response(response_type, discovered_notes(), vec![]);
        assert_eq!(response["type"], response_type);
    }
}

#[test]
fn only_a_table_with_a_mapped_column_reaches_the_schema_metadata() {
    let tables = discovered(vec![
        table_omitting("orders", &["id"], &[("payload", "JSON")]),
        table_omitting("logs", &[], &[("payload", "JSON")]),
    ]);

    let response = schema_response("createVirtualSchema", discovered_notes(), tables);

    let exposed = response["schemaMetadata"]["tables"]
        .as_array()
        .expect("schemaMetadata.tables is an array");
    assert_eq!(exposed.len(), 1, "only `orders` maps: {exposed:?}");
    assert_eq!(exposed[0]["name"], "orders");
    let columns = exposed[0]["columns"]
        .as_array()
        .expect("a table declares its columns");
    assert_eq!(columns.len(), 1, "only `id` maps: {columns:?}");
    assert_eq!(columns[0]["name"], "id");
}

#[test]
fn the_recorded_notes_carry_the_state_pushdown_reads_back() {
    let tables = discovered(vec![table("orders", &["id"])]);

    let notes = recorded_notes(&schema_response(
        "createVirtualSchema",
        discovered_notes(),
        tables,
    ));

    assert_eq!(notes.driver, "mysql");
    assert_eq!(notes.driver_version.as_deref(), Some(RESOLVED_VERSION));
    assert_eq!(
        notes.driver_path,
        "/buckets/bfsdefault/default/adbc-drivers/libadbc_driver_mysql.so"
    );
    assert_eq!(notes.connection, "MY_MYSQL_CONNECTION");
    assert_eq!(notes.database.segments(), ["shop"]);
    assert!(notes.omitted.is_empty(), "nothing was omitted: {notes:?}");
}

#[test]
fn the_recorded_notes_carry_both_kinds_of_omission() {
    let tables = discovered(vec![
        table_omitting("orders", &["id"], &[("payload", "JSON")]),
        table_omitting("logs", &[], &[("payload", "JSON")]),
    ]);

    let notes = recorded_notes(&schema_response(
        "createVirtualSchema",
        discovered_notes(),
        tables,
    ));

    let column_omission = notes
        .omitted
        .iter()
        .find(|omission| {
            omission.column.as_deref() == Some("payload") && omission.table == "orders"
        })
        .expect("the unmappable column of a kept table is recorded");
    assert_eq!(column_omission.source_type.as_deref(), Some("JSON"));
    assert!(!column_omission.reason.is_empty());

    let table_omission = notes
        .omitted
        .iter()
        .find(|omission| omission.table == "logs" && omission.column.is_none())
        .expect("a table left with no mappable column is recorded");
    assert!(!table_omission.reason.is_empty());
}

#[test]
fn the_response_carries_no_credential() {
    let tables = discovered(vec![table("orders", &["id"])]);

    let response = schema_response("createVirtualSchema", discovered_notes(), tables);

    let rendered = response.to_string().to_lowercase();
    assert!(
        !rendered.contains("password"),
        "the response must carry no credential: {response}"
    );
    assert!(
        rendered.contains("my_mysql_connection"),
        "the response must reference the CONNECTION by name: {response}"
    );
}

#[test]
fn refresh_echoes_the_tables_the_request_asked_for() {
    let tables = discovered(vec![table("orders", &["id"])]);
    let request = json!({"type": "refresh", "requestedTables": ["orders"]});

    let response = echo_requested_tables(
        &request,
        schema_response("refresh", discovered_notes(), tables),
    );

    assert_eq!(response["requestedTables"], json!(["orders"]));
}

#[test]
fn a_refresh_naming_no_table_gets_no_requested_tables_back() {
    let request = json!({"type": "refresh"});

    let response = echo_requested_tables(
        &request,
        schema_response("refresh", discovered_notes(), vec![]),
    );

    assert!(
        response.get("requestedTables").is_none(),
        "a request naming no table must not grow the field: {response}"
    );
}

#[test]
fn empty_namespace_yields_empty_table_list() {
    let (tables, omissions) = exposed_tables(vec![]);
    assert!(tables.is_empty());
    assert!(omissions.is_empty());
}

#[test]
fn a_mapped_column_carries_its_name_type_and_nullability() {
    let (table_values, omissions) = exposed_tables(vec![table("orders", &["id"])]);

    assert!(omissions.is_empty());
    assert_eq!(table_values.len(), 1);
    assert_eq!(table_values[0]["name"], "orders");
    let column = &table_values[0]["columns"][0];
    assert_eq!(column["name"], "id");
    assert_eq!(column["isNullable"], true);
    assert_eq!(column["dataType"]["type"], "DECIMAL");
}

/// The source decided which column did not map and why; what this asserts is
/// that neither the kept column nor the recorded omission is lost on the way
/// into the response.
#[test]
fn an_omission_the_source_recorded_is_carried_through() {
    let (table_values, omissions) = exposed_tables(vec![table_omitting(
        "orders",
        &["id"],
        &[("payload", "JSON")],
    )]);

    let columns = table_values[0]["columns"].as_array().unwrap();
    assert_eq!(columns.len(), 1, "only the mapped column must be exposed");
    assert_eq!(columns[0]["name"], "id");

    assert_eq!(omissions.len(), 1);
    assert_eq!(omissions[0].table, "orders");
    assert_eq!(omissions[0].column.as_deref(), Some("payload"));
    assert_eq!(omissions[0].source_type.as_deref(), Some("JSON"));
}

/// The one omission rule that belongs to the schema rather than to any
/// technology: a table nothing of which mapped is not exposed at all, and
/// says so beside its columns' own omissions.
#[test]
fn a_table_with_no_mapped_column_is_omitted_entirely() {
    let (table_values, omissions) =
        exposed_tables(vec![table_omitting("logs", &[], &[("payload", "JSON")])]);

    assert!(table_values.is_empty(), "the table itself must not appear");
    assert!(
        omissions
            .iter()
            .any(|omission| omission.table == "logs" && omission.column.is_none()),
        "the table must be recorded as omitted: {omissions:?}"
    );
    assert!(
        omissions
            .iter()
            .any(|omission| omission.column.as_deref() == Some("payload")),
        "the column's own omission must survive beside the table's: {omissions:?}"
    );
}

#[test]
fn the_qualifier_is_the_path_component_of_the_connection_address() {
    assert_eq!(
        parse_qualifier("mysql://172.28.5.10:3306/shop", Driver::MySql)
            .unwrap()
            .segments(),
        ["shop"]
    );
}

#[test]
fn the_qualifier_path_stops_at_a_query_string() {
    assert_eq!(
        parse_qualifier("mysql://host:3306/shop?useSSL=false", Driver::MySql)
            .unwrap()
            .segments(),
        ["shop"]
    );
}

/// One path element per qualifier segment, so an address whose path is deeper
/// than one element is read as the segments it spells rather than as one name
/// containing a separator. Asserted on the split alone: how many elements a
/// driver needs is its own declaration's answer, never this function's, and
/// the refusal is exercised by the test below.
#[test]
fn a_deeper_address_path_is_read_as_the_elements_it_spells() {
    assert_eq!(
        address_path_elements("trino://172.28.5.12:8443/memory/default"),
        ["memory", "default"]
    );
    assert_eq!(
        address_path_elements("trino://172.28.5.12:8443/memory/default?x=1"),
        ["memory", "default"]
    );
    assert!(address_path_elements("mysql://172.28.5.10:3306").is_empty());
    assert!(address_path_elements("mysql://172.28.5.10:3306/").is_empty());
}

/// Release 0.2 took everything after the first `/` as one database name, so a
/// `postgresql://host/db/extra` address silently named the database `db/extra`.
/// Once a qualifier can be deeper than one segment that leniency becomes a way
/// to address a different object than the statement asked for, so the element
/// count is checked against the driver's own declaration and both directions
/// fail — an operator cannot tell which end to correct from one count alone.
#[test]
fn an_address_not_spelling_its_drivers_qualifier_is_refused_naming_the_driver_and_both_counts() {
    for (driver, address, carried) in [
        (Driver::MySql, "mysql://172.28.5.10:3306", 0),
        (Driver::MySql, "mysql://172.28.5.10:3306/", 0),
        (Driver::MySql, "mysql://172.28.5.10:3306/shop/extra", 2),
        (Driver::Postgres, "postgresql://172.28.5.11:5432", 0),
        (
            Driver::Postgres,
            "postgresql://172.28.5.11:5432/shop/extra",
            2,
        ),
        (Driver::Trino, "trino://172.28.5.12:8443/memory", 1),
        (
            Driver::Trino,
            "trino://172.28.5.12:8443/memory/adbc_vs/extra",
            3,
        ),
    ] {
        let needed = driver.qualifier_segment_count();
        let Err(error) = parse_qualifier(address, driver) else {
            panic!(
                "`{address}` spells {carried} qualifier segments, not the {needed} `{}` needs",
                driver.as_str()
            );
        };

        let message = error.to_string();
        assert!(
            matches!(error, SchemaError::ConnectionAddressQualifierArity { .. }),
            "the refusal must be the qualifier-arity one: {message}"
        );
        assert!(
            message.contains(driver.as_str()),
            "the error must name the driver whose qualifier the address did not spell: {message}"
        );
        assert!(
            message.contains(&format!("spells {carried} qualifier segments")),
            "the error must state how many segments the address carried: {message}"
        );
        assert!(
            message.contains(&format!("with {needed}")),
            "the error must state how many segments the driver needs: {message}"
        );
    }
}

/// An address can spell the right number of path elements and still name
/// nothing with one of them. This is reachable only for a driver qualifying
/// with more than one segment: a single non-empty element cannot be blank, so
/// for MySQL and PostgreSQL a blank path fails the arity check first.
#[test]
fn an_address_spelling_a_blank_qualifier_segment_is_refused_naming_its_position() {
    for (address, position) in [
        ("trino://172.28.5.12:8443/memory/", 2),
        ("trino://172.28.5.12:8443//adbc_vs", 1),
    ] {
        let Err(error) = parse_qualifier(address, Driver::Trino) else {
            panic!("`{address}` names nothing with one of its path elements");
        };

        let message = error.to_string();
        assert!(
            matches!(error, SchemaError::ConnectionAddressQualifierSegment { .. }),
            "the refusal must be the qualifier-segment one: {message}"
        );
        assert!(
            message.contains(&position.to_string()),
            "the error must state which segment is empty: {message}"
        );
        assert!(
            message.contains(Driver::Trino.as_str()),
            "the error must name the driver whose qualifier could not be built: {message}"
        );
    }
}

#[test]
fn an_address_carrying_the_drivers_own_scheme_is_accepted() {
    check_connection_scheme("mysql://172.28.5.10:3306/shop", Driver::MySql)
        .expect("an address whose scheme names the resolved driver must be accepted unchanged");
}

/// The scheme is a URI component, and a `CONNECTION` object is operator-typed
/// text, so the comparison ignores case exactly as `DRIVER` does.
#[test]
fn an_address_carrying_the_scheme_in_another_case_is_accepted() {
    check_connection_scheme("MySQL://172.28.5.10:3306/shop", Driver::MySql)
        .expect("a scheme differing only in case names the same driver");
}

#[test]
fn an_address_carrying_another_drivers_scheme_is_refused_naming_both() {
    let error = check_connection_scheme("postgresql://172.28.5.11:5432/shop", Driver::MySql)
        .expect_err("an address addressed at another driver must be refused");

    let message = error.to_string();
    assert!(
        matches!(error, SchemaError::ConnectionSchemeMismatch { .. }),
        "the refusal must be the scheme one: {message}"
    );
    assert!(
        message.contains("postgresql") && message.contains("mysql"),
        "the error must name the scheme the address carries and the driver named: {message}"
    );
}

#[test]
fn an_address_carrying_no_scheme_is_refused_naming_the_driver() {
    let error = check_connection_scheme("172.28.5.10:3306/shop", Driver::MySql)
        .expect_err("an address that is not a driver URI must be refused");

    let message = error.to_string();
    assert!(
        matches!(error, SchemaError::ConnectionAddressMissingScheme { .. }),
        "the refusal must name the address as carrying no scheme: {message}"
    );
    assert!(
        message.contains("mysql") && message.contains("172.28.5.10:3306/shop"),
        "the error must name the driver required and the address received: {message}"
    );
}

/// Which database names a source can be queried for is that source's own
/// answer, so the schema resolves the name and passes it on rather than
/// judging it: MySQL refuses a quote or a backslash because its catalog
/// queries interpolate the name, and a PostgreSQL database named `it's` — a
/// name PostgreSQL itself accepts — must reach the source that has no such
/// literal to escape.
#[test]
fn a_database_name_carrying_a_quote_or_a_backslash_is_resolved_rather_than_judged() {
    for address in [
        "postgresql://172.28.5.11:5432/it's",
        "postgresql://172.28.5.11:5432/back\\slash",
    ] {
        let qualifier = parse_qualifier(address, Driver::Postgres)
            .unwrap_or_else(|error| panic!("`{address}` names a database: {error}"));

        assert_eq!(
            address,
            format!("postgresql://172.28.5.11:5432/{}", qualifier.first()),
            "the resolved name must be the address's path component, character for character"
        );
    }
}

#[test]
fn a_request_carrying_only_current_properties_keeps_them() {
    let request = json!({
        "type": "createVirtualSchema",
        "schemaMetadataInfo": {"properties": {"DRIVER": "mysql", "CONNECTION_NAME": "A"}},
    });

    let properties = read_properties(&request);

    assert_eq!(required_property(&properties, "DRIVER").unwrap(), "mysql");
    assert_eq!(
        required_property(&properties, CONNECTION_PROPERTY).unwrap(),
        "A"
    );
}

#[test]
fn a_property_the_request_changes_wins_over_the_current_one() {
    let request = json!({
        "type": "setProperties",
        "schemaMetadataInfo": {"properties": {"DRIVER": "mysql", "CONNECTION_NAME": "A"}},
        "properties": {"CONNECTION_NAME": "B"},
    });

    let properties = read_properties(&request);

    assert_eq!(
        required_property(&properties, CONNECTION_PROPERTY).unwrap(),
        "B",
        "re-discovery must run against the changed CONNECTION_NAME, not the one it replaced"
    );
    assert_eq!(
        required_property(&properties, "DRIVER").unwrap(),
        "mysql",
        "a property the request leaves alone must survive the merge"
    );
}

#[test]
fn a_property_the_request_sets_to_null_is_removed() {
    let request = json!({
        "type": "setProperties",
        "schemaMetadataInfo": {"properties": {"DRIVER": "mysql", "CONNECTION_NAME": "A"}},
        "properties": {"CONNECTION_NAME": null},
    });

    let properties = read_properties(&request);

    let error = required_property(&properties, CONNECTION_PROPERTY)
        .expect_err("a property the request removed is no longer set");
    assert!(
        matches!(error, SchemaError::MissingProperty(name) if name == CONNECTION_PROPERTY),
        "removing a required property must be reported as a missing one: {error}"
    );
}

/// `vs-adapter/create-virtual-schema`'s precedence scenario, `refresh`/
/// `setProperties` clause: a request that nulls out its current
/// `BUCKETFS_CONNECTION` removes the property from what `driver_source` sees,
/// same as a `createVirtualSchema` statement that never named one.
#[test]
fn a_set_properties_request_nulling_the_bucketfs_connection_takes_the_default_route() {
    let request = json!({
        "type": "setProperties",
        "schemaMetadataInfo": {
            "properties": {
                "DRIVER": "mysql",
                "CONNECTION_NAME": "A",
                "BUCKETFS_CONNECTION": "B",
            }
        },
        "properties": {"BUCKETFS_CONNECTION": null},
    });

    let properties = read_properties(&request);
    let source = driver_source(&properties);

    assert!(
        matches!(&source, DriverSource::DefaultBucketFs),
        "nulling BUCKETFS_CONNECTION must take the default route: {source:?}"
    );
}

/// `vs-adapter/create-virtual-schema`, `driver-provisioning/driver-path-override`:
/// a statement naming a driver library supplies it, so the write credential
/// self-install would need is not a property it has to carry.
#[test]
fn a_statement_setting_driver_path_supplies_its_own_library() {
    let source = driver_source(&properties(json!({
        "DRIVER_PATH": "/buckets/bfsdefault/default/mine/libadbc_driver_postgresql.so",
    })));

    assert!(
        matches!(&source, DriverSource::Supplied(path)
            if path == "/buckets/bfsdefault/default/mine/libadbc_driver_postgresql.so"),
        "the path must be carried through exactly as given: {source:?}"
    );
}

/// A `setProperties` that adds `DRIVER_PATH` to a schema created with a
/// `BUCKETFS_CONNECTION` leaves both properties set, and the supplied library
/// still wins: `DRIVER_PATH` skips self-install entirely rather than being one
/// input to it.
#[test]
fn a_driver_path_supplies_the_library_even_beside_a_bucketfs_connection() {
    let source = driver_source(&properties(json!({
        "BUCKETFS_CONNECTION": "B",
        "DRIVER_PATH": "/buckets/bfsdefault/default/mine/libadbc_driver_mysql.so",
    })));

    assert!(
        matches!(&source, DriverSource::Supplied(_)),
        "a supplied library is used even when a write credential is also named: {source:?}"
    );
}

/// A `DRIVER_PATH` set to nothing is not a supplied library: an empty property
/// is how every other property here reads as absent, and reading it as a path
/// would refuse the statement for a file name nobody typed.
#[test]
fn a_statement_setting_an_empty_driver_path_self_installs() {
    let source = driver_source(&properties(json!({
        "DRIVER_PATH": "",
        "BUCKETFS_CONNECTION": "B",
    })));

    assert!(
        matches!(&source, DriverSource::SelfInstall(name) if name == "B"),
        "an empty DRIVER_PATH must read as absent: {source:?}"
    );
}

#[test]
fn a_statement_without_driver_path_self_installs_through_its_bucketfs_connection() {
    let source = driver_source(&properties(json!({"BUCKETFS_CONNECTION": "B"})));

    assert!(
        matches!(&source, DriverSource::SelfInstall(name) if name == "B"),
        "the BUCKETFS_CONNECTION named is the one the write goes through: {source:?}"
    );
}

/// `vs-adapter/create-virtual-schema`: a statement naming neither `DRIVER_PATH`
/// nor `BUCKETFS_CONNECTION` takes the reserved-connection route rather than
/// being refused before any connection is resolved. An empty
/// `BUCKETFS_CONNECTION` reads the same way `optional_property` reads every
/// other empty property — as absent — so it takes the same route rather than
/// being read as the precedence scenario's explicit level.
#[test]
fn a_statement_naming_no_bucketfs_connection_takes_the_default_route() {
    for properties_value in [
        json!({"CONNECTION_NAME": "A"}),
        json!({"CONNECTION_NAME": "A", "BUCKETFS_CONNECTION": ""}),
    ] {
        let source = driver_source(&properties(properties_value.clone()));

        assert!(
            matches!(&source, DriverSource::DefaultBucketFs),
            "an absent BUCKETFS_CONNECTION must take the default route: {source:?} \
             (properties: {properties_value})"
        );
    }
}

/// `driver-provisioning/driver-version-resolution`: under a version range the
/// driver version a virtual schema runs is the registry's answer at run time,
/// so the notes report the version resolution selected for *this* schema.
/// Driven per driver, because a compiled-in constant would report one
/// technology's version for the other's schema.
#[test]
fn a_provisioned_library_records_the_version_that_was_resolved_for_it() {
    for (driver, version) in [(Driver::MySql, "0.6.1"), (Driver::Postgres, "1.12.0")] {
        let installed = InstalledDriver {
            path: PathBuf::from(format!(
                "/buckets/bfsdefault/default/adbc-drivers/{}/{version}/{}",
                driver.as_str(),
                driver.package().library
            )),
            version: Version::parse(version).expect("the fixture version parses"),
        };

        let library = DriverLibrary::provisioned(installed);

        assert_eq!(
            library.version.as_deref(),
            Some(version),
            "`{}` must record the version resolved for it rather than another driver's",
            driver.as_str()
        );
        assert!(
            library.path.contains(&format!("/{version}/")),
            "the recorded path must be the versioned one provisioning installed: {}",
            library.path
        );
    }
}

/// `driver-provisioning/driver-path-override`: adbc-vs never learned the
/// version of a library the operator supplied, so it records none rather than
/// reporting a version this schema is not running.
#[test]
fn a_supplied_library_records_no_driver_version() {
    let library = DriverLibrary::supplied(PathBuf::from(
        "/buckets/bfsdefault/default/mine/libadbc_driver_mysql.so",
    ));

    assert_eq!(
        library.path,
        "/buckets/bfsdefault/default/mine/libadbc_driver_mysql.so"
    );
    assert_eq!(
        library.version, None,
        "a version adbc-vs did not resolve must not be reported as one it did"
    );
}

/// The index request's own URL and transport cause, as an unreachable registry
/// hands them to the last-resort route.
const UNREACHABLE_INDEX_URL: &str = "https://dbc-cdn.columnar.tech/index.yaml";
const UNREACHABLE_TRANSPORT_CAUSE: &str = "error trying to connect: dns error";

/// A resolution failure that names no connection of its own, so a test
/// asserting the message names `ADBC_VS_BUCKETFS` is asserting the message
/// rather than reading the cause back.
const UNRESOLVED_CAUSE: &str = "the session holds no access to it";

/// A mount holding a readable driver library at the default location, in the
/// shape an operator who followed the documentation leaves behind.
///
/// The mount is a temporary directory rather than the compiled-in one, which is
/// the whole reason `open_driver_library` takes it as an argument: a branch
/// chosen by whether `/buckets` holds a file is not assertable below the
/// end-to-end tier.
fn mount_holding_the_default_library(driver: Driver) -> TempDir {
    let mount = TempDir::new().expect("a temporary mount must be creatable");
    let library = provisioning::default_driver_path(mount.path(), driver);
    std::fs::create_dir_all(
        library
            .parent()
            .expect("the default location names a directory"),
    )
    .expect("the default location's directories must be creatable");
    std::fs::write(&library, b"\x7fELF").expect("the pre-placed library must be writable");
    mount
}

/// An index request that reached no host at all, the one failure the reserved
/// connection route has a last resort for.
fn an_index_that_reached_no_host() -> ProvisioningError {
    ProvisioningError::RegistryUnreachable {
        url: UNREACHABLE_INDEX_URL.to_string(),
        driver: Driver::MySql.as_str().to_string(),
        cause: UNREACHABLE_TRANSPORT_CAUSE.to_string(),
    }
}

/// `driver-provisioning/driver-path-override`: a readable library at the
/// default location answers when the reserved connection did not resolve, and
/// records no driver version because nothing resolved one.
#[test]
fn the_default_location_answers_a_connection_that_did_not_resolve() {
    let mount = mount_holding_the_default_library(Driver::MySql);

    let library = default_location_library(
        mount.path(),
        Driver::MySql,
        FallbackCause::ConnectionUnresolved(UNRESOLVED_CAUSE.to_string()),
    )
    .expect("a readable library at the default location answers a credential that never was");

    assert_eq!(
        Path::new(&library.path),
        provisioning::default_driver_path(mount.path(), Driver::MySql),
        "the library must be loaded from exactly the default location"
    );
    assert_eq!(
        library.version, None,
        "nothing resolved a version on this route, so none may be reported: {library:?}"
    );
}

/// The same location answers the other trigger, and answers it identically: the
/// two causes differ in what failed, never in what the route then does.
#[test]
fn the_default_location_answers_a_registry_that_answered_nothing() {
    let mount = mount_holding_the_default_library(Driver::MySql);

    let library = default_location_library(
        mount.path(),
        Driver::MySql,
        FallbackCause::RegistryUnreachable {
            url: UNREACHABLE_INDEX_URL.to_string(),
            cause: UNREACHABLE_TRANSPORT_CAUSE.to_string(),
        },
    )
    .expect("a readable library at the default location answers an unreachable registry");

    assert_eq!(
        Path::new(&library.path),
        provisioning::default_driver_path(mount.path(), Driver::MySql),
        "the library must be loaded from exactly the default location"
    );
    assert_eq!(
        library.version, None,
        "an index request that reached no host resolved no version: {library:?}"
    );
}

/// `driver-provisioning/driver-path-override`: with no route left, the refusal
/// is the whole diagnosis, so it names the location it looked at, the driver it
/// looked for, the reserved connection and why that connection failed, and both
/// properties that state a route explicitly.
#[test]
fn no_driver_route_names_the_location_the_connection_and_both_properties() {
    let mount = TempDir::new().expect("a temporary mount must be creatable");

    let error = default_location_library(
        mount.path(),
        Driver::MySql,
        FallbackCause::ConnectionUnresolved(UNRESOLVED_CAUSE.to_string()),
    )
    .expect_err("an empty default location leaves the statement no route at all");

    let message = error.to_string();
    let location = provisioning::default_driver_path(mount.path(), Driver::MySql);
    for named in [
        location.to_string_lossy().as_ref(),
        Driver::MySql.as_str(),
        DEFAULT_BUCKETFS_CONNECTION,
        UNRESOLVED_CAUSE,
        BUCKETFS_CONNECTION_PROPERTY,
        DRIVER_PATH_PROPERTY,
    ] {
        assert!(
            message.contains(named),
            "the refusal must name `{named}`: {message}"
        );
    }
}

/// `driver-provisioning/driver-path-override`: on this trigger the reserved
/// connection resolved, so the message reports the registry rather than the
/// connection, and carries no text written for the explicit route.
#[test]
fn no_driver_route_names_the_registry_it_could_not_reach() {
    let mount = TempDir::new().expect("a temporary mount must be creatable");

    let error = default_location_library(
        mount.path(),
        Driver::MySql,
        FallbackCause::RegistryUnreachable {
            url: UNREACHABLE_INDEX_URL.to_string(),
            cause: UNREACHABLE_TRANSPORT_CAUSE.to_string(),
        },
    )
    .expect_err("an empty default location leaves the statement no route at all");

    let message = error.to_string();
    let location = provisioning::default_driver_path(mount.path(), Driver::MySql);
    for named in [
        UNREACHABLE_INDEX_URL,
        UNREACHABLE_TRANSPORT_CAUSE,
        location.to_string_lossy().as_ref(),
        BUCKETFS_CONNECTION_PROPERTY,
        DRIVER_PATH_PROPERTY,
    ] {
        assert!(
            message.contains(named),
            "the refusal must name `{named}`: {message}"
        );
    }
    assert!(
        !message.contains(UNRESOLVED_CONNECTION_SENTENCE),
        "the reserved connection resolved on this route, so naming it is fine and reporting \
         it as the failure is not: {message}"
    );
    assert!(
        !message.contains("no path on the mount to pre-load"),
        "this message names the very location it looked at, so nothing may deny that the \
         location exists: {message}"
    );
}

/// `driver-provisioning/driver-path-override`: something *is* at the default
/// location and cannot be loaded, so the refusal says why rather than reporting
/// the location as empty.
///
/// The operator who reaches this is the air-gapped one who pre-placed a library
/// and got the path shape or the mode bits slightly wrong, and a message that
/// only says no file is there sends them to look in the wrong place. A
/// directory standing where the library belongs is the reproducible form of
/// that mistake: it is the one unreadable kind a test can create without
/// depending on the mode bits a build agent runs under.
#[test]
fn no_driver_route_names_why_the_default_location_was_unusable() {
    let mount = TempDir::new().expect("a temporary mount must be creatable");
    let location = provisioning::default_driver_path(mount.path(), Driver::MySql);
    std::fs::create_dir_all(&location)
        .expect("a directory standing where the library belongs must be creatable");

    let error = default_location_library(
        mount.path(),
        Driver::MySql,
        FallbackCause::ConnectionUnresolved(UNRESOLVED_CAUSE.to_string()),
    )
    .expect_err("a directory at the default location is no driver library");

    let message = error.to_string();
    for named in [
        location.to_string_lossy().as_ref(),
        "it is not a regular file",
    ] {
        assert!(
            message.contains(named),
            "an unusable file at the default location must be reported as unusable and not \
             as absent, so the refusal must name `{named}`: {message}"
        );
    }
}

/// `vs-adapter/create-virtual-schema` and
/// `driver-provisioning/driver-version-resolution`: only the reserved-connection
/// route has a last resort, and only an index request that reached no host
/// reaches it.
///
/// The `SelfInstall` pairing is the call the explicit route makes in
/// production, because both credential routes funnel their `ensure_driver`
/// failure through this one function rather than each deciding for itself.
#[test]
fn an_explicit_bucketfs_connection_never_falls_back() {
    let explicit = DriverSource::SelfInstall("MY_OWN_BUCKETFS".to_string());
    assert!(
        self_install_fallback(&explicit, &an_index_that_reached_no_host()).is_none(),
        "an explicitly named BucketFS connection is an instruction, so even the one failure \
         the reserved route falls back on surfaces unchanged"
    );

    assert!(
        self_install_fallback(
            &DriverSource::Supplied("/mine/libadbc_driver_mysql.so".to_string()),
            &an_index_that_reached_no_host()
        )
        .is_none(),
        "a supplied library never reaches ensure_driver at all, so this pairing cannot arise \
         in production and must answer None if it ever does"
    );

    let answered: [(&str, ProvisioningError); 5] = [
        (
            "a registry that answered an HTTP status",
            ProvisioningError::RegistryRejected {
                url: UNREACHABLE_INDEX_URL.to_string(),
                status: 503,
                driver: Driver::MySql.as_str().to_string(),
            },
        ),
        (
            "a package download that reached no host, which runs only after a version \
             resolved and the versioned presence check ran",
            ProvisioningError::PackageUnreachable {
                url: "https://dbc-cdn.columnar.tech/mysql/0.6.1/mysql_linux_amd64.tar.gz"
                    .to_string(),
                driver: Driver::MySql.as_str().to_string(),
                cause: "error trying to connect: connection refused".to_string(),
            },
        ),
        (
            "an index body that stopped arriving after its status did",
            ProvisioningError::TransferInterrupted {
                url: UNREACHABLE_INDEX_URL.to_string(),
                driver: Driver::MySql.as_str().to_string(),
                cause: "connection closed before message completed".to_string(),
            },
        ),
        (
            "a package body that stopped arriving after its status did",
            ProvisioningError::TransferInterrupted {
                url: "https://dbc-cdn.columnar.tech/mysql/0.6.1/mysql_linux_amd64.tar.gz"
                    .to_string(),
                driver: Driver::MySql.as_str().to_string(),
                cause: "connection closed before message completed".to_string(),
            },
        ),
        (
            "a BucketFS write that was refused",
            ProvisioningError::BucketFsRejected {
                url: "https://exasol:2581/default".to_string(),
                status: 403,
            },
        ),
    ];
    for (failure, error) in answered {
        assert!(
            self_install_fallback(&DriverSource::DefaultBucketFs, &error).is_none(),
            "{failure} says nothing about the cluster's egress, so the reserved route must \
             fail the statement rather than fall back"
        );
    }
}

/// The other half of the same rule: the one pairing that does fall back carries
/// the accessor's two facts through unchanged, so the last-resort error can
/// compose its own sentence from them.
#[test]
fn the_reserved_connection_falls_back_on_an_index_that_reached_no_host() {
    let fallback = self_install_fallback(
        &DriverSource::DefaultBucketFs,
        &an_index_that_reached_no_host(),
    );

    let Some(FallbackCause::RegistryUnreachable { url, cause }) = fallback else {
        panic!(
            "the reserved route must fall back on an index request that reached no host, \
                which is the only failure that says the cluster has no egress: {fallback:?}"
        );
    };
    assert_eq!(url, UNREACHABLE_INDEX_URL);
    assert_eq!(cause, UNREACHABLE_TRANSPORT_CAUSE);
}

/// The `DRIVER` property as Exasol delivers it: a string among the statement's
/// other properties, not a bare value.
fn properties(value: Value) -> serde_json::Map<String, Value> {
    value
        .as_object()
        .expect("a property set is a JSON object")
        .clone()
}

/// `vs-adapter/create-virtual-schema`: the refusal is the whole diagnosis an
/// operator gets, so it names the value it rejected and every value it would
/// have accepted — read off the identities, so it can never name one driver as
/// the only supported one while a second ships.
#[test]
fn unsupported_driver_value_errors() {
    let error = resolve_driver(&properties(json!({"DRIVER": "snowflake"})))
        .expect_err("a driver this release does not ship must be refused");

    assert!(matches!(error, SchemaError::UnsupportedDriver(_)));
    let message = error.to_string();
    assert!(
        message.contains("snowflake"),
        "the refusal must name the value it rejected: {message}"
    );
    for driver in Driver::ALL {
        assert!(
            message.contains(driver.as_str()),
            "the refusal must name accepted driver `{}`: {message}",
            driver.as_str()
        );
    }
}

#[test]
fn driver_value_is_accepted_case_insensitively() {
    assert_eq!(
        resolve_driver(&properties(json!({"DRIVER": "MySQL"})))
            .expect("mysql must be accepted regardless of case"),
        Driver::MySql
    );
}

#[test]
fn a_statement_naming_no_driver_reports_the_missing_property() {
    let error = resolve_driver(&properties(json!({"CONNECTION_NAME": "A"})))
        .expect_err("a statement with no DRIVER cannot resolve one");

    assert!(
        matches!(error, SchemaError::MissingProperty("DRIVER")),
        "a DRIVER that is absent is missing rather than unsupported: {error}"
    );
}

/// `createVirtualSchema`, `refresh`, and `setProperties` all reach the source
/// through `discover`, and `discover` reaches its tables only through
/// `discover_supported_source`, so a source this release cannot render
/// against is refused on every one of the three paths. It has to be:
/// `setProperties` can point an existing virtual schema at a different
/// server, and `refresh` re-discovers against whatever its `CONNECTION`
/// resolves to now, so a schema created against a supported server can be
/// retargeted at one that supports nothing its dialect renders. Which
/// precondition a source states is the source's own business; that no table
/// is read from a source that failed it is this module's.
#[test]
fn a_source_failing_its_precondition_is_refused_before_any_table_is_discovered() {
    let mut source = FakeMetadataSource::refusing("this source fails its own precondition `p`");

    let Err(error) = discover_supported_source(&mut source) else {
        panic!("a source that fails its own precondition must be refused");
    };

    let message = error.to_string();
    assert!(
        message.contains("this source fails its own precondition `p`"),
        "the source's own reason must reach the operator unchanged: {message}"
    );
    assert!(
        !source.tables_discovered,
        "the check must refuse the source before a table list is built from it"
    );
}

#[test]
fn a_source_meeting_its_precondition_discovers_its_tables() {
    let mut source = FakeMetadataSource::holding(vec![table("orders", &["id"])]);

    let tables = discover_supported_source(&mut source)
        .expect("a source that meets its own precondition must be discovered from");

    assert_eq!(
        tables.len(),
        1,
        "discovery must return the source's own tables: {:?}",
        tables.iter().map(|table| &table.name).collect::<Vec<_>>()
    );
    assert!(source.tables_discovered);
}
