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

use adbc_vs_scan::driver::Driver;

use crate::adapter_notes::SourceQualifier;
use crate::exa_type::ExasolType;
use crate::pushdown_query::{
    self, AggregateCall, BetweenPredicate, Comparison, Expression, FromClause,
    InConstListPredicate, LikePredicate, Literal, Predicate, Query, SelectItem, SourceTable,
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

pub(crate) fn not(expression: Value) -> Value {
    json!({"type": "predicate_not", "expression": expression})
}

pub(crate) fn and(expressions: Value) -> Value {
    json!({"type": "predicate_and", "expressions": expressions})
}

pub(crate) fn or(expressions: Value) -> Value {
    json!({"type": "predicate_or", "expressions": expressions})
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

/// The source types a converted table's `VARCHAR(36)` `uid` and `BOOLEAN`
/// `flag` columns record, so a dialect's tests drive every position through a
/// real read expression of their own technology.
#[derive(Clone, Copy)]
pub(crate) struct ConvertedNotes {
    pub uid: &'static str,
    pub flag: &'static str,
}

/// An involved table whose `uid` and `flag` columns record `notes` and whose
/// `id`, `note`, and `BOOLEAN` `active` columns carry no notes.
fn converted_table(name: &str, notes: ConvertedNotes) -> Value {
    json!({
        "name": name,
        "columns": [
            {"name": "id", "dataType": {"type": "DECIMAL", "precision": 19, "scale": 0}},
            {"name": "note", "dataType": {"type": "VARCHAR", "size": 100, "characterSet": "UTF8"}},
            {
                "name": "uid",
                "dataType": {"type": "VARCHAR", "size": 36, "characterSet": "UTF8"},
                "adapterNotes": json!({"sourceType": notes.uid}).to_string(),
            },
            {
                "name": "flag",
                "dataType": {"type": "BOOLEAN"},
                "adapterNotes": json!({"sourceType": notes.flag}).to_string(),
            },
            {"name": "active", "dataType": {"type": "BOOLEAN"}},
        ],
    })
}

/// A request over the converted `orders` table carrying `pushdown_request`'s
/// elements beside its `from` element.
fn converted_request(notes: ConvertedNotes, mut pushdown_request: Value) -> Value {
    pushdown_request["type"] = json!("select");
    pushdown_request["from"] = json!({"type": "table", "name": "orders"});
    json!({
        "type": "pushdown",
        "pushdownRequest": pushdown_request,
        "involvedTables": [converted_table("orders", notes)],
    })
}

/// A request over the converted `orders` table projecting `id`, filtered by
/// `filter`.
pub(crate) fn converted_table_filtered_request(notes: ConvertedNotes, filter: Value) -> Value {
    converted_request(
        notes,
        json!({"selectList": [column("id")], "filter": filter}),
    )
}

/// `SELECT uid FROM orders WHERE uid = 'a'`, over a converted `uid`.
pub(crate) fn converted_column_filtered_request(notes: ConvertedNotes) -> Value {
    converted_request(
        notes,
        json!({
            "selectList": [column("uid")],
            "filter": equal(column("uid"), string_literal("a")),
        }),
    )
}

/// A request grouping by `group_by` over the converted `orders` table,
/// projecting `select_list` declared as `data_types`.
pub(crate) fn converted_column_grouped_request(
    notes: ConvertedNotes,
    select_list: Value,
    data_types: Value,
    group_by: &str,
) -> Value {
    converted_request(
        notes,
        json!({
            "selectList": select_list,
            "selectListDataTypes": data_types,
            "aggregationType": "group_by",
            "groupBy": [column(group_by)],
        }),
    )
}

/// A `function_aggregate` entry over `column_name`.
pub(crate) fn aggregate_over(function: &str, column_name: &str, distinct: bool) -> Value {
    json!({
        "type": "function_aggregate",
        "name": function,
        "arguments": [column(column_name)],
        "distinct": distinct,
    })
}

/// `orders` joined to `customers` on their converted `uid` columns,
/// projecting `orders.id`, which carries no notes.
pub(crate) fn converted_column_joined_request(notes: ConvertedNotes) -> Value {
    let qualified =
        |name: &str, table: &str| json!({"type": "column", "name": name, "tableName": table});
    json!({
        "type": "pushdown",
        "pushdownRequest": {
            "type": "select",
            "from": {
                "type": "join",
                "join_type": "inner",
                "left": {"type": "table", "name": "orders"},
                "right": {"type": "table", "name": "customers"},
                "condition": equal(qualified("uid", "orders"), qualified("uid", "customers")),
            },
            "selectList": [qualified("id", "orders")],
        },
        "involvedTables": [
            converted_table("orders", notes),
            converted_table("customers", notes),
        ],
    })
}

pub(crate) fn decimal_column() -> pushdown_query::ColumnRef {
    pushdown_query::ColumnRef {
        name: "id".to_string(),
        declared_type: ExasolType::Decimal {
            precision: 19,
            scale: 0,
        },
        source_type: None,
        table_name: None,
        table_alias: None,
    }
}

pub(crate) fn varchar_column() -> pushdown_query::ColumnRef {
    pushdown_query::ColumnRef {
        name: "note".to_string(),
        declared_type: ExasolType::Varchar { size: 100 },
        source_type: None,
        table_name: None,
        table_alias: None,
    }
}

pub(crate) fn date_column() -> pushdown_query::ColumnRef {
    pushdown_query::ColumnRef {
        name: "shipped".to_string(),
        declared_type: ExasolType::Date,
        source_type: None,
        table_name: None,
        table_alias: None,
    }
}

pub(crate) fn timestamp_column() -> pushdown_query::ColumnRef {
    pushdown_query::ColumnRef {
        name: "logged".to_string(),
        declared_type: ExasolType::Timestamp { precision: 3 },
        source_type: None,
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
        having: None,
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
        having: None,
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
        source_type: None,
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
        having: None,
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

/// `entry` with `DISTINCT` set, which Exasol sends as a `distinct` field.
pub(crate) fn distinct(mut entry: Value) -> Value {
    entry["distinct"] = json!(true);
    entry
}

/// The type Exasol supplies for `SUM` over `total`, a `DECIMAL(12,4)`.
pub(crate) fn sum_type() -> Value {
    json!({"type": "DECIMAL", "precision": 23, "scale": 4})
}

/// The type Exasol supplies for `AVG` and for `SUM` over a `DOUBLE` operand.
pub(crate) fn double_type() -> Value {
    json!({"type": "DOUBLE"})
}

/// A `function_aggregate` select-list entry over at most one argument. `None`
/// is the `COUNT(*)` shape: Exasol sends it carrying no `arguments` at all,
/// which is how it is told apart from `COUNT(column)`.
pub(crate) fn aggregate(function: &str, argument: Option<Value>) -> Value {
    let mut entry = json!({"type": "function_aggregate", "name": function});
    if let Some(argument) = argument {
        entry["arguments"] = json!([argument]);
    }
    entry
}

/// A `pushdown` request over `orders` carrying `select_list` and the
/// `selectListDataTypes` Exasol supplies for it, which is the only type source
/// an aggregate or a select-list predicate entry has.
pub(crate) fn typed_request(select_list: Value, data_types: Value) -> Value {
    let mut request = request(orders_columns(), Some(select_list));
    request["pushdownRequest"]["selectListDataTypes"] = data_types;
    request
}

/// A `pushdown` request whose `aggregationType` is `group_by` over `group_by`.
pub(crate) fn grouped_request(select_list: Value, data_types: Value, group_by: Value) -> Value {
    let mut request = typed_request(select_list, data_types);
    request["pushdownRequest"]["aggregationType"] = json!("group_by");
    request["pushdownRequest"]["groupBy"] = group_by;
    request
}

/// The type Exasol supplies for `COUNT` over any operand, measured on Exasol
/// 2025.2.1.
pub(crate) fn count_type() -> Value {
    json!({"type": "DECIMAL", "precision": 18, "scale": 0})
}

/// The type Exasol supplies for `MIN` or `MAX` over `note`, a `VARCHAR(100)`:
/// the operand's own type, spelled without the `characterSet` field
/// `involvedTables[].columns[].dataType` carries.
pub(crate) fn note_type() -> Value {
    json!({"type": "VARCHAR", "size": 100})
}

/// The type Exasol supplies for `id`, a `DECIMAL(19,0)`: the operand's own
/// type, as `MIN`, `MAX`, and a projected grouping column all carry.
pub(crate) fn id_type() -> Value {
    json!({"type": "DECIMAL", "precision": 19, "scale": 0})
}

/// The type Exasol supplies for a select-list predicate: one `BOOLEAN` column.
pub(crate) fn boolean_type() -> Value {
    json!({"type": "BOOLEAN"})
}

/// A column node as Exasol sends one inside a join: qualified by the
/// `tableName` of the table it belongs to, which arrives on every column node
/// of a join.
pub(crate) fn joined_column(name: &str, table: &str) -> Value {
    json!({"type": "column", "name": name, "tableName": table})
}

pub(crate) fn joined_orders_columns() -> Value {
    json!([
        {"name": "id", "dataType": {"type": "DECIMAL", "precision": 19, "scale": 0}},
        {"name": "customer_id", "dataType": {"type": "DECIMAL", "precision": 19, "scale": 0}},
        {"name": "note", "dataType": {"type": "VARCHAR", "size": 100, "characterSet": "UTF8"}},
    ])
}

fn joined_customers_columns() -> Value {
    json!([
        {"name": "id", "dataType": {"type": "DECIMAL", "precision": 19, "scale": 0}},
        {"name": "name", "dataType": {"type": "VARCHAR", "size": 100, "characterSet": "UTF8"}},
    ])
}

/// The `from` element joining `orders` to `customers` over the foreign key, in
/// the shape a join whose query aliased no table arrives in: neither table
/// carries an `alias` and both condition operands carry a `tableName`.
pub(crate) fn orders_to_customers(join_type: &str) -> Value {
    json!({
        "type": "join",
        "join_type": join_type,
        "left": {"type": "table", "name": "orders"},
        "right": {"type": "table", "name": "customers"},
        "condition": {
            "type": "predicate_equal",
            "left": joined_column("customer_id", "orders"),
            "right": joined_column("id", "customers"),
        },
    })
}

/// A `pushdown` request over `from`, carrying `select_list` and the two
/// involved tables `orders_to_customers` names.
pub(crate) fn join_request(from: Value, select_list: Value) -> Value {
    json!({
        "type": "pushdown",
        "pushdownRequest": {
            "type": "select",
            "from": from,
            "selectList": select_list,
        },
        "involvedTables": [
            {"name": "orders", "columns": joined_orders_columns()},
            {"name": "customers", "columns": joined_customers_columns()},
        ],
    })
}

/// A `Query` over `orders` with the given select list and nothing else, for
/// the refusal every dialect makes of a projection that selects nothing.
pub(crate) fn empty_projection_error(
    dialect: &dyn SqlDialect,
    namespace: SourceQualifier,
) -> String {
    let query = Query {
        select_list: vec![],
        filter: None,
        aggregation: None,
        having: None,
        from: FromClause::Table(SourceTable {
            namespace,
            name: "orders".to_string(),
            alias: None,
        }),
    };
    dialect
        .render(&query)
        .expect_err("a query projecting no column must fail")
        .to_string()
}

/// Asserts every empty `AND`, `OR`, and `IN` group fails to render, naming its
/// form, through `filter_render_error`.
pub(crate) fn assert_empty_predicate_groups_fail(
    filter_render_error: &dyn Fn(Value) -> String,
    dialect_name: &str,
) {
    for (filter, named) in [
        (json!({"type": "predicate_and", "expressions": []}), "AND"),
        (json!({"type": "predicate_or", "expressions": []}), "OR"),
        (
            json!({
                "type": "predicate_in_constlist",
                "expression": column("id"),
                "arguments": [],
            }),
            "IN",
        ),
    ] {
        let error = filter_render_error(filter);

        assert!(
            error.contains(named),
            "the error must name the empty {dialect_name} form: {error}"
        );
    }
}

/// Asserts an argument-less `MIN` and a `DISTINCT COUNT(*)` both fail to
/// render through `render_error`, naming the fragment the dialect refused.
pub(crate) fn assert_argumentless_aggregates_fail(
    render_error: &dyn Fn(&Value) -> String,
    dialect_name: &str,
) {
    let min_error = render_error(&typed_request(
        json!([aggregate("MIN", None)]),
        json!([count_type()]),
    ));

    assert!(
        min_error.contains("MIN"),
        "the error must name the aggregate that carries no argument: {min_error}"
    );

    let mut distinct_count = aggregate("COUNT", None);
    distinct_count["distinct"] = json!(true);
    let count_error = render_error(&typed_request(
        json!([distinct_count]),
        json!([count_type()]),
    ));

    assert!(
        count_error.contains("DISTINCT"),
        "the error must name the fragment {dialect_name} has no form for: {count_error}"
    );
}

/// Asserts a `GROUP BY` over no column fails to render through `render_error`.
pub(crate) fn assert_empty_group_by_fails(
    render_error: &dyn Fn(&Value) -> String,
    dialect_name: &str,
) {
    let request = grouped_request(
        json!([aggregate("COUNT", None)]),
        json!([count_type()]),
        json!([]),
    );

    let error = render_error(&request);

    assert!(
        error.contains("GROUP BY"),
        "the error must name the empty {dialect_name} clause: {error}"
    );
}

/// The `shipped` `DATE` and `logged` `TIMESTAMP` columns the temporal-literal
/// tests filter over.
pub(crate) fn temporal_columns() -> Value {
    json!([
        {"name": "shipped", "dataType": {"type": "DATE"}},
        {"name": "logged", "dataType": {"type": "TIMESTAMP"}},
    ])
}

/// `shipped` equal to a `DATE` literal.
pub(crate) fn date_filter() -> Value {
    equal(
        column("shipped"),
        json!({"type": "literal_date", "value": "2015-12-01"}),
    )
}

/// `logged` equal to a `TIMESTAMP` literal.
pub(crate) fn timestamp_filter() -> Value {
    equal(
        column("logged"),
        json!({"type": "literal_timestamp", "value": "2015-12-01 12:01:01.123"}),
    )
}

/// The statements one request renders to with its aggregate function names in
/// lower case and in upper case, over `count_column` for the `COUNT`.
pub(crate) fn mixed_case_statements(
    render_statement: &dyn Fn(&Value) -> String,
    count_column: &str,
) -> (String, String) {
    let entries = |count: &str, min: &str, sum: &str, avg: &str| {
        json!([
            aggregate(count, Some(column(count_column))),
            aggregate(min, Some(column("note"))),
            aggregate(sum, Some(column("total"))),
            aggregate(avg, Some(column("total"))),
        ])
    };
    let types = json!([count_type(), note_type(), sum_type(), double_type()]);
    let lower_case = render_statement(&typed_request(
        entries("count", "min", "sum", "avg"),
        types.clone(),
    ));
    let upper_case = render_statement(&typed_request(entries("COUNT", "MIN", "SUM", "AVG"), types));
    (lower_case, upper_case)
}

/// Asserts an unknown aggregate function fails to render, named back in either
/// case.
pub(crate) fn assert_unknown_aggregate_named(render_error: &dyn Fn(&Value) -> String) {
    for (function, named) in [("STDDEV", "STDDEV"), ("stddev", "STDDEV")] {
        let error = render_error(&typed_request(
            json!([aggregate(function, Some(column("total")))]),
            json!([{"type": "DOUBLE"}]),
        ));

        assert!(
            error.contains(named),
            "the error must name the aggregate function it received: {error}"
        );
    }
}

/// `AVG` and `AVG(DISTINCT)` over `total`.
pub(crate) fn avg_request() -> Value {
    typed_request(
        json!([
            aggregate("AVG", Some(column("total"))),
            distinct(aggregate("AVG", Some(column("total")))),
        ]),
        json!([double_type(), double_type()]),
    )
}

/// A grouped and a single-group request, both carrying `12 < COUNT(*)` as
/// their `having` element.
pub(crate) fn having_requests() -> (Value, Value) {
    let having = less(numeric_literal("12"), aggregate("COUNT", None));
    let mut grouped = grouped_request(
        json!([column("note"), aggregate("COUNT", None)]),
        json!([note_type(), count_type()]),
        json!([column("note")]),
    );
    grouped["pushdownRequest"]["having"] = having.clone();

    let mut single = typed_request(json!([aggregate("COUNT", None)]), json!([count_type()]));
    single["pushdownRequest"]["aggregationType"] = json!("single_group");
    single["pushdownRequest"]["having"] = having;
    (grouped, single)
}

/// `orders` joined to `customers` by `join_type`, projecting `orders.id`.
pub(crate) fn join_type_request(join_type: &str) -> Value {
    join_request(
        orders_to_customers(join_type),
        json!([joined_column("id", "orders")]),
    )
}

/// Asserts each of `join_types` fails to render through `render_error`, named
/// back.
pub(crate) fn assert_join_types_named(
    render_error: &dyn Fn(&Value) -> String,
    join_types: &[&str],
) {
    for join_type in join_types {
        let error = render_error(&join_type_request(join_type));

        assert!(
            error.contains(join_type),
            "the error must name the join type it received: {error}"
        );
    }
}

/// `orders` joined to `customers`, whose result joins `regions` by
/// `outer_join_type`, projecting `regions.code`.
pub(crate) fn nested_join_request(outer_join_type: &str) -> Value {
    let mut request = join_request(
        json!({
            "type": "join",
            "join_type": outer_join_type,
            "left": orders_to_customers("inner"),
            "right": {"type": "table", "name": "regions"},
            "condition": {
                "type": "predicate_equal",
                "left": joined_column("id", "customers"),
                "right": joined_column("customer_id", "regions"),
            },
        }),
        json!([joined_column("code", "regions")]),
    );
    request["involvedTables"]
        .as_array_mut()
        .expect("involvedTables must be an array")
        .push(json!({
            "name": "regions",
            "columns": [
                {"name": "customer_id", "dataType": {"type": "DECIMAL", "precision": 19, "scale": 0}},
                {"name": "code", "dataType": {"type": "VARCHAR", "size": 10, "characterSet": "UTF8"}},
            ],
        }));
    request
}

/// The `orders` to `customers` join filtered on `orders.note = 'x'`, projecting
/// `select_list`.
pub(crate) fn filtered_join_request(select_list: Value) -> Value {
    let mut request = join_request(orders_to_customers("inner"), select_list);
    request["pushdownRequest"]["filter"] =
        equal(joined_column("note", "orders"), string_literal("x"));
    request
}

/// The filtered join grouped by `group_column`, projecting it beside
/// `COUNT(*)` declared as `key_type`.
pub(crate) fn filtered_grouped_join_request(group_column: Value, key_type: Value) -> Value {
    let mut request =
        filtered_join_request(json!([group_column.clone(), aggregate("COUNT", None)]));
    request["pushdownRequest"]["selectListDataTypes"] = json!([key_type, count_type()]);
    request["pushdownRequest"]["aggregationType"] = json!("group_by");
    request["pushdownRequest"]["groupBy"] = json!([group_column]);
    request
}

/// A join whose left table the request aliases `t2`, the alias the dialect
/// derives for the right one.
pub(crate) fn request_aliased_join_request() -> Value {
    let mut from = orders_to_customers("inner");
    from["left"]["alias"] = json!("t2");
    from["condition"]["left"] = json!({
        "type": "column", "name": "customer_id", "tableName": "orders", "tableAlias": "t2",
    });
    join_request(
        from,
        json!([
            {"type": "column", "name": "id", "tableName": "orders", "tableAlias": "t2"},
            joined_column("id", "customers"),
        ]),
    )
}

/// `orders` joined to itself under the aliases `a` and `b`.
pub(crate) fn self_join_request() -> Value {
    json!({
        "type": "pushdown",
        "pushdownRequest": {
            "type": "select",
            "from": {
                "type": "join",
                "join_type": "inner",
                "left": {"type": "table", "name": "orders", "alias": "a"},
                "right": {"type": "table", "name": "orders", "alias": "b"},
                "condition": {
                    "type": "predicate_equal",
                    "left": {"type": "column", "name": "id", "tableName": "orders", "tableAlias": "a"},
                    "right": {"type": "column", "name": "customer_id", "tableName": "orders", "tableAlias": "b"},
                },
            },
            "selectList": [
                {"type": "column", "name": "id", "tableName": "orders", "tableAlias": "a"},
                {"type": "column", "name": "id", "tableName": "orders", "tableAlias": "b"},
            ],
        },
        "involvedTables": [
            {"name": "orders", "columns": joined_orders_columns()},
            {"name": "orders", "columns": joined_orders_columns()},
        ],
    })
}

/// Asserts a join column carrying no table reference fails to render, naming
/// `column_spelling` and both spellings it lacks.
pub(crate) fn assert_unreferenced_join_column_fails(
    render_error: &dyn Fn(&Value) -> String,
    column_spelling: &str,
) {
    let error = render_error(&join_request(
        orders_to_customers("inner"),
        json!([column("note")]),
    ));

    assert!(
        error.contains(column_spelling)
            && error.contains("tableName")
            && error.contains("tableAlias"),
        "the error must name the column and both spellings it lacks: {error}"
    );
}

/// Asserts `orders` joined to an unaliased `orders` fails to render, naming
/// `column_spelling` and the reference both tables answer to.
pub(crate) fn assert_ambiguous_join_reference_fails(
    render_error: &dyn Fn(&Value) -> String,
    column_spelling: &str,
) {
    let mut from = orders_to_customers("inner");
    from["right"] = json!({"type": "table", "name": "orders"});

    let error = render_error(&join_request(from, json!([joined_column("id", "orders")])));

    assert!(
        error.contains(column_spelling)
            && error.contains("orders")
            && error.contains("more than one"),
        "the error must name the column and the reference two tables answer to: {error}"
    );
}

/// The filters the BOOLEAN-column predicate test renders, in order.
pub(crate) fn boolean_predicate_filters() -> Vec<Value> {
    vec![
        column("flag"),
        column("active"),
        not(column("flag")),
        and(json!([column("flag"), column("active")])),
        or(json!([
            column("active"),
            less(numeric_literal("5"), column("id"))
        ])),
    ]
}

/// The clauses a comparison whose right operand is a predicate renders to
/// through `where_clause_over`.
pub(crate) fn predicate_operand_clauses(
    where_clause_over: &dyn Fn(Value, Value) -> String,
) -> Vec<String> {
    let columns = json!([
        {"name": "active", "dataType": {"type": "BOOLEAN"}},
        {"name": "id", "dataType": {"type": "DECIMAL", "precision": 19, "scale": 0}},
    ]);
    [
        less(numeric_literal("5"), column("id")),
        not(column("active")),
    ]
    .into_iter()
    .map(|operand| where_clause_over(columns.clone(), equal(column("active"), operand)))
    .collect()
}

/// The filtered, grouped, and joined statements over a converted `uid`
/// column, the grouped one also projecting the key itself when `select_key`.
pub(crate) fn column_position_statements(
    render_statement: &dyn Fn(&Value) -> String,
    notes: ConvertedNotes,
    select_key: bool,
) -> [String; 3] {
    let key = [(column("uid"), json!({"type": "VARCHAR", "size": 36}))];
    let aggregates = [
        (
            aggregate_over("MIN", "uid", false),
            json!({"type": "VARCHAR", "size": 36}),
        ),
        (
            aggregate_over("COUNT", "uid", true),
            json!({"type": "DECIMAL", "precision": 18, "scale": 0}),
        ),
    ];
    let (select_list, data_types): (Vec<Value>, Vec<Value>) = key
        .into_iter()
        .take(usize::from(select_key))
        .chain(aggregates)
        .unzip();

    [
        render_statement(&converted_column_filtered_request(notes)),
        render_statement(&converted_column_grouped_request(
            notes,
            Value::Array(select_list),
            Value::Array(data_types),
            "uid",
        )),
        render_statement(&converted_column_joined_request(notes)),
    ]
}

/// `SUM` and `SUM(DISTINCT)` over `total`.
pub(crate) fn sum_request() -> Value {
    typed_request(
        json!([
            aggregate("SUM", Some(column("total"))),
            distinct(aggregate("SUM", Some(column("total")))),
        ]),
        json!([sum_type(), sum_type()]),
    )
}

/// A request projecting `entries`, each paired with its declared type.
pub(crate) fn typed_entries_request(entries: Vec<(Value, Value)>) -> Value {
    let (select_list, data_types): (Vec<Value>, Vec<Value>) = entries.into_iter().unzip();
    typed_request(Value::Array(select_list), Value::Array(data_types))
}

/// A request projecting `entries`, each paired with its declared type, over a
/// `ratio` `DOUBLE` column alone.
pub(crate) fn ratio_request(entries: Vec<(Value, Value)>) -> Value {
    let mut request = typed_entries_request(entries);
    request["involvedTables"][0]["columns"] = json!([
        {"name": "ratio", "dataType": {"type": "DOUBLE"}},
    ]);
    request
}

/// `SUM` and `SUM(DISTINCT)` over `ratio`, each declared `DOUBLE`.
pub(crate) fn ratio_sum_entries() -> Vec<(Value, Value)> {
    vec![
        (aggregate("SUM", Some(column("ratio"))), double_type()),
        (
            distinct(aggregate("SUM", Some(column("ratio")))),
            double_type(),
        ),
    ]
}

/// The `SUM` and `AVG` entries over the character column `note` and the
/// character literal `'5'`, each declared `DOUBLE`.
pub(crate) fn character_sum_avg_entries() -> Vec<(Value, Value)> {
    [
        aggregate("SUM", Some(column("note"))),
        distinct(aggregate("SUM", Some(column("note")))),
        aggregate("AVG", Some(column("note"))),
        distinct(aggregate("AVG", Some(column("note")))),
        aggregate("SUM", Some(string_literal("5"))),
    ]
    .into_iter()
    .map(|entry| (entry, double_type()))
    .collect()
}

/// Each capability the dialect tests exercise, with the fragment the MySQL,
/// PostgreSQL, and Trino dialects must each put in the statement for it, `""` where a
/// dialect does not advertise it.
const EXERCISED_FRAGMENTS: &[(&str, [&str; 3])] = &[
    (
        "SELECTLIST_PROJECTION",
        [
            "SELECT `id` FROM",
            r#"SELECT "id" FROM"#,
            r#"SELECT "id" FROM"#,
        ],
    ),
    (
        "SELECTLIST_EXPRESSIONS",
        [
            "SELECT (`id` = 2)",
            r#"SELECT ("id" = 2)"#,
            r#"SELECT ("id" = 2)"#,
        ],
    ),
    (
        "FILTER_EXPRESSIONS",
        [
            "WHERE (`id` = 2)",
            r#"WHERE ("id" = 2)"#,
            r#"WHERE ("id" = 2)"#,
        ],
    ),
    (
        "AGGREGATE_SINGLE_GROUP",
        [
            "SELECT COUNT(*) FROM",
            "SELECT COUNT(*) FROM",
            "SELECT COUNT(*) FROM",
        ],
    ),
    (
        "AGGREGATE_GROUP_BY_COLUMN",
        ["GROUP BY `id`", r#"GROUP BY "id""#, r#"GROUP BY "id""#],
    ),
    (
        "AGGREGATE_GROUP_BY_TUPLE",
        ["", "", r#"GROUP BY "id", "note""#],
    ),
    (
        "FN_AGG_COUNT",
        ["COUNT(`id`)", r#"COUNT("id")"#, r#"COUNT("id")"#],
    ),
    ("FN_AGG_COUNT_STAR", ["COUNT(*)", "COUNT(*)", "COUNT(*)"]),
    (
        "FN_AGG_COUNT_DISTINCT",
        [
            "COUNT(DISTINCT `id`)",
            r#"COUNT(DISTINCT "id")"#,
            r#"COUNT(DISTINCT "id")"#,
        ],
    ),
    ("FN_AGG_MIN", ["MIN(`id`)", r#"MIN("id")"#, r#"MIN("id")"#]),
    ("FN_AGG_MAX", ["MAX(`id`)", r#"MAX("id")"#, r#"MAX("id")"#]),
    (
        "AGGREGATE_HAVING",
        [
            " HAVING (0 < COUNT(*))",
            " HAVING (0 < COUNT(*))",
            " HAVING (0 < COUNT(*))",
        ],
    ),
    (
        "FN_AGG_SUM",
        [
            "SUM(`id`)",
            r#"SUM("id")"#,
            r#"SUM(CAST("id" AS DECIMAL(38,0)))"#,
        ],
    ),
    (
        "FN_AGG_SUM_DISTINCT",
        [
            "SUM(DISTINCT `id`)",
            r#"SUM(DISTINCT "id")"#,
            r#"SUM(DISTINCT CAST("id" AS DECIMAL(38,0)))"#,
        ],
    ),
    (
        "FN_AGG_AVG",
        [
            "(CAST(SUM(`id`) AS DOUBLE) / COUNT(`id`))",
            r#"(CAST(SUM("id") AS DOUBLE PRECISION) / COUNT("id"))"#,
            r#"(CAST(SUM(CAST("id" AS DECIMAL(38,0))) AS DOUBLE) / COUNT("id"))"#,
        ],
    ),
    (
        "FN_AGG_AVG_DISTINCT",
        [
            "(CAST(SUM(DISTINCT `id`) AS DOUBLE) / COUNT(DISTINCT `id`))",
            r#"(CAST(SUM(DISTINCT "id") AS DOUBLE PRECISION) / COUNT(DISTINCT "id"))"#,
            r#"(CAST(SUM(DISTINCT CAST("id" AS DECIMAL(38,0))) AS DOUBLE) / COUNT(DISTINCT "id"))"#,
        ],
    ),
    (
        "FN_PRED_AND",
        [
            "((`id` = 1) AND (`id` = 2))",
            r#"(("id" = 1) AND ("id" = 2))"#,
            r#"(("id" = 1) AND ("id" = 2))"#,
        ],
    ),
    (
        "FN_PRED_OR",
        [
            "((`id` = 1) OR (`id` = 2))",
            r#"(("id" = 1) OR ("id" = 2))"#,
            r#"(("id" = 1) OR ("id" = 2))"#,
        ],
    ),
    (
        "FN_PRED_NOT",
        [
            "(NOT (`id` = 2))",
            r#"(NOT ("id" = 2))"#,
            r#"(NOT ("id" = 2))"#,
        ],
    ),
    (
        "FN_PRED_EQUAL",
        ["(`id` = 2)", r#"("id" = 2)"#, r#"("id" = 2)"#],
    ),
    (
        "FN_PRED_NOTEQUAL",
        ["(`id` <> 2)", r#"("id" <> 2)"#, r#"("id" <> 2)"#],
    ),
    (
        "FN_PRED_LESS",
        ["(`id` < 2)", r#"("id" < 2)"#, r#"("id" < 2)"#],
    ),
    (
        "FN_PRED_LESSEQUAL",
        ["(`id` <= 2)", r#"("id" <= 2)"#, r#"("id" <= 2)"#],
    ),
    (
        "FN_PRED_IS_NULL",
        ["(`id` IS NULL)", r#"("id" IS NULL)"#, r#"("id" IS NULL)"#],
    ),
    (
        "FN_PRED_IS_NOT_NULL",
        [
            "(`id` IS NOT NULL)",
            r#"("id" IS NOT NULL)"#,
            r#"("id" IS NOT NULL)"#,
        ],
    ),
    (
        "FN_PRED_LIKE",
        ["LIKE _utf8mb4 X'57696425'", "LIKE E'Wid%'", "LIKE 'Wid%')"],
    ),
    (
        "FN_PRED_BETWEEN",
        ["BETWEEN 1 AND 9", "BETWEEN 1 AND 9", "BETWEEN 1 AND 9"],
    ),
    (
        "FN_PRED_IN_CONSTLIST",
        ["IN (1, 2)", "IN (1, 2)", "IN (1, 2)"],
    ),
    ("LITERAL_NULL", ["= NULL", "= NULL", "= NULL"]),
    ("LITERAL_EXACTNUMERIC", ["= 2", "= 2", "= 2"]),
    ("LITERAL_DOUBLE", ["= 2.5", "= 2.5", "= DOUBLE '2.5'"]),
    (
        "LITERAL_STRING",
        ["_utf8mb4 X'776964676574'", "E'widget'", "'widget'"],
    ),
    (
        "LITERAL_DATE",
        [
            "DATE '2015-12-01'",
            "DATE '2015-12-01'",
            "DATE '2015-12-01'",
        ],
    ),
    (
        "LITERAL_TIMESTAMP",
        [
            "TIMESTAMP '2015-12-01 12:01:01.123'",
            "TIMESTAMP '2015-12-01 12:01:01.123'",
            "TIMESTAMP '2015-12-01 12:01:01.123'",
        ],
    ),
    (
        "JOIN",
        [
            "`shop`.`orders` AS `t1` INNER JOIN `shop`.`customers` AS `t2`",
            r#""public"."orders" AS "t1" INNER JOIN "public"."customers" AS "t2""#,
            r#""memory"."adbc_vs"."orders" AS "t1" INNER JOIN "memory"."adbc_vs"."customers" AS "t2""#,
        ],
    ),
    (
        "JOIN_TYPE_INNER",
        ["INNER JOIN", "INNER JOIN", "INNER JOIN"],
    ),
    (
        "JOIN_TYPE_LEFT_OUTER",
        ["LEFT OUTER JOIN", "LEFT OUTER JOIN", "LEFT OUTER JOIN"],
    ),
    (
        "JOIN_TYPE_RIGHT_OUTER",
        ["RIGHT OUTER JOIN", "RIGHT OUTER JOIN", "RIGHT OUTER JOIN"],
    ),
    (
        "JOIN_TYPE_FULL_OUTER",
        ["", "FULL OUTER JOIN", "FULL OUTER JOIN"],
    ),
    (
        "JOIN_CONDITION_EQUI",
        [
            "ON (`t1`.`customer_id` = `t2`.`id`)",
            r#"ON ("t1"."customer_id" = "t2"."id")"#,
            r#"ON ("t1"."customer_id" = "t2"."id")"#,
        ],
    ),
    (
        "FN_PRED_LIKE_ESCAPE",
        ["ESCAPE _utf8mb4 X'5C'", r"ESCAPE E'\\'", ""],
    ),
];

/// The request model that exercises `capability` over `namespace`, and the
/// fragment `driver`'s dialect must put in the statement for it. The request
/// shapes are the protocol's, so one table builds them for every dialect, and
/// each dialect's own expectation is its column of [`EXERCISED_FRAGMENTS`].
pub(crate) fn query_exercising(
    driver: Driver,
    namespace: &SourceQualifier,
    capability: &str,
) -> (&'static str, Query) {
    let from_clause = || from(namespace);
    let filtered_query = |filter: Predicate| filtered(namespace, filter);
    let aggregated_query = |select_list: Vec<SelectItem>| aggregated(namespace, select_list);
    let joined_for = |join_type: &str| joined_query(namespace, join_type);
    let unexercised = || -> ! {
        panic!(
            "capability `{capability}` is advertised but this test has no fixture that exercises it; \
             add one alongside whichever task added the capability"
        )
    };

    let column = match driver {
        Driver::MySql => 0,
        Driver::Postgres => 1,
        Driver::Trino => 2,
    };
    let fragment = EXERCISED_FRAGMENTS
        .iter()
        .find(|(name, _)| *name == capability)
        .map_or("", |(_, fragments)| fragments[column]);
    if fragment.is_empty() {
        unexercised();
    }

    let query = match capability {
        "SELECTLIST_PROJECTION" => Query {
            select_list: select_list(),
            filter: None,
            aggregation: None,
            having: None,
            from: from_clause(),
        },
        "SELECTLIST_EXPRESSIONS" => Query {
            select_list: vec![SelectItem {
                expression: Expression::Predicate(Box::new(id_equals(numeric("2")))),
                declared_type: ExasolType::Boolean,
            }],
            filter: None,
            aggregation: None,
            having: None,
            from: from_clause(),
        },
        "FILTER_EXPRESSIONS" => filtered_query(id_equals(numeric("2"))),
        "AGGREGATE_SINGLE_GROUP" => aggregated_query(vec![aggregate_item(count_star())]),
        "AGGREGATE_GROUP_BY_COLUMN" => Query {
            select_list: select_list(),
            filter: None,
            aggregation: Some(pushdown_query::Aggregation::GroupBy(vec![decimal_column()])),
            having: None,
            from: from_clause(),
        },
        "AGGREGATE_GROUP_BY_TUPLE" => Query {
            select_list: vec![
                SelectItem {
                    expression: Expression::Column(decimal_column()),
                    declared_type: decimal_column().declared_type,
                },
                SelectItem {
                    expression: Expression::Column(varchar_column()),
                    declared_type: varchar_column().declared_type,
                },
            ],
            filter: None,
            aggregation: Some(pushdown_query::Aggregation::GroupBy(vec![
                decimal_column(),
                varchar_column(),
            ])),
            having: None,
            from: from_clause(),
        },
        "FN_AGG_COUNT" => aggregated_query(vec![aggregate_item(aggregate_call(
            "COUNT",
            Expression::Column(decimal_column()),
            false,
        ))]),
        "FN_AGG_COUNT_STAR" => aggregated_query(vec![aggregate_item(count_star())]),
        "FN_AGG_COUNT_DISTINCT" => aggregated_query(vec![aggregate_item(aggregate_call(
            "COUNT",
            Expression::Column(decimal_column()),
            true,
        ))]),
        "FN_AGG_MIN" => aggregated_query(vec![aggregate_item(aggregate_call(
            "MIN",
            Expression::Column(decimal_column()),
            false,
        ))]),
        "FN_AGG_MAX" => aggregated_query(vec![aggregate_item(aggregate_call(
            "MAX",
            Expression::Column(decimal_column()),
            false,
        ))]),
        "AGGREGATE_HAVING" => Query {
            having: Some(Predicate::Less(Comparison {
                left: numeric("0"),
                right: Expression::Aggregate(count_star()),
            })),
            ..aggregated_query(vec![aggregate_item(count_star())])
        },
        "FN_AGG_SUM" => aggregated_query(vec![aggregate_item(aggregate_call(
            "SUM",
            Expression::Column(decimal_column()),
            false,
        ))]),
        "FN_AGG_SUM_DISTINCT" => aggregated_query(vec![aggregate_item(aggregate_call(
            "SUM",
            Expression::Column(decimal_column()),
            true,
        ))]),
        "FN_AGG_AVG" => aggregated_query(vec![aggregate_item(aggregate_call(
            "AVG",
            Expression::Column(decimal_column()),
            false,
        ))]),
        "FN_AGG_AVG_DISTINCT" => aggregated_query(vec![aggregate_item(aggregate_call(
            "AVG",
            Expression::Column(decimal_column()),
            true,
        ))]),
        "FN_PRED_AND" => filtered_query(Predicate::And(vec![
            id_equals(numeric("1")),
            id_equals(numeric("2")),
        ])),
        "FN_PRED_OR" => filtered_query(Predicate::Or(vec![
            id_equals(numeric("1")),
            id_equals(numeric("2")),
        ])),
        "FN_PRED_NOT" => filtered_query(Predicate::Not(Box::new(id_equals(numeric("2"))))),
        "FN_PRED_EQUAL" => filtered_query(id_equals(numeric("2"))),
        "FN_PRED_NOTEQUAL" => filtered_query(Predicate::NotEqual(Comparison {
            left: Expression::Column(decimal_column()),
            right: numeric("2"),
        })),
        "FN_PRED_LESS" => filtered_query(Predicate::Less(Comparison {
            left: Expression::Column(decimal_column()),
            right: numeric("2"),
        })),
        "FN_PRED_LESSEQUAL" => filtered_query(Predicate::LessEqual(Comparison {
            left: Expression::Column(decimal_column()),
            right: numeric("2"),
        })),
        "FN_PRED_IS_NULL" => {
            filtered_query(Predicate::IsNull(Expression::Column(decimal_column())))
        }
        "FN_PRED_IS_NOT_NULL" => {
            filtered_query(Predicate::IsNotNull(Expression::Column(decimal_column())))
        }
        "FN_PRED_LIKE" => filtered_query(Predicate::Like(LikePredicate {
            expression: Expression::Column(varchar_column()),
            pattern: Expression::Literal(Literal::String("Wid%".to_string())),
            escape_char: None,
        })),
        "FN_PRED_BETWEEN" => filtered_query(Predicate::Between(BetweenPredicate {
            expression: Expression::Column(decimal_column()),
            left: numeric("1"),
            right: numeric("9"),
        })),
        "FN_PRED_IN_CONSTLIST" => filtered_query(Predicate::InConstList(InConstListPredicate {
            expression: Expression::Column(decimal_column()),
            arguments: vec![numeric("1"), numeric("2")],
        })),
        "LITERAL_NULL" => filtered_query(id_equals(Expression::Literal(Literal::Null))),
        "LITERAL_EXACTNUMERIC" => filtered_query(id_equals(numeric("2"))),
        "LITERAL_DOUBLE" => filtered_query(id_equals(Expression::Literal(Literal::Double(
            "2.5".to_string(),
        )))),
        "LITERAL_STRING" => filtered_query(Predicate::Equal(Comparison {
            left: Expression::Column(varchar_column()),
            right: Expression::Literal(Literal::String("widget".to_string())),
        })),
        "LITERAL_DATE" => filtered_query(Predicate::Equal(Comparison {
            left: Expression::Column(date_column()),
            right: Expression::Literal(Literal::Date("2015-12-01".to_string())),
        })),
        "LITERAL_TIMESTAMP" => filtered_query(Predicate::Equal(Comparison {
            left: Expression::Column(timestamp_column()),
            right: Expression::Literal(Literal::Timestamp("2015-12-01 12:01:01.123".to_string())),
        })),
        "JOIN" => joined_for("inner"),
        "JOIN_TYPE_INNER" => joined_for("inner"),
        "JOIN_TYPE_LEFT_OUTER" => joined_for("left_outer"),
        "JOIN_TYPE_RIGHT_OUTER" => joined_for("right_outer"),
        "JOIN_TYPE_FULL_OUTER" => joined_for("full_outer"),
        "JOIN_CONDITION_EQUI" => joined_for("inner"),
        "FN_PRED_LIKE_ESCAPE" => filtered_query(Predicate::Like(LikePredicate {
            expression: Expression::Column(varchar_column()),
            pattern: Expression::Literal(Literal::String("Wid%".to_string())),
            escape_char: Some(Expression::Literal(Literal::String("\\".to_string()))),
        })),
        _ => unexercised(),
    };
    (fragment, query)
}

/// Asserts the `DATE` and `TIMESTAMP` equality filters render to `expected`
/// through `where_clause_over`.
pub(crate) fn assert_temporal_literals_render_typed(
    where_clause_over: &dyn Fn(Value, Value) -> String,
    expected: &str,
) {
    let clause = where_clause_over(
        temporal_columns(),
        and(json!([date_filter(), timestamp_filter()])),
    );

    assert_eq!(clause, expected);
}

/// Asserts lower-case and upper-case aggregate function names render to the
/// same statement, `expected`.
pub(crate) fn assert_mixed_case_renders_identically(
    render_statement: &dyn Fn(&Value) -> String,
    count_column: &str,
    expected: &str,
) {
    let (lower_case, upper_case) = mixed_case_statements(render_statement, count_column);

    assert_eq!(
        lower_case, upper_case,
        "two requests differing only in the aggregate function's case must render identically"
    );
    assert_eq!(lower_case, expected);
}

/// Asserts the grouped and the single-group `HAVING` requests render to
/// `expected`, in that order.
pub(crate) fn assert_having_renders(
    render_statement: &dyn Fn(&Value) -> String,
    expected: [&str; 2],
) {
    let (grouped, single) = having_requests();

    assert_eq!(render_statement(&grouped), expected[0]);
    assert_eq!(render_statement(&single), expected[1]);
}

/// Asserts `AVG` renders to `expected` and never as the source's own `AVG`.
pub(crate) fn assert_avg_renders_as_quotient(
    render_statement: &dyn Fn(&Value) -> String,
    expected: &str,
    dialect_name: &str,
) {
    let sql = render_statement(&avg_request());

    assert_eq!(sql, expected);
    assert!(
        !sql.contains("AVG("),
        "{dialect_name}'s own AVG must not render: {sql}"
    );
}

/// Asserts the converted-column statements of [`column_position_statements`]
/// equal `expected`.
pub(crate) fn assert_column_positions(
    render_statement: &dyn Fn(&Value) -> String,
    notes: ConvertedNotes,
    select_key: bool,
    expected: [&str; 3],
) {
    assert_eq!(
        column_position_statements(render_statement, notes, select_key),
        expected
    );
}

/// Asserts each BOOLEAN-column filter renders as `prefix` followed by the
/// matching `expected` clause.
pub(crate) fn assert_boolean_predicates(
    render_statement: &dyn Fn(&Value) -> String,
    notes: ConvertedNotes,
    prefix: &str,
    expected: [&str; 5],
) {
    for (filter, clause) in boolean_predicate_filters().into_iter().zip(expected) {
        let sql = render_statement(&converted_table_filtered_request(notes, filter));

        assert_eq!(sql, format!("{prefix}{clause}"));
    }
}
