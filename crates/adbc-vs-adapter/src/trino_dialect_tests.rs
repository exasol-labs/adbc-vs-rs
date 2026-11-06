use serde_json::{json, Value};

use super::*;
use adbc_vs_scan::driver::Driver;

use crate::adapter_notes::SourceQualifier;
use crate::dialect_test_fixtures as fixtures;
use crate::dialect_test_fixtures::{
    aggregate, column, count_type, distinct, double_literal, double_type, equal, filtered_request,
    grouped_request, join_request, joined_column, less, numeric_literal, orders_columns,
    orders_to_customers, request, string_literal, sum_type, typed_request,
};
use crate::pushdown_query::{self, Query};

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
    let date_clause = where_clause_over(fixtures::temporal_columns(), fixtures::date_filter());
    assert_eq!(date_clause, r#"("shipped" = DATE '2015-12-01')"#);

    let timestamp_clause =
        where_clause_over(fixtures::temporal_columns(), fixtures::timestamp_filter());
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
        let sql = render_statement(&fixtures::join_type_request(join_type));
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
    fixtures::assert_mixed_case_renders_identically(
        &render_statement,
        "id",
        r#"SELECT COUNT("id"), MIN("note"), SUM(CAST("total" AS DECIMAL(38,4))), (CAST(SUM(CAST("total" AS DECIMAL(38,4))) AS DOUBLE) / COUNT("total")) FROM "memory"."adbc_vs"."orders""#,
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
    let request = fixtures::ratio_request(fixtures::ratio_sum_entries());

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
    fixtures::assert_avg_renders_as_quotient(
        &render_statement,
        r#"SELECT (CAST(SUM(CAST("total" AS DECIMAL(38,4))) AS DOUBLE) / COUNT("total")), (CAST(SUM(DISTINCT CAST("total" AS DECIMAL(38,4))) AS DOUBLE) / COUNT(DISTINCT "total")) FROM "memory"."adbc_vs"."orders""#,
        "Trino",
    );
}

/// `vs-adapter/pushdown-aggregation`, scenario "A BOOLEAN or character SUM and
/// AVG operand renders converted as Exasol converts it": Trino defines no `sum`
/// over a boolean or a character type, so a predicate sums as an `INTEGER`, and
/// a character column or literal reads through `NULLIF` and a `DOUBLE` cast,
/// with the `AVG` count taking the operand before the cast.
#[test]
fn sum_and_avg_convert_boolean_and_character_operands() {
    let predicate = less(numeric_literal("2"), column("id"));
    let mut entries = vec![
        (aggregate("SUM", Some(predicate.clone())), sum_type()),
        (aggregate("AVG", Some(predicate)), double_type()),
    ];
    entries.extend(fixtures::character_sum_avg_entries());
    let sql = render_statement(&fixtures::typed_entries_request(entries));

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
    fixtures::assert_having_renders(
        &render_statement,
        [
            r#"SELECT "note", COUNT(*) FROM "memory"."adbc_vs"."orders" GROUP BY "note" HAVING (12 < COUNT(*))"#,
            r#"SELECT COUNT(*) FROM "memory"."adbc_vs"."orders" HAVING (12 < COUNT(*))"#,
        ],
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
    fixtures::assert_unreferenced_join_column_fails(&render_error, "note");
}

/// One table joined to itself is told apart only by the alias the request
/// assigns each copy. With neither copy aliased, both answer to the same
/// reference, and picking the first would silently join one copy's column
/// against itself.
#[test]
fn a_reference_more_than_one_joined_table_answers_to_fails_naming_it() {
    fixtures::assert_ambiguous_join_reference_fails(&render_error, "id");
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
    let request = fixtures::nested_join_request("left_outer");

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
    let request = fixtures::self_join_request();

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
    let request =
        fixtures::filtered_grouped_join_request(joined_column("note", "orders"), note_type());

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
    fixtures::query_exercising(Driver::Trino, &namespace(), capability)
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
    fixtures::assert_column_positions(
        &render_statement,
        CONVERTED_NOTES,
        true,
        [
            r#"SELECT "uid" FROM "memory"."adbc_vs"."orders" WHERE ("uid" = 'a')"#,
            r#"SELECT "uid", MIN("uid"), COUNT(DISTINCT "uid") FROM "memory"."adbc_vs"."orders" GROUP BY "uid""#,
            r#"SELECT "t1"."id" FROM ("memory"."adbc_vs"."orders" AS "t1" INNER JOIN "memory"."adbc_vs"."customers" AS "t2" ON ("t1"."uid" = "t2"."uid"))"#,
        ],
    );
}

/// `vs-adapter/pushdown-filter`, scenario "BOOLEAN column is a predicate on its
/// own": the predicate is the column's read expression, with no comparison
/// added, alone and under `NOT`, `AND`, and `OR`.
#[test]
fn boolean_column_predicate_renders_through_its_read_expression() {
    fixtures::assert_boolean_predicates(
        &render_statement,
        CONVERTED_NOTES,
        r#"SELECT "id" FROM "memory"."adbc_vs"."orders" WHERE "#,
        [
            r#""flag""#,
            r#""active""#,
            r#"(NOT "flag")"#,
            r#"("flag" AND "active")"#,
            r#"("active" OR (5 < "id"))"#,
        ],
    );
}

/// Measured on Exasol 2025.1.11, `WHERE b = (d > 5)` arrives delegated with a
/// predicate as the comparison's right operand.
#[test]
fn predicate_operand_of_a_comparison_renders_parenthesized() {
    let clauses = fixtures::predicate_operand_clauses(&where_clause_over);

    assert_eq!(
        clauses,
        [
            r#"("active" = (5 < "id"))"#,
            r#"("active" = (NOT "active"))"#
        ]
    );
}
