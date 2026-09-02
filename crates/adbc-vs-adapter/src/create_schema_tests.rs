use semver::Version;
use serde_json::json;

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
    fn namespace(&self) -> &str {
        "shop"
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
        database: "shop".to_string(),
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
    assert_eq!(notes.database, "shop");
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
fn database_is_the_path_component_of_the_connection_address() {
    assert_eq!(
        parse_database("mysql://172.28.5.10:3306/shop").unwrap(),
        "shop"
    );
}

#[test]
fn address_without_a_database_path_errors() {
    let error = parse_database("mysql://172.28.5.10:3306").unwrap_err();
    assert!(matches!(
        error,
        SchemaError::ConnectionAddressMissingDatabase(_)
    ));
}

#[test]
fn address_with_an_empty_database_path_errors() {
    let error = parse_database("mysql://172.28.5.10:3306/").unwrap_err();
    assert!(matches!(
        error,
        SchemaError::ConnectionAddressMissingDatabase(_)
    ));
}

#[test]
fn database_path_stops_at_a_query_string() {
    assert_eq!(
        parse_database("mysql://host:3306/shop?useSSL=false").unwrap(),
        "shop"
    );
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
        let database = parse_database(address)
            .unwrap_or_else(|error| panic!("`{address}` names a database: {error}"));

        assert_eq!(
            address,
            format!("postgresql://172.28.5.11:5432/{database}"),
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

/// `vs-adapter/create-virtual-schema`, `driver-provisioning/driver-path-override`:
/// a statement naming a driver library supplies it, so the write credential
/// self-install would need is not a property it has to carry.
#[test]
fn a_statement_setting_driver_path_supplies_its_own_library() {
    let source = driver_source(&properties(json!({
        "DRIVER_PATH": "/buckets/bfsdefault/default/mine/libadbc_driver_postgresql.so",
    })))
    .expect("a statement that supplies its own driver library needs no BUCKETFS_CONNECTION");

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
    })))
    .expect("a statement carrying both properties is not ambiguous");

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
    })))
    .expect("an empty DRIVER_PATH leaves self-install as the route");

    assert!(
        matches!(&source, DriverSource::SelfInstall(name) if name == "B"),
        "an empty DRIVER_PATH must read as absent: {source:?}"
    );
}

#[test]
fn a_statement_without_driver_path_self_installs_through_its_bucketfs_connection() {
    let source = driver_source(&properties(json!({"BUCKETFS_CONNECTION": "B"})))
        .expect("a statement supplying no library provisions one");

    assert!(
        matches!(&source, DriverSource::SelfInstall(name) if name == "B"),
        "the BUCKETFS_CONNECTION named is the one the write goes through: {source:?}"
    );
}

/// `vs-adapter/create-virtual-schema`: `BUCKETFS_CONNECTION` is optional only
/// while `DRIVER_PATH` is set. Without one, self-install is the route and the
/// write credential is required — and it is missed before any connection is
/// resolved, so a statement that cannot provision never provisions.
#[test]
fn bucketfs_connection_required_without_driver_path() {
    let error = driver_source(&properties(json!({"CONNECTION_NAME": "A"})))
        .expect_err("self-install needs the write credential BUCKETFS_CONNECTION names");

    assert!(
        matches!(error, SchemaError::MissingProperty("BUCKETFS_CONNECTION")),
        "the refusal must name the property the statement left out: {error}"
    );
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
    let mut source = FakeMetadataSource::refusing("this server is missing collation `c`");

    let Err(error) = discover_supported_source(&mut source) else {
        panic!("a source that fails its own precondition must be refused");
    };

    let message = error.to_string();
    assert!(
        message.contains("this server is missing collation `c`"),
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
