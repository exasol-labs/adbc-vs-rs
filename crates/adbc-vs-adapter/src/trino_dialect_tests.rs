use serde_json::{json, Value};

use super::*;
use crate::adapter_notes::SourceQualifier;
use crate::dialect_test_fixtures as fixtures;
use crate::dialect_test_fixtures::{
    aggregate, column, count_type, distinct, double_literal, double_type, equal, filtered_request,
    grouped_request, join_request, joined_column, joined_orders_columns, less, numeric_literal,
    orders_columns, orders_to_customers, request, string_literal, sum_type, typed_request,
};
use crate::exa_type::ExasolType;
use crate::pushdown_query::{
    self, BetweenPredicate, Comparison, Expression, FromClause, InConstListPredicate,
    LikePredicate, Literal, Predicate, Query, SelectItem,
};

/// The namespace a Trino virtual schema records and every rendered table is
/// qualified by: the catalog and the schema, the two segments
/// `drivers/trino.toml` declares.
fn namespace() -> SourceQualifier {
    SourceQualifier::of(vec!["memory".to_string(), "adbc_vs".to_string()])
        .expect("two segments must build a qualifier")
}

/// The Trino statement `request` renders to, parsed first so every assertion
/// covers the request shape Exasol sends rather than a hand-built model.
fn render_statement(request: &Value) -> String {
    fixtures::render_statement(&TrinoDialect, &namespace(), request)
}

fn render(columns: Value, select_list: Option<Value>) -> String {
    render_statement(&request(columns, select_list))
}

/// The error rendering `request` fails with, for a request the parser
/// accepts and this dialect has no form for.
fn render_error(request: &Value) -> String {
    fixtures::render_error(&TrinoDialect, &namespace(), request)
}

/// The `WHERE` clause `filter` renders to over `columns`, parsed from a
/// request first so each assertion covers the shape Exasol sends.
fn where_clause_over(columns: Value, filter: Value) -> String {
    fixtures::where_clause_over(&TrinoDialect, &namespace(), columns, filter)
}

/// The `WHERE` clause `filter` renders to over the `orders` columns.
fn where_clause(filter: Value) -> String {
    where_clause_over(orders_columns(), filter)
}

/// The error rendering `filter` fails with, for a filter the parser accepts
/// and Trino has no form for.
fn filter_render_error(filter: Value) -> String {
    render_error(&filtered_request(orders_columns(), filter))
}

// ---------------------------------------------------------------------------
// Identifiers, qualifier, projection
// ---------------------------------------------------------------------------

/// Trino folds an unquoted identifier to lower case, so a mixed-case column
/// reaching the source unquoted would name a column that does not exist. The
/// fixture carries one for that reason: a lower-case-only fixture cannot tell
/// a dialect that quotes from one that does not.
#[test]
fn every_identifier_is_quoted_whatever_its_case() {
    let columns = json!([
        {"name": "id", "dataType": {"type": "DECIMAL", "precision": 19, "scale": 0}},
        {"name": "OrderId", "dataType": {"type": "DECIMAL", "precision": 19, "scale": 0}},
    ]);

    let sql = render(columns, Some(json!([column("id"), column("OrderId")])));

    assert!(sql.contains(r#""id""#), "{sql}");
    assert!(
        sql.contains(r#""OrderId""#),
        "a mixed-case identifier must be quoted with its case intact: {sql}"
    );
    assert!(sql.contains(r#""memory"."adbc_vs"."orders""#), "{sql}");
}

#[test]
fn double_quote_in_identifier_is_doubled() {
    let columns =
        json!([{"name": "a\"b", "dataType": {"type": "DECIMAL", "precision": 5, "scale": 0}}]);
    let sql = render(columns, Some(json!([column("a\"b")])));
    assert!(sql.contains(r#""a""b""#), "{sql}");
}

#[test]
fn a_table_is_qualified_by_both_recorded_namespace_segments() {
    let sql = render(orders_columns(), Some(json!([column("id")])));
    assert!(sql.contains(r#"FROM "memory"."adbc_vs"."orders""#), "{sql}");
}

#[test]
fn named_columns_are_projected_in_the_requests_order() {
    let sql = render(
        orders_columns(),
        Some(json!([column("note"), column("id")])),
    );
    assert_eq!(
        sql,
        r#"SELECT "note", "id" FROM "memory"."adbc_vs"."orders""#
    );
}

#[test]
fn absent_select_list_projects_every_column() {
    let sql = render(orders_columns(), None);
    assert_eq!(
        sql,
        r#"SELECT "id", "total", "note" FROM "memory"."adbc_vs"."orders""#
    );
}

#[test]
fn an_empty_select_list_projects_a_single_constant() {
    let sql = render(orders_columns(), Some(json!([])));
    assert_eq!(sql, r#"SELECT 1 FROM "memory"."adbc_vs"."orders""#);
}

#[test]
fn a_query_projecting_nothing_fails_rather_than_rendering_an_empty_projection() {
    let error = fixtures::empty_projection_error(&TrinoDialect, namespace());
    assert!(error.contains("empty"), "{error}");
}

#[test]
fn comparison_operands_keep_request_order() {
    let clause = where_clause(less(numeric_literal("5"), column("id")));
    assert_eq!(clause, r#"(5 < "id")"#);
}

// ---------------------------------------------------------------------------
// No collation, anywhere
// ---------------------------------------------------------------------------

#[test]
fn no_operand_carries_a_collation_suffix_in_any_position() {
    let columns = json!([
        {"name": "note", "dataType": {"type": "VARCHAR", "size": 100, "characterSet": "UTF8"}},
        {"name": "id", "dataType": {"type": "DECIMAL", "precision": 19, "scale": 0}},
    ]);

    let less_clause = where_clause_over(columns.clone(), less(column("note"), string_literal("m")));
    assert_eq!(less_clause, r#"("note" < 'm')"#);
    assert!(!less_clause.contains("COLLATE"), "{less_clause}");

    let between_request = json!({
        "type": "pushdown",
        "pushdownRequest": {
            "type": "select",
            "from": {"type": "table", "name": "orders"},
            "selectList": [{"type": "column", "name": "note"}],
            "filter": {
                "type": "predicate_between",
                "expression": column("note"),
                "left": string_literal("a"),
                "right": string_literal("z"),
            },
        },
        "involvedTables": [{"name": "orders", "columns": columns.clone()}],
    });
    let sql = render_statement(&between_request);
    assert!(!sql.contains("COLLATE"), "{sql}");

    let group_request = json!({
        "type": "pushdown",
        "pushdownRequest": {
            "type": "select",
            "from": {"type": "table", "name": "orders"},
            "selectList": [{"type": "column", "name": "note"}],
            "aggregationType": "group_by",
            "groupBy": [{"type": "column", "name": "note"}],
        },
        "involvedTables": [{"name": "orders", "columns": columns.clone()}],
    });
    let sql = render_statement(&group_request);
    assert!(!sql.contains("COLLATE"), "{sql}");

    let min_request = json!({
        "type": "pushdown",
        "pushdownRequest": {
            "type": "select",
            "from": {"type": "table", "name": "orders"},
            "selectList": [{
                "type": "function_aggregate",
                "name": "min",
                "arguments": [column("note")],
            }],
            "selectListDataTypes": [{"type": "VARCHAR", "size": 100}],
            "aggregationType": "single_group",
        },
        "involvedTables": [{"name": "orders", "columns": columns}],
    });
    let sql = render_statement(&min_request);
    assert!(!sql.contains("COLLATE"), "{sql}");
}

// ---------------------------------------------------------------------------
// Literals
// ---------------------------------------------------------------------------

#[test]
fn string_literal_doubles_only_the_single_quote() {
    let clause = where_clause(equal(column("note"), string_literal(r"a\b'c")));
    assert_eq!(clause, r#"("note" = 'a\b''c')"#);
}

#[test]
fn a_null_literal_renders_as_null() {
    let clause = where_clause(equal(column("id"), json!({"type": "literal_null"})));
    assert_eq!(clause, r#"("id" = NULL)"#);
}

#[test]
fn an_exact_numeric_literal_renders_bare() {
    let clause = where_clause(equal(column("id"), numeric_literal("2")));
    assert_eq!(clause, r#"("id" = 2)"#);
}

#[test]
fn a_floating_point_literal_renders_as_typed_double() {
    let clause = where_clause(equal(column("total"), double_literal("2.5")));
    assert_eq!(clause, r#"("total" = DOUBLE '2.5')"#);
}

#[test]
fn temporal_literals_render_typed() {
    let columns = json!([
        {"name": "shipped", "dataType": {"type": "DATE"}},
        {"name": "logged", "dataType": {"type": "TIMESTAMP"}},
    ]);
    let date_clause = where_clause_over(
        columns.clone(),
        equal(
            column("shipped"),
            json!({"type": "literal_date", "value": "2015-12-01"}),
        ),
    );
    assert_eq!(date_clause, r#"("shipped" = DATE '2015-12-01')"#);

    let timestamp_clause = where_clause_over(
        columns,
        equal(
            column("logged"),
            json!({"type": "literal_timestamp", "value": "2015-12-01 12:01:01.123"}),
        ),
    );
    assert_eq!(
        timestamp_clause,
        r#"("logged" = TIMESTAMP '2015-12-01 12:01:01.123')"#
    );
}

#[test]
fn a_literal_that_could_change_the_statement_shape_fails_to_render() {
    let error = filter_render_error(equal(column("id"), numeric_literal("1;DROP")));
    assert!(error.contains("1;DROP"), "{error}");
}

#[test]
fn a_double_literal_that_could_end_the_string_early_fails_to_render() {
    let error = filter_render_error(equal(column("total"), double_literal("2.5'; --")));
    assert!(error.contains("2.5'; --"), "{error}");
}

// ---------------------------------------------------------------------------
// LIKE
// ---------------------------------------------------------------------------

fn like(pattern: Value, escape_char: Option<Value>) -> Value {
    let mut node = json!({
        "type": "predicate_like",
        "expression": column("note"),
        "pattern": pattern,
    });
    if let Some(escape_char) = escape_char {
        node["escapeChar"] = escape_char;
    }
    node
}

/// A backslash in a delegated pattern is an ordinary character at Trino, so
/// the pattern has to reach the statement unchanged and with no `ESCAPE`
/// clause beside it.
#[test]
fn like_renders_its_pattern_with_no_escape_clause() {
    for pattern in ["ab%", r"ab\%"] {
        let clause = where_clause(like(string_literal(pattern), None));

        assert_eq!(clause, format!(r#"("note" LIKE '{pattern}')"#));
    }
}

#[test]
fn a_like_carrying_an_escape_char_fails_naming_the_capability() {
    let error = filter_render_error(like(string_literal("Wid!%get"), Some(string_literal("!"))));

    assert!(error.contains("FN_PRED_LIKE_ESCAPE"), "{error}");
    assert!(error.contains("'!'"), "{error}");
    assert!(!error.contains("  "), "{error}");
}

// ---------------------------------------------------------------------------
// Joins
// ---------------------------------------------------------------------------

#[test]
fn inner_join_renders_positionally_aliased_and_qualified() {
    let sql = render_statement(&join_request(
        orders_to_customers("inner"),
        json!([joined_column("id", "orders")]),
    ));
    assert_eq!(
        sql,
        r#"SELECT "t1"."id" FROM ("memory"."adbc_vs"."orders" AS "t1" INNER JOIN "memory"."adbc_vs"."customers" AS "t2" ON ("t1"."customer_id" = "t2"."id"))"#
    );
}

#[test]
fn each_join_type_renders_its_own_keyword() {
    for (join_type, keyword) in [
        ("inner", "INNER JOIN"),
        ("left_outer", "LEFT OUTER JOIN"),
        ("right_outer", "RIGHT OUTER JOIN"),
        ("full_outer", "FULL OUTER JOIN"),
    ] {
        let sql = render_statement(&join_request(
            orders_to_customers(join_type),
            json!([joined_column("id", "orders")]),
        ));
        assert!(sql.contains(keyword), "{join_type}: {sql}");
    }
}

#[test]
fn unsupported_join_type_named_in_error() {
    let error_query = join_request(
        orders_to_customers("semi"),
        json!([joined_column("id", "orders")]),
    );
    let error = render_error(&error_query);
    assert!(error.contains("semi"), "{error}");
}

#[test]
fn unresolvable_column_named_in_error() {
    let error = render_error(&join_request(
        orders_to_customers("inner"),
        json!([joined_column("note", "nowhere")]),
    ));
    assert!(error.contains("nowhere"), "{error}");
}

#[test]
fn a_single_table_query_renders_no_alias_even_when_the_request_assigned_one() {
    let request = json!({
        "type": "pushdown",
        "pushdownRequest": {
            "type": "select",
            "from": {"type": "table", "name": "orders", "alias": "o"},
            "selectList": [column("id")],
        },
        "involvedTables": [{"name": "orders", "columns": orders_columns()}],
    });
    let sql = render_statement(&request);
    assert_eq!(sql, r#"SELECT "id" FROM "memory"."adbc_vs"."orders""#);
}

// ---------------------------------------------------------------------------
// Aggregation and GROUP BY
// ---------------------------------------------------------------------------

#[test]
fn single_group_renders_without_group_by() {
    let mut built = typed_request(json!([aggregate("count", None)]), json!([count_type()]));
    built["pushdownRequest"]["aggregationType"] = json!("single_group");

    let sql = render_statement(&built);
    assert_eq!(sql, r#"SELECT COUNT(*) FROM "memory"."adbc_vs"."orders""#);
    assert!(!sql.contains("GROUP BY"), "{sql}");
}

#[test]
fn a_grouped_aggregation_renders_group_by() {
    let sql = render_statement(&grouped_request(
        json!([column("id")]),
        json!([{"type": "DECIMAL", "precision": 19, "scale": 0}]),
        json!([column("id")]),
    ));
    assert!(sql.ends_with(r#"GROUP BY "id""#), "{sql}");
}

#[test]
fn a_multi_column_group_by_renders_every_key() {
    let sql = render_statement(&grouped_request(
        json!([column("id"), column("note")]),
        json!([
            {"type": "DECIMAL", "precision": 19, "scale": 0},
            {"type": "VARCHAR", "size": 100},
        ]),
        json!([column("id"), column("note")]),
    ));
    assert!(sql.ends_with(r#"GROUP BY "id", "note""#), "{sql}");
}

#[test]
fn count_star_differs_from_count_column() {
    let mut star_request = typed_request(json!([aggregate("count", None)]), json!([count_type()]));
    star_request["pushdownRequest"]["aggregationType"] = json!("single_group");

    let mut column_request = typed_request(
        json!([aggregate("count", Some(column("id")))]),
        json!([count_type()]),
    );
    column_request["pushdownRequest"]["aggregationType"] = json!("single_group");

    assert!(render_statement(&star_request).contains("COUNT(*)"));
    assert!(render_statement(&column_request).contains(r#"COUNT("id")"#));
}

#[test]
fn unrenderable_aggregate_named_in_error() {
    let mut built = typed_request(
        json!([aggregate("stddev", Some(column("id")))]),
        json!([count_type()]),
    );
    built["pushdownRequest"]["aggregationType"] = json!("single_group");

    let error = render_error(&built);
    assert!(error.contains("STDDEV"), "{error}");
}

/// The function name Exasol sends in any case must render as Trino's canonical
/// spelling, so the case a request arrives in never reaches the statement.
#[test]
fn mixed_case_function_names_render_identically() {
    let entries = |count: &str, min: &str, sum: &str, avg: &str| {
        json!([
            aggregate(count, Some(column("id"))),
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

    assert_eq!(
        lower_case, upper_case,
        "the case a function name arrives in must not reach the generated statement"
    );
    assert_eq!(
        lower_case,
        r#"SELECT COUNT("id"), MIN("note"), SUM(CAST("total" AS DECIMAL(38,4))), (CAST(SUM(CAST("total" AS DECIMAL(38,4))) AS DOUBLE) / COUNT("total")) FROM "memory"."adbc_vs"."orders""#
    );
}

fn note_type() -> Value {
    json!({"type": "VARCHAR", "size": 100, "characterSet": "UTF8"})
}

/// `vs-adapter/trino-pushdown`, scenario "SUM over an exact-numeric column sums
/// in Trino's 38-digit decimal": an operand declared `DECIMAL(p,s)` is cast to
/// `DECIMAL(38,s)` inside `SUM`, with and without `DISTINCT`, because Trino's
/// `sum` returns its input's type.
#[test]
fn sum_over_a_decimal_operand_casts_it_to_decimal_38() {
    let sql = render_statement(&typed_request(
        json!([
            aggregate("SUM", Some(column("id"))),
            aggregate("SUM", Some(column("total"))),
            distinct(aggregate("SUM", Some(column("total")))),
        ]),
        json!([sum_type(), sum_type(), sum_type()]),
    ));

    assert_eq!(
        sql,
        r#"SELECT SUM(CAST("id" AS DECIMAL(38,0))), SUM(CAST("total" AS DECIMAL(38,4))), SUM(DISTINCT CAST("total" AS DECIMAL(38,4))) FROM "memory"."adbc_vs"."orders""#
    );
}

/// `vs-adapter/pushdown-aggregation`, scenario "SUM renders as the source's SUM
/// over the operand": `DISTINCT` precedes the operand exactly when the request
/// sets it, and an operand declared `DOUBLE` renders without a cast.
#[test]
fn sum_renders_with_and_without_distinct() {
    let mut request = typed_request(
        json!([
            aggregate("SUM", Some(column("ratio"))),
            distinct(aggregate("SUM", Some(column("ratio")))),
        ]),
        json!([double_type(), double_type()]),
    );
    request["involvedTables"][0]["columns"] = json!([
        {"name": "ratio", "dataType": {"type": "DOUBLE"}},
    ]);

    assert_eq!(
        render_statement(&request),
        r#"SELECT SUM("ratio"), SUM(DISTINCT "ratio") FROM "memory"."adbc_vs"."orders""#
    );
}

/// `vs-adapter/pushdown-aggregation`, scenario "AVG renders as the double
/// quotient of SUM and COUNT": the quotient is parenthesized, both halves carry
/// `DISTINCT` for `AVG(DISTINCT)`, and Trino's own `AVG`, which returns
/// `decimal` over an exact-numeric operand, never renders.
#[test]
fn avg_renders_as_the_double_quotient_of_sum_and_count() {
    let sql = render_statement(&typed_request(
        json!([
            aggregate("AVG", Some(column("total"))),
            distinct(aggregate("AVG", Some(column("total")))),
        ]),
        json!([double_type(), double_type()]),
    ));

    assert_eq!(
        sql,
        r#"SELECT (CAST(SUM(CAST("total" AS DECIMAL(38,4))) AS DOUBLE) / COUNT("total")), (CAST(SUM(DISTINCT CAST("total" AS DECIMAL(38,4))) AS DOUBLE) / COUNT(DISTINCT "total")) FROM "memory"."adbc_vs"."orders""#
    );
    assert!(
        !sql.contains("AVG("),
        "Trino's own AVG must not render: {sql}"
    );
}

/// `vs-adapter/pushdown-aggregation`, scenario "A BOOLEAN or character SUM and
/// AVG operand renders converted as Exasol converts it": Trino defines no `sum`
/// over a boolean or a character type, so a predicate sums as an `INTEGER`, and
/// a character column or literal reads through `NULLIF` and a `DOUBLE` cast,
/// with the `AVG` count taking the operand before the cast.
#[test]
fn sum_and_avg_convert_boolean_and_character_operands() {
    let sql = render_statement(&typed_request(
        json!([
            aggregate("SUM", Some(less(numeric_literal("2"), column("id")))),
            aggregate("AVG", Some(less(numeric_literal("2"), column("id")))),
            aggregate("SUM", Some(column("note"))),
            distinct(aggregate("SUM", Some(column("note")))),
            aggregate("AVG", Some(column("note"))),
            distinct(aggregate("AVG", Some(column("note")))),
            aggregate("SUM", Some(string_literal("5"))),
        ]),
        json!([
            sum_type(),
            double_type(),
            double_type(),
            double_type(),
            double_type(),
            double_type(),
            double_type(),
        ]),
    ));

    assert_eq!(
        sql,
        r#"SELECT SUM(CAST((2 < "id") AS INTEGER)), (CAST(SUM(CAST((2 < "id") AS INTEGER)) AS DOUBLE) / COUNT((2 < "id"))), SUM(CAST(NULLIF("note", '') AS DOUBLE)), SUM(DISTINCT CAST(NULLIF("note", '') AS DOUBLE)), (CAST(SUM(CAST(NULLIF("note", '') AS DOUBLE)) AS DOUBLE) / COUNT(NULLIF("note", ''))), (CAST(SUM(DISTINCT CAST(NULLIF("note", '') AS DOUBLE)) AS DOUBLE) / COUNT(DISTINCT NULLIF("note", ''))), SUM(CAST(NULLIF('5', '') AS DOUBLE)) FROM "memory"."adbc_vs"."orders""#
    );
}

/// `vs-adapter/pushdown-aggregation`, scenario "A having element renders as a
/// HAVING clause": the clause follows `GROUP BY`, and a single-group request
/// carries the clause with no `GROUP BY`.
#[test]
fn having_renders_after_group_by_and_on_a_single_group() {
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

    assert_eq!(
        render_statement(&grouped),
        r#"SELECT "note", COUNT(*) FROM "memory"."adbc_vs"."orders" GROUP BY "note" HAVING (12 < COUNT(*))"#
    );
    assert_eq!(
        render_statement(&single),
        r#"SELECT COUNT(*) FROM "memory"."adbc_vs"."orders" HAVING (12 < COUNT(*))"#
    );
}

// ---------------------------------------------------------------------------
// Determinism
// ---------------------------------------------------------------------------

#[test]
fn a_request_renders_byte_identically_on_every_call() {
    let request = request(orders_columns(), None);
    let query = pushdown_query::parse(&request, &namespace()).expect("must parse");
    let first = TrinoDialect.render(&query).expect("must render");
    let second = TrinoDialect.render(&query).expect("must render");
    assert_eq!(first, second);
}

// ---------------------------------------------------------------------------
// Empty forms and unresolvable references
// ---------------------------------------------------------------------------

/// This dialect re-implements every empty-form guard independently of
/// `postgres_dialect`, so the sibling dialects' coverage of these branches
/// carries none of the risk here. Trino writes no empty `AND`, `OR`, or `IN`
/// list, and a renderer that dropped the node instead would hand Exasol rows
/// it never asked for.
#[test]
fn a_predicate_with_no_operand_fails_rather_than_rendering_an_empty_group() {
    fixtures::assert_empty_predicate_groups_fail(&filter_render_error, "Trino");
}

/// Trino writes no empty `GROUP BY` clause, and a renderer that dropped the
/// clause instead would aggregate the whole table into one row where Exasol
/// asked for one row per group.
#[test]
fn an_aggregation_grouping_by_no_column_fails_rather_than_rendering_an_empty_clause() {
    fixtures::assert_empty_group_by_fails(&render_error, "Trino");
}

/// `COUNT(*)` is the only aggregate Trino writes over no argument, and it
/// counts rows rather than values, so `COUNT(DISTINCT *)` is not a form
/// either. Dropping the `DISTINCT` Exasol believes was delegated would count
/// every row where it asked for the distinct values.
#[test]
fn an_aggregate_carrying_no_argument_fails_unless_it_is_count_star() {
    fixtures::assert_argumentless_aggregates_fail(&render_error, "Trino");
}

/// A reference carrying neither half has nothing to resolve against, and it
/// cannot be rendered bare either: Trino would resolve the name against
/// whichever joined table declares it.
#[test]
fn a_join_column_carrying_no_table_reference_at_all_fails_naming_it() {
    let error = render_error(&join_request(
        orders_to_customers("inner"),
        json!([column("note")]),
    ));

    assert!(
        error.contains("note") && error.contains("tableName") && error.contains("tableAlias"),
        "the error must name the column and both spellings it lacks: {error}"
    );
}

/// One table joined to itself is told apart only by the alias the request
/// assigns each copy. With neither copy aliased, both answer to the same
/// reference, and picking the first would silently join one copy's column
/// against itself.
#[test]
fn a_reference_more_than_one_joined_table_answers_to_fails_naming_it() {
    let mut from = orders_to_customers("inner");
    from["right"] = json!({"type": "table", "name": "orders"});

    let error = render_error(&join_request(from, json!([joined_column("id", "orders")])));

    assert!(
        error.contains("id") && error.contains("orders") && error.contains("more than one"),
        "the error must name the column and the reference two tables answer to: {error}"
    );
}

// ---------------------------------------------------------------------------
// Join tree shape and clause order
// ---------------------------------------------------------------------------

/// A nested join renders inside its own parentheses, so the tree the request
/// sent is the tree Trino reads: an outer join whose operand order or nesting
/// shifted would return different rows. This dialect owns its own alias
/// counter, so the positional aliases are proved here rather than inherited.
#[test]
fn a_nested_join_renders_the_inner_join_parenthesized_inside_the_outer() {
    let mut request = join_request(
        json!({
            "type": "join",
            "join_type": "left_outer",
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

    let sql = render_statement(&request);

    assert_eq!(
        sql,
        r#"SELECT "t3"."code" FROM (("memory"."adbc_vs"."orders" AS "t1" INNER JOIN "memory"."adbc_vs"."customers" AS "t2" ON ("t1"."customer_id" = "t2"."id")) LEFT OUTER JOIN "memory"."adbc_vs"."regions" AS "t3" ON ("t2"."id" = "t3"."customer_id"))"#
    );
}

/// A self-join names one table twice, and the two copies are told apart only
/// by the aliases the request assigned. Each resolves to its own positional
/// alias, so the condition compares the two copies rather than one with
/// itself.
#[test]
fn a_self_join_resolves_each_copy_to_its_own_alias() {
    let request = json!({
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
    });

    let sql = render_statement(&request);

    assert_eq!(
        sql,
        r#"SELECT "t1"."id", "t2"."id" FROM ("memory"."adbc_vs"."orders" AS "t1" INNER JOIN "memory"."adbc_vs"."orders" AS "t2" ON ("t1"."id" = "t2"."customer_id"))"#
    );
}

/// A join carrying both a `filter` and a `groupBy` in the same request must
/// render every clause together, in the required order: `FROM`, `WHERE`,
/// `GROUP BY`. Each tier renders its own fragment; only this test proves the
/// three compose over a joined `FROM` rather than only over a single table,
/// and this dialect assembles its clauses itself.
#[test]
fn join_composes_with_filter_and_group_by() {
    let mut request = join_request(
        orders_to_customers("inner"),
        json!([joined_column("note", "orders"), aggregate("COUNT", None)]),
    );
    request["pushdownRequest"]["filter"] = json!({
        "type": "predicate_equal",
        "left": joined_column("note", "orders"),
        "right": string_literal("x"),
    });
    request["pushdownRequest"]["selectListDataTypes"] = json!([
        {"type": "VARCHAR", "size": 100, "characterSet": "UTF8"},
        count_type(),
    ]);
    request["pushdownRequest"]["aggregationType"] = json!("group_by");
    request["pushdownRequest"]["groupBy"] = json!([joined_column("note", "orders")]);

    let sql = render_statement(&request);

    assert_eq!(
        sql,
        concat!(
            r#"SELECT "t1"."note", COUNT(*) FROM ("memory"."adbc_vs"."orders" AS "t1" INNER JOIN "#,
            r#""memory"."adbc_vs"."customers" AS "t2" ON ("t1"."customer_id" = "t2"."id")) "#,
            r#"WHERE ("t1"."note" = 'x') GROUP BY "t1"."note""#
        ),
        "the FROM, WHERE, and GROUP BY clauses must all appear, in that order: {sql}"
    );
}

// ---------------------------------------------------------------------------
// Capability fixture table, driven by `capabilities_tests`
// ---------------------------------------------------------------------------

pub(crate) fn query_exercising(capability: &str) -> (&'static str, Query) {
    use fixtures::{
        aggregate_call, aggregate_item, count_star, date_column, decimal_column, id_equals,
        numeric, select_list, timestamp_column, varchar_column,
    };

    fn from() -> FromClause {
        fixtures::from(&namespace())
    }

    fn filtered(filter: Predicate) -> Query {
        fixtures::filtered(&namespace(), filter)
    }

    fn aggregated(select_list: Vec<SelectItem>) -> Query {
        fixtures::aggregated(&namespace(), select_list)
    }

    fn joined_query(join_type: &str) -> Query {
        fixtures::joined_query(&namespace(), join_type)
    }

    match capability {
        "SELECTLIST_PROJECTION" => (
            r#"SELECT "id" FROM"#,
            Query {
                select_list: select_list(),
                filter: None,
                aggregation: None,
                having: None,
                from: from(),
            },
        ),
        "SELECTLIST_EXPRESSIONS" => (
            r#"SELECT ("id" = 2)"#,
            Query {
                select_list: vec![SelectItem {
                    expression: Expression::Predicate(Box::new(id_equals(numeric("2")))),
                    declared_type: ExasolType::Boolean,
                }],
                filter: None,
                aggregation: None,
                having: None,
                from: from(),
            },
        ),
        "FILTER_EXPRESSIONS" => (r#"WHERE ("id" = 2)"#, filtered(id_equals(numeric("2")))),
        "AGGREGATE_SINGLE_GROUP" => (
            "SELECT COUNT(*) FROM",
            aggregated(vec![aggregate_item(count_star())]),
        ),
        "AGGREGATE_GROUP_BY_COLUMN" => (
            r#"GROUP BY "id""#,
            Query {
                select_list: select_list(),
                filter: None,
                aggregation: Some(pushdown_query::Aggregation::GroupBy(vec![decimal_column()])),
                having: None,
                from: from(),
            },
        ),
        "AGGREGATE_GROUP_BY_TUPLE" => (
            r#"GROUP BY "id", "note""#,
            Query {
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
                from: from(),
            },
        ),
        "FN_AGG_COUNT" => (
            r#"COUNT("id")"#,
            aggregated(vec![aggregate_item(aggregate_call(
                "COUNT",
                Expression::Column(decimal_column()),
                false,
            ))]),
        ),
        "FN_AGG_COUNT_STAR" => ("COUNT(*)", aggregated(vec![aggregate_item(count_star())])),
        "FN_AGG_COUNT_DISTINCT" => (
            r#"COUNT(DISTINCT "id")"#,
            aggregated(vec![aggregate_item(aggregate_call(
                "COUNT",
                Expression::Column(decimal_column()),
                true,
            ))]),
        ),
        "FN_AGG_MIN" => (
            r#"MIN("id")"#,
            aggregated(vec![aggregate_item(aggregate_call(
                "MIN",
                Expression::Column(decimal_column()),
                false,
            ))]),
        ),
        "FN_AGG_MAX" => (
            r#"MAX("id")"#,
            aggregated(vec![aggregate_item(aggregate_call(
                "MAX",
                Expression::Column(decimal_column()),
                false,
            ))]),
        ),
        "AGGREGATE_HAVING" => (
            " HAVING (0 < COUNT(*))",
            Query {
                having: Some(Predicate::Less(Comparison {
                    left: numeric("0"),
                    right: Expression::Aggregate(count_star()),
                })),
                ..aggregated(vec![aggregate_item(count_star())])
            },
        ),
        "FN_AGG_SUM" => (
            r#"SUM(CAST("id" AS DECIMAL(38,0)))"#,
            aggregated(vec![aggregate_item(aggregate_call(
                "SUM",
                Expression::Column(decimal_column()),
                false,
            ))]),
        ),
        "FN_AGG_SUM_DISTINCT" => (
            r#"SUM(DISTINCT CAST("id" AS DECIMAL(38,0)))"#,
            aggregated(vec![aggregate_item(aggregate_call(
                "SUM",
                Expression::Column(decimal_column()),
                true,
            ))]),
        ),
        "FN_AGG_AVG" => (
            r#"(CAST(SUM(CAST("id" AS DECIMAL(38,0))) AS DOUBLE) / COUNT("id"))"#,
            aggregated(vec![aggregate_item(aggregate_call(
                "AVG",
                Expression::Column(decimal_column()),
                false,
            ))]),
        ),
        "FN_AGG_AVG_DISTINCT" => (
            r#"(CAST(SUM(DISTINCT CAST("id" AS DECIMAL(38,0))) AS DOUBLE) / COUNT(DISTINCT "id"))"#,
            aggregated(vec![aggregate_item(aggregate_call(
                "AVG",
                Expression::Column(decimal_column()),
                true,
            ))]),
        ),
        "FN_PRED_AND" => (
            r#"(("id" = 1) AND ("id" = 2))"#,
            filtered(Predicate::And(vec![
                id_equals(numeric("1")),
                id_equals(numeric("2")),
            ])),
        ),
        "FN_PRED_OR" => (
            r#"(("id" = 1) OR ("id" = 2))"#,
            filtered(Predicate::Or(vec![
                id_equals(numeric("1")),
                id_equals(numeric("2")),
            ])),
        ),
        "FN_PRED_NOT" => (
            r#"(NOT ("id" = 2))"#,
            filtered(Predicate::Not(Box::new(id_equals(numeric("2"))))),
        ),
        "FN_PRED_EQUAL" => (r#"("id" = 2)"#, filtered(id_equals(numeric("2")))),
        "FN_PRED_NOTEQUAL" => (
            r#"("id" <> 2)"#,
            filtered(Predicate::NotEqual(Comparison {
                left: Expression::Column(decimal_column()),
                right: numeric("2"),
            })),
        ),
        "FN_PRED_LESS" => (
            r#"("id" < 2)"#,
            filtered(Predicate::Less(Comparison {
                left: Expression::Column(decimal_column()),
                right: numeric("2"),
            })),
        ),
        "FN_PRED_LESSEQUAL" => (
            r#"("id" <= 2)"#,
            filtered(Predicate::LessEqual(Comparison {
                left: Expression::Column(decimal_column()),
                right: numeric("2"),
            })),
        ),
        "FN_PRED_IS_NULL" => (
            r#"("id" IS NULL)"#,
            filtered(Predicate::IsNull(Expression::Column(decimal_column()))),
        ),
        "FN_PRED_IS_NOT_NULL" => (
            r#"("id" IS NOT NULL)"#,
            filtered(Predicate::IsNotNull(Expression::Column(decimal_column()))),
        ),
        "FN_PRED_LIKE" => (
            "LIKE 'Wid%')",
            filtered(Predicate::Like(LikePredicate {
                expression: Expression::Column(varchar_column()),
                pattern: Expression::Literal(Literal::String("Wid%".to_string())),
                escape_char: None,
            })),
        ),
        "FN_PRED_BETWEEN" => (
            "BETWEEN 1 AND 9",
            filtered(Predicate::Between(BetweenPredicate {
                expression: Expression::Column(decimal_column()),
                left: numeric("1"),
                right: numeric("9"),
            })),
        ),
        "FN_PRED_IN_CONSTLIST" => (
            "IN (1, 2)",
            filtered(Predicate::InConstList(InConstListPredicate {
                expression: Expression::Column(decimal_column()),
                arguments: vec![numeric("1"), numeric("2")],
            })),
        ),
        "LITERAL_NULL" => (
            "= NULL",
            filtered(id_equals(Expression::Literal(Literal::Null))),
        ),
        "LITERAL_EXACTNUMERIC" => ("= 2", filtered(id_equals(numeric("2")))),
        "LITERAL_DOUBLE" => (
            "= DOUBLE '2.5'",
            filtered(id_equals(Expression::Literal(Literal::Double(
                "2.5".to_string(),
            )))),
        ),
        "LITERAL_STRING" => (
            "'widget'",
            filtered(Predicate::Equal(Comparison {
                left: Expression::Column(varchar_column()),
                right: Expression::Literal(Literal::String("widget".to_string())),
            })),
        ),
        "LITERAL_DATE" => (
            "DATE '2015-12-01'",
            filtered(Predicate::Equal(Comparison {
                left: Expression::Column(date_column()),
                right: Expression::Literal(Literal::Date("2015-12-01".to_string())),
            })),
        ),
        "LITERAL_TIMESTAMP" => (
            "TIMESTAMP '2015-12-01 12:01:01.123'",
            filtered(Predicate::Equal(Comparison {
                left: Expression::Column(timestamp_column()),
                right: Expression::Literal(Literal::Timestamp(
                    "2015-12-01 12:01:01.123".to_string(),
                )),
            })),
        ),
        "JOIN" => (
            r#""memory"."adbc_vs"."orders" AS "t1" INNER JOIN "memory"."adbc_vs"."customers" AS "t2""#,
            joined_query("inner"),
        ),
        "JOIN_TYPE_INNER" => ("INNER JOIN", joined_query("inner")),
        "JOIN_TYPE_LEFT_OUTER" => ("LEFT OUTER JOIN", joined_query("left_outer")),
        "JOIN_TYPE_RIGHT_OUTER" => ("RIGHT OUTER JOIN", joined_query("right_outer")),
        "JOIN_TYPE_FULL_OUTER" => ("FULL OUTER JOIN", joined_query("full_outer")),
        "JOIN_CONDITION_EQUI" => (
            r#"ON ("t1"."customer_id" = "t2"."id")"#,
            joined_query("inner"),
        ),
        other => panic!(
            "capability `{other}` is advertised but this test has no fixture that exercises it; \
             add one alongside whichever task added the capability"
        ),
    }
}

/// Trino source types a column records. Trino's read expression is the bare
/// column for every type, so a recorded type changes no position.
const CONVERTED_NOTES: fixtures::ConvertedNotes = fixtures::ConvertedNotes {
    uid: "varchar(36)",
    flag: "boolean",
};

/// `vs-adapter/pushdown-dialect`, scenario "A column reference renders through
/// its read expression in every position": a column recording its source type
/// reads as the bare quoted column in the select list, a `WHERE` comparison, a
/// `GROUP BY` key, a `MIN` and a `COUNT(DISTINCT)` operand, and an equi-join
/// condition, as a column without notes does.
#[test]
fn column_reference_renders_identically_in_every_position() {
    let filtered = render_statement(&fixtures::converted_column_filtered_request(
        CONVERTED_NOTES,
    ));
    assert_eq!(
        filtered,
        r#"SELECT "uid" FROM "memory"."adbc_vs"."orders" WHERE ("uid" = 'a')"#
    );

    let grouped = render_statement(&fixtures::converted_column_grouped_request(
        CONVERTED_NOTES,
        json!([
            column("uid"),
            fixtures::aggregate_over("MIN", "uid", false),
            fixtures::aggregate_over("COUNT", "uid", true),
        ]),
        json!([
            {"type": "VARCHAR", "size": 36},
            {"type": "VARCHAR", "size": 36},
            {"type": "DECIMAL", "precision": 18, "scale": 0},
        ]),
        "uid",
    ));
    assert_eq!(
        grouped,
        r#"SELECT "uid", MIN("uid"), COUNT(DISTINCT "uid") FROM "memory"."adbc_vs"."orders" GROUP BY "uid""#
    );

    let joined = render_statement(&fixtures::converted_column_joined_request(CONVERTED_NOTES));
    assert_eq!(
        joined,
        r#"SELECT "t1"."id" FROM ("memory"."adbc_vs"."orders" AS "t1" INNER JOIN "memory"."adbc_vs"."customers" AS "t2" ON ("t1"."uid" = "t2"."uid"))"#
    );
}

/// `vs-adapter/pushdown-filter`, scenario "BOOLEAN column is a predicate on its
/// own": the predicate is the column's read expression, with no comparison
/// added, alone and under `NOT`, `AND`, and `OR`.
#[test]
fn boolean_column_predicate_renders_through_its_read_expression() {
    for (filter, expected) in [
        (column("flag"), r#""flag""#),
        (column("active"), r#""active""#),
        (fixtures::not(column("flag")), r#"(NOT "flag")"#),
        (
            fixtures::and(json!([column("flag"), column("active")])),
            r#"("flag" AND "active")"#,
        ),
        (
            fixtures::or(json!([
                column("active"),
                less(numeric_literal("5"), column("id"))
            ])),
            r#"("active" OR (5 < "id"))"#,
        ),
    ] {
        let sql = render_statement(&fixtures::converted_table_filtered_request(
            CONVERTED_NOTES,
            filter,
        ));

        assert_eq!(
            sql,
            format!(r#"SELECT "id" FROM "memory"."adbc_vs"."orders" WHERE {expected}"#)
        );
    }
}

/// Measured on Exasol 2025.1.11, `WHERE b = (d > 5)` arrives delegated with a
/// predicate as the comparison's right operand.
#[test]
fn predicate_operand_of_a_comparison_renders_parenthesized() {
    let columns = json!([
        {"name": "active", "dataType": {"type": "BOOLEAN"}},
        {"name": "id", "dataType": {"type": "DECIMAL", "precision": 19, "scale": 0}},
    ]);
    for (operand, expected) in [
        (
            less(numeric_literal("5"), column("id")),
            r#"("active" = (5 < "id"))"#,
        ),
        (
            fixtures::not(column("active")),
            r#"("active" = (NOT "active"))"#,
        ),
    ] {
        let clause = where_clause_over(columns.clone(), equal(column("active"), operand));

        assert_eq!(clause, expected);
    }
}
