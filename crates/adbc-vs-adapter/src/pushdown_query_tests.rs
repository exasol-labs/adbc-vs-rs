use serde_json::json;

use super::*;

const DATABASE: &str = "shop";

/// The one-segment qualifier a MySQL virtual schema records, as the model
/// carries it.
fn namespace() -> SourceQualifier {
    SourceQualifier::single(DATABASE)
}

/// A single-table `pushdown` request over `orders`, carrying the three
/// involved columns whose types every assertion below reads back.
fn request(select_list: Option<Value>) -> Value {
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

fn involved_column(name: &str, data_type: Value) -> Value {
    json!({
        "type": "pushdown",
        "pushdownRequest": {"type": "select", "from": {"type": "table", "name": "orders"}},
        "involvedTables": [{"name": "orders", "columns": [{"name": name, "dataType": data_type}]}],
    })
}

/// A reference to one of the three `orders` columns, carrying the type the
/// involved table declares for it, because the parser resolves the type
/// alongside the name.
fn column(name: &str) -> Expression {
    let declared_type = match name {
        "id" => ExasolType::Decimal {
            precision: 19,
            scale: 0,
        },
        "total" => ExasolType::Decimal {
            precision: 12,
            scale: 4,
        },
        "note" => ExasolType::Varchar { size: 100 },
        other => panic!("the `orders` fixture declares no column `{other}`"),
    };

    Expression::Column(ColumnRef {
        name: name.to_string(),
        declared_type,
        table_name: None,
        table_alias: None,
    })
}

fn id_item() -> SelectItem {
    SelectItem {
        expression: column("id"),
        declared_type: ExasolType::Decimal {
            precision: 19,
            scale: 0,
        },
    }
}

fn total_item() -> SelectItem {
    SelectItem {
        expression: column("total"),
        declared_type: ExasolType::Decimal {
            precision: 12,
            scale: 4,
        },
    }
}

fn note_item() -> SelectItem {
    SelectItem {
        expression: column("note"),
        declared_type: ExasolType::Varchar { size: 100 },
    }
}

fn parse_failure(request: &Value) -> String {
    parse(request, &namespace()).expect_err("the request must fail to parse")
}

#[test]
fn the_from_tree_names_the_source_table_and_the_recorded_database() {
    let query = parse(&request(None), &namespace()).expect("parse must succeed");

    assert_eq!(
        query.from,
        FromClause::Table(SourceTable {
            namespace: SourceQualifier::single("shop"),
            name: "orders".to_string(),
            alias: None,
        })
    );
}

#[test]
fn an_absent_select_list_projects_every_involved_column_in_declared_order() {
    let query = parse(&request(None), &namespace()).expect("parse must succeed");

    assert_eq!(
        query.select_list,
        vec![id_item(), total_item(), note_item()]
    );
}

#[test]
fn a_named_select_list_keeps_the_request_order_and_prunes_the_rest() {
    let select_list = json!([
        {"type": "column", "name": "note"},
        {"type": "column", "name": "id"},
    ]);

    let query = parse(&request(Some(select_list)), &namespace()).expect("parse must succeed");

    assert_eq!(query.select_list, vec![note_item(), id_item()]);
}

#[test]
fn an_empty_select_list_projects_one_row_marker_declaring_decimal_one_zero() {
    let query = parse(&request(Some(json!([]))), &namespace()).expect("parse must succeed");

    assert_eq!(
        query.select_list,
        vec![SelectItem {
            expression: Expression::RowMarker,
            declared_type: ExasolType::Decimal {
                precision: 1,
                scale: 0,
            },
        }]
    );
}

#[test]
fn a_select_list_naming_a_column_the_involved_table_lacks_names_it_in_the_error() {
    let select_list = json!([{"type": "column", "name": "shipped_at"}]);

    let message = parse_failure(&request(Some(select_list)));

    assert!(
        message.contains("shipped_at"),
        "the error must name the column it could not resolve: {message}"
    );
}

#[test]
fn a_select_list_that_is_not_an_array_names_the_field_in_the_error() {
    let message = parse_failure(&request(Some(json!("id"))));

    assert!(
        message.contains("selectList"),
        "the error must name the field it could not read: {message}"
    );
}

#[test]
fn a_select_list_entry_carrying_no_type_field_is_reported() {
    let message = parse_failure(&request(Some(json!([{"name": "id"}]))));

    assert!(
        message.contains("type"),
        "the error must state that the entry carries no `type` field: {message}"
    );
}

#[test]
fn an_order_by_element_is_refused() {
    let mut request = request(None);
    request["pushdownRequest"]["orderBy"] =
        json!([{"expression": {"type": "column", "name": "id"}, "isAscending": true}]);

    let message = parse_failure(&request);

    assert!(
        message.contains("orderBy"),
        "the error must name the refused element: {message}"
    );
    assert!(
        message.contains("ORDER_BY_COLUMN"),
        "the error must name the withheld capabilities that gate the element: {message}"
    );
}

#[test]
fn a_limit_element_is_refused() {
    let mut request = request(None);
    request["pushdownRequest"]["limit"] = json!({"numElements": 10});

    let message = parse_failure(&request);

    assert!(
        message.contains("limit"),
        "the error must name the refused element: {message}"
    );
    assert!(
        message.contains("LIMIT_WITH_OFFSET"),
        "the error must name the withheld capabilities that gate the element: {message}"
    );
}

/// `AGGREGATE_HAVING` is withheld for the reason the ordering capabilities
/// are: Exasol applies the clause itself at no correctness cost, and no
/// renderer here writes it. Reading past the element instead would return a
/// statement Exasol believes has `HAVING` applied.
#[test]
fn a_having_element_is_refused() {
    let mut request = request(None);
    request["pushdownRequest"]["having"] = json!({
        "type": "predicate_less",
        "left": {"type": "literal_exactnumeric", "value": "1"},
        "right": {"type": "function_aggregate", "name": "COUNT"},
    });

    let message = parse_failure(&request);

    assert!(
        message.contains("having"),
        "the error must name the refused element: {message}"
    );
    assert!(
        message.contains("AGGREGATE_HAVING"),
        "the error must name the withheld capability that gates the element: {message}"
    );
}

#[test]
fn a_select_list_entry_naming_no_column_is_reported_with_the_entry() {
    let message = parse_failure(&request(Some(json!([{"type": "column"}]))));

    assert!(
        message.contains("names no column"),
        "the error must state what the entry is missing: {message}"
    );
}

#[test]
fn a_request_carrying_no_from_element_names_it_in_the_error() {
    let mut request = request(None);
    request["pushdownRequest"]
        .as_object_mut()
        .expect("pushdownRequest must be an object")
        .remove("from");

    let message = parse_failure(&request);

    assert!(
        message.contains("from"),
        "the error must name the missing element: {message}"
    );
}

#[test]
fn a_request_naming_no_involved_table_is_reported() {
    let request = json!({
        "type": "pushdown",
        "pushdownRequest": {"type": "select", "from": {"type": "table", "name": "orders"}},
    });

    let message = parse_failure(&request);

    assert!(
        message.contains("involved table"),
        "the error must state that no involved table was supplied: {message}"
    );
}

#[test]
fn an_involved_table_carrying_no_columns_field_is_reported() {
    let request = json!({
        "type": "pushdown",
        "pushdownRequest": {"type": "select", "from": {"type": "table", "name": "orders"}},
        "involvedTables": [{"name": "orders"}],
    });

    let message = parse_failure(&request);

    assert!(
        message.contains("columns"),
        "the error must state that no columns were supplied: {message}"
    );
}

#[test]
fn an_involved_table_declaring_no_columns_fails_rather_than_projecting_nothing() {
    let request = json!({
        "type": "pushdown",
        "pushdownRequest": {"type": "select", "from": {"type": "table", "name": "orders"}},
        "involvedTables": [{"name": "orders", "columns": []}],
    });

    let message = parse_failure(&request);

    assert!(
        message.contains("orders"),
        "the error must name the table that declares no column: {message}"
    );
}

#[test]
fn an_involved_column_carrying_no_name_is_reported() {
    let request = json!({
        "type": "pushdown",
        "pushdownRequest": {"type": "select", "from": {"type": "table", "name": "orders"}},
        "involvedTables": [{
            "name": "orders",
            "columns": [{"dataType": {"type": "DATE"}}],
        }],
    });

    let message = parse_failure(&request);

    assert!(
        message.contains("name"),
        "the error must state that a column name is missing: {message}"
    );
}

#[test]
fn an_involved_column_carrying_no_data_type_names_the_column_in_the_error() {
    let request = json!({
        "type": "pushdown",
        "pushdownRequest": {"type": "select", "from": {"type": "table", "name": "orders"}},
        "involvedTables": [{"name": "orders", "columns": [{"name": "id"}]}],
    });

    let message = parse_failure(&request);

    assert!(
        message.contains("id") && message.contains("dataType"),
        "the error must name the column and the missing field: {message}"
    );
}

/// `GEOMETRY` rather than `BOOLEAN`: `BOOLEAN` is a declarable Exasol type,
/// so the parser accepts it, which `exa_type`'s own tests pin.
#[test]
fn an_involved_column_of_an_undeclarable_exasol_type_is_reported() {
    let message = parse_failure(&involved_column("shape", json!({"type": "GEOMETRY"})));

    assert!(
        message.contains("GEOMETRY"),
        "the error must name the type it cannot declare: {message}"
    );
}

fn request_with_filter(filter: Value) -> Value {
    let mut request = request(None);
    request["pushdownRequest"]["filter"] = filter;
    request
}

fn filter_of(request: &Value, namespace: &SourceQualifier) -> Predicate {
    parse(request, namespace)
        .expect("parse must succeed")
        .filter
        .expect("the request carries a filter")
}

#[test]
fn an_absent_filter_element_parses_to_no_filter() {
    let query = parse(&request(None), &namespace()).expect("parse must succeed");

    assert_eq!(query.filter, None);
}

#[test]
fn a_swapped_comparison_keeps_its_operand_order() {
    let filter = json!({
        "type": "predicate_less",
        "left": {"type": "literal_exactnumeric", "value": "5"},
        "right": {"type": "column", "name": "id"},
    });

    let predicate = filter_of(&request_with_filter(filter), &namespace());

    assert_eq!(
        predicate,
        Predicate::Less(Comparison {
            left: Expression::Literal(Literal::ExactNumeric("5".to_string())),
            right: column("id"),
        })
    );
}

#[test]
fn each_comparison_kind_parses_its_own_variant() {
    let cases = [
        (
            "predicate_equal",
            Predicate::Equal as fn(Comparison) -> Predicate,
        ),
        ("predicate_notequal", Predicate::NotEqual),
        ("predicate_less", Predicate::Less),
        ("predicate_lessequal", Predicate::LessEqual),
    ];

    for (predicate_type, variant) in cases {
        let filter = json!({
            "type": predicate_type,
            "left": {"type": "column", "name": "id"},
            "right": {"type": "literal_exactnumeric", "value": "2"},
        });

        let predicate = filter_of(&request_with_filter(filter), &namespace());

        assert_eq!(
            predicate,
            variant(Comparison {
                left: column("id"),
                right: Expression::Literal(Literal::ExactNumeric("2".to_string())),
            }),
            "`{predicate_type}` must parse into its matching Predicate variant"
        );
    }
}

#[test]
fn and_or_nest_their_expressions_list_and_not_wraps_a_single_predicate() {
    let filter = json!({
        "type": "predicate_not",
        "expression": {
            "type": "predicate_or",
            "expressions": [
                {
                    "type": "predicate_and",
                    "expressions": [
                        {
                            "type": "predicate_equal",
                            "left": {"type": "column", "name": "id"},
                            "right": {"type": "literal_exactnumeric", "value": "1"},
                        },
                        {
                            "type": "predicate_equal",
                            "left": {"type": "column", "name": "total"},
                            "right": {"type": "literal_exactnumeric", "value": "2"},
                        },
                    ],
                },
                {
                    "type": "predicate_is_null",
                    "expression": {"type": "column", "name": "note"},
                },
            ],
        },
    });

    let predicate = filter_of(&request_with_filter(filter), &namespace());

    assert_eq!(
        predicate,
        Predicate::Not(Box::new(Predicate::Or(vec![
            Predicate::And(vec![
                Predicate::Equal(Comparison {
                    left: column("id"),
                    right: Expression::Literal(Literal::ExactNumeric("1".to_string())),
                }),
                Predicate::Equal(Comparison {
                    left: column("total"),
                    right: Expression::Literal(Literal::ExactNumeric("2".to_string())),
                }),
            ]),
            Predicate::IsNull(column("note")),
        ])))
    );
}

#[test]
fn is_null_and_is_not_null_wrap_the_operand_expression() {
    let is_null =
        json!({"type": "predicate_is_null", "expression": {"type": "column", "name": "note"}});
    let is_not_null =
        json!({"type": "predicate_is_not_null", "expression": {"type": "column", "name": "note"}});

    assert_eq!(
        filter_of(&request_with_filter(is_null), &namespace()),
        Predicate::IsNull(column("note"))
    );
    assert_eq!(
        filter_of(&request_with_filter(is_not_null), &namespace()),
        Predicate::IsNotNull(column("note"))
    );
}

#[test]
fn like_carries_its_pattern_and_an_optional_escape_char() {
    let with_escape = json!({
        "type": "predicate_like",
        "expression": {"type": "column", "name": "note"},
        "pattern": {"type": "literal_string", "value": "Wid%"},
        "escapeChar": {"type": "literal_string", "value": "\\"},
    });
    let without_escape = json!({
        "type": "predicate_like",
        "expression": {"type": "column", "name": "note"},
        "pattern": {"type": "literal_string", "value": "Wid%"},
    });

    assert_eq!(
        filter_of(&request_with_filter(with_escape), &namespace()),
        Predicate::Like(LikePredicate {
            expression: column("note"),
            pattern: Expression::Literal(Literal::String("Wid%".to_string())),
            escape_char: Some(Expression::Literal(Literal::String("\\".to_string()))),
        })
    );
    assert_eq!(
        filter_of(&request_with_filter(without_escape), &namespace()),
        Predicate::Like(LikePredicate {
            expression: column("note"),
            pattern: Expression::Literal(Literal::String("Wid%".to_string())),
            escape_char: None,
        })
    );
}

#[test]
fn between_keeps_its_bounds_in_request_order() {
    let filter = json!({
        "type": "predicate_between",
        "expression": {"type": "column", "name": "id"},
        "left": {"type": "literal_exactnumeric", "value": "1"},
        "right": {"type": "literal_exactnumeric", "value": "9"},
    });

    let predicate = filter_of(&request_with_filter(filter), &namespace());

    assert_eq!(
        predicate,
        Predicate::Between(BetweenPredicate {
            expression: column("id"),
            left: Expression::Literal(Literal::ExactNumeric("1".to_string())),
            right: Expression::Literal(Literal::ExactNumeric("9".to_string())),
        })
    );
}

#[test]
fn in_constlist_keeps_its_arguments_in_request_order() {
    let filter = json!({
        "type": "predicate_in_constlist",
        "expression": {"type": "column", "name": "id"},
        "arguments": [
            {"type": "literal_exactnumeric", "value": "3"},
            {"type": "literal_exactnumeric", "value": "1"},
            {"type": "literal_exactnumeric", "value": "2"},
        ],
    });

    let predicate = filter_of(&request_with_filter(filter), &namespace());

    assert_eq!(
        predicate,
        Predicate::InConstList(InConstListPredicate {
            expression: column("id"),
            arguments: vec![
                Expression::Literal(Literal::ExactNumeric("3".to_string())),
                Expression::Literal(Literal::ExactNumeric("1".to_string())),
                Expression::Literal(Literal::ExactNumeric("2".to_string())),
            ],
        })
    );
}

#[test]
fn every_delegated_literal_kind_parses_into_its_matching_variant() {
    let cases = [
        (json!({"type": "literal_null"}), Literal::Null),
        (
            json!({"type": "literal_exactnumeric", "value": "123.45"}),
            Literal::ExactNumeric("123.45".to_string()),
        ),
        (
            json!({"type": "literal_double", "value": "1.5e+02"}),
            Literal::Double("1.5e+02".to_string()),
        ),
        (
            json!({"type": "literal_string", "value": "widget"}),
            Literal::String("widget".to_string()),
        ),
        (
            json!({"type": "literal_date", "value": "2015-12-01"}),
            Literal::Date("2015-12-01".to_string()),
        ),
        (
            json!({"type": "literal_timestamp", "value": "2015-12-01 12:01:01.123"}),
            Literal::Timestamp("2015-12-01 12:01:01.123".to_string()),
        ),
    ];

    for (literal, expected) in cases {
        let filter = json!({
            "type": "predicate_is_null",
            "expression": literal.clone(),
        });

        let predicate = filter_of(&request_with_filter(filter), &namespace());

        assert_eq!(
            predicate,
            Predicate::IsNull(Expression::Literal(expected.clone())),
            "{literal} must parse into {expected:?}"
        );
    }
}

#[test]
fn a_filter_predicate_carrying_no_type_field_is_reported() {
    let message = parse_failure(&request_with_filter(
        json!({"left": {"type": "column", "name": "id"}}),
    ));

    assert!(
        message.contains("type"),
        "the error must state that the predicate carries no `type` field: {message}"
    );
}

#[test]
fn a_filter_predicate_of_an_unrenderable_type_names_it_in_the_error() {
    let message = parse_failure(&request_with_filter(json!({"type": "predicate_greater"})));

    assert!(
        message.contains("predicate_greater"),
        "the error must name the predicate type it cannot render: {message}"
    );
}

#[test]
fn a_filter_expression_of_an_unrenderable_type_names_it_in_the_error() {
    let filter = json!({
        "type": "predicate_is_null",
        "expression": {"type": "function_scalar", "name": "UPPER"},
    });

    let message = parse_failure(&request_with_filter(filter));

    assert!(
        message.contains("function_scalar"),
        "the error must name the expression type it cannot render: {message}"
    );
}

#[test]
fn a_filter_expression_naming_a_column_the_involved_table_lacks_names_it_in_the_error() {
    let filter = json!({
        "type": "predicate_is_null",
        "expression": {"type": "column", "name": "shipped_at"},
    });

    let message = parse_failure(&request_with_filter(filter));

    assert!(
        message.contains("shipped_at"),
        "the error must name the column it could not resolve: {message}"
    );
}

#[test]
fn a_filter_predicate_missing_its_expressions_array_is_reported() {
    let message = parse_failure(&request_with_filter(json!({"type": "predicate_and"})));

    assert!(
        message.contains("expressions"),
        "the error must name the missing field: {message}"
    );
}

#[test]
fn a_literal_missing_its_value_field_is_reported() {
    let filter = json!({
        "type": "predicate_is_null",
        "expression": {"type": "literal_exactnumeric"},
    });

    let message = parse_failure(&request_with_filter(filter));

    assert!(
        message.contains("value"),
        "the error must name the missing field: {message}"
    );
}

fn request_with_typed_select_list(select_list: Value, data_types: Value) -> Value {
    let mut request = request(Some(select_list));
    request["pushdownRequest"]["selectListDataTypes"] = data_types;
    request
}

#[test]
fn aggregation_type_single_group_parses_with_no_group_by_columns() {
    let mut request = request(None);
    request["pushdownRequest"]["aggregationType"] = json!("single_group");

    let query = parse(&request, &namespace()).expect("parse must succeed");

    assert_eq!(query.aggregation, Some(Aggregation::SingleGroup));
}

#[test]
fn aggregation_type_group_by_parses_its_columns_in_request_order() {
    let mut request = request(None);
    request["pushdownRequest"]["aggregationType"] = json!("group_by");
    request["pushdownRequest"]["groupBy"] = json!([
        {"type": "column", "name": "note"},
        {"type": "column", "name": "id"},
    ]);

    let query = parse(&request, &namespace()).expect("parse must succeed");

    assert_eq!(
        query.aggregation,
        Some(Aggregation::GroupBy(vec![
            ColumnRef {
                name: "note".to_string(),
                declared_type: ExasolType::Varchar { size: 100 },
                table_name: None,
                table_alias: None,
            },
            ColumnRef {
                name: "id".to_string(),
                declared_type: ExasolType::Decimal {
                    precision: 19,
                    scale: 0,
                },
                table_name: None,
                table_alias: None,
            },
        ]))
    );
}

#[test]
fn an_absent_aggregation_type_parses_to_no_aggregation() {
    let query = parse(&request(None), &namespace()).expect("parse must succeed");

    assert_eq!(query.aggregation, None);
}

#[test]
fn a_group_by_aggregation_type_with_no_group_by_field_is_reported() {
    let mut request = request(None);
    request["pushdownRequest"]["aggregationType"] = json!("group_by");

    let message = parse_failure(&request);

    assert!(
        message.contains("groupBy"),
        "the error must name the missing field: {message}"
    );
}

#[test]
fn a_group_by_entry_of_an_unsupported_type_is_reported() {
    let mut request = request(None);
    request["pushdownRequest"]["aggregationType"] = json!("group_by");
    request["pushdownRequest"]["groupBy"] = json!([{"type": "function_scalar", "name": "UPPER"}]);

    let message = parse_failure(&request);

    assert!(
        message.contains("function_scalar"),
        "the error must name the unsupported groupBy entry type: {message}"
    );
}

#[test]
fn a_group_by_entry_naming_a_column_the_involved_table_lacks_is_reported() {
    let mut request = request(None);
    request["pushdownRequest"]["aggregationType"] = json!("group_by");
    request["pushdownRequest"]["groupBy"] = json!([{"type": "column", "name": "shipped_at"}]);

    let message = parse_failure(&request);

    assert!(
        message.contains("shipped_at"),
        "the error must name the column it could not resolve: {message}"
    );
}

#[test]
fn an_unrecognized_aggregation_type_is_reported() {
    let mut request = request(None);
    request["pushdownRequest"]["aggregationType"] = json!("group_by_expression");

    let message = parse_failure(&request);

    assert!(
        message.contains("group_by_expression"),
        "the error must name the unrecognized aggregationType: {message}"
    );
}

#[test]
fn a_function_aggregate_select_item_parses_its_name_argument_and_distinct_flag() {
    let select_list = json!([{
        "type": "function_aggregate",
        "name": "sum",
        "distinct": true,
        "arguments": [{"type": "column", "name": "total"}],
    }]);
    let data_types = json!([{"type": "DECIMAL", "precision": 23, "scale": 4}]);

    let query = parse(
        &request_with_typed_select_list(select_list, data_types),
        &namespace(),
    )
    .expect("parse must succeed");

    assert_eq!(
        query.select_list,
        vec![SelectItem {
            expression: Expression::Aggregate(AggregateCall {
                function: "SUM".to_string(),
                argument: Some(Box::new(column("total"))),
                distinct: true,
            }),
            declared_type: ExasolType::Decimal {
                precision: 23,
                scale: 4,
            },
        }]
    );
}

#[test]
fn a_function_aggregate_carrying_no_arguments_parses_with_no_argument() {
    let select_list = json!([{"type": "function_aggregate", "name": "COUNT"}]);
    let data_types = json!([{"type": "DECIMAL", "precision": 18, "scale": 0}]);

    let query = parse(
        &request_with_typed_select_list(select_list, data_types),
        &namespace(),
    )
    .expect("parse must succeed");

    assert_eq!(
        query.select_list[0].expression,
        Expression::Aggregate(AggregateCall {
            function: "COUNT".to_string(),
            argument: None,
            distinct: false,
        })
    );
}

#[test]
fn a_function_aggregate_name_is_matched_case_insensitively() {
    let lower = json!([{
        "type": "function_aggregate",
        "name": "avg",
        "arguments": [{"type": "column", "name": "total"}],
    }]);
    let upper = json!([{
        "type": "function_aggregate",
        "name": "AVG",
        "arguments": [{"type": "column", "name": "total"}],
    }]);
    let data_types = json!([{"type": "DOUBLE"}]);

    let lower_query = parse(
        &request_with_typed_select_list(lower, data_types.clone()),
        &namespace(),
    )
    .expect("parse must succeed");
    let upper_query = parse(
        &request_with_typed_select_list(upper, data_types),
        &namespace(),
    )
    .expect("parse must succeed");

    assert_eq!(
        lower_query, upper_query,
        "two requests differing only in the aggregate function's case must parse identically"
    );
    let Expression::Aggregate(call) = &lower_query.select_list[0].expression else {
        panic!("expected an aggregate expression");
    };
    assert_eq!(call.function, "AVG");
}

#[test]
fn a_function_aggregate_entry_naming_no_function_is_reported() {
    let select_list = json!([{"type": "function_aggregate"}]);
    let data_types = json!([{"type": "DOUBLE"}]);

    let message = parse_failure(&request_with_typed_select_list(select_list, data_types));

    assert!(
        message.contains("names no function"),
        "the error must state what the entry is missing: {message}"
    );
}

#[test]
fn a_function_aggregate_entry_carrying_more_than_one_argument_is_reported() {
    let select_list = json!([{
        "type": "function_aggregate",
        "name": "SUM",
        "arguments": [
            {"type": "column", "name": "id"},
            {"type": "column", "name": "total"},
        ],
    }]);
    let data_types = json!([{"type": "DECIMAL", "precision": 23, "scale": 4}]);

    let message = parse_failure(&request_with_typed_select_list(select_list, data_types));

    assert!(
        message.contains("more than one argument"),
        "the error must state that the aggregate carries too many arguments: {message}"
    );
}

#[test]
fn a_non_column_select_list_entry_with_no_declared_type_fails() {
    let select_list = json!([{"type": "function_aggregate", "name": "COUNT"}]);

    let message = parse_failure(&request(Some(select_list)));

    assert!(
        message.contains("selectListDataTypes"),
        "the error must name the missing field: {message}"
    );
}

#[test]
fn a_predicate_select_list_entry_parses_into_a_predicate_expression() {
    let select_list = json!([{
        "type": "predicate_less",
        "left": {"type": "literal_exactnumeric", "value": "5"},
        "right": {"type": "column", "name": "id"},
    }]);
    let data_types = json!([{"type": "BOOLEAN"}]);

    let query = parse(
        &request_with_typed_select_list(select_list, data_types),
        &namespace(),
    )
    .expect("parse must succeed");

    assert_eq!(
        query.select_list,
        vec![SelectItem {
            expression: Expression::Predicate(Box::new(Predicate::Less(Comparison {
                left: Expression::Literal(Literal::ExactNumeric("5".to_string())),
                right: column("id"),
            }))),
            declared_type: ExasolType::Boolean,
        }]
    );
}

#[test]
fn a_predicate_select_list_entry_wrapping_an_aggregate_operand_parses_the_aggregate() {
    let select_list = json!([{
        "type": "predicate_less",
        "left": {"type": "literal_exactnumeric", "value": "0"},
        "right": {"type": "function_aggregate", "name": "COUNT"},
    }]);
    let data_types = json!([{"type": "BOOLEAN"}]);

    let query = parse(
        &request_with_typed_select_list(select_list, data_types),
        &namespace(),
    )
    .expect("parse must succeed");

    assert_eq!(
        query.select_list,
        vec![SelectItem {
            expression: Expression::Predicate(Box::new(Predicate::Less(Comparison {
                left: Expression::Literal(Literal::ExactNumeric("0".to_string())),
                right: Expression::Aggregate(AggregateCall {
                    function: "COUNT".to_string(),
                    argument: None,
                    distinct: false,
                }),
            }))),
            declared_type: ExasolType::Boolean,
        }]
    );
}

#[test]
fn a_predicate_select_list_entry_with_no_declared_type_fails() {
    let select_list = json!([{
        "type": "predicate_is_null",
        "expression": {"type": "column", "name": "note"},
    }]);

    let message = parse_failure(&request(Some(select_list)));

    assert!(
        message.contains("selectListDataTypes"),
        "the error must name the missing field: {message}"
    );
}

#[test]
fn every_exasol_type_the_adapter_maps_round_trips_into_a_declared_type() {
    let cases = [
        (json!({"type": "DATE"}), ExasolType::Date),
        (json!({"type": "TIMESTAMP"}), ExasolType::Timestamp),
        (json!({"type": "DOUBLE"}), ExasolType::Double),
        (
            json!({"type": "CHAR", "size": 5, "characterSet": "UTF8"}),
            ExasolType::Char { size: 5 },
        ),
    ];

    for (data_type, expected) in cases {
        let query = parse(&involved_column("c", data_type.clone()), &namespace())
            .expect("parse must succeed");
        assert_eq!(
            query.select_list[0].declared_type, expected,
            "{data_type} must declare {expected:?}"
        );
    }
}

/// A `pushdown` request joining `orders` and `customers` on their shared
/// `id`-named columns, each aliased and each column reference qualified —
/// the shape `vs-adapter/pushdown-join`'s background describes as the
/// protocol's own convention once a join is in play.
fn join_request(join_type: &str, select_list: Option<Value>) -> Value {
    let mut pushdown_request = json!({
        "type": "select",
        "from": {
            "type": "join",
            "join_type": join_type,
            "left": {"type": "table", "name": "orders", "alias": "o"},
            "right": {"type": "table", "name": "customers", "alias": "c"},
            "condition": {
                "type": "predicate_equal",
                "left": {"type": "column", "name": "customer_id", "tableName": "orders", "tableAlias": "o"},
                "right": {"type": "column", "name": "id", "tableName": "customers", "tableAlias": "c"},
            },
        },
    });
    if let Some(select_list) = select_list {
        pushdown_request["selectList"] = select_list;
    }

    json!({
        "type": "pushdown",
        "pushdownRequest": pushdown_request,
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
                    {"name": "name", "dataType": {"type": "VARCHAR", "size": 100, "characterSet": "UTF8"}},
                ],
            },
        ],
    })
}

fn decimal_ref(name: &str, table_name: &str, table_alias: Option<&str>) -> ColumnRef {
    ColumnRef {
        name: name.to_string(),
        declared_type: ExasolType::Decimal {
            precision: 19,
            scale: 0,
        },
        table_name: Some(table_name.to_string()),
        table_alias: table_alias.map(str::to_string),
    }
}

/// A column node's `tableName` travels with the [`ColumnRef`] beside its
/// `tableAlias`, because resolving a join's column to its table needs
/// whichever of the two the request supplied. Measured on Exasol 2025.2.1, a
/// join whose query aliased no table sends `tableName` alone — neither the
/// column node nor the `from` element carries an alias — so a model carrying
/// the alias only could not resolve the common shape at all.
#[test]
fn a_column_node_carries_the_table_name_it_names() {
    let select_list = json!([{"type": "column", "name": "id", "tableName": "customers"}]);

    let query =
        parse(&join_request("inner", Some(select_list)), &namespace()).expect("parse must succeed");

    assert_eq!(
        query.select_list,
        vec![SelectItem {
            expression: Expression::Column(decimal_ref("id", "customers", None)),
            declared_type: ExasolType::Decimal {
                precision: 19,
                scale: 0,
            },
        }]
    );
}

#[test]
fn a_join_from_element_parses_its_type_left_right_and_condition() {
    let query = parse(&join_request("inner", None), &namespace()).expect("parse must succeed");

    assert_eq!(
        query.from,
        FromClause::Join(Box::new(Join {
            join_type: "inner".to_string(),
            left: Box::new(FromClause::Table(SourceTable {
                namespace: namespace(),
                name: "orders".to_string(),
                alias: Some("o".to_string()),
            })),
            right: Box::new(FromClause::Table(SourceTable {
                namespace: namespace(),
                name: "customers".to_string(),
                alias: Some("c".to_string()),
            })),
            condition: Predicate::Equal(Comparison {
                left: Expression::Column(decimal_ref("customer_id", "orders", Some("o"))),
                right: Expression::Column(decimal_ref("id", "customers", Some("c"))),
            }),
        }))
    );
}

/// `join_type` is kept as the protocol's own spelling rather than validated
/// here: which values a dialect renders is a capability question decided at
/// render time, the same rule `AggregateCall::function` follows for a
/// function name the parser does not recognize.
#[test]
fn every_join_type_spelling_parses_verbatim() {
    for join_type in ["inner", "left_outer", "right_outer", "full_outer", "cross"] {
        let query =
            parse(&join_request(join_type, None), &namespace()).expect("parse must succeed");

        match query.from {
            FromClause::Join(join) => assert_eq!(join.join_type, join_type),
            other => panic!("expected a join, got {other:?}"),
        }
    }
}

#[test]
fn a_nested_join_parses_the_inner_join_as_the_left_element() {
    let mut request = join_request("inner", None);
    request["pushdownRequest"]["from"]["left"] = json!({
        "type": "join",
        "join_type": "left_outer",
        "left": {"type": "table", "name": "orders", "alias": "o"},
        "right": {"type": "table", "name": "customers", "alias": "c"},
        "condition": {
            "type": "predicate_equal",
            "left": {"type": "column", "name": "customer_id", "tableName": "orders", "tableAlias": "o"},
            "right": {"type": "column", "name": "id", "tableName": "customers", "tableAlias": "c"},
        },
    });

    let query = parse(&request, &namespace()).expect("parse must succeed");

    match query.from {
        FromClause::Join(outer) => match *outer.left {
            FromClause::Join(inner) => assert_eq!(inner.join_type, "left_outer"),
            other => {
                panic!("expected the outer join's left element to be a nested join, got {other:?}")
            }
        },
        other => panic!("expected a join, got {other:?}"),
    }
}

#[test]
fn a_from_element_carrying_no_type_field_is_reported() {
    let mut request = join_request("inner", None);
    request["pushdownRequest"]["from"]
        .as_object_mut()
        .expect("from must be an object")
        .remove("type");

    let message = parse_failure(&request);

    assert!(
        message.contains("type"),
        "the error must state that the `from` element carries no `type` field: {message}"
    );
}

#[test]
fn a_from_element_of_an_unsupported_type_is_reported() {
    let mut request = join_request("inner", None);
    request["pushdownRequest"]["from"]["type"] = json!("subquery");

    let message = parse_failure(&request);

    assert!(
        message.contains("subquery"),
        "the error must name the unsupported `from` element type: {message}"
    );
}

#[test]
fn a_join_element_missing_join_type_is_reported() {
    let mut request = join_request("inner", None);
    request["pushdownRequest"]["from"]
        .as_object_mut()
        .expect("from must be an object")
        .remove("join_type");

    let message = parse_failure(&request);

    assert!(
        message.contains("join_type"),
        "the error must name the missing field: {message}"
    );
}

#[test]
fn a_join_element_missing_left_is_reported() {
    let mut request = join_request("inner", None);
    request["pushdownRequest"]["from"]
        .as_object_mut()
        .expect("from must be an object")
        .remove("left");

    let message = parse_failure(&request);

    assert!(
        message.contains("left"),
        "the error must name the missing field: {message}"
    );
}

#[test]
fn a_join_element_missing_right_is_reported() {
    let mut request = join_request("inner", None);
    request["pushdownRequest"]["from"]
        .as_object_mut()
        .expect("from must be an object")
        .remove("right");

    let message = parse_failure(&request);

    assert!(
        message.contains("right"),
        "the error must name the missing field: {message}"
    );
}

#[test]
fn a_join_element_missing_condition_is_reported() {
    let mut request = join_request("inner", None);
    request["pushdownRequest"]["from"]
        .as_object_mut()
        .expect("from must be an object")
        .remove("condition");

    let message = parse_failure(&request);

    assert!(
        message.contains("condition"),
        "the error must name the missing field: {message}"
    );
}

/// Two joined tables both declaring an `id` column: each select-list entry
/// names its own table by `tableName`, so each resolves to its own table's
/// column rather than one shadowing the other.
#[test]
fn select_list_columns_sharing_a_name_resolve_to_their_own_table() {
    let select_list = json!([
        {"type": "column", "name": "id", "tableName": "orders", "tableAlias": "o"},
        {"type": "column", "name": "id", "tableName": "customers", "tableAlias": "c"},
    ]);

    let query =
        parse(&join_request("inner", Some(select_list)), &namespace()).expect("parse must succeed");

    assert_eq!(
        query.select_list,
        vec![
            SelectItem {
                expression: Expression::Column(decimal_ref("id", "orders", Some("o"))),
                declared_type: ExasolType::Decimal {
                    precision: 19,
                    scale: 0,
                },
            },
            SelectItem {
                expression: Expression::Column(decimal_ref("id", "customers", Some("c"))),
                declared_type: ExasolType::Decimal {
                    precision: 19,
                    scale: 0,
                },
            },
        ]
    );
}

/// A `tableName` that names neither joined table is not a parse failure:
/// `vs-adapter/pushdown-join` places resolving a column reference against
/// the `from` tree's own aliases at render time, so the parser falls back to
/// resolving the column by name alone and still succeeds when that name is
/// unambiguous, exactly as `name` alone always did before a join could
/// arrive.
#[test]
fn a_table_name_matching_no_involved_table_falls_back_to_resolving_by_column_name() {
    let select_list = json!([
        {"type": "column", "name": "name", "tableName": "does_not_exist", "tableAlias": "x"},
    ]);

    let query =
        parse(&join_request("inner", Some(select_list)), &namespace()).expect("parse must succeed");

    assert_eq!(
        query.select_list,
        vec![SelectItem {
            expression: Expression::Column(ColumnRef {
                name: "name".to_string(),
                declared_type: ExasolType::Varchar { size: 100 },
                table_name: Some("does_not_exist".to_string()),
                table_alias: Some("x".to_string()),
            }),
            declared_type: ExasolType::Varchar { size: 100 },
        }]
    );
}

/// A column named `id` with no `tableName` to disambiguate it, over a join
/// where both tables declare an `id` column, cannot be resolved.
#[test]
fn an_unqualified_column_ambiguous_between_two_tables_names_it_in_the_error() {
    let select_list = json!([{"type": "column", "name": "id"}]);

    let message = parse_failure(&join_request("inner", Some(select_list)));

    assert!(
        message.contains("id"),
        "the error must name the ambiguous column: {message}"
    );
}
