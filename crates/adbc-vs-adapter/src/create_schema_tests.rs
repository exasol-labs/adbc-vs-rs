use serde_json::json;

use super::*;
use crate::metadata::{DiscoveredColumn, DiscoveredTable, MetadataError, SourceCollations};
use crate::type_mapping::MySqlColumn;

fn mysql_column(data_type: &str) -> MySqlColumn {
    MySqlColumn {
        data_type: data_type.to_string(),
        character_length: Some(100),
        character_set: Some("utf8mb4".to_string()),
        numeric_precision: Some(10),
        numeric_scale: Some(0),
        is_unsigned: false,
        is_nullable: true,
    }
}

fn table(name: &str, columns: &[(&str, &str)]) -> DiscoveredTable {
    DiscoveredTable {
        name: name.to_string(),
        columns: columns
            .iter()
            .map(|(column, data_type)| DiscoveredColumn {
                name: (*column).to_string(),
                mysql: mysql_column(data_type),
            })
            .collect(),
    }
}

/// A [`MetadataSource`] that answers with tables it was handed, so the
/// response assembly is driven in the adapter's own vocabulary without a
/// MySQL connection anywhere — which is what the seam exists for. It records
/// whether it was asked for them, which is how a test tells a source refused
/// before discovery from one refused after it.
struct FakeMetadataSource {
    tables: Vec<DiscoveredTable>,
    tables_discovered: bool,
}

impl FakeMetadataSource {
    fn holding(tables: Vec<DiscoveredTable>) -> Self {
        Self {
            tables,
            tables_discovered: false,
        }
    }
}

impl MetadataSource for FakeMetadataSource {
    fn discover_tables(&mut self, _database: &str) -> Result<Vec<DiscoveredTable>, MetadataError> {
        self.tables_discovered = true;
        Ok(std::mem::take(&mut self.tables))
    }
}

fn discovered(tables: Vec<DiscoveredTable>) -> Vec<DiscoveredTable> {
    FakeMetadataSource::holding(tables)
        .discover_tables("shop")
        .expect("the fake source answers with the tables it was handed")
}

/// The notes [`discover`] hands on: everything but the omission list, which
/// only mapping the columns can produce.
fn discovered_notes() -> AdapterNotes {
    AdapterNotes {
        driver: SUPPORTED_DRIVER.to_string(),
        driver_version: provisioning::DRIVER_VERSION.to_string(),
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
        table("orders", &[("id", "INT"), ("payload", "JSON")]),
        table("logs", &[("payload", "JSON")]),
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
    let tables = discovered(vec![table("orders", &[("id", "INT")])]);

    let notes = recorded_notes(&schema_response(
        "createVirtualSchema",
        discovered_notes(),
        tables,
    ));

    assert_eq!(notes.driver, "mysql");
    assert_eq!(notes.driver_version, provisioning::DRIVER_VERSION);
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
        table("orders", &[("id", "INT"), ("payload", "JSON")]),
        table("logs", &[("payload", "JSON")]),
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
    assert_eq!(column_omission.mysql_type.as_deref(), Some("JSON"));
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
    let tables = discovered(vec![table("orders", &[("id", "INT")])]);

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
    let tables = discovered(vec![table("orders", &[("id", "INT")])]);
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
fn empty_table_list_maps_to_no_tables_and_no_omissions() {
    let (tables, omissions) = map_tables(vec![]);
    assert!(tables.is_empty());
    assert!(omissions.is_empty());
}

#[test]
fn a_mapped_column_carries_its_name_type_and_nullability() {
    let tables = vec![DiscoveredTable {
        name: "orders".to_string(),
        columns: vec![DiscoveredColumn {
            name: "id".to_string(),
            mysql: mysql_column("INT"),
        }],
    }];

    let (table_values, omissions) = map_tables(tables);

    assert!(omissions.is_empty());
    assert_eq!(table_values.len(), 1);
    assert_eq!(table_values[0]["name"], "orders");
    let column = &table_values[0]["columns"][0];
    assert_eq!(column["name"], "id");
    assert_eq!(column["isNullable"], true);
    assert_eq!(column["dataType"]["type"], "DECIMAL");
}

#[test]
fn an_unmappable_column_is_omitted_and_recorded() {
    let tables = vec![DiscoveredTable {
        name: "orders".to_string(),
        columns: vec![
            DiscoveredColumn {
                name: "id".to_string(),
                mysql: mysql_column("INT"),
            },
            DiscoveredColumn {
                name: "payload".to_string(),
                mysql: mysql_column("JSON"),
            },
        ],
    }];

    let (table_values, omissions) = map_tables(tables);

    let columns = table_values[0]["columns"].as_array().unwrap();
    assert_eq!(columns.len(), 1, "only the mappable column must be kept");
    assert_eq!(columns[0]["name"], "id");

    assert_eq!(omissions.len(), 1);
    assert_eq!(omissions[0].table, "orders");
    assert_eq!(omissions[0].column.as_deref(), Some("payload"));
    assert_eq!(omissions[0].mysql_type.as_deref(), Some("JSON"));
}

#[test]
fn a_table_with_no_mappable_column_is_omitted_entirely() {
    let tables = vec![DiscoveredTable {
        name: "logs".to_string(),
        columns: vec![DiscoveredColumn {
            name: "payload".to_string(),
            mysql: mysql_column("JSON"),
        }],
    }];

    let (table_values, omissions) = map_tables(tables);

    assert!(table_values.is_empty(), "the table itself must not appear");
    assert!(
        omissions
            .iter()
            .any(|omission| omission.table == "logs" && omission.column.is_none()),
        "the table must be recorded as omitted: {omissions:?}"
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
fn a_database_name_containing_a_single_quote_is_rejected() {
    let error = reject_unsafe_database_name("sho'p").unwrap_err();
    assert!(
        matches!(
            error,
            SchemaError::UnsafeDatabaseName {
                character: '\'',
                ..
            }
        ),
        "expected UnsafeDatabaseName naming the quote, got: {error}"
    );
}

#[test]
fn a_database_name_containing_a_backslash_is_rejected() {
    let error = reject_unsafe_database_name("sho\\p").unwrap_err();
    assert!(
        matches!(
            error,
            SchemaError::UnsafeDatabaseName {
                character: '\\',
                ..
            }
        ),
        "expected UnsafeDatabaseName naming the backslash, got: {error}"
    );
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

#[test]
fn unsupported_driver_value_errors() {
    let error = validate_driver("postgresql").unwrap_err();
    assert!(matches!(error, SchemaError::UnsupportedDriver(_)));
}

#[test]
fn driver_value_is_accepted_case_insensitively() {
    validate_driver("MySQL").expect("mysql must be accepted regardless of case");
}

fn collations(names: &[&str], server_version: &str) -> SourceCollations {
    SourceCollations {
        names: names.iter().map(|name| (*name).to_string()).collect(),
        server_version: server_version.to_string(),
    }
}

#[test]
fn missing_collation_rejected_with_version_named() {
    for missing in REQUIRED_COLLATIONS {
        let present: Vec<&str> = REQUIRED_COLLATIONS
            .into_iter()
            .filter(|&collation| collation != missing)
            .collect();
        let source = collations(&present, "8.0.16-log");

        let error = check_required_collations(&source)
            .expect_err(&format!("a server missing {missing} must be rejected"));
        let message = error.to_string();
        assert!(
            message.contains(missing),
            "error must name the missing collation {missing}, got: {message}"
        );
        assert!(
            message.contains("8.0.16-log"),
            "error must name the server's reported version, got: {message}"
        );
        assert!(
            message.contains(MINIMUM_MYSQL_VERSION),
            "error must name the minimum supported MySQL version {MINIMUM_MYSQL_VERSION}, got: {message}"
        );
    }
}

/// `createVirtualSchema`, `refresh`, and `setProperties` all reach the source
/// through `discover`, and `discover` reaches its tables only through
/// `discover_supported_source`, so a source this release cannot render
/// against is refused on every one of the three paths. It has to be:
/// `setProperties` can point an existing virtual schema at a different MySQL
/// server, and `refresh` re-discovers against whatever its `CONNECTION`
/// resolves to now, so a schema created against a supported server can be
/// retargeted at one that supports nothing the dialect renders. Decision
/// `[13]` rejected leaving that to query time, where the operator reads MySQL
/// error 1273 on every delegated character predicate instead of one rejection
/// naming the collation.
#[test]
fn a_source_missing_a_required_collation_is_refused_before_any_table_is_discovered() {
    for missing in REQUIRED_COLLATIONS {
        let present: Vec<&str> = REQUIRED_COLLATIONS
            .into_iter()
            .filter(|&collation| collation != missing)
            .collect();
        let mut source = FakeMetadataSource::holding(vec![table("orders", &[("id", "INT")])]);

        let Err(error) =
            discover_supported_source(collations(&present, "8.0.16-log"), &mut source, "shop")
        else {
            panic!("a source missing {missing} must be refused");
        };

        let message = error.to_string();
        assert!(
            matches!(error, SchemaError::MissingCollation { .. }),
            "the refusal must be the missing-collation one: {message}"
        );
        assert!(
            message.contains(missing)
                && message.contains("8.0.16-log")
                && message.contains(MINIMUM_MYSQL_VERSION),
            "the error must name the missing collation, the version the source reported, and the \
             minimum supported MySQL version: {message}"
        );
        assert!(
            !source.tables_discovered,
            "the check must refuse the source before a table list is built from it"
        );
    }
}

#[test]
fn a_source_carrying_both_required_collations_discovers_its_tables() {
    let mut source = FakeMetadataSource::holding(vec![table("orders", &[("id", "INT")])]);

    let tables = discover_supported_source(
        collations(&REQUIRED_COLLATIONS, "8.4.0"),
        &mut source,
        "shop",
    )
    .expect("a source carrying both required collations must be discovered from");

    assert_eq!(
        tables.len(),
        1,
        "discovery must return the source's own tables: {:?}",
        tables.iter().map(|table| &table.name).collect::<Vec<_>>()
    );
    assert!(source.tables_discovered);
}

#[test]
fn a_server_carrying_both_required_collations_is_accepted() {
    let source = collations(&REQUIRED_COLLATIONS, "8.4.0");

    check_required_collations(&source)
        .expect("a server listing both required collations must not be rejected");
}

#[test]
fn a_server_is_not_rejected_on_its_version_string_alone() {
    let source = collations(&REQUIRED_COLLATIONS, "not-a-real-version-string");

    check_required_collations(&source).expect(
        "a fork reporting an unexpected version but supplying both collations must still work",
    );
}
