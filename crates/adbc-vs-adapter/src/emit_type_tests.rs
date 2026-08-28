use serde_json::{json, Value};

use crate::adapter_notes::AdapterNotes;
use crate::exa_type::ExasolType;
use crate::mysql_dialect::MySqlDialect;
use crate::pushdown;
use crate::pushdown_query::{self, Expression, SelectItem};

const DATABASE: &str = "shop";

/// A single-table `pushdown` request over `orders`, carrying the three
/// involved columns every assertion below reads back, with `select_list`
/// and `select_list_data_types` wired in only when given.
fn request(select_list: Option<Value>, select_list_data_types: Option<Value>) -> Value {
    let mut pushdown_request = json!({
        "type": "select",
        "from": {"type": "table", "name": "orders"},
    });
    if let Some(select_list) = select_list {
        pushdown_request["selectList"] = select_list;
    }
    if let Some(types) = select_list_data_types {
        pushdown_request["selectListDataTypes"] = types;
    }

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
    })
}

fn parse(
    select_list: Option<Value>,
    select_list_data_types: Option<Value>,
) -> Result<pushdown_query::Query, String> {
    pushdown_query::parse(&request(select_list, select_list_data_types), DATABASE)
}

#[test]
fn declared_types_come_from_select_list_data_types() {
    let select_list = json!([
        {"type": "column", "name": "id"},
        {"type": "column", "name": "note"},
    ]);
    let types = json!([{"type": "VARCHAR", "size": 9}, {"type": "DATE"}]);

    let query = parse(Some(select_list), Some(types)).expect("parse must succeed");

    assert_eq!(
        query.select_list[0].declared_type,
        ExasolType::Varchar { size: 9 },
        "selectListDataTypes overrides `id`'s own DECIMAL type, proving the type came from it \
         rather than from involvedTables"
    );
    assert_eq!(query.select_list[1].declared_type, ExasolType::Date);
}

#[test]
fn select_list_data_types_length_mismatch_named_in_error() {
    let select_list = json!([
        {"type": "column", "name": "id"},
        {"type": "column", "name": "note"},
    ]);
    let types = json!([{"type": "DECIMAL", "precision": 19, "scale": 0}]);

    let error =
        parse(Some(select_list), Some(types)).expect_err("a length mismatch must fail to parse");

    assert!(
        error.contains('2') && error.contains('1'),
        "the error must name both lengths: {error}"
    );
}

#[test]
fn absent_select_list_data_types_named_in_error() {
    let select_list = json!([{"type": "predicate_less"}]);

    let error =
        parse(Some(select_list), None).expect_err("a non-column entry with no type must fail");

    assert!(
        error.contains("selectListDataTypes"),
        "the error must name the missing field: {error}"
    );
    assert!(
        error.contains("8.6.0"),
        "the error must name the minimum Exasol version: {error}"
    );
}

#[test]
fn absent_select_list_types_come_from_involved_tables() {
    let query = parse(None, None).expect("parse must succeed");

    assert_eq!(query.select_list.len(), 3);
    assert_eq!(
        query.select_list[0].declared_type,
        ExasolType::Decimal {
            precision: 19,
            scale: 0
        }
    );
}

#[test]
fn empty_select_list_declares_decimal_one_zero() {
    let query = parse(Some(json!([])), None).expect("parse must succeed");

    assert_eq!(
        query.select_list,
        vec![SelectItem {
            expression: Expression::RowMarker,
            declared_type: ExasolType::Decimal {
                precision: 1,
                scale: 0
            },
        }]
    );
}

#[test]
fn column_only_select_list_types_come_from_involved_tables() {
    let select_list = json!([{"type": "column", "name": "note"}]);

    let query = parse(Some(select_list), None).expect("parse must succeed");

    assert_eq!(
        query.select_list[0].declared_type,
        ExasolType::Varchar { size: 100 }
    );
}

#[test]
fn undeclarable_type_named_in_error() {
    let select_list = json!([{"type": "column", "name": "id"}]);
    let types = json!([{"type": "GEOMETRY"}]);

    let error = parse(Some(select_list), Some(types)).expect_err("an undeclarable type must fail");

    assert!(
        error.contains("GEOMETRY"),
        "the error must name the type it received: {error}"
    );
}

#[test]
fn boolean_declared_type_reaches_the_emits_clause() {
    let adapter_notes = AdapterNotes {
        driver: "mysql".to_string(),
        driver_version: "v0.6.0".to_string(),
        driver_path: "/buckets/bfsdefault/default/adbc-drivers/libadbc_driver_mysql.so".to_string(),
        connection: "MY_MYSQL_CONNECTION".to_string(),
        database: DATABASE.to_string(),
        omitted: vec![],
    };

    let mut request = request(
        Some(json!([{"type": "column", "name": "id"}])),
        Some(json!([{"type": "BOOLEAN"}])),
    );
    request["schemaMetadataInfo"] = json!({"adapterNotes": adapter_notes.to_json_string()});

    let response =
        pushdown::pushdown_response(&request, &MySqlDialect).expect("pushdown must succeed");

    let sql = response["sql"].as_str().expect("sql must be a string");
    assert!(
        sql.contains(r#""c1" BOOLEAN"#),
        "a select-list entry selectListDataTypes declares BOOLEAN must reach the EMITS clause \
         with no size and no precision: {sql}"
    );
}
