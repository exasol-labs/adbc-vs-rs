use serde_json::{json, Value};

use super::*;
use crate::adapter_notes::SourceQualifier;
use crate::dialect_test_fixtures as fixtures;
use crate::dialect_test_fixtures::{
    column as column_operand, equal, filtered_request, less, numeric_literal, orders_columns,
    request, string_literal,
};
use crate::exa_type::ExasolType;
use crate::pushdown_query;

const DATABASE: &str = "shop";

/// The namespace a MySQL virtual schema records and every rendered table is
/// qualified by: the one database the connection address names.
fn namespace() -> SourceQualifier {
    SourceQualifier::single(DATABASE)
}

/// The MySQL statement `request` renders to, parsed first so every assertion
/// covers the request shape Exasol sends rather than a hand-built model.
fn render_statement(request: &Value) -> String {
    fixtures::render_statement(&MySqlDialect, &namespace(), request)
}

fn render(columns: Value, select_list: Option<Value>) -> String {
    render_statement(&request(columns, select_list))
}

/// The error rendering `request` fails with, for a request the parser
/// accepts and MySQL has no form for.
fn render_error(request: &Value) -> String {
    fixtures::render_error(&MySqlDialect, &namespace(), request)
}

/// The `WHERE` clause `filter` renders to over `columns`, parsed from a
/// request first so each assertion covers the shape Exasol sends.
fn where_clause_over(columns: Value, filter: Value) -> String {
    fixtures::where_clause_over(&MySqlDialect, &namespace(), columns, filter)
}

/// The `WHERE` clause `filter` renders to over the `orders` columns.
fn where_clause(filter: Value) -> String {
    where_clause_over(orders_columns(), filter)
}

/// The error rendering `filter` fails with, for a filter the parser accepts
/// and MySQL has no form for.
fn filter_render_error(filter: Value) -> String {
    render_error(&filtered_request(orders_columns(), filter))
}

#[test]
fn character_operands_carry_the_comparison_collation() {
    let clause = where_clause(equal(column_operand("note"), string_literal("widget")));

    assert_eq!(
        clause,
        "(`note` COLLATE utf8mb4_0900_as_cs = _utf8mb4 X'776964676574' COLLATE \
         utf8mb4_0900_as_cs)"
    );
}

/// Both operands of a comparison may be character-typed with no literal
/// between them, and MySQL's default `utf8mb4_0900_ai_ci` reports the stored
/// `Widget` as equal to the stored `widget` where Exasol reports them
/// unequal. A rule keying on the literal rather than on the type would
/// render this comparison bare and return rows Exasol never asked for.
#[test]
fn every_character_typed_operand_carries_the_collation() {
    let columns = json!([
        {"name": "note", "dataType": {"type": "VARCHAR", "size": 100, "characterSet": "UTF8"}},
        {"name": "code", "dataType": {"type": "CHAR", "size": 5, "characterSet": "UTF8"}},
    ]);

    let clause = where_clause_over(
        columns,
        equal(column_operand("note"), column_operand("code")),
    );

    assert_eq!(
        clause,
        "(`note` COLLATE utf8mb4_0900_as_cs = `code` COLLATE utf8mb4_0900_as_cs)"
    );
}

/// Exasol normalizes `id > 5` into `predicate_less` with the operands
/// swapped, so the renderer reproduces the order it was given rather than
/// assuming the column comes first.
#[test]
fn swapped_comparison_keeps_operand_order() {
    let clause = where_clause(json!({
        "type": "predicate_less",
        "left": numeric_literal("5"),
        "right": column_operand("id"),
    }));

    assert_eq!(clause, "(5 < `id`)");
}

#[test]
fn each_comparison_operator_renders_its_mysql_counterpart() {
    for (predicate_type, operator) in [
        ("predicate_equal", "="),
        ("predicate_notequal", "<>"),
        ("predicate_less", "<"),
        ("predicate_lessequal", "<="),
    ] {
        let clause = where_clause(json!({
            "type": predicate_type,
            "left": column_operand("id"),
            "right": numeric_literal("2"),
        }));

        assert_eq!(clause, format!("(`id` {operator} 2)"));
    }
}

#[test]
fn string_literal_renders_as_hex() {
    for (value, expected) in [
        ("widget", "_utf8mb4 X'776964676574'"),
        ("", "_utf8mb4 X''"),
        ("a'b", "_utf8mb4 X'612762'"),
        ("a\\b", "_utf8mb4 X'615C62'"),
        ("Gîzmo", "_utf8mb4 X'47C3AE7A6D6F'"),
    ] {
        let clause = where_clause(equal(column_operand("note"), string_literal(value)));

        assert!(
            clause.contains(expected),
            "the literal `{value}` must render as `{expected}`: {clause}"
        );
        assert!(
            value.is_empty() || !clause.contains(value),
            "the literal `{value}` must not reach the statement as raw characters: {clause}"
        );
    }
}

#[test]
fn temporal_literals_render_typed() {
    let columns = json!([
        {"name": "shipped", "dataType": {"type": "DATE"}},
        {"name": "logged", "dataType": {"type": "TIMESTAMP"}},
    ]);

    let clause = where_clause_over(
        columns,
        json!({
            "type": "predicate_and",
            "expressions": [
                equal(column_operand("shipped"), json!({"type": "literal_date", "value": "2015-12-01"})),
                equal(
                    column_operand("logged"),
                    json!({"type": "literal_timestamp", "value": "2015-12-01 12:01:01.123"}),
                ),
            ],
        }),
    );

    assert_eq!(
        clause,
        "((`shipped` = DATE '2015-12-01') AND (`logged` = TIMESTAMP '2015-12-01 12:01:01.123'))"
    );
}

#[test]
fn nested_predicates_are_parenthesized() {
    let clause = where_clause(json!({
        "type": "predicate_not",
        "expression": {
            "type": "predicate_or",
            "expressions": [
                equal(column_operand("id"), numeric_literal("1")),
                {
                    "type": "predicate_lessequal",
                    "left": column_operand("total"),
                    "right": {"type": "literal_double", "value": "2.5"},
                },
            ],
        },
    }));

    assert_eq!(clause, "(NOT ((`id` = 1) OR (`total` <= 2.5)))");
}

/// A request carrying no `escapeChar` must render no `ESCAPE` clause, which
/// the exact match this test pins for `without_escape` already proves.
#[test]
fn like_renders_pattern_and_escape() {
    let with_escape = where_clause(json!({
        "type": "predicate_like",
        "expression": column_operand("note"),
        "pattern": string_literal("Wid%"),
        "escapeChar": string_literal("\\"),
    }));

    assert_eq!(
        with_escape,
        "(`note` COLLATE utf8mb4_0900_as_cs LIKE _utf8mb4 X'57696425' ESCAPE _utf8mb4 X'5C')"
    );

    let without_escape = where_clause(json!({
        "type": "predicate_like",
        "expression": column_operand("note"),
        "pattern": string_literal("Wid%"),
    }));

    assert_eq!(
        without_escape,
        "(`note` COLLATE utf8mb4_0900_as_cs LIKE _utf8mb4 X'57696425')"
    );
}

/// A null test must not be rendered as a comparison against a null literal,
/// which is never true, and it compares no character value, so it carries no
/// collation either — both of which the exact match below already proves.
#[test]
fn null_predicates_render_without_comparison() {
    for (predicate_type, expected) in [
        ("predicate_is_null", "(`note` IS NULL)"),
        ("predicate_is_not_null", "(`note` IS NOT NULL)"),
    ] {
        let clause = where_clause(json!({
            "type": predicate_type,
            "expression": column_operand("note"),
        }));

        assert_eq!(clause, expected);
    }
}

/// The collation belongs on the compared operand, not on the list: the exact
/// match this test pins carries `COLLATE` exactly once, on `note` rather than
/// inside the `IN` list.
#[test]
fn between_and_in_keep_operand_order() {
    let between = where_clause(json!({
        "type": "predicate_between",
        "expression": column_operand("id"),
        "left": numeric_literal("9"),
        "right": numeric_literal("1"),
    }));

    assert_eq!(between, "(`id` BETWEEN 9 AND 1)");

    let in_list = where_clause(json!({
        "type": "predicate_in_constlist",
        "expression": column_operand("note"),
        "arguments": [string_literal("c"), string_literal("a"), string_literal("b")],
    }));

    assert_eq!(
        in_list,
        "(`note` COLLATE utf8mb4_0900_as_cs IN (_utf8mb4 X'63', _utf8mb4 X'61', _utf8mb4 X'62'))"
    );
}

/// MySQL writes no empty `AND`, `OR`, or `IN` list, and a renderer that
/// dropped the node instead would hand Exasol rows it never asked for.
#[test]
fn a_predicate_with_no_operand_fails_rather_than_rendering_an_empty_group() {
    for (filter, named) in [
        (json!({"type": "predicate_and", "expressions": []}), "AND"),
        (json!({"type": "predicate_or", "expressions": []}), "OR"),
        (
            json!({
                "type": "predicate_in_constlist",
                "expression": column_operand("id"),
                "arguments": [],
            }),
            "IN",
        ),
    ] {
        let error = filter_render_error(filter);

        assert!(
            error.contains(named),
            "the error must name the empty MySQL form: {error}"
        );
    }
}

/// The transition case between empty (refused above) and multi-operand
/// (covered by `nested_predicates_are_parenthesized` and
/// `between_and_in_keep_operand_order`): one operand renders bare inside its
/// own parentheses, with no operator to join it to.
#[test]
fn a_single_operand_junction_or_in_list_renders_bare() {
    let and = where_clause(json!({
        "type": "predicate_and",
        "expressions": [equal(column_operand("id"), numeric_literal("2"))],
    }));
    assert_eq!(and, "((`id` = 2))");

    let or = where_clause(json!({
        "type": "predicate_or",
        "expressions": [equal(column_operand("id"), numeric_literal("2"))],
    }));
    assert_eq!(or, "((`id` = 2))");

    let in_list = where_clause(json!({
        "type": "predicate_in_constlist",
        "expression": column_operand("id"),
        "arguments": [numeric_literal("2")],
    }));
    assert_eq!(in_list, "(`id` IN (2))");
}

/// `vs-adapter/pushdown-filter` requires that no literal can change the
/// shape of the generated statement. A string literal earns that by hex
/// encoding; a numeric or temporal literal, which MySQL accepts only spelled
/// out, earns it by carrying nothing that could.
#[test]
fn a_literal_that_could_change_the_statement_shape_fails_to_render() {
    for (literal, fragment) in [
        (numeric_literal("1 OR 1=1"), "1 OR 1=1"),
        (
            json!({"type": "literal_double", "value": "1)/**/OR(1"}),
            "1)/**/OR(1",
        ),
        (
            json!({"type": "literal_date", "value": "2015-12-01' OR '1"}),
            "2015-12-01' OR '1",
        ),
        (
            json!({"type": "literal_timestamp", "value": ""}),
            "TIMESTAMP",
        ),
        (numeric_literal(""), "0123456789+-.eE"),
        (
            json!({"type": "literal_double", "value": ""}),
            "0123456789+-.eE",
        ),
    ] {
        let error = filter_render_error(equal(column_operand("id"), literal));

        assert!(
            error.contains(fragment),
            "the error must name the literal it refused: {error}"
        );
    }
}

#[test]
fn a_null_literal_renders_as_mysql_null() {
    let clause = where_clause(equal(column_operand("id"), json!({"type": "literal_null"})));

    assert_eq!(clause, "(`id` = NULL)");
}

#[test]
fn named_columns_are_projected() {
    let select_list = json!([{"type": "column", "name": "id"}, {"type": "column", "name": "note"}]);

    let sql = render(orders_columns(), Some(select_list));

    assert_eq!(sql, "SELECT `id`, `note` FROM `shop`.`orders`");
}

/// An absent select list projects every mapped column by name rather than
/// `*`, which the exact match this test pins already proves.
#[test]
fn absent_select_list_projects_all() {
    let sql = render(orders_columns(), None);

    assert_eq!(sql, "SELECT `id`, `total`, `note` FROM `shop`.`orders`");
}

#[test]
fn empty_select_list_projects_constant() {
    let sql = render(orders_columns(), Some(json!([])));

    assert_eq!(sql, "SELECT 1 FROM `shop`.`orders`");
}

#[test]
fn backtick_is_doubled() {
    let columns = json!([{"name": "a`b", "dataType": {"type": "DATE"}}]);
    let sql = render(columns, None);

    assert_eq!(sql, "SELECT `a``b` FROM `shop`.`orders`");
}

#[test]
fn a_query_projecting_nothing_fails_rather_than_rendering_an_empty_projection() {
    let query = Query {
        select_list: vec![],
        filter: None,
        aggregation: None,
        from: FromClause::Table(SourceTable {
            namespace: SourceQualifier::single(DATABASE),
            name: "orders".to_string(),
            alias: None,
        }),
    };

    let error = MySqlDialect
        .render(&query)
        .expect_err("a query projecting no column must fail");

    assert!(
        error.to_string().contains("select list"),
        "the error must name the empty part of the query: {error}"
    );
}

/// A `function_aggregate` select-list entry over at most one argument.
/// `None` is the `COUNT(*)` shape: Exasol sends it carrying no `arguments`
/// at all, which is how it is told apart from `COUNT(column)`.
fn aggregate(function: &str, argument: Option<Value>) -> Value {
    let mut entry = json!({"type": "function_aggregate", "name": function});
    if let Some(argument) = argument {
        entry["arguments"] = json!([argument]);
    }
    entry
}

/// A `pushdown` request over `orders` carrying `select_list` and the
/// `selectListDataTypes` Exasol supplies for it, which is the only type
/// source an aggregate entry has.
fn typed_request(select_list: Value, data_types: Value) -> Value {
    let mut request = request(orders_columns(), Some(select_list));
    request["pushdownRequest"]["selectListDataTypes"] = data_types;
    request
}

/// The type Exasol supplies for `COUNT` over any operand, measured on
/// Exasol 2025.2.1.
fn count_type() -> Value {
    json!({"type": "DECIMAL", "precision": 18, "scale": 0})
}

/// The type Exasol supplies for `MIN` or `MAX` over `total`, a
/// `DECIMAL(12,4)` — the operand's own type, exactly as `SUM` no longer
/// reaches this dialect to declare a wider one for it.
fn total_type() -> Value {
    json!({"type": "DECIMAL", "precision": 12, "scale": 4})
}

/// The type Exasol supplies for `MIN` or `MAX` over `note`, a
/// `VARCHAR(100)` — the operand's own type, spelled without the
/// `characterSet` field `involvedTables[].columns[].dataType` carries.
fn note_type() -> Value {
    json!({"type": "VARCHAR", "size": 100})
}

/// A single-group aggregation groups by nothing, which the exact match this
/// test pins already proves: it carries no `GROUP BY`.
#[test]
fn single_group_renders_without_group_by() {
    let mut request = typed_request(
        json!([aggregate("COUNT", Some(column_operand("total")))]),
        json!([count_type()]),
    );
    request["pushdownRequest"]["aggregationType"] = json!("single_group");

    let sql = render_statement(&request);

    assert_eq!(sql, "SELECT COUNT(`total`) FROM `shop`.`orders`");
}

/// The type Exasol supplies for `id`, a `DECIMAL(19,0)` — the operand's own
/// type, as `MIN`, `MAX`, and a projected grouping column all carry.
fn id_type() -> Value {
    json!({"type": "DECIMAL", "precision": 19, "scale": 0})
}

/// A `pushdown` request over `orders` whose `aggregationType` is `group_by`
/// over `group_by`, the shape the one collation rule spanning the `GROUP BY`
/// clause and the select list is read from.
fn grouped_request(select_list: Value, data_types: Value, group_by: Value) -> Value {
    let mut request = typed_request(select_list, data_types);
    request["pushdownRequest"]["aggregationType"] = json!("group_by");
    request["pushdownRequest"]["groupBy"] = group_by;
    request
}

/// Exasol orders `groupBy` independently of `selectList`, so the two orders
/// are asserted against each other rather than assumed to agree.
#[test]
fn group_by_keeps_request_order() {
    let request = grouped_request(
        json!([
            column_operand("id"),
            column_operand("note"),
            aggregate("COUNT", Some(column_operand("total"))),
        ]),
        json!([id_type(), note_type(), count_type()]),
        json!([column_operand("note"), column_operand("id")]),
    );

    let sql = render_statement(&request);

    assert_eq!(
        sql,
        "SELECT `id`, `note` COLLATE utf8mb4_0900_as_cs, COUNT(`total`) FROM `shop`.`orders` \
         GROUP BY `note` COLLATE utf8mb4_0900_as_cs, `id`"
    );
}

/// The grouping collation is one rule over two clauses, not two rules.
/// Measured on MySQL 8.4, `SELECT v, COUNT(*) FROM n GROUP BY v COLLATE
/// utf8mb4_0900_as_cs` fails with error 1055 under `ONLY_FULL_GROUP_BY`,
/// which `mysql:8.4` sets by default: MySQL does not see a bare `v` as
/// functionally dependent on a collated grouping key. Collating both
/// occurrences runs and returns the four groups Exasol returns over
/// `Widget`, `widget`, `Gizmo`, `Gîzmo`, where the column's own
/// `utf8mb4_0900_ai_ci` yields two.
///
/// A `MIN` over that same column keeps the ordering collation in the same
/// statement — measured valid, because an aggregate is never subject to the
/// mode and the value `MIN` returns is chosen by order rather than by
/// equality.
///
/// The grouping key and its bare select-list occurrence must carry the same
/// collation, or MySQL rejects the statement with error 1055 — which the
/// exact match this test pins already proves, since it carries
/// `` `note` COLLATE utf8mb4_0900_as_cs `` in both the select list and the
/// `GROUP BY` clause.
#[test]
fn grouped_character_column_carries_the_comparison_collation_in_both_clauses() {
    let request = grouped_request(
        json!([
            column_operand("note"),
            column_operand("id"),
            aggregate("MIN", Some(column_operand("note"))),
        ]),
        json!([note_type(), id_type(), note_type()]),
        json!([column_operand("note"), column_operand("id")]),
    );

    let sql = render_statement(&request);

    assert_eq!(
        sql,
        "SELECT `note` COLLATE utf8mb4_0900_as_cs, `id`, MIN(`note` COLLATE utf8mb4_0900_bin) \
         FROM `shop`.`orders` GROUP BY `note` COLLATE utf8mb4_0900_as_cs, `id`"
    );
}

/// Exasol qualifies a column node with `tableName`, with `tableAlias`, or
/// with both, and it qualifies the `groupBy` occurrence of a column
/// independently of the `selectList` occurrence of the same column. One
/// column named two ways is still one column, and both occurrences must
/// render identically: MySQL matches a select-list expression against a
/// `GROUP BY` expression whole and rejects the statement with error 1055
/// under `ONLY_FULL_GROUP_BY` when the two differ at all. Deciding it by
/// whole-struct equality of the two references would answer that a
/// differently qualified occurrence is a different column, render the
/// projection bare beside a collated key, and produce exactly that
/// rejection.
#[test]
fn a_grouping_key_is_recognized_however_the_request_qualifies_its_occurrences() {
    let request = grouped_request(
        json!([
            {"type": "column", "name": "note", "tableName": "orders"},
            aggregate("COUNT", None),
        ]),
        json!([note_type(), count_type()]),
        json!([column_operand("note")]),
    );

    let sql = render_statement(&request);

    assert_eq!(
        sql,
        "SELECT `note` COLLATE utf8mb4_0900_as_cs, COUNT(*) FROM `shop`.`orders` \
         GROUP BY `note` COLLATE utf8mb4_0900_as_cs"
    );
}

/// The same rule inside a join, where the reference also decides the alias
/// each occurrence is qualified by. Both occurrences resolve to the one
/// table `customers`, one through its `tableAlias` and one through its
/// `tableName`, so both must carry the same alias and the same collation.
#[test]
fn a_joined_grouping_key_is_recognized_however_the_request_qualifies_its_occurrences() {
    let mut request = join_request(
        orders_to_customers("inner"),
        json!([
            {"type": "column", "name": "name", "tableName": "customers", "tableAlias": "customers"},
            aggregate("COUNT", None),
        ]),
    );
    request["pushdownRequest"]["selectListDataTypes"] = json!([note_type(), count_type()]);
    request["pushdownRequest"]["aggregationType"] = json!("group_by");
    request["pushdownRequest"]["groupBy"] = json!([joined_column("name", "customers")]);

    let sql = render_statement(&request);

    let occurrence = "`t2`.`name` COLLATE utf8mb4_0900_as_cs";
    assert_eq!(
        sql.matches(occurrence).count(),
        2,
        "both occurrences of the grouping key must carry the same alias and the same collation, \
         whichever field the request qualified each one by: {sql}"
    );
}

/// A numeric grouping key has no character values to compare, so it carries
/// no collation in either clause — the rule reads off the operand's declared
/// type, as every collation decision in this dialect does. The exact match
/// this test pins already proves the absence: it carries no `COLLATE`.
#[test]
fn numeric_grouping_key_carries_no_collation() {
    let sql = render_statement(&grouped_request(
        json!([column_operand("id"), aggregate("COUNT", None)]),
        json!([id_type(), count_type()]),
        json!([column_operand("id")]),
    ));

    assert_eq!(
        sql,
        "SELECT `id`, COUNT(*) FROM `shop`.`orders` GROUP BY `id`"
    );
}

/// The collation reads off both the operand's declared type and whether the
/// query groups by that column, never off the type alone: a projection-only
/// statement renders byte-identically to what this dialect rendered before
/// the grouping rule existed.
#[test]
fn ungrouped_projection_carries_no_collation() {
    let sql = render(orders_columns(), Some(json!([column_operand("note")])));

    assert_eq!(sql, "SELECT `note` FROM `shop`.`orders`");
}

/// A grouping key the select list does not project needs no second
/// occurrence: `ONLY_FULL_GROUP_BY` constrains the select list, and there is
/// nothing in it to constrain. Measured valid on MySQL 8.4.
#[test]
fn an_unprojected_grouping_key_carries_the_collation_alone() {
    let sql = render_statement(&grouped_request(
        json!([aggregate("COUNT", None)]),
        json!([count_type()]),
        json!([column_operand("note")]),
    ));

    assert_eq!(
        sql,
        "SELECT COUNT(*) FROM `shop`.`orders` GROUP BY `note` COLLATE utf8mb4_0900_as_cs"
    );
}

/// `DISTINCT` reduces values by equality, so a character operand of one
/// carries the collation equality is measured correct under: over `Widget`,
/// `widget`, `Gizmo`, and `Gîzmo`, MySQL's default `utf8mb4_0900_ai_ci`
/// returns a `COUNT(DISTINCT)` of 2 where Exasol returns 4.
///
/// `COUNT` over the same column without `distinct` reads neither equality
/// nor order and carries nothing, and `MIN` keeps the ordering collation
/// whatever `distinct` holds, because the value it returns is chosen by
/// order.
#[test]
fn distinct_character_operand_carries_the_comparison_collation() {
    let mut distinct_count = aggregate("COUNT", Some(column_operand("note")));
    distinct_count["distinct"] = json!(true);
    let mut distinct_numeric_count = aggregate("COUNT", Some(column_operand("id")));
    distinct_numeric_count["distinct"] = json!(true);
    let mut distinct_min = aggregate("MIN", Some(column_operand("note")));
    distinct_min["distinct"] = json!(true);

    let sql = render_statement(&typed_request(
        json!([
            distinct_count,
            distinct_numeric_count,
            aggregate("COUNT", Some(column_operand("note"))),
            distinct_min,
        ]),
        json!([count_type(), count_type(), count_type(), note_type()]),
    ));

    assert_eq!(
        sql,
        "SELECT COUNT(DISTINCT `note` COLLATE utf8mb4_0900_as_cs), COUNT(DISTINCT `id`), \
         COUNT(`note`), MIN(DISTINCT `note` COLLATE utf8mb4_0900_bin) FROM `shop`.`orders`"
    );
}

/// `COUNT(*)` counts rows and `COUNT(column)` counts that column's non-null
/// values, so rendering one as the other is a wrong value rather than a
/// syntax error.
#[test]
fn count_star_differs_from_count_column() {
    let sql = render_statement(&typed_request(
        json!([
            aggregate("COUNT", None),
            aggregate("COUNT", Some(column_operand("note"))),
        ]),
        json!([count_type(), count_type()]),
    ));

    assert_eq!(sql, "SELECT COUNT(*), COUNT(`note`) FROM `shop`.`orders`");
}

/// Measured on Exasol 2025.2.1, one request carried `sum`, `min`, and `max`
/// in lower case beside `AVG` and `COUNT` in upper case, so the case a
/// request arrives in must not reach the generated statement.
#[test]
fn mixed_case_function_names_render_identically() {
    let entries = |count: &str, min: &str| {
        json!([
            aggregate(count, Some(column_operand("total"))),
            aggregate(min, Some(column_operand("note"))),
        ])
    };
    let types = json!([count_type(), note_type()]);

    let lower_case = render_statement(&typed_request(entries("count", "min"), types.clone()));
    let upper_case = render_statement(&typed_request(entries("COUNT", "MIN"), types));

    assert_eq!(
        lower_case, upper_case,
        "two requests differing only in the aggregate function's case must render identically"
    );
    assert_eq!(
        lower_case,
        "SELECT COUNT(`total`), MIN(`note` COLLATE utf8mb4_0900_bin) FROM `shop`.`orders`"
    );
}

/// Exasol orders character values by code point and pads no `VARCHAR`, so a
/// character `MIN` or `MAX` carries the collation that reproduces both.
/// `COUNT` reads no order, and a numeric operand has no collation to carry.
///
/// The comparison collation orders by collation weight and returns `a` where
/// Exasol returns `B`, so ordering must not borrow it — which the exact
/// match this test pins already proves: it carries no `utf8mb4_0900_as_cs`.
#[test]
fn only_min_and_max_over_a_character_operand_carry_the_ordering_collation() {
    let sql = render_statement(&typed_request(
        json!([
            aggregate("MIN", Some(column_operand("note"))),
            aggregate("MAX", Some(column_operand("note"))),
            aggregate("COUNT", Some(column_operand("note"))),
            aggregate("MIN", Some(column_operand("total"))),
        ]),
        json!([note_type(), note_type(), count_type(), total_type()]),
    ));

    assert_eq!(
        sql,
        "SELECT MIN(`note` COLLATE utf8mb4_0900_bin), MAX(`note` COLLATE utf8mb4_0900_bin), \
         COUNT(`note`), MIN(`total`) FROM `shop`.`orders`"
    );
}

/// Rendering the aggregate's argument alone would leave Exasol aggregating
/// nothing and returning the wrong value, so an unknown function fails. `AVG`
/// and `SUM` fail the same way: both are withheld capabilities, so neither
/// reaches `renderable_function` from an advertising Exasol, but a request
/// that somehow sends one anyway must still fail naming it rather than
/// rendering an argument Exasol never asked to aggregate.
#[test]
fn unrenderable_aggregate_named_in_error() {
    for (function, named) in [
        ("STDDEV", "STDDEV"),
        ("stddev", "STDDEV"),
        ("AVG", "AVG"),
        ("SUM", "SUM"),
    ] {
        let error = render_error(&typed_request(
            json!([aggregate(function, Some(column_operand("total")))]),
            json!([{"type": "DOUBLE"}]),
        ));

        assert!(
            error.contains(named),
            "the error must name the aggregate function it received: {error}"
        );
    }
}

/// `COUNT(*)` is the only aggregate MySQL writes over no argument, and it
/// counts rows rather than values, so `COUNT(DISTINCT *)` is not a form
/// either. Dropping the `DISTINCT` Exasol believes was delegated would
/// count every row where it asked for the distinct values.
#[test]
fn an_aggregate_carrying_no_argument_fails_unless_it_is_count_star() {
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
        "the error must name the fragment MySQL has no form for: {count_error}"
    );
}

/// MySQL writes no empty `GROUP BY` clause, and a renderer that dropped the
/// clause instead would aggregate the whole table into one row where Exasol
/// asked for one row per group.
#[test]
fn an_aggregation_grouping_by_no_column_fails_rather_than_rendering_an_empty_clause() {
    let mut request = typed_request(json!([aggregate("COUNT", None)]), json!([count_type()]));
    request["pushdownRequest"]["aggregationType"] = json!("group_by");
    request["pushdownRequest"]["groupBy"] = json!([]);

    let error = render_error(&request);

    assert!(
        error.contains("GROUP BY"),
        "the error must name the empty MySQL clause: {error}"
    );
}

/// The type Exasol supplies for a select-list predicate, measured on Exasol
/// 2025.2.1: a delegated predicate emits one `BOOLEAN` column.
fn boolean_type() -> Value {
    json!({"type": "BOOLEAN"})
}

/// The five predicate kinds measured arriving in select-list position over
/// the query's own grouping column — `"g" > 'x'`, `"g" IS NULL`,
/// `"g" LIKE 'a%'`, `"g" IN ('a','b')`, and `"g" BETWEEN 'a' AND 'z'` — each
/// reading `operand` outside any aggregate.
fn select_list_predicates_over(operand: Value) -> Vec<Value> {
    vec![
        less(string_literal("x"), operand.clone()),
        json!({"type": "predicate_is_null", "expression": operand.clone()}),
        json!({
            "type": "predicate_like",
            "expression": operand.clone(),
            "pattern": string_literal("a%"),
        }),
        json!({
            "type": "predicate_in_constlist",
            "expression": operand.clone(),
            "arguments": [string_literal("a"), string_literal("b")],
        }),
        json!({
            "type": "predicate_between",
            "expression": operand,
            "left": string_literal("a"),
            "right": string_literal("z"),
        }),
    ]
}

/// Measured on Exasol 2025.2.1, five predicate kinds arrive in select-list
/// position reading the query's own character `GROUP BY` column, and measured
/// on MySQL 8.4 every rendering of that shape fails with error 1055 under
/// `ONLY_FULL_GROUP_BY`, including the one leaving the operand uncollated:
/// MySQL matches a select-list expression against a `GROUP BY` expression
/// whole and credits no sub-expression of the collated grouping key. The
/// adapter therefore fails first, naming the column, rather than forwarding a
/// statement the source rejects against a query the operator never wrote.
#[test]
fn a_select_list_predicate_reading_the_grouped_character_column_names_it_in_the_error() {
    for predicate in select_list_predicates_over(column_operand("note")) {
        let error = render_error(&grouped_request(
            json!([column_operand("note"), predicate.clone()]),
            json!([note_type(), boolean_type()]),
            json!([column_operand("note")]),
        ));

        assert!(
            error.contains("note"),
            "the error must name the grouped column the predicate reads: {error}, for {predicate}"
        );
    }
}

/// Measured on Exasol 2025.2.1, an `AND` of two comparisons reaches the
/// select list as one entry, so the grouped column can sit anywhere in a
/// nested operand tree rather than directly under the entry. MySQL rejects
/// the statement for the same reason at any depth, so the search follows the
/// tree rather than reading the entry's immediate operands.
#[test]
fn a_grouped_character_column_nested_inside_a_select_list_predicate_still_fails() {
    let error = render_error(&grouped_request(
        json!([
            column_operand("note"),
            json!({
                "type": "predicate_and",
                "expressions": [
                    less(numeric_literal("0"), aggregate("COUNT", None)),
                    {
                        "type": "predicate_not",
                        "expression": less(string_literal("x"), column_operand("note")),
                    },
                ],
            }),
        ]),
        json!([note_type(), boolean_type()]),
        json!([column_operand("note")]),
    ));

    assert!(
        error.contains("note"),
        "the search must follow the operand tree rather than the entry's immediate operands, \
         because MySQL rejects the statement at any depth: {error}"
    );
}

/// The grouping key is a collated expression whether or not the select list
/// projects it, so MySQL rejects the variant Exasol was measured to send
/// carrying no bare grouped column at all, and the adapter fails on it too.
#[test]
fn an_unprojected_grouped_character_column_read_by_a_select_list_predicate_still_fails() {
    let error = render_error(&grouped_request(
        json!([less(string_literal("x"), column_operand("note"))]),
        json!([boolean_type()]),
        json!([column_operand("note")]),
    ));

    assert!(
        error.contains("note"),
        "the grouping key is collated whether or not it is projected, so the error must still \
         name it: {error}"
    );
}

/// A numeric grouping key renders bare, so MySQL matches the predicate's
/// operand against the whole `GROUP BY` expression and credits it as
/// functionally dependent — measured on MySQL 8.4, four rows rather than
/// error 1055. The rejection therefore reads off the same declared type the
/// grouping collation reads, not off grouping alone. A numeric grouping key
/// carries no collation in either occurrence, which the exact match this
/// test pins already proves.
#[test]
fn a_select_list_predicate_over_a_numeric_grouping_key_renders() {
    let sql = render_statement(&grouped_request(
        json!([
            column_operand("id"),
            less(numeric_literal("1"), column_operand("id")),
        ]),
        json!([id_type(), boolean_type()]),
        json!([column_operand("id")]),
    ));

    assert_eq!(
        sql,
        "SELECT `id`, (1 < `id`) FROM `shop`.`orders` GROUP BY `id`"
    );
}

/// An aggregate reduces its group to one value, so MySQL matches it whole and
/// never reads the grouped column inside it bare. Measured on MySQL 8.4, both
/// predicates return the four rows Exasol returns beside a collated grouping
/// key.
///
/// `MIN` returns one of its argument's own values, so it carries that
/// argument's type: the comparison collation lands on the aggregate while the
/// argument keeps the ordering collation the returned value is chosen by.
#[test]
fn a_select_list_predicate_over_an_aggregate_renders() {
    let sql = render_statement(&grouped_request(
        json!([
            column_operand("note"),
            less(numeric_literal("0"), aggregate("COUNT", None)),
            less(
                string_literal("x"),
                aggregate("MIN", Some(column_operand("note"))),
            ),
        ]),
        json!([note_type(), boolean_type(), boolean_type()]),
        json!([column_operand("note")]),
    ));

    assert_eq!(
        sql,
        "SELECT `note` COLLATE utf8mb4_0900_as_cs, (0 < COUNT(*)), (_utf8mb4 X'78' COLLATE \
         utf8mb4_0900_as_cs < MIN(`note` COLLATE utf8mb4_0900_bin) COLLATE utf8mb4_0900_as_cs) \
         FROM `shop`.`orders` GROUP BY `note` COLLATE utf8mb4_0900_as_cs"
    );
}

/// A query grouping by nothing carries no grouping key for a select-list
/// predicate to disagree with, so the character column renders as an ordinary
/// comparison operand. `single_group` and no `aggregationType` at all are the
/// two such shapes Exasol sends.
#[test]
fn a_select_list_predicate_renders_where_the_query_groups_by_nothing() {
    let ungrouped = render_statement(&typed_request(
        json!([less(string_literal("x"), column_operand("note"))]),
        json!([boolean_type()]),
    ));

    assert_eq!(
        ungrouped,
        "SELECT (_utf8mb4 X'78' COLLATE utf8mb4_0900_as_cs < `note` COLLATE utf8mb4_0900_as_cs) \
         FROM `shop`.`orders`"
    );

    let mut single_group = typed_request(
        json!([less(numeric_literal("0"), aggregate("COUNT", None))]),
        json!([boolean_type()]),
    );
    single_group["pushdownRequest"]["aggregationType"] = json!("single_group");

    assert_eq!(
        render_statement(&single_group),
        "SELECT (0 < COUNT(*)) FROM `shop`.`orders`"
    );
}

/// A select-list entry that is neither a column nor a predicate falls through
/// to the same literal renderer a filter operand reaches, so the select-list
/// path stays a routing decision rather than a second literal rule.
///
/// Built as a model rather than parsed from a request: no capability this
/// release advertises makes Exasol send a literal select-list entry, and the
/// assertion is about which renderer the entry reaches.
#[test]
fn a_literal_select_list_entry_renders_through_the_literal_renderer() {
    let query = Query {
        select_list: vec![pushdown_query::SelectItem {
            expression: Expression::Literal(Literal::String("x".to_string())),
            declared_type: ExasolType::Varchar { size: 100 },
        }],
        filter: None,
        aggregation: None,
        from: FromClause::Table(SourceTable {
            namespace: SourceQualifier::single(DATABASE),
            name: "orders".to_string(),
            alias: None,
        }),
    };

    let sql = MySqlDialect
        .render(&query)
        .expect("a literal select-list entry must render");

    assert_eq!(sql, "SELECT _utf8mb4 X'78' FROM `shop`.`orders`");
}

/// One fixture per advertised capability — a `Query` shaped so rendering it
/// actually exercises what the capability promises, paired with the fragment
/// the rendered statement must then carry. Grows by one arm every time a
/// later task advertises a new capability: an entry with no fixture here is
/// the invariant test failing loudly instead of silently skipping a
/// capability nothing renders.
///
/// The fragment is what makes the invariant read the statement rather than
/// only its `Ok`. `vs-adapter/pushdown-dialect` requires that no capability
/// reach a fallback that drops the fragment, and a renderer that dropped an
/// `ESCAPE`, a `DISTINCT`, or a `NOT` would return `Ok` over a statement that
/// selects different rows than Exasol asked for. Each fragment is the part of
/// the statement its own capability contributes, and it travels in the same
/// arm as the fixture it is true of, so a changed fixture cannot leave behind
/// a fragment that no longer follows from it.
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

    fn aggregated(select_list: Vec<pushdown_query::SelectItem>) -> Query {
        fixtures::aggregated(&namespace(), select_list)
    }

    fn joined_query(join_type: &str) -> Query {
        fixtures::joined_query(&namespace(), join_type)
    }

    match capability {
        "SELECTLIST_PROJECTION" => (
            "SELECT `id` FROM",
            Query {
                select_list: select_list(),
                filter: None,
                aggregation: None,
                from: from(),
            },
        ),
        "SELECTLIST_EXPRESSIONS" => (
            "SELECT (`id` = 2)",
            Query {
                select_list: vec![pushdown_query::SelectItem {
                    expression: Expression::Predicate(Box::new(id_equals(numeric("2")))),
                    declared_type: ExasolType::Boolean,
                }],
                filter: None,
                aggregation: None,
                from: from(),
            },
        ),
        "FILTER_EXPRESSIONS" => ("WHERE (`id` = 2)", filtered(id_equals(numeric("2")))),
        "AGGREGATE_SINGLE_GROUP" => (
            "SELECT COUNT(*) FROM",
            aggregated(vec![aggregate_item(count_star())]),
        ),
        "AGGREGATE_GROUP_BY_COLUMN" => (
            "GROUP BY `id`",
            Query {
                select_list: select_list(),
                filter: None,
                aggregation: Some(pushdown_query::Aggregation::GroupBy(vec![decimal_column()])),
                from: from(),
            },
        ),
        "FN_AGG_COUNT" => (
            "COUNT(`id`)",
            aggregated(vec![aggregate_item(aggregate_call(
                "COUNT",
                Expression::Column(decimal_column()),
                false,
            ))]),
        ),
        "FN_AGG_COUNT_STAR" => ("COUNT(*)", aggregated(vec![aggregate_item(count_star())])),
        "FN_AGG_COUNT_DISTINCT" => (
            "COUNT(DISTINCT `id`)",
            aggregated(vec![aggregate_item(aggregate_call(
                "COUNT",
                Expression::Column(decimal_column()),
                true,
            ))]),
        ),
        "FN_AGG_MIN" => (
            "MIN(`id`)",
            aggregated(vec![aggregate_item(aggregate_call(
                "MIN",
                Expression::Column(decimal_column()),
                false,
            ))]),
        ),
        "FN_AGG_MAX" => (
            "MAX(`id`)",
            aggregated(vec![aggregate_item(aggregate_call(
                "MAX",
                Expression::Column(decimal_column()),
                false,
            ))]),
        ),
        "FN_PRED_AND" => (
            "((`id` = 1) AND (`id` = 2))",
            filtered(Predicate::And(vec![
                id_equals(numeric("1")),
                id_equals(numeric("2")),
            ])),
        ),
        "FN_PRED_OR" => (
            "((`id` = 1) OR (`id` = 2))",
            filtered(Predicate::Or(vec![
                id_equals(numeric("1")),
                id_equals(numeric("2")),
            ])),
        ),
        "FN_PRED_NOT" => (
            "(NOT (`id` = 2))",
            filtered(Predicate::Not(Box::new(id_equals(numeric("2"))))),
        ),
        "FN_PRED_EQUAL" => ("(`id` = 2)", filtered(id_equals(numeric("2")))),
        "FN_PRED_NOTEQUAL" => (
            "(`id` <> 2)",
            filtered(Predicate::NotEqual(Comparison {
                left: Expression::Column(decimal_column()),
                right: numeric("2"),
            })),
        ),
        "FN_PRED_LESS" => (
            "(`id` < 2)",
            filtered(Predicate::Less(Comparison {
                left: Expression::Column(decimal_column()),
                right: numeric("2"),
            })),
        ),
        "FN_PRED_LESSEQUAL" => (
            "(`id` <= 2)",
            filtered(Predicate::LessEqual(Comparison {
                left: Expression::Column(decimal_column()),
                right: numeric("2"),
            })),
        ),
        "FN_PRED_IS_NULL" => (
            "(`id` IS NULL)",
            filtered(Predicate::IsNull(Expression::Column(decimal_column()))),
        ),
        "FN_PRED_IS_NOT_NULL" => (
            "(`id` IS NOT NULL)",
            filtered(Predicate::IsNotNull(Expression::Column(decimal_column()))),
        ),
        "FN_PRED_LIKE" => (
            "LIKE _utf8mb4 X'57696425'",
            filtered(Predicate::Like(LikePredicate {
                expression: Expression::Column(varchar_column()),
                pattern: Expression::Literal(Literal::String("Wid%".to_string())),
                escape_char: None,
            })),
        ),
        "FN_PRED_LIKE_ESCAPE" => (
            "ESCAPE _utf8mb4 X'5C'",
            filtered(Predicate::Like(LikePredicate {
                expression: Expression::Column(varchar_column()),
                pattern: Expression::Literal(Literal::String("Wid%".to_string())),
                escape_char: Some(Expression::Literal(Literal::String("\\".to_string()))),
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
            "= 2.5",
            filtered(id_equals(Expression::Literal(Literal::Double(
                "2.5".to_string(),
            )))),
        ),
        "LITERAL_STRING" => (
            "_utf8mb4 X'776964676574'",
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
            "`shop`.`orders` AS `t1` INNER JOIN `shop`.`customers` AS `t2`",
            joined_query("inner"),
        ),
        "JOIN_TYPE_INNER" => ("INNER JOIN", joined_query("inner")),
        "JOIN_TYPE_LEFT_OUTER" => ("LEFT OUTER JOIN", joined_query("left_outer")),
        "JOIN_TYPE_RIGHT_OUTER" => ("RIGHT OUTER JOIN", joined_query("right_outer")),
        "JOIN_CONDITION_EQUI" => ("ON (`t1`.`customer_id` = `t2`.`id`)", joined_query("inner")),
        other => panic!(
            "capability `{other}` is advertised but this test has no fixture that exercises it; \
             add one alongside whichever task added the capability"
        ),
    }
}

/// A column node as Exasol sends one inside a join: qualified by the
/// `tableName` of the table it belongs to. Measured on Exasol 2025.2.1, that
/// field arrives on every column node of a join, and `tableAlias` joins it
/// only once the operator's query aliased the table.
fn joined_column(name: &str, table: &str) -> Value {
    json!({"type": "column", "name": name, "tableName": table})
}

fn joined_orders_columns() -> Value {
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

/// The `from` element joining `orders` to `customers` over the foreign key,
/// in the shape a join whose query aliased no table arrives in: neither table
/// carries an `alias` and both condition operands carry a `tableName`.
fn orders_to_customers(join_type: &str) -> Value {
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
/// involved tables both `orders_to_customers` names.
fn join_request(from: Value, select_list: Value) -> Value {
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

/// The two `id` columns share a name, so the projection is readable only
/// because each carries the alias of the table it belongs to. Both tables
/// carry one even though the request assigned neither: the alias comes from
/// the table's position in the `from` tree, which is what keeps the set
/// unique whatever the request supplies.
#[test]
fn a_join_renders_one_statement_with_every_table_aliased_and_every_column_qualified() {
    let sql = render_statement(&join_request(
        orders_to_customers("inner"),
        json!([
            joined_column("id", "orders"),
            joined_column("id", "customers"),
        ]),
    ));

    assert_eq!(
        sql,
        "SELECT `t1`.`id`, `t2`.`id` FROM (`shop`.`orders` AS `t1` INNER JOIN \
         `shop`.`customers` AS `t2` ON (`t1`.`customer_id` = `t2`.`id`))"
    );
}

#[test]
fn each_join_type_renders_its_mysql_keyword_in_the_requests_operand_order() {
    for (join_type, keyword) in [
        ("inner", "INNER JOIN"),
        ("left_outer", "LEFT OUTER JOIN"),
        ("right_outer", "RIGHT OUTER JOIN"),
    ] {
        let sql = render_statement(&join_request(
            orders_to_customers(join_type),
            json!([joined_column("id", "orders")]),
        ));

        assert_eq!(
            sql,
            format!(
                "SELECT `t1`.`id` FROM (`shop`.`orders` AS `t1` {keyword} \
                 `shop`.`customers` AS `t2` ON (`t1`.`customer_id` = `t2`.`id`))"
            ),
            "`{join_type}` must render `{keyword}` over the request's own left and right"
        );
    }
}

/// `full_outer` reaches the renderer because the model keeps `join_type`
/// verbatim, and MySQL implements no `FULL OUTER JOIN`. Substituting another
/// join type would return rows Exasol never asked for, so the value is named
/// back instead.
#[test]
fn an_unrenderable_join_type_fails_naming_the_value_it_received() {
    for join_type in ["full_outer", "cross"] {
        let error = render_error(&join_request(
            orders_to_customers(join_type),
            json!([joined_column("id", "orders")]),
        ));

        assert!(
            error.contains(join_type),
            "the error must name the join type it received: {error}"
        );
    }
}

/// A nested join renders inside its own parentheses, so the tree the request
/// sent is the tree MySQL reads: an outer join whose operand order or nesting
/// shifted would return different rows.
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
        "SELECT `t3`.`code` FROM ((`shop`.`orders` AS `t1` INNER JOIN `shop`.`customers` AS `t2` \
         ON (`t1`.`customer_id` = `t2`.`id`)) LEFT OUTER JOIN `shop`.`regions` AS `t3` \
         ON (`t2`.`id` = `t3`.`customer_id`))"
    );
}

/// Every position a column reaches the renderer from carries the alias of the
/// table it belongs to: the select list, the `WHERE` clause, and the join's
/// own condition. An unqualified reference two joined tables both declare is
/// what MySQL rejects with error 1052.
#[test]
fn every_column_position_in_a_join_carries_its_tables_alias() {
    let mut request = join_request(
        orders_to_customers("inner"),
        json!([joined_column("name", "customers")]),
    );
    request["pushdownRequest"]["filter"] = json!({
        "type": "predicate_equal",
        "left": joined_column("note", "orders"),
        "right": string_literal("x"),
    });

    let sql = render_statement(&request);

    assert_eq!(
        sql,
        "SELECT `t2`.`name` FROM (`shop`.`orders` AS `t1` INNER JOIN `shop`.`customers` AS `t2` \
         ON (`t1`.`customer_id` = `t2`.`id`)) WHERE (`t1`.`note` COLLATE utf8mb4_0900_as_cs = \
         _utf8mb4 X'78' COLLATE utf8mb4_0900_as_cs)"
    );
}

/// A numeric grouping key carries the alias and no collation, so aliasing
/// does not turn the type-keyed collation rule into an unconditional suffix
/// — which the exact match this test pins already proves.
#[test]
fn a_grouped_numeric_column_carries_the_alias_and_no_collation() {
    let mut request = join_request(
        orders_to_customers("inner"),
        json!([joined_column("id", "customers"), aggregate("COUNT", None)]),
    );
    request["pushdownRequest"]["selectListDataTypes"] = json!([id_type(), count_type()]);
    request["pushdownRequest"]["aggregationType"] = json!("group_by");
    request["pushdownRequest"]["groupBy"] = json!([joined_column("id", "customers")]);

    let sql = render_statement(&request);

    assert_eq!(
        sql,
        "SELECT `t2`.`id`, COUNT(*) FROM (`shop`.`orders` AS `t1` INNER JOIN \
         `shop`.`customers` AS `t2` ON (`t1`.`customer_id` = `t2`.`id`)) GROUP BY `t2`.`id`"
    );
}

/// A join carrying both a `filter` and a `groupBy` in the same request must
/// render every clause together, in MySQL's required order: `FROM`, `WHERE`,
/// `GROUP BY`. Each tier renders its own fragment; only this test proves the
/// three compose over a joined `FROM` rather than only over a single table.
#[test]
fn join_composes_with_filter_and_group_by() {
    let mut request = join_request(
        orders_to_customers("inner"),
        json!([joined_column("name", "customers"), aggregate("COUNT", None)]),
    );
    request["pushdownRequest"]["filter"] = json!({
        "type": "predicate_equal",
        "left": joined_column("note", "orders"),
        "right": string_literal("x"),
    });
    request["pushdownRequest"]["selectListDataTypes"] = json!([note_type(), count_type()]);
    request["pushdownRequest"]["aggregationType"] = json!("group_by");
    request["pushdownRequest"]["groupBy"] = json!([joined_column("name", "customers")]);

    let sql = render_statement(&request);

    let grouping_key = "`t2`.`name` COLLATE utf8mb4_0900_as_cs";
    assert_eq!(
        sql,
        format!(
            "SELECT {grouping_key}, COUNT(*) FROM (`shop`.`orders` AS `t1` INNER JOIN \
             `shop`.`customers` AS `t2` ON (`t1`.`customer_id` = `t2`.`id`)) WHERE \
             (`t1`.`note` COLLATE utf8mb4_0900_as_cs = _utf8mb4 X'78' COLLATE utf8mb4_0900_as_cs) \
             GROUP BY {grouping_key}"
        ),
        "the FROM, WHERE, and GROUP BY clauses must all appear, in that order: {sql}"
    );
}

/// A reference naming a table the `from` tree does not hold has no alias to
/// carry, and MySQL would resolve the unqualified name against whichever
/// joined table happens to declare the column. The error names both halves
/// of the reference it could not resolve.
#[test]
fn a_column_naming_no_table_in_the_from_tree_fails_naming_the_column_and_the_reference() {
    let error = render_error(&join_request(
        orders_to_customers("inner"),
        json!([joined_column("note", "nowhere")]),
    ));

    assert!(
        error.contains("`note`") && error.contains("nowhere"),
        "the error must name the column and the table reference: {error}"
    );
}

/// An operator's own alias resolves the reference and is never rendered.
/// Rendering it would put a user-chosen identifier in the alias namespace
/// this dialect has to keep unique: measured, MySQL rejects a repeated alias
/// with error 1066, which `FROM "orders" "t2" JOIN "customers"` would produce
/// against a positionally derived `t2`.
#[test]
fn a_request_alias_resolves_the_reference_without_reaching_the_statement() {
    let mut from = orders_to_customers("inner");
    from["left"]["alias"] = json!("t2");
    from["condition"]["left"] = json!({
        "type": "column", "name": "customer_id", "tableName": "orders", "tableAlias": "t2",
    });

    let sql = render_statement(&join_request(
        from,
        json!([
            {"type": "column", "name": "id", "tableName": "orders", "tableAlias": "t2"},
            joined_column("id", "customers"),
        ]),
    ));

    assert_eq!(
        sql,
        "SELECT `t1`.`id`, `t2`.`id` FROM (`shop`.`orders` AS `t1` INNER JOIN \
         `shop`.`customers` AS `t2` ON (`t1`.`customer_id` = `t2`.`id`))"
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
        "SELECT `t1`.`id`, `t2`.`id` FROM (`shop`.`orders` AS `t1` INNER JOIN \
         `shop`.`orders` AS `t2` ON (`t1`.`id` = `t2`.`customer_id`))"
    );
}

/// A single-table query qualifies nothing and aliases nothing: adding the
/// join tier changes no statement this dialect already generated.
#[test]
fn a_single_table_query_renders_no_alias_even_when_the_request_assigned_one() {
    let mut request = request(orders_columns(), Some(json!([column_operand("id")])));
    request["pushdownRequest"]["from"]["alias"] = json!("o");

    assert_eq!(
        render_statement(&request),
        "SELECT `id` FROM `shop`.`orders`"
    );
}

/// A reference carrying neither half has nothing to resolve against. It
/// cannot be rendered bare either: MySQL would resolve the name against
/// whichever joined table declares it, which is error 1052 when two of them
/// do and the wrong table's column when one does.
#[test]
fn a_join_column_carrying_no_table_reference_at_all_fails_naming_it() {
    let error = render_error(&join_request(
        orders_to_customers("inner"),
        json!([column_operand("note")]),
    ));

    assert!(
        error.contains("`note`") && error.contains("tableName") && error.contains("tableAlias"),
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
        error.contains("`id`") && error.contains("orders") && error.contains("more than one"),
        "the error must name the column and the reference two tables answer to: {error}"
    );
}
