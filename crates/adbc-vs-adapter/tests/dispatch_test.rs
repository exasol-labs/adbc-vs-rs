//! Exercises `adapter_call` for every request path that needs no live
//! MySQL or Exasol connection: malformed input, an unsupported request
//! type, property validation, connection-URI validation, the bare
//! `dropVirtualSchema` response, and a pushdown response built entirely
//! from request data.
//!
//! The scenarios that need live metadata discovery — a successful
//! `createVirtualSchema`, `refresh` picking up a new column, and so on —
//! are typed E2E in the plan and live in `e2e_mysql_test.rs`, not here.

use std::collections::HashMap;

use adbc_vs_adapter::dispatch::adapter_call;
use adbc_vs_scan::driver::Driver;
use exasol_udf_sdk::connect_back::ConnectionObject;
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;
use exasol_udf_sdk::value::Value;
use serde_json::json;

/// A `UdfContext` double that resolves exactly the `CONNECTION` objects a
/// test hands it and does nothing else — no column input, no emitted rows.
struct FakeCtx {
    connections: HashMap<String, ConnectionObject>,
}

impl FakeCtx {
    fn new() -> Self {
        Self {
            connections: HashMap::new(),
        }
    }

    fn with_connection(mut self, name: &str, object: ConnectionObject) -> Self {
        self.connections.insert(name.to_string(), object);
        self
    }
}

impl UdfContext for FakeCtx {
    fn num_columns(&self) -> usize {
        0
    }

    fn get(&self, _col: usize) -> Result<&Value, UdfError> {
        Err(UdfError::Type("FakeCtx carries no input columns".into()))
    }

    fn emit(&mut self, _values: &[Value]) -> Result<(), UdfError> {
        Ok(())
    }

    fn next(&mut self) -> Result<bool, UdfError> {
        Ok(false)
    }

    fn connection(&self, name: &str) -> Result<ConnectionObject, UdfError> {
        self.connections
            .get(name)
            .cloned()
            .ok_or_else(|| UdfError::User(format!("no CONNECTION object named `{name}`")))
    }
}

fn create_request(properties: serde_json::Value) -> String {
    json!({
        "type": "createVirtualSchema",
        "schemaMetadataInfo": {
            "properties": properties,
        },
    })
    .to_string()
}

/// The `schemaMetadataInfo.adapterNotes` every fixture below needs:
/// `createVirtualSchema` must run before a pushdown or a capability response
/// can be built, and this is the state it would have recorded.
fn adapter_notes_json() -> String {
    json!({
        "driver": "mysql",
        "driverVersion": "v0.6.0",
        "driverPath": "/buckets/bfsdefault/default/adbc-drivers/libadbc_driver_mysql.so",
        "connection": "MY_MYSQL_CONNECTION",
        "database": "shop",
        "omitted": [],
    })
    .to_string()
}

fn capabilities_request(schema_metadata_info: serde_json::Value) -> String {
    json!({
        "type": "getCapabilities",
        "schemaMetadataInfo": schema_metadata_info,
    })
    .to_string()
}

/// `vs-adapter/driver-selection`: the capability response is built for the
/// driver the request's own notes record, not for a driver this arm picked.
#[test]
fn capabilities_follow_the_driver_in_adapter_notes() {
    let mut ctx = FakeCtx::new();
    let request = capabilities_request(json!({"adapterNotes": adapter_notes_json()}));

    let response = adapter_call(&mut ctx, &request).expect("getCapabilities must succeed");

    let value: serde_json::Value = serde_json::from_str(&response).expect("response must be JSON");
    assert_eq!(value["type"], "getCapabilities");
    let capabilities = value["capabilities"]
        .as_array()
        .unwrap_or_else(|| panic!("the response must carry a capability array: {value}"));
    assert!(
        capabilities
            .iter()
            .any(|entry| entry == "SELECTLIST_PROJECTION"),
        "the resolved dialect's own set must be advertised: {value}"
    );
}

/// Erroring rather than defaulting is safe only because no `getCapabilities`
/// can precede the `createVirtualSchema` that writes the notes: gate 2.1
/// measured that `CREATE VIRTUAL SCHEMA`, `ALTER ... REFRESH`, and
/// `ALTER ... SET` all succeed against an adapter whose `getCapabilities`
/// always fails, so Exasol sends it only at query-compile time. Defaulting
/// instead would advertise one dialect's set over another's source, which
/// Exasol answers with wrong rows rather than an error.
#[test]
fn capabilities_without_adapter_notes_fails_named() {
    let mut ctx = FakeCtx::new();

    let error = adapter_call(&mut ctx, &capabilities_request(json!({})))
        .expect_err("a request recording no driver must not be answered for a guessed one");

    let message = error.to_string();
    assert!(
        message.contains("adapterNotes"),
        "the error must name the field it could not read: {message}"
    );
}

/// `vs-adapter/driver-selection`: both dialect-reading arms read the driver
/// out of one field. Driven through a driver neither can serve, so each has to
/// have read the field to fail on it — and neither can have fallen back to a
/// dialect of its own.
#[test]
fn capabilities_and_pushdown_resolve_one_field() {
    let notes = json!({
        "driver": "snowflake",
        "driverVersion": "v1.0.0",
        "driverPath": "/buckets/bfsdefault/default/adbc-drivers/libadbc_driver_snowflake.so",
        "connection": "MY_CONNECTION",
        "database": "shop",
        "omitted": [],
    })
    .to_string();
    let pushdown = json!({
        "type": "pushdown",
        "pushdownRequest": {"type": "select", "from": {"type": "table", "name": "orders"}},
        "involvedTables": [{
            "name": "orders",
            "columns": [
                {"name": "id", "dataType": {"type": "DECIMAL", "precision": 19, "scale": 0}},
            ],
        }],
        "schemaMetadataInfo": {"adapterNotes": notes.clone()},
    })
    .to_string();

    for (request_type, request) in [
        (
            "getCapabilities",
            capabilities_request(json!({"adapterNotes": notes})),
        ),
        ("pushdown", pushdown),
    ] {
        let mut ctx = FakeCtx::new();

        let error = adapter_call(&mut ctx, &request)
            .expect_err("a request recording a driver this release does not ship must fail")
            .to_string();

        assert!(
            error.contains("snowflake"),
            "`{request_type}` must resolve its driver from the notes, and name what it read: \
             {error}"
        );
    }
}

/// `vs-adapter/driver-selection`: the address is passed to the driver
/// unchanged, so a mismatch caught here is a property mismatch named as one
/// rather than a connection failure reported from inside the wrong driver.
#[test]
fn connection_scheme_must_match_named_driver() {
    let mut ctx = FakeCtx::new().with_connection(
        "MY_SOURCE_CONNECTION",
        ConnectionObject {
            kind: "postgresql".to_string(),
            address: "postgresql://172.28.5.11:5432/shop".to_string(),
            user: "postgres".to_string(),
            password: "secret".to_string(),
        },
    );
    let request = create_request(json!({
        "DRIVER": "mysql",
        "CONNECTION_NAME": "MY_SOURCE_CONNECTION",
        "BUCKETFS_CONNECTION": "MY_BUCKETFS_CONNECTION",
    }));

    let error =
        adapter_call(&mut ctx, &request).expect_err("a mismatched address scheme must fail");

    let message = error.to_string();
    assert!(
        message.contains("mysql"),
        "the error must name the driver the statement asked for: {message}"
    );
    assert!(
        message.contains("postgresql"),
        "the error must name the scheme the address carries: {message}"
    );
}

#[test]
fn drop_returns_bare_response() {
    let mut ctx = FakeCtx::new();

    let response =
        adapter_call(&mut ctx, r#"{"type":"dropVirtualSchema"}"#).expect("drop must succeed");

    let value: serde_json::Value = serde_json::from_str(&response).expect("response must be JSON");
    assert_eq!(value, json!({"type": "dropVirtualSchema"}));
}

#[test]
fn unknown_request_type_quotes_value() {
    let mut ctx = FakeCtx::new();

    let error = adapter_call(&mut ctx, r#"{"type":"frobnicate"}"#)
        .expect_err("an unsupported request type must fail");

    let message = error.to_string();
    assert!(
        message.contains("\"frobnicate\""),
        "the error must quote the unsupported type value: {message}"
    );
}

#[test]
fn malformed_json_reports_parse_failure() {
    let mut ctx = FakeCtx::new();

    let error = adapter_call(&mut ctx, "not json").expect_err("malformed JSON must fail");

    let message = error.to_string().to_lowercase();
    assert!(
        message.contains("json") && message.contains("pars"),
        "the error must identify a request-parsing failure: {message}"
    );
}

/// `vs-adapter/create-virtual-schema`: a property the statement has to carry is
/// named when it is left out. `DRIVER` and `CONNECTION_NAME` are unconditional;
/// `BUCKETFS_CONNECTION` no longer is — its absence is covered by
/// `an_unnamed_bucketfs_connection_falls_back_to_the_unresolved_reserved_one`
/// below instead, since it no longer fails before the source `CONNECTION` is
/// resolved.
///
/// The `DRIVER_PATH` case asserts a negative, which is the strongest claim this
/// tier can make: property validation passing means the request goes on to open
/// the source and discover its tables, which needs a live connection this test
/// has none of. That it fails for some *other* reason is exactly the point —
/// the missing-property refusal is gone. The success half is
/// `e2e_postgres_test.rs::driver_path_is_accepted_and_bypasses_provisioning`.
#[test]
fn missing_property_named_in_error() {
    for (missing, properties) in [
        (
            "DRIVER",
            json!({
                "CONNECTION_NAME": "MY_MYSQL_CONNECTION",
                "BUCKETFS_CONNECTION": "MY_BUCKETFS_CONNECTION",
            }),
        ),
        (
            "CONNECTION_NAME",
            json!({
                "DRIVER": "mysql",
                "BUCKETFS_CONNECTION": "MY_BUCKETFS_CONNECTION",
            }),
        ),
    ] {
        let mut ctx = FakeCtx::new();

        let error = adapter_call(&mut ctx, &create_request(properties))
            .expect_err(&format!("a missing {missing} must fail"));

        assert!(
            error.to_string().contains(missing),
            "the error must name the missing property `{missing}`: {error}"
        );
    }
}

/// `vs-adapter/create-virtual-schema`'s precedence scenario, default level: a
/// statement naming no `BUCKETFS_CONNECTION` reaches the reserved connection
/// `ADBC_VS_BUCKETFS` rather than being refused for a missing property — this
/// context resolves the source `CONNECTION` but not the reserved one, so the
/// failure surfaces only once self-install tries to use it.
///
/// This reads the real `/buckets/bfsdefault/default` mount, because the
/// dispatch tier cannot inject one, so it assumes no pre-placed library
/// already exists on the host running the test.
#[test]
fn an_unnamed_bucketfs_connection_falls_back_to_the_unresolved_reserved_one() {
    let mut ctx = FakeCtx::new().with_connection(
        "MY_MYSQL_CONNECTION",
        ConnectionObject {
            kind: "mysql".to_string(),
            address: "mysql://172.28.5.10:3306/shop".to_string(),
            user: "root".to_string(),
            password: "secret".to_string(),
        },
    );
    let request = create_request(json!({
        "DRIVER": "mysql",
        "CONNECTION_NAME": "MY_MYSQL_CONNECTION",
    }));

    let error = adapter_call(&mut ctx, &request)
        .expect_err("this context resolves no reserved BucketFS connection")
        .to_string();

    assert!(
        error.contains("BUCKETFS_CONNECTION"),
        "the reserved connection's own resolution failure must still name the write-credential \
         property: {error}"
    );
    assert!(
        error.contains("ADBC_VS_BUCKETFS"),
        "the error must name the reserved connection itself, not just the property that would \
         have named one: {error}"
    );
    assert!(
        !error.contains("missing required property"),
        "the removed missing-property refusal must not resurface: {error}"
    );
}

#[test]
fn bucketfs_connection_is_not_missing_when_a_driver_path_is_supplied() {
    let mut ctx = FakeCtx::new();
    let request = create_request(json!({
        "DRIVER": "mysql",
        "CONNECTION_NAME": "MY_MYSQL_CONNECTION",
        "DRIVER_PATH": "/buckets/bfsdefault/default/mine/libadbc_driver_mysql.so",
    }));

    let error = adapter_call(&mut ctx, &request)
        .expect_err("this context resolves no CONNECTION, so the request cannot succeed")
        .to_string();

    assert!(
        !error.contains("BUCKETFS_CONNECTION"),
        "a statement supplying its own driver library must not be refused for the write \
         credential self-install would have needed: {error}"
    );
}

/// `vs-adapter/create-virtual-schema`, `vs-adapter/driver-selection`: the
/// accepted set is read off the identities rather than written into the
/// message, so a release adding a driver cannot leave the refusal naming fewer
/// drivers than it supports — and cannot name any one driver as the only one.
#[test]
fn unsupported_driver_value_fails_named() {
    let mut ctx = FakeCtx::new();
    let request = create_request(json!({
        "DRIVER": "snowflake",
        "CONNECTION_NAME": "MY_SOURCE_CONNECTION",
        "BUCKETFS_CONNECTION": "MY_BUCKETFS_CONNECTION",
    }));

    let error = adapter_call(&mut ctx, &request).expect_err("an unsupported DRIVER must fail");

    let message = error.to_string();
    assert!(
        message.contains("snowflake"),
        "the error must name the value it received: {message}"
    );
    for driver in Driver::ALL {
        assert!(
            message.contains(driver.as_str()),
            "the error must list supported driver `{}`: {message}",
            driver.as_str()
        );
    }
}

#[test]
fn connection_uri_without_database_errors() {
    let mut ctx = FakeCtx::new().with_connection(
        "MY_MYSQL_CONNECTION",
        ConnectionObject {
            kind: "mysql".to_string(),
            address: "mysql://172.28.5.10:3306".to_string(),
            user: "root".to_string(),
            password: "secret".to_string(),
        },
    );
    let request = create_request(json!({
        "DRIVER": "mysql",
        "CONNECTION_NAME": "MY_MYSQL_CONNECTION",
        "BUCKETFS_CONNECTION": "MY_BUCKETFS_CONNECTION",
    }));

    let error = adapter_call(&mut ctx, &request).expect_err("a database-less address must fail");

    assert!(
        error.to_string().contains("database"),
        "the error must state that a database path is needed: {error}"
    );
}

#[test]
fn pushdown_sql_has_no_credential() {
    const DISTINCTIVE_PASSWORD: &str = "sw0rdfish-pushdown-secret";
    let mut ctx = FakeCtx::new().with_connection(
        "MY_MYSQL_CONNECTION",
        ConnectionObject {
            kind: "mysql".to_string(),
            address: "mysql://172.28.5.10:3306/shop".to_string(),
            user: "root".to_string(),
            password: DISTINCTIVE_PASSWORD.to_string(),
        },
    );
    let adapter_notes = adapter_notes_json();

    let request = json!({
        "type": "pushdown",
        "pushdownRequest": {
            "type": "select",
            "from": {"type": "table", "name": "orders"},
        },
        "involvedTables": [{
            "name": "orders",
            "columns": [
                {"name": "id", "dataType": {"type": "DECIMAL", "precision": 19, "scale": 0}},
            ],
        }],
        "schemaMetadataInfo": {
            "adapterNotes": adapter_notes,
        },
    })
    .to_string();

    let response = adapter_call(&mut ctx, &request).expect("pushdown must succeed");

    assert!(
        !response.contains(DISTINCTIVE_PASSWORD),
        "response must carry no credential, even though the CONNECTION_NAME the request names \
         resolves to one on this context: {response}"
    );
    assert!(
        response.to_lowercase().contains("my_mysql_connection"),
        "response must reference the CONNECTION by name: {response}"
    );
}

#[test]
fn order_by_and_limit_elements_are_refused() {
    let orders_table = json!({
        "name": "orders",
        "columns": [
            {"name": "id", "dataType": {"type": "DECIMAL", "precision": 19, "scale": 0}},
        ],
    });

    for (element, value) in [
        (
            "orderBy",
            json!([{"expression": {"type": "column", "name": "id"}, "isAscending": true}]),
        ),
        ("limit", json!({"numElements": 10})),
    ] {
        let mut ctx = FakeCtx::new();
        let mut pushdown_request = json!({
            "type": "select",
            "from": {"type": "table", "name": "orders"},
        });
        pushdown_request[element] = value;

        let request = json!({
            "type": "pushdown",
            "pushdownRequest": pushdown_request,
            "involvedTables": [orders_table.clone()],
            "schemaMetadataInfo": {
                "adapterNotes": adapter_notes_json(),
            },
        })
        .to_string();

        let error = adapter_call(&mut ctx, &request)
            .expect_err(&format!("a `{element}` element must be refused"));

        assert!(
            error.to_string().contains(element),
            "the error must name the refused element `{element}`: {error}"
        );
    }
}

/// A fragment reaches the dialect either as a node kind the model does not
/// carry or as a function the dialect does not render, and both must name
/// what they refused rather than drop it: Exasol never re-applies a fragment
/// it believes was delegated.
#[test]
fn unrenderable_fragment_named_in_error() {
    for (entry, declared_type, named) in [
        (
            json!({
                "type": "function_scalar",
                "name": "UPPER",
                "arguments": [{"type": "column", "name": "id"}],
            }),
            json!({"type": "VARCHAR", "size": 100}),
            "function_scalar",
        ),
        (
            json!({
                "type": "function_aggregate",
                "name": "STDDEV",
                "arguments": [{"type": "column", "name": "id"}],
            }),
            json!({"type": "DOUBLE"}),
            "STDDEV",
        ),
    ] {
        let mut ctx = FakeCtx::new();
        let request = json!({
            "type": "pushdown",
            "pushdownRequest": {
                "type": "select",
                "from": {"type": "table", "name": "orders"},
                "selectList": [entry],
                "selectListDataTypes": [declared_type],
            },
            "involvedTables": [{
                "name": "orders",
                "columns": [
                    {"name": "id", "dataType": {"type": "DECIMAL", "precision": 19, "scale": 0}},
                ],
            }],
            "schemaMetadataInfo": {
                "adapterNotes": adapter_notes_json(),
            },
        })
        .to_string();

        let error =
            adapter_call(&mut ctx, &request).expect_err("an unrenderable fragment must fail");

        assert!(
            error.to_string().contains(named),
            "the error must name the fragment it could not render: {error}"
        );
    }
}
