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

/// The `schemaMetadataInfo.adapterNotes` every pushdown fixture below needs:
/// `createVirtualSchema` must run before a pushdown can be built, and this
/// is the state it would have recorded.
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

#[test]
fn missing_property_named_in_error() {
    let mut ctx = FakeCtx::new();
    let request = create_request(json!({
        "CONNECTION_NAME": "MY_MYSQL_CONNECTION",
        "BUCKETFS_CONNECTION": "MY_BUCKETFS_CONNECTION",
    }));

    let error = adapter_call(&mut ctx, &request).expect_err("a missing DRIVER must fail");

    assert!(
        error.to_string().contains("DRIVER"),
        "the error must name the missing property: {error}"
    );
}

#[test]
fn unsupported_driver_named_in_error() {
    let mut ctx = FakeCtx::new();
    let request = create_request(json!({
        "DRIVER": "postgresql",
        "CONNECTION_NAME": "MY_MYSQL_CONNECTION",
        "BUCKETFS_CONNECTION": "MY_BUCKETFS_CONNECTION",
    }));

    let error = adapter_call(&mut ctx, &request).expect_err("an unsupported DRIVER must fail");

    let message = error.to_string();
    assert!(
        message.contains("postgresql"),
        "must name the value received: {message}"
    );
    assert!(
        message.contains("mysql"),
        "must name the only supported driver: {message}"
    );
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
