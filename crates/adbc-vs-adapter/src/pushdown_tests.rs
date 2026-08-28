use serde_json::json;

use super::*;
use crate::exa_type::ExasolType;
use crate::mysql_dialect::MySqlDialect;

fn request(select_list: Option<Value>) -> Value {
    let mut pushdown_request = json!({
        "type": "select",
        "from": {"type": "table", "name": "orders"},
    });
    if let Some(select_list) = select_list {
        pushdown_request["selectList"] = select_list;
    }

    let adapter_notes = AdapterNotes {
        driver: "mysql".to_string(),
        driver_version: "v0.6.0".to_string(),
        driver_path: "/buckets/bfsdefault/default/adbc-drivers/libadbc_driver_mysql.so".to_string(),
        connection: "MY_MYSQL_CONNECTION".to_string(),
        database: "shop".to_string(),
        omitted: vec![],
    };

    json!({
        "type": "pushdown",
        "pushdownRequest": pushdown_request,
        "involvedTables": [{
            "name": "orders",
            "columns": [
                {"name": "id", "dataType": {"type": "DECIMAL", "precision": 19, "scale": 0}},
                {"name": "total", "dataType": {"type": "DECIMAL", "precision": 12, "scale": 4}},
                {"name": "note", "dataType": {"type": "VARCHAR", "size": 100, "characterSet": "UTF8"}},
            ],
        }],
        "schemaMetadataInfo": {
            "adapterNotes": adapter_notes.to_json_string(),
        },
    })
}

#[test]
fn quote_exasol_identifier_doubles_an_embedded_quote() {
    assert_eq!(quote_exasol_identifier("a\"b"), "\"a\"\"b\"");
    assert_eq!(quote_exasol_identifier("id"), "\"id\"");
}

#[test]
fn named_select_list_projects_and_orders_the_named_columns() {
    let response = pushdown_response(
        &request(Some(json!([
            {"type": "column", "name": "id"},
            {"type": "column", "name": "note"},
        ]))),
        &MySqlDialect,
    )
    .expect("pushdown must succeed");

    let sql = response["sql"].as_str().expect("sql must be a string");
    assert!(sql.contains("SELECT `id`, `note` FROM `shop`.`orders`"));
    assert!(sql.contains(r#""c1" DECIMAL(19,0)"#));
    assert!(sql.contains(r#""c2" VARCHAR(100) UTF8"#));
    assert!(
        sql.contains(r#""name":"c1""#) && sql.contains(r#""name":"c2""#),
        "the embedded scan specification must carry the same positional names the EMITS \
         clause declares: {sql}"
    );
}

/// A join whose two tables both declare an `id` column, projected side by
/// side. The EMITS clause and the scan specification must still declare two
/// distinct positional names for them, the same rule a single-table select
/// list follows above, now proven over a joined `from` tree.
#[test]
fn a_join_projecting_two_columns_of_the_same_name_gets_distinct_positional_names() {
    let adapter_notes = AdapterNotes {
        driver: "mysql".to_string(),
        driver_version: "v0.6.0".to_string(),
        driver_path: "/buckets/bfsdefault/default/adbc-drivers/libadbc_driver_mysql.so".to_string(),
        connection: "MY_MYSQL_CONNECTION".to_string(),
        database: "shop".to_string(),
        omitted: vec![],
    };

    let request = json!({
        "type": "pushdown",
        "pushdownRequest": {
            "type": "select",
            "from": {
                "type": "join",
                "join_type": "inner",
                "left": {"type": "table", "name": "orders"},
                "right": {"type": "table", "name": "customers"},
                "condition": {
                    "type": "predicate_equal",
                    "left": {"type": "column", "name": "customer_id", "tableName": "orders"},
                    "right": {"type": "column", "name": "id", "tableName": "customers"},
                },
            },
            "selectList": [
                {"type": "column", "name": "id", "tableName": "orders"},
                {"type": "column", "name": "id", "tableName": "customers"},
            ],
        },
        "involvedTables": [
            {
                "name": "orders",
                "columns": [
                    {"name": "id", "dataType": {"type": "DECIMAL", "precision": 19, "scale": 0}},
                    {"name": "customer_id", "dataType": {"type": "DECIMAL", "precision": 19, "scale": 0}},
                ],
            },
            {
                "name": "customers",
                "columns": [
                    {"name": "id", "dataType": {"type": "DECIMAL", "precision": 19, "scale": 0}},
                ],
            },
        ],
        "schemaMetadataInfo": {
            "adapterNotes": adapter_notes.to_json_string(),
        },
    });

    let response = pushdown_response(&request, &MySqlDialect).expect("pushdown must succeed");

    let sql = response["sql"].as_str().expect("sql must be a string");
    assert!(
        sql.contains(r#""c1" DECIMAL(19,0)"#) && sql.contains(r#""c2" DECIMAL(19,0)"#),
        "the EMITS clause must declare two distinct positional names for the two id columns: {sql}"
    );
    assert!(
        sql.contains(r#""name":"c1""#) && sql.contains(r#""name":"c2""#),
        "the scan specification must carry the same positional names the EMITS clause declares: {sql}"
    );
}

#[test]
fn select_list_entries_sharing_a_source_column_name_get_distinct_positional_names() {
    let query = Query {
        select_list: vec![
            pushdown_query::SelectItem {
                expression: pushdown_query::Expression::Column(pushdown_query::ColumnRef {
                    name: "id".to_string(),
                    declared_type: ExasolType::Decimal {
                        precision: 19,
                        scale: 0,
                    },
                    table_name: None,
                    table_alias: None,
                }),
                declared_type: ExasolType::Decimal {
                    precision: 19,
                    scale: 0,
                },
            },
            pushdown_query::SelectItem {
                expression: pushdown_query::Expression::Column(pushdown_query::ColumnRef {
                    name: "id".to_string(),
                    declared_type: ExasolType::Decimal {
                        precision: 19,
                        scale: 0,
                    },
                    table_name: None,
                    table_alias: None,
                }),
                declared_type: ExasolType::Decimal {
                    precision: 19,
                    scale: 0,
                },
            },
        ],
        filter: None,
        aggregation: None,
        from: pushdown_query::FromClause::Table(pushdown_query::SourceTable {
            schema: "shop".to_string(),
            name: "orders".to_string(),
            alias: None,
        }),
    };

    let columns = emitted_columns(&query);

    assert_ne!(
        columns[0].name, columns[1].name,
        "two select-list entries over the same source column name must still get pairwise \
         distinct EMITS names: {columns:?}"
    );
}

#[test]
fn absent_select_list_projects_every_declared_column_in_order() {
    let response = pushdown_response(&request(None), &MySqlDialect).expect("pushdown must succeed");

    let sql = response["sql"].as_str().unwrap();
    assert!(sql.contains("SELECT `id`, `total`, `note` FROM `shop`.`orders`"));
    assert!(sql.contains(r#""c1" DECIMAL(19,0)"#));
    assert!(sql.contains(r#""c2" DECIMAL(12,4)"#));
    assert!(sql.contains(r#""c3" VARCHAR(100) UTF8"#));
}

#[test]
fn empty_select_list_declares_exactly_one_emitted_column() {
    let response =
        pushdown_response(&request(Some(json!([]))), &MySqlDialect).expect("pushdown must succeed");

    let sql = response["sql"].as_str().unwrap();
    assert!(sql.contains("SELECT 1 FROM `shop`.`orders`"));
    let emits_count = sql.matches(" DECIMAL").count() + sql.matches(" VARCHAR").count();
    assert_eq!(
        emits_count, 1,
        "exactly one column must be declared in the EMITS clause: {sql}"
    );
}

#[test]
fn response_carries_no_credential() {
    let response = pushdown_response(&request(None), &MySqlDialect).expect("pushdown must succeed");

    let sql = response["sql"].as_str().unwrap().to_lowercase();
    assert!(!sql.contains("password"));
    assert!(sql.contains("my_mysql_connection"));
}

#[test]
fn the_scan_call_selects_the_scan_script_over_one_row() {
    let response = pushdown_response(&request(None), &MySqlDialect).expect("pushdown must succeed");

    let sql = response["sql"].as_str().unwrap();
    assert!(
        sql.starts_with(&format!("SELECT {SCAN_SCRIPT_NAME}('")),
        "an emitting scalar script is called from the select list under the name the harness \
         registers, not named as a table: {sql}"
    );
    assert!(
        sql.ends_with(" FROM DUAL"),
        "the scan call needs exactly one input row to run once: {sql}"
    );
}
