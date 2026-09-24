//! The query model every dialect's tests are driven with, built once.
//!
//! Each dialect is tested independently of the others on purpose: what it
//! renders, which capabilities it advertises, and which forms it refuses are
//! its own, and a shared expectation would let one dialect's regression hide
//! behind another's. None of that applies to the *request* a dialect is driven
//! with. A `pushdown` request, the query model it parses into, and the Exasol
//! types its columns declare carry no dialect-specific content at all, so
//! three copies of their builders only meant three places to edit when the
//! model changed and no place that would fail if two drifted apart.
//!
//! The split this module draws is therefore: everything that names a dialect
//! or a namespace stays in that dialect's own test file, and everything that
//! builds the dialect-agnostic model lives here. The helpers that need a
//! dialect or a namespace take it as a parameter, so each test file supplies
//! its own and its call sites read exactly as before.

use serde_json::{json, Value};

use crate::adapter_notes::SourceQualifier;
use crate::exa_type::ExasolType;
use crate::pushdown_query::{
    self, AggregateCall, Comparison, Expression, FromClause, Literal, Predicate, Query, SelectItem,
    SourceTable,
};
use crate::sql_dialect::SqlDialect;

/// A single-table `pushdown` request over `orders`, rendered through the
/// parser so each assertion covers the request shape Exasol sends rather than
/// a hand-built model.
pub(crate) fn request(columns: Value, select_list: Option<Value>) -> Value {
    let mut pushdown_request = json!({
        "type": "select",
        "from": {"type": "table", "name": "orders"},
    });
    if let Some(select_list) = select_list {
        pushdown_request["selectList"] = select_list;
    }

    json!({
        "type": "pushdown",
        "pushdownRequest": pushdown_request,
        "involvedTables": [{"name": "orders", "columns": columns}],
    })
}

pub(crate) fn orders_columns() -> Value {
    json!([
        {"name": "id", "dataType": {"type": "DECIMAL", "precision": 19, "scale": 0}},
        {"name": "total", "dataType": {"type": "DECIMAL", "precision": 12, "scale": 4}},
        {"name": "note", "dataType": {"type": "VARCHAR", "size": 100, "characterSet": "UTF8"}},
    ])
}

pub(crate) fn column(name: &str) -> Value {
    json!({"type": "column", "name": name})
}

/// A `pushdown` request carrying a `filter` element, projecting one column so
/// every assertion reads the `WHERE` clause rather than the projection around
/// it.
pub(crate) fn filtered_request(columns: Value, filter: Value) -> Value {
    let projected = columns[0]["name"].clone();
    json!({
        "type": "pushdown",
        "pushdownRequest": {
            "type": "select",
            "from": {"type": "table", "name": "orders"},
            "selectList": [{"type": "column", "name": projected}],
            "filter": filter,
        },
        "involvedTables": [{"name": "orders", "columns": columns}],
    })
}

pub(crate) fn numeric_literal(value: &str) -> Value {
    json!({"type": "literal_exactnumeric", "value": value})
}

pub(crate) fn double_literal(value: &str) -> Value {
    json!({"type": "literal_double", "value": value})
}

pub(crate) fn string_literal(value: &str) -> Value {
    json!({"type": "literal_string", "value": value})
}

pub(crate) fn equal(left: Value, right: Value) -> Value {
    json!({"type": "predicate_equal", "left": left, "right": right})
}

pub(crate) fn less(left: Value, right: Value) -> Value {
    json!({"type": "predicate_less", "left": left, "right": right})
}

/// The statement `request` renders to through `dialect`, parsed first so every
/// assertion covers the request shape Exasol sends.
pub(crate) fn render_statement(
    dialect: &dyn SqlDialect,
    namespace: &SourceQualifier,
    request: &Value,
) -> String {
    let query = pushdown_query::parse(request, namespace).expect("the request must parse");
    dialect.render(&query).expect("the query must render")
}

/// The error rendering `request` through `dialect` fails with, for a request
/// the parser accepts and that dialect has no form for.
pub(crate) fn render_error(
    dialect: &dyn SqlDialect,
    namespace: &SourceQualifier,
    request: &Value,
) -> String {
    let query = pushdown_query::parse(request, namespace).expect("the request must parse");
    dialect
        .render(&query)
        .expect_err("the query must fail to render")
        .to_string()
}

/// The `WHERE` clause `filter` renders to over `columns`, parsed from a
/// request first so each assertion covers the shape Exasol sends.
pub(crate) fn where_clause_over(
    dialect: &dyn SqlDialect,
    namespace: &SourceQualifier,
    columns: Value,
    filter: Value,
) -> String {
    let sql = render_statement(dialect, namespace, &filtered_request(columns, filter));
    let (_, clause) = sql
        .split_once(" WHERE ")
        .unwrap_or_else(|| panic!("the rendered statement must carry a WHERE clause: {sql}"));
    clause.to_string()
}

pub(crate) fn decimal_column() -> pushdown_query::ColumnRef {
    pushdown_query::ColumnRef {
        name: "id".to_string(),
        declared_type: ExasolType::Decimal {
            precision: 19,
            scale: 0,
        },
        table_name: None,
        table_alias: None,
    }
}

pub(crate) fn varchar_column() -> pushdown_query::ColumnRef {
    pushdown_query::ColumnRef {
        name: "note".to_string(),
        declared_type: ExasolType::Varchar { size: 100 },
        table_name: None,
        table_alias: None,
    }
}

pub(crate) fn date_column() -> pushdown_query::ColumnRef {
    pushdown_query::ColumnRef {
        name: "shipped".to_string(),
        declared_type: ExasolType::Date,
        table_name: None,
        table_alias: None,
    }
}

pub(crate) fn timestamp_column() -> pushdown_query::ColumnRef {
    pushdown_query::ColumnRef {
        name: "logged".to_string(),
        declared_type: ExasolType::Timestamp,
        table_name: None,
        table_alias: None,
    }
}

pub(crate) fn select_list() -> Vec<SelectItem> {
    let column = decimal_column();
    vec![SelectItem {
        expression: Expression::Column(column.clone()),
        declared_type: column.declared_type,
    }]
}

pub(crate) fn from(namespace: &SourceQualifier) -> FromClause {
    FromClause::Table(SourceTable {
        namespace: namespace.clone(),
        name: "orders".to_string(),
        alias: None,
    })
}

pub(crate) fn filtered(namespace: &SourceQualifier, filter: Predicate) -> Query {
    Query {
        select_list: select_list(),
        filter: Some(filter),
        aggregation: None,
        from: from(namespace),
    }
}

pub(crate) fn id_equals(right: Expression) -> Predicate {
    Predicate::Equal(Comparison {
        left: Expression::Column(decimal_column()),
        right,
    })
}

pub(crate) fn numeric(value: &str) -> Expression {
    Expression::Literal(Literal::ExactNumeric(value.to_string()))
}

pub(crate) fn aggregated(namespace: &SourceQualifier, select_list: Vec<SelectItem>) -> Query {
    Query {
        select_list,
        filter: None,
        aggregation: Some(pushdown_query::Aggregation::SingleGroup),
        from: from(namespace),
    }
}

pub(crate) fn aggregate_item(call: AggregateCall) -> SelectItem {
    SelectItem {
        expression: Expression::Aggregate(call),
        declared_type: ExasolType::Decimal {
            precision: 18,
            scale: 0,
        },
    }
}

pub(crate) fn count_star() -> AggregateCall {
    AggregateCall {
        function: "COUNT".to_string(),
        argument: None,
        distinct: false,
    }
}

pub(crate) fn aggregate_call(
    function: &str,
    argument: Expression,
    distinct: bool,
) -> AggregateCall {
    AggregateCall {
        function: function.to_string(),
        argument: Some(Box::new(argument)),
        distinct,
    }
}

pub(crate) fn joined_column_ref(name: &str, table: &str) -> pushdown_query::ColumnRef {
    pushdown_query::ColumnRef {
        name: name.to_string(),
        declared_type: ExasolType::Decimal {
            precision: 19,
            scale: 0,
        },
        table_name: Some(table.to_string()),
        table_alias: None,
    }
}

pub(crate) fn joined_query(namespace: &SourceQualifier, join_type: &str) -> Query {
    Query {
        select_list: vec![SelectItem {
            expression: Expression::Column(joined_column_ref("id", "orders")),
            declared_type: ExasolType::Decimal {
                precision: 19,
                scale: 0,
            },
        }],
        filter: None,
        aggregation: None,
        from: FromClause::Join(Box::new(pushdown_query::Join {
            join_type: join_type.to_string(),
            left: Box::new(FromClause::Table(SourceTable {
                namespace: namespace.clone(),
                name: "orders".to_string(),
                alias: None,
            })),
            right: Box::new(FromClause::Table(SourceTable {
                namespace: namespace.clone(),
                name: "customers".to_string(),
                alias: None,
            })),
            condition: Predicate::Equal(Comparison {
                left: Expression::Column(joined_column_ref("customer_id", "orders")),
                right: Expression::Column(joined_column_ref("id", "customers")),
            }),
        })),
    }
}
