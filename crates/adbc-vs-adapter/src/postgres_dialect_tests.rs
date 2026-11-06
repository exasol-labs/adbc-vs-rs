use serde_json::{json, Value};

use super::*;
use crate::sql_dialect::SqlDialect;
use adbc_vs_scan::driver::Driver;

use crate::adapter_notes::SourceQualifier;
use crate::dialect_test_fixtures as fixtures;
use crate::dialect_test_fixtures::{
    aggregate, boolean_type, column, count_type, double_type, equal, filtered_request,
    grouped_request, id_type, join_request, joined_column, less, note_type, numeric_literal,
    orders_columns, orders_to_customers, request, string_literal, sum_type, typed_request,
};
use crate::exa_type::ExasolType;
use crate::pushdown_query::{
    self, Expression, FromClause, Literal, Query, SelectItem, SourceTable,
};

/// The namespace a PostgreSQL virtual schema records and every rendered table
/// is qualified by: the `public` schema, never the database the connection
/// address names.
const NAMESPACE: &str = "public";

/// The namespace a PostgreSQL virtual schema records and every rendered table
/// is qualified by.
fn namespace() -> SourceQualifier {
    SourceQualifier::single(NAMESPACE)
}

/// The PostgreSQL statement `request` renders to, parsed first so every
/// assertion covers the request shape Exasol sends rather than a hand-built
/// model.
fn render_statement(request: &Value) -> String {
    fixtures::render_statement(&PostgresDialect, &namespace(), request)
}

fn render(columns: Value, select_list: Option<Value>) -> String {
    render_statement(&request(columns, select_list))
}

/// The error rendering `request` fails with, for a request the parser accepts
/// and this dialect has no form for.
fn render_error(request: &Value) -> String {
    fixtures::render_error(&PostgresDialect, &namespace(), request)
}

/// The `WHERE` clause `filter` renders to over `columns`, parsed from a
/// request first so each assertion covers the shape Exasol sends.
fn where_clause_over(columns: Value, filter: Value) -> String {
    fixtures::where_clause_over(&PostgresDialect, &namespace(), columns, filter)
}

/// The `WHERE` clause `filter` renders to over the `orders` columns.
fn where_clause(filter: Value) -> String {
    where_clause_over(orders_columns(), filter)
}

/// The error rendering `filter` fails with, for a filter the parser accepts
/// and PostgreSQL has no form for.
fn filter_render_error(filter: Value) -> String {
    render_error(&filtered_request(orders_columns(), filter))
}

/// One character column, one numeric, one temporal: the declared types a
/// collation suffix could be keyed on, so a test over them shows none is.
fn typed_columns() -> Value {
    json!([
        {"name": "note", "dataType": {"type": "VARCHAR", "size": 100, "characterSet": "UTF8"}},
        {"name": "id", "dataType": {"type": "DECIMAL", "precision": 19, "scale": 0}},
        {"name": "shipped", "dataType": {"type": "DATE"}},
    ])
}

/// `vs-adapter/pushdown-dialect`: Exasol normalizes `id > 5` into
/// `predicate_less` with the operands swapped, so the renderer reproduces the
/// order it was given rather than assuming the column comes first.
#[test]
fn comparison_operands_keep_request_order() {
    let clause = where_clause(less(numeric_literal("5"), column("id")));

    assert_eq!(clause, r#"(5 < "id")"#);
}

/// `vs-adapter/pushdown-filter` and `vs-adapter/postgresql-pushdown`: every
/// predicate this dialect renders, in the standard form PostgreSQL spells it,
/// over a numeric column. `predicate_notequal` is `<>`, a null test is `IS NULL`
/// rather than a comparison against a null literal, and the `BETWEEN` bounds
/// and `IN` entries keep the request's own order.
#[test]
fn standard_predicate_forms_render() {
    for (filter, expected) in [
        (equal(column("id"), numeric_literal("2")), r#"("id" = 2)"#),
        (
            json!({"type": "predicate_notequal", "left": column("id"), "right": numeric_literal("2")}),
            r#"("id" <> 2)"#,
        ),
        (less(column("id"), numeric_literal("2")), r#"("id" < 2)"#),
        (
            json!({"type": "predicate_lessequal", "left": column("id"), "right": numeric_literal("2")}),
            r#"("id" <= 2)"#,
        ),
        (
            json!({"type": "predicate_is_null", "expression": column("id")}),
            r#"("id" IS NULL)"#,
        ),
        (
            json!({"type": "predicate_is_not_null", "expression": column("id")}),
            r#"("id" IS NOT NULL)"#,
        ),
        (
            json!({
                "type": "predicate_like",
                "expression": column("note"),
                "pattern": string_literal("Wid%"),
            }),
            r#"("note" LIKE E'Wid%')"#,
        ),
        (
            json!({
                "type": "predicate_between",
                "expression": column("id"),
                "left": numeric_literal("9"),
                "right": numeric_literal("1"),
            }),
            r#"("id" BETWEEN 9 AND 1)"#,
        ),
        (
            json!({
                "type": "predicate_in_constlist",
                "expression": column("id"),
                "arguments": [numeric_literal("3"), numeric_literal("1")],
            }),
            r#"("id" IN (3, 1))"#,
        ),
    ] {
        let clause = where_clause(filter.clone());

        assert_eq!(clause, expected, "for {filter}");
    }
}

/// `vs-adapter/postgresql-pushdown`: a string literal renders as the escape
/// string constant `E'<body>'` with a backslash doubled and a single quote
/// doubled, unconditionally rather than per value, so neither
/// `standard_conforming_strings` nor `backslash_quote` changes what the source
/// reads. Gate 2.3(c) measured all four settings combinations returning the
/// stored bytes for the `a\b'c€d` case below.
#[test]
fn string_literal_is_escape_string_constant() {
    for (value, expected) in [
        ("widget", r"E'widget'"),
        ("", r"E''"),
        ("a'b", r"E'a''b'"),
        (r"a\b", r"E'a\\b'"),
        ("Gîzmo", r"E'Gîzmo'"),
        (r"a\b'c€d", r"E'a\\b''c€d'"),
    ] {
        let clause = where_clause(equal(column("note"), string_literal(value)));

        assert_eq!(
            clause,
            format!(r#"("note" = {expected})"#),
            "the literal `{value}` must render as `{expected}`"
        );
    }
}

/// `vs-adapter/postgresql-pushdown`: a date or timestamp renders as a typed
/// literal, so PostgreSQL coerces no string by its own rules.
#[test]
fn temporal_literals_render_typed() {
    fixtures::assert_temporal_literals_render_typed(
        &where_clause_over,
        r#"(("shipped" = DATE '2015-12-01') AND ("logged" = TIMESTAMP '2015-12-01 12:01:01.123'))"#,
    );
}

#[test]
fn a_null_literal_renders_as_postgresql_null() {
    let clause = where_clause(equal(column("id"), json!({"type": "literal_null"})));

    assert_eq!(clause, r#"("id" = NULL)"#);
}

/// Every predicate renders as its own parenthesized group, so the request's
/// own nesting reaches PostgreSQL without either side reasoning about
/// operator precedence.
#[test]
fn nested_predicates_are_parenthesized() {
    let clause = where_clause(json!({
        "type": "predicate_not",
        "expression": {
            "type": "predicate_or",
            "expressions": [
                equal(column("id"), numeric_literal("1")),
                {
                    "type": "predicate_lessequal",
                    "left": column("total"),
                    "right": {"type": "literal_double", "value": "2.5"},
                },
            ],
        },
    }));

    assert_eq!(clause, r#"(NOT (("id" = 1) OR ("total" <= 2.5)))"#);
}

/// `vs-adapter/pushdown-filter`: a `LIKE` renders an explicit `ESCAPE` clause
/// exactly when the request carries an `escapeChar`, and its pattern is an
/// escape-string constant like every other string literal.
///
/// `vs-adapter/postgresql-collation`: the operand carries no collation
/// suffix, so the `LIKE` runs under the column's own collation.
#[test]
fn like_renders_its_escape_clause() {
    let with_escape = where_clause(json!({
        "type": "predicate_like",
        "expression": column("note"),
        "pattern": string_literal("Wid%"),
        "escapeChar": string_literal("\\"),
    }));

    assert_eq!(with_escape, r#"("note" LIKE E'Wid%' ESCAPE E'\\')"#);

    let without_escape = where_clause(json!({
        "type": "predicate_like",
        "expression": column("note"),
        "pattern": string_literal("Wid%"),
    }));

    assert_eq!(without_escape, r#"("note" LIKE E'Wid%')"#);
    assert!(
        !without_escape.contains("COLLATE"),
        "a `LIKE` operand carries no collation on PostgreSQL: {without_escape}"
    );
}

/// `vs-adapter/pushdown-filter`: the `BETWEEN` bounds and the `IN` entries
/// keep the order the request gave them, never a sorted or normalized one.
///
/// `vs-adapter/postgresql-collation`: the `BETWEEN` operand, its bounds, and
/// the `IN` compared operand all render bare.
#[test]
fn between_and_in_keep_operand_order() {
    let between = where_clause(json!({
        "type": "predicate_between",
        "expression": column("note"),
        "left": string_literal("z"),
        "right": string_literal("a"),
    }));

    assert_eq!(
        between, r#"("note" BETWEEN E'z' AND E'a')"#,
        "the bounds keep the request's order and carry no suffix"
    );

    let in_list = where_clause(json!({
        "type": "predicate_in_constlist",
        "expression": column("note"),
        "arguments": [string_literal("c"), string_literal("a"), string_literal("b")],
    }));

    assert_eq!(in_list, r#"("note" IN (E'c', E'a', E'b'))"#);
    assert!(
        !in_list.contains("COLLATE"),
        "an `IN` compared operand carries no collation on PostgreSQL: {in_list}"
    );
}

/// `vs-adapter/postgresql-collation`: no operand of any predicate, grouping
/// key, `DISTINCT` operand, `MIN` or `MAX` operand, or join key carries a
/// collation suffix, whatever its declared type. PostgreSQL evaluates each
/// under the collation the source applies to the column, so a character
/// operand and a numeric or temporal one render the same way: bare.
#[test]
fn no_operand_carries_a_collation_suffix_in_any_position() {
    let ordering = |left: &str, right: &str| json!({"type": "predicate_less", "left": column(left), "right": column(right)});
    let predicates = [
        ordering("note", "note"),
        ordering("id", "id"),
        ordering("shipped", "shipped"),
        json!({"type": "predicate_lessequal", "left": column("note"), "right": string_literal("B")}),
        equal(column("note"), column("note")),
        json!({"type": "predicate_notequal", "left": column("note"), "right": column("note")}),
        json!({"type": "predicate_is_null", "expression": column("note")}),
        json!({"type": "predicate_is_not_null", "expression": column("note")}),
        json!({
            "type": "predicate_like",
            "expression": column("note"),
            "pattern": string_literal("a%"),
        }),
        json!({
            "type": "predicate_in_constlist",
            "expression": column("note"),
            "arguments": [string_literal("a")],
        }),
        json!({
            "type": "predicate_between",
            "expression": column("note"),
            "left": string_literal("a"),
            "right": string_literal("z"),
        }),
        json!({
            "type": "predicate_between",
            "expression": column("id"),
            "left": numeric_literal("1"),
            "right": numeric_literal("9"),
        }),
        json!({
            "type": "predicate_between",
            "expression": column("shipped"),
            "left": json!({"type": "literal_date", "value": "2015-01-01"}),
            "right": json!({"type": "literal_date", "value": "2015-12-31"}),
        }),
    ];
    let mut statements = Vec::new();

    for predicate in &predicates {
        statements.push(where_clause_over(typed_columns(), predicate.clone()));

        let mut select_list = request(typed_columns(), Some(json!([predicate.clone()])));
        select_list["pushdownRequest"]["selectListDataTypes"] = json!([boolean_type()]);
        statements.push(render_statement(&select_list));
    }

    let mut distinct_count = aggregate("COUNT", Some(column("note")));
    distinct_count["distinct"] = json!(true);
    let mut distinct_min = aggregate("MIN", Some(column("note")));
    distinct_min["distinct"] = json!(true);
    let mut grouped = grouped_request(
        json!([
            column("note"),
            column("id"),
            column("shipped"),
            aggregate("MIN", Some(column("note"))),
            aggregate("MAX", Some(column("note"))),
            distinct_min,
            distinct_count,
            aggregate("MIN", Some(column("id"))),
            aggregate("MAX", Some(column("shipped"))),
            less(string_literal("x"), aggregate("MIN", Some(column("note")))),
        ]),
        json!([
            note_type(),
            id_type(),
            {"type": "DATE"},
            note_type(),
            note_type(),
            note_type(),
            count_type(),
            id_type(),
            {"type": "DATE"},
            boolean_type(),
        ]),
        json!([column("note"), column("id"), column("shipped")]),
    );
    grouped["involvedTables"][0]["columns"] = typed_columns();
    statements.push(render_statement(&grouped));

    let mut from = orders_to_customers("full_outer");
    from["condition"] = json!({
        "type": "predicate_equal",
        "left": joined_column("note", "orders"),
        "right": joined_column("name", "customers"),
    });
    let mut joined = join_request(from, json!([joined_column("name", "customers")]));
    joined["pushdownRequest"]["filter"] =
        less(string_literal("m"), joined_column("note", "orders"));
    statements.push(render_statement(&joined));

    for statement in &statements {
        assert!(
            !statement.contains("COLLATE"),
            "no position carries a collation suffix on PostgreSQL: {statement}"
        );
    }
    assert_eq!(
        where_clause_over(typed_columns(), ordering("note", "note")),
        r#"("note" < "note")"#
    );
    assert_eq!(
        render_statement(&grouped),
        r#"SELECT "note", "id", "shipped", MIN("note"), MAX("note"), MIN(DISTINCT "note"), COUNT(DISTINCT "note"), MIN("id"), MAX("shipped"), (E'x' < MIN("note")) FROM "public"."orders" GROUP BY "note", "id", "shipped""#
    );
}

/// `vs-adapter/pushdown-filter`: a predicate delegated into the select list
/// renders as a projected boolean rather than reaching a second renderer.
///
/// `vs-adapter/postgresql-collation`: its character operand carries no
/// suffix.
#[test]
fn select_list_predicate_renders_as_projected_boolean() {
    let mut request = request(
        orders_columns(),
        Some(json!([less(string_literal("x"), column("note"))])),
    );
    request["pushdownRequest"]["selectListDataTypes"] = json!([{"type": "BOOLEAN"}]);

    let sql = render_statement(&request);

    assert_eq!(sql, r#"SELECT (E'x' < "note") FROM "public"."orders""#);

    let mut equality = request.clone();
    equality["pushdownRequest"]["selectList"] = json!([equal(column("note"), string_literal("x"))]);

    assert_eq!(
        render_statement(&equality),
        r#"SELECT ("note" = E'x') FROM "public"."orders""#,
        "a select-list equality renders bare as well"
    );

    let mut single_group = request;
    single_group["pushdownRequest"]["selectList"] = json!([json!({
        "type": "predicate_less",
        "left": {"type": "literal_exactnumeric", "value": "0"},
        "right": {"type": "function_aggregate", "name": "COUNT"},
    })]);
    single_group["pushdownRequest"]["aggregationType"] = json!("single_group");

    assert_eq!(
        render_statement(&single_group),
        r#"SELECT (0 < COUNT(*)) FROM "public"."orders""#,
        "a query grouping by nothing carries no grouping key for the predicate to disagree with"
    );
}

/// `vs-adapter/pushdown-filter` requires that no literal can change the shape
/// of the generated statement. A string literal earns that by the escape
/// string constant; a numeric or temporal literal, which PostgreSQL accepts
/// only spelled out, earns it by carrying nothing that could.
///
/// PostgreSQL's `--` comment needs no following space, unlike MySQL's, so the
/// character-set rule the MySQL dialect uses would admit `1--2` and comment
/// out the rest of the statement. The numeric check is a spelling check
/// instead.
#[test]
fn a_literal_that_could_change_the_statement_shape_fails_to_render() {
    for (literal, fragment) in [
        (numeric_literal("1 OR 1=1"), "1 OR 1=1"),
        (numeric_literal("1--2"), "1--2"),
        (numeric_literal("1.2.3"), "1.2.3"),
        (numeric_literal(""), "numeric literal"),
        (
            json!({"type": "literal_double", "value": "1)/**/OR(1"}),
            "1)/**/OR(1",
        ),
        (json!({"type": "literal_double", "value": "2e"}), "2e"),
        (
            json!({"type": "literal_date", "value": "2015-12-01' OR '1"}),
            "2015-12-01' OR '1",
        ),
        (
            json!({"type": "literal_timestamp", "value": ""}),
            "TIMESTAMP",
        ),
    ] {
        let error = filter_render_error(equal(column("id"), literal.clone()));

        assert!(
            error.contains(fragment),
            "the error must name the literal it refused: {error}, for {literal}"
        );
    }
}

/// PostgreSQL writes no empty `AND`, `OR`, or `IN` list, and a renderer that
/// dropped the node instead would hand Exasol rows it never asked for.
#[test]
fn a_predicate_with_no_operand_fails_rather_than_rendering_an_empty_group() {
    fixtures::assert_empty_predicate_groups_fail(&filter_render_error, "PostgreSQL");
}

/// The transition case between empty, refused above, and multi-operand: one
/// operand renders bare inside its own parentheses, with no operator to join
/// it to.
#[test]
fn a_single_operand_junction_or_in_list_renders_bare() {
    let and = where_clause(json!({
        "type": "predicate_and",
        "expressions": [equal(column("id"), numeric_literal("2"))],
    }));
    assert_eq!(and, r#"(("id" = 2))"#);

    let or = where_clause(json!({
        "type": "predicate_or",
        "expressions": [equal(column("id"), numeric_literal("2"))],
    }));
    assert_eq!(or, r#"(("id" = 2))"#);

    let in_list = where_clause(json!({
        "type": "predicate_in_constlist",
        "expression": column("id"),
        "arguments": [numeric_literal("2")],
    }));
    assert_eq!(in_list, r#"("id" IN (2))"#);
}

#[test]
fn named_columns_are_projected_in_the_requests_order() {
    let sql = render(
        orders_columns(),
        Some(json!([column("note"), column("id")])),
    );

    assert_eq!(sql, r#"SELECT "note", "id" FROM "public"."orders""#);
}

/// `vs-adapter/pushdown-projection`: an absent select list projects every
/// mapped column in the virtual table's declared order rather than `*`, so a
/// source column added after creation cannot break the declared `EMITS`
/// arity.
#[test]
fn absent_select_list_projects_every_column() {
    let sql = render(orders_columns(), None);

    assert_eq!(
        sql, r#"SELECT "id", "total", "note" FROM "public"."orders""#,
        "an absent select list projects the virtual table's columns, never `*`"
    );
}

/// `vs-adapter/pushdown-projection`: Exasol sends an empty select list when
/// only the row count matters. A statement projecting no column is invalid
/// PostgreSQL and would lose the count, so one constant stands in for it and
/// the statement still reads the source table.
#[test]
fn an_empty_select_list_projects_a_single_constant() {
    let sql = render(orders_columns(), Some(json!([])));

    assert_eq!(sql, r#"SELECT 1 FROM "public"."orders""#);
}

/// `vs-adapter/postgresql-pushdown`: an unquoted PostgreSQL identifier folds
/// to lower case, so a mixed-case source name reaches an object that does not
/// exist unless every identifier is quoted — including one that is already
/// lower case, since a rule that quoted only when it looked necessary would
/// be a second decision able to disagree with the first.
#[test]
fn every_identifier_is_quoted_whatever_its_case() {
    let columns = json!([
        {"name": "OrderId", "dataType": {"type": "DECIMAL", "precision": 19, "scale": 0}},
        {"name": "note", "dataType": {"type": "VARCHAR", "size": 100, "characterSet": "UTF8"}},
    ]);

    let sql = render(columns, None);

    assert_eq!(sql, r#"SELECT "OrderId", "note" FROM "public"."orders""#);
}

/// `vs-adapter/postgresql-pushdown`: a double quote inside an identifier is
/// escaped by doubling it, and the identifier stays inside one pair of quotes,
/// so no source name can end the quoted identifier early and change the shape
/// of the statement.
#[test]
fn double_quote_in_identifier_is_doubled() {
    let columns = json!([
        {"name": r#"od"d"#, "dataType": {"type": "DECIMAL", "precision": 19, "scale": 0}},
    ]);

    let sql = render(columns, None);

    assert_eq!(sql, r#"SELECT "od""d" FROM "public"."orders""#);
    assert!(
        !sql.contains(r#""od"d""#),
        "the raw name must not reach the statement: {sql}"
    );
}

/// `vs-adapter/postgresql-type-mapping`: the table is qualified by the
/// namespace `adapterNotes` records, which is the `public` schema and not the
/// database the connection address names — a PostgreSQL database is not a SQL
/// qualifier.
#[test]
fn a_table_is_qualified_by_the_recorded_namespace() {
    let sql = render(orders_columns(), Some(json!([column("id")])));

    assert!(
        sql.ends_with(r#"FROM "public"."orders""#),
        "the table must be qualified by the recorded namespace: {sql}"
    );
}

/// `vs-adapter/pushdown-join`: the two `id` columns share a name, so the
/// projection is readable only because each carries the alias of the table it
/// belongs to. Both tables carry one even though the request assigned neither:
/// the alias comes from the table's position in the `from` tree, which is what
/// keeps the set unique whatever the request supplies.
#[test]
fn inner_join_renders_one_statement() {
    let sql = render_statement(&join_request(
        orders_to_customers("inner"),
        json!([
            joined_column("id", "orders"),
            joined_column("id", "customers"),
        ]),
    ));

    assert_eq!(
        sql,
        r#"SELECT "t1"."id", "t2"."id" FROM ("public"."orders" AS "t1" INNER JOIN "public"."customers" AS "t2" ON ("t1"."customer_id" = "t2"."id"))"#
    );
}

/// `vs-adapter/pushdown-join` and `vs-adapter/postgresql-pushdown`: every join
/// type this dialect renders spells its own PostgreSQL keyword over the
/// request's own left and right. `full_outer` is the one this list holds and
/// the MySQL dialect's does not; gate 2.3(a) measured that join executing on
/// PostgreSQL 18.6 and returning the unmatched row of each side.
#[test]
fn each_join_type_renders() {
    for (join_type, keyword) in [
        ("inner", "INNER JOIN"),
        ("left_outer", "LEFT OUTER JOIN"),
        ("right_outer", "RIGHT OUTER JOIN"),
        ("full_outer", "FULL OUTER JOIN"),
    ] {
        let sql = render_statement(&fixtures::join_type_request(join_type));

        assert_eq!(
            sql,
            format!(
                r#"SELECT "t1"."id" FROM ("public"."orders" AS "t1" {keyword} "public"."customers" AS "t2" ON ("t1"."customer_id" = "t2"."id"))"#
            ),
            "`{join_type}` must render `{keyword}` over the request's own left and right"
        );
    }
}

/// A `join_type` outside the four reaches the renderer because the model keeps
/// the value verbatim. Substituting a join type this dialect does render would
/// return rows Exasol never asked for, so the value is named back instead.
#[test]
fn unsupported_join_type_named_per_dialect() {
    fixtures::assert_join_types_named(&render_error, &["cross", "left_semi"]);

    let mysql_error = crate::mysql_dialect::MySqlDialect
        .render(
            &pushdown_query::parse(
                &join_request(
                    orders_to_customers("full_outer"),
                    json!([joined_column("id", "orders")]),
                ),
                &SourceQualifier::single(NAMESPACE),
            )
            .expect("the request must parse"),
        )
        .expect_err("MySQL implements no `FULL OUTER JOIN`");

    assert!(
        mysql_error.to_string().contains("full_outer"),
        "the join type this dialect renders and MySQL's does not must still be named back by \
         MySQL's: {mysql_error}"
    );
}

/// `vs-adapter/pushdown-join`: a nested join renders inside its own
/// parentheses, so the tree the request sent is the tree PostgreSQL reads — an
/// outer join whose operand order or nesting shifted would return different
/// rows.
#[test]
fn nested_join_renders_nested() {
    let request = fixtures::nested_join_request("full_outer");

    let sql = render_statement(&request);

    assert_eq!(
        sql,
        r#"SELECT "t3"."code" FROM (("public"."orders" AS "t1" INNER JOIN "public"."customers" AS "t2" ON ("t1"."customer_id" = "t2"."id")) FULL OUTER JOIN "public"."regions" AS "t3" ON ("t2"."id" = "t3"."customer_id"))"#
    );
}

/// `vs-adapter/pushdown-join`: every position a column reaches the renderer
/// from carries the alias of the table it belongs to — the select list, the
/// `WHERE` clause, and the join's own condition.
///
/// `vs-adapter/postgresql-collation`: a character equality operand renders
/// bare beside its alias qualification.
#[test]
fn joined_tables_are_aliased_and_qualified() {
    let request = fixtures::filtered_join_request(json!([joined_column("name", "customers")]));

    let sql = render_statement(&request);

    assert_eq!(
        sql,
        r#"SELECT "t2"."name" FROM ("public"."orders" AS "t1" INNER JOIN "public"."customers" AS "t2" ON ("t1"."customer_id" = "t2"."id")) WHERE ("t1"."note" = E'x')"#
    );
}

/// `vs-adapter/pushdown-join`: a join carrying both a `filter` and a `groupBy`
/// in the same request renders every clause together, in PostgreSQL's required
/// order — `FROM`, `WHERE`, `GROUP BY`. Each tier renders its own fragment;
/// only this test proves the three compose over a joined `FROM`.
#[test]
fn join_composes_with_filter_and_grouping() {
    let request =
        fixtures::filtered_grouped_join_request(joined_column("name", "customers"), note_type());

    let sql = render_statement(&request);

    assert_eq!(
        sql,
        r#"SELECT "t2"."name", COUNT(*) FROM ("public"."orders" AS "t1" INNER JOIN "public"."customers" AS "t2" ON ("t1"."customer_id" = "t2"."id")) WHERE ("t1"."note" = E'x') GROUP BY "t2"."name""#
    );
}

/// `vs-adapter/pushdown-join`: a reference naming a table the `from` tree does
/// not hold has no alias to carry, and a reference carrying neither spelling
/// has nothing to resolve against. Rendering either bare would leave
/// PostgreSQL to resolve the name against whichever joined table declares it.
#[test]
fn unresolvable_column_named_in_error() {
    let unknown_table = render_error(&join_request(
        orders_to_customers("inner"),
        json!([joined_column("note", "nowhere")]),
    ));

    assert!(
        unknown_table.contains("`note`") && unknown_table.contains("nowhere"),
        "the error must name the column and the table reference: {unknown_table}"
    );

    let no_reference = render_error(&join_request(
        orders_to_customers("inner"),
        json!([column("note")]),
    ));

    assert!(
        no_reference.contains("`note`")
            && no_reference.contains("tableName")
            && no_reference.contains("tableAlias"),
        "the error must name the column and both spellings it lacks: {no_reference}"
    );
}

/// One table joined to itself is told apart only by the alias the request
/// assigns each copy, and each copy resolves to its own positional alias, so
/// the condition compares the two copies rather than one with itself. With
/// neither copy aliased, both answer to the same reference and the reference
/// fails rather than silently picking the first.
#[test]
fn a_shared_table_name_resolves_by_alias_or_fails() {
    let self_join = fixtures::self_join_request();

    assert_eq!(
        render_statement(&self_join),
        r#"SELECT "t1"."id", "t2"."id" FROM ("public"."orders" AS "t1" INNER JOIN "public"."orders" AS "t2" ON ("t1"."id" = "t2"."customer_id"))"#
    );

    fixtures::assert_ambiguous_join_reference_fails(&render_error, "`id`");
}

/// An operator's own alias resolves the reference and is never rendered.
/// Rendering it would put a user-chosen identifier into the alias namespace
/// this dialect has to keep unique, where `FROM "orders" "t2" JOIN "customers"`
/// would produce `t2` for two different tables.
#[test]
fn a_request_alias_resolves_the_reference_without_reaching_the_statement() {
    let sql = render_statement(&fixtures::request_aliased_join_request());

    assert_eq!(
        sql,
        r#"SELECT "t1"."id", "t2"."id" FROM ("public"."orders" AS "t1" INNER JOIN "public"."customers" AS "t2" ON ("t1"."customer_id" = "t2"."id"))"#
    );
}

/// A single-table query qualifies nothing and aliases nothing: adding the join
/// tier changes no statement this dialect already generated.
#[test]
fn a_single_table_query_renders_no_alias_even_when_the_request_assigned_one() {
    let mut request = request(orders_columns(), Some(json!([column("id")])));
    request["pushdownRequest"]["from"]["alias"] = json!("o");

    assert_eq!(
        render_statement(&request),
        r#"SELECT "id" FROM "public"."orders""#
    );
}

/// A single-group aggregation groups by nothing, which the exact match this
/// test pins already proves: it carries no `GROUP BY`.
#[test]
fn single_group_renders_without_group_by() {
    let mut request = typed_request(
        json!([aggregate("COUNT", Some(column("total")))]),
        json!([count_type()]),
    );
    request["pushdownRequest"]["aggregationType"] = json!("single_group");

    let sql = render_statement(&request);

    assert_eq!(sql, r#"SELECT COUNT("total") FROM "public"."orders""#);
}

/// Exasol orders `groupBy` independently of `selectList`, so the two orders
/// are asserted against each other rather than assumed to agree.
#[test]
fn group_by_keeps_request_order() {
    let sql = render_statement(&grouped_request(
        json!([
            column("id"),
            column("note"),
            aggregate("COUNT", Some(column("total"))),
        ]),
        json!([id_type(), note_type(), count_type()]),
        json!([column("note"), column("id")]),
    ));

    assert_eq!(
        sql,
        r#"SELECT "id", "note", COUNT("total") FROM "public"."orders" GROUP BY "note", "id""#
    );
}

/// `COUNT(*)` counts rows and `COUNT(column)` counts that column's non-null
/// values, so rendering one as the other is a wrong value rather than a syntax
/// error.
#[test]
fn count_star_differs_from_count_column() {
    let sql = render_statement(&typed_request(
        json!([
            aggregate("COUNT", None),
            aggregate("COUNT", Some(column("note"))),
        ]),
        json!([count_type(), count_type()]),
    ));

    assert_eq!(
        sql,
        r#"SELECT COUNT(*), COUNT("note") FROM "public"."orders""#
    );
}

/// Measured on Exasol 2025.2.1, one request carried `sum`, `min`, and `max` in
/// lower case beside `AVG` and `COUNT` in upper case, so the case a request
/// arrives in must not reach the generated statement.
#[test]
fn mixed_case_function_names_render_identically() {
    fixtures::assert_mixed_case_renders_identically(
        &render_statement,
        "total",
        r#"SELECT COUNT("total"), MIN("note"), SUM("total"), (CAST(SUM("total") AS DOUBLE PRECISION) / COUNT("total")) FROM "public"."orders""#,
    );
}

/// Rendering the aggregate's argument alone would leave Exasol aggregating
/// nothing and returning the wrong value, so an unknown function fails.
#[test]
fn unrenderable_aggregate_named_in_error() {
    fixtures::assert_unknown_aggregate_named(&render_error);
}

/// `vs-adapter/pushdown-aggregation`, scenario "SUM renders as the source's
/// SUM over the operand": PostgreSQL's own `SUM` over the column, with
/// `DISTINCT` before the operand exactly when the request sets it. A `DECIMAL`
/// operand renders without a cast, because PostgreSQL's `sum` over an integer
/// or `numeric` operand is already exact.
#[test]
fn sum_renders_with_and_without_distinct() {
    let sql = render_statement(&fixtures::sum_request());

    assert_eq!(
        sql,
        r#"SELECT SUM("total"), SUM(DISTINCT "total") FROM "public"."orders""#
    );
}

/// `vs-adapter/postgresql-pushdown`, scenario "SUM over a real column
/// accumulates in double precision": PostgreSQL's `sum` over `real` returns
/// `real`, so an operand declared `DOUBLE` is cast before it is summed, with
/// and without `DISTINCT`, and the `AVG` count takes the operand uncast.
#[test]
fn sum_over_a_double_operand_casts_it_to_double_precision() {
    let mut entries = fixtures::ratio_sum_entries();
    entries.push((aggregate("AVG", Some(column("ratio"))), double_type()));
    let request = fixtures::ratio_request(entries);

    assert_eq!(
        render_statement(&request),
        r#"SELECT SUM(CAST("ratio" AS DOUBLE PRECISION)), SUM(DISTINCT CAST("ratio" AS DOUBLE PRECISION)), (CAST(SUM(CAST("ratio" AS DOUBLE PRECISION)) AS DOUBLE PRECISION) / COUNT("ratio")) FROM "public"."orders""#
    );
}

/// `vs-adapter/pushdown-aggregation`, scenario "AVG renders as the
/// double-precision quotient of SUM and COUNT": the quotient is parenthesized,
/// both halves carry `DISTINCT` for `AVG(DISTINCT)`, and PostgreSQL's own
/// `AVG`, which returns `numeric` over an exact-numeric operand, never renders.
#[test]
fn avg_renders_as_the_double_quotient_of_sum_and_count() {
    fixtures::assert_avg_renders_as_quotient(
        &render_statement,
        r#"SELECT (CAST(SUM("total") AS DOUBLE PRECISION) / COUNT("total")), (CAST(SUM(DISTINCT "total") AS DOUBLE PRECISION) / COUNT(DISTINCT "total")) FROM "public"."orders""#,
        "PostgreSQL",
    );
}

/// `vs-adapter/pushdown-aggregation`, scenario "A BOOLEAN or character SUM and
/// AVG operand renders converted as Exasol converts it": PostgreSQL defines no
/// `sum` over a boolean or a character type, so a `BOOLEAN` column or
/// predicate sums as an `INTEGER`, and a character column or literal reads
/// through `NULLIF` and a `DOUBLE PRECISION` cast, with the `AVG` count taking
/// the operand before the cast.
#[test]
fn sum_and_avg_convert_boolean_and_character_operands() {
    let mut entries = vec![
        (
            aggregate("SUM", Some(less(numeric_literal("2"), column("id")))),
            sum_type(),
        ),
        (aggregate("SUM", Some(column("flag"))), sum_type()),
        (aggregate("AVG", Some(column("flag"))), double_type()),
    ];
    entries.extend(fixtures::character_sum_avg_entries());
    let mut request = fixtures::typed_entries_request(entries);
    request["involvedTables"][0]["columns"] = json!([
        {"name": "id", "dataType": id_type()},
        {"name": "flag", "dataType": boolean_type(), "adapterNotes": r#"{"sourceType":"boolean"}"#},
        {"name": "note", "dataType": {"type": "VARCHAR", "size": 100, "characterSet": "UTF8"}},
    ]);

    assert_eq!(
        render_statement(&request),
        r#"SELECT SUM(CAST((2 < "id") AS INTEGER)), SUM(CAST("flag" AS INTEGER)), (CAST(SUM(CAST("flag" AS INTEGER)) AS DOUBLE PRECISION) / COUNT("flag")), SUM(CAST(NULLIF("note", '') AS DOUBLE PRECISION)), SUM(DISTINCT CAST(NULLIF("note", '') AS DOUBLE PRECISION)), (CAST(SUM(CAST(NULLIF("note", '') AS DOUBLE PRECISION)) AS DOUBLE PRECISION) / COUNT(NULLIF("note", ''))), (CAST(SUM(DISTINCT CAST(NULLIF("note", '') AS DOUBLE PRECISION)) AS DOUBLE PRECISION) / COUNT(DISTINCT NULLIF("note", ''))), SUM(CAST(NULLIF(E'5', '') AS DOUBLE PRECISION)) FROM "public"."orders""#
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
            r#"SELECT "note", COUNT(*) FROM "public"."orders" GROUP BY "note" HAVING (12 < COUNT(*))"#,
            r#"SELECT COUNT(*) FROM "public"."orders" HAVING (12 < COUNT(*))"#,
        ],
    );
}

/// `COUNT(*)` is the only aggregate PostgreSQL writes over no argument, and it
/// counts rows rather than values, so `COUNT(DISTINCT *)` is not a form either.
#[test]
fn an_aggregate_carrying_no_argument_fails_unless_it_is_count_star() {
    fixtures::assert_argumentless_aggregates_fail(&render_error, "PostgreSQL");
}

/// PostgreSQL writes no empty `GROUP BY` clause, and a renderer that dropped
/// the clause instead would aggregate the whole table into one row where Exasol
/// asked for one row per group.
#[test]
fn an_aggregation_grouping_by_no_column_fails_rather_than_rendering_an_empty_clause() {
    fixtures::assert_empty_group_by_fails(&render_error, "PostgreSQL");
}

/// `vs-adapter/postgresql-collation`: a select-list predicate reading the
/// query's own character `GROUP BY` key outside any aggregate is rendered,
/// because the key renders bare and PostgreSQL's full-group-by rule reads
/// column references rather than matching whole expressions.
///
/// Five predicate kinds arrive in that position; each must render, with the
/// grouping key bare in both clauses.
#[test]
fn a_select_list_predicate_reading_the_grouped_character_column_renders() {
    let predicates = [
        less(string_literal("x"), column("note")),
        json!({"type": "predicate_is_null", "expression": column("note")}),
        json!({
            "type": "predicate_like",
            "expression": column("note"),
            "pattern": string_literal("a%"),
        }),
        json!({
            "type": "predicate_in_constlist",
            "expression": column("note"),
            "arguments": [string_literal("a"), string_literal("b")],
        }),
        json!({
            "type": "predicate_between",
            "expression": column("note"),
            "left": string_literal("a"),
            "right": string_literal("z"),
        }),
    ];

    for predicate in predicates {
        let sql = render_statement(&grouped_request(
            json!([column("note"), predicate.clone()]),
            json!([note_type(), boolean_type()]),
            json!([column("note")]),
        ));

        assert!(
            sql.starts_with(r#"SELECT "note", ("#) && sql.ends_with(r#"GROUP BY "note""#),
            "the grouping key must render bare in both clauses: {sql}, for {predicate}"
        );
    }

    assert_eq!(
        render_statement(&grouped_request(
            json!([column("note"), less(string_literal("x"), column("note"))]),
            json!([note_type(), boolean_type()]),
            json!([column("note")]),
        )),
        r#"SELECT "note", (E'x' < "note") FROM "public"."orders" GROUP BY "note""#
    );
}

/// `vs-adapter/pushdown-aggregation`: an aggregate reaches the select list
/// inside a predicate as `COUNT(*) > 0`, and the whole entry is delegated as
/// one `BOOLEAN` column, with no collation suffix on the aggregate, its
/// argument, or the literal it is compared against.
#[test]
fn select_list_predicate_over_an_aggregate_renders() {
    let sql = render_statement(&grouped_request(
        json!([
            column("note"),
            less(numeric_literal("0"), aggregate("COUNT", None)),
            less(string_literal("x"), aggregate("MIN", Some(column("note"))),),
        ]),
        json!([note_type(), boolean_type(), boolean_type()]),
        json!([column("note")]),
    ));

    assert_eq!(
        sql,
        r#"SELECT "note", (0 < COUNT(*)), (E'x' < MIN("note")) FROM "public"."orders" GROUP BY "note""#
    );
}

/// `vs-adapter/result-ordering`: `orderBy` and `limit` stay in Exasol because
/// their capabilities are withheld. An element arriving anyway means the
/// advertised set and the rejection list have drifted apart, and reading past
/// it would hand Exasol a statement it believes carries the clause.
#[test]
fn order_by_and_limit_elements_fail_named() {
    for (element, value) in [
        ("orderBy", json!([column("id")])),
        ("limit", json!({"numElements": 10})),
    ] {
        let mut request = request(orders_columns(), Some(json!([column("id")])));
        request["pushdownRequest"][element] = value;

        let error = pushdown_query::parse(&request, &SourceQualifier::single(NAMESPACE))
            .expect_err("a request carrying an undelegated element must fail");

        assert!(
            error.contains(element),
            "the error must name the element it refused: {error}"
        );
    }
}

/// A query projecting no column is not valid PostgreSQL, and a renderer that
/// emitted an empty select list would fail at the source rather than here.
///
/// Built as a model rather than parsed from a request: the parser resolves an
/// empty `selectList` into the row-marker constant, so this shape reaches the
/// dialect only from a future model change.
#[test]
fn a_query_projecting_nothing_fails_rather_than_rendering_an_empty_projection() {
    let error = fixtures::empty_projection_error(&PostgresDialect, namespace());

    assert!(
        error.contains("select list"),
        "the error must name the empty part of the query: {error}"
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
        select_list: vec![SelectItem {
            expression: Expression::Literal(Literal::String(r"a\b'c".to_string())),
            declared_type: ExasolType::Varchar { size: 100 },
        }],
        filter: None,
        aggregation: None,
        having: None,
        from: FromClause::Table(SourceTable {
            namespace: SourceQualifier::single(NAMESPACE),
            name: "orders".to_string(),
            alias: None,
        }),
    };

    let sql = PostgresDialect
        .render(&query)
        .expect("a literal select-list entry must render");

    assert_eq!(sql, r#"SELECT E'a\\b''c' FROM "public"."orders""#);
}

/// `vs-adapter/pushdown-dialect`: one request renders byte-for-byte
/// identically on every call, so `EXPLAIN VIRTUAL` and the statement the scan
/// runs are the same text. The alias assignment is the only state a render
/// walks with, so the request asserted here is a nested join carrying a filter
/// and a grouping — the shape that walks it deepest.
#[test]
fn a_request_renders_byte_identically_on_every_call() {
    let mut request = join_request(
        orders_to_customers("full_outer"),
        json!([joined_column("name", "customers"), aggregate("COUNT", None)]),
    );
    request["pushdownRequest"]["filter"] =
        less(string_literal("m"), joined_column("note", "orders"));
    request["pushdownRequest"]["selectListDataTypes"] = json!([note_type(), count_type()]);
    request["pushdownRequest"]["aggregationType"] = json!("group_by");
    request["pushdownRequest"]["groupBy"] = json!([joined_column("name", "customers")]);

    let query = pushdown_query::parse(&request, &SourceQualifier::single(NAMESPACE))
        .expect("the request must parse");
    let first = PostgresDialect.render(&query).expect("render must succeed");
    let second = PostgresDialect.render(&query).expect("render must succeed");

    assert_eq!(
        first, second,
        "the same request must render byte-for-byte identically on every call"
    );
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
/// reach a fallback that drops the fragment, and each fragment is the part of
/// the statement its own capability contributes, travelling in the same arm as
/// the fixture it is true of.
pub(crate) fn query_exercising(capability: &str) -> (&'static str, Query) {
    fixtures::query_exercising(Driver::Postgres, &namespace(), capability)
}

/// PostgreSQL source types whose read expression converts: a `uuid` reads as
/// text and a `bit` as whether any bit is set.
const CONVERTED_NOTES: fixtures::ConvertedNotes = fixtures::ConvertedNotes {
    uid: "uuid",
    flag: "bit",
};

/// `vs-adapter/pushdown-dialect`, scenario "A column reference renders through
/// its read expression in every position": a converted column reads through
/// one read expression in the select list, a `WHERE` comparison, a `GROUP BY`
/// key, a `MIN` and a `COUNT(DISTINCT)` operand, and an equi-join condition,
/// and a column without notes reads bare.
#[test]
fn column_reference_renders_identically_in_every_position() {
    fixtures::assert_column_positions(
        &render_statement,
        CONVERTED_NOTES,
        true,
        [
            r#"SELECT CAST("uid" AS TEXT) FROM "public"."orders" WHERE (CAST("uid" AS TEXT) = E'a')"#,
            r#"SELECT CAST("uid" AS TEXT), MIN(CAST("uid" AS TEXT)), COUNT(DISTINCT CAST("uid" AS TEXT)) FROM "public"."orders" GROUP BY CAST("uid" AS TEXT)"#,
            r#"SELECT "t1"."id" FROM ("public"."orders" AS "t1" INNER JOIN "public"."customers" AS "t2" ON (CAST("t1"."uid" AS TEXT) = CAST("t2"."uid" AS TEXT)))"#,
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
        r#"SELECT "id" FROM "public"."orders" WHERE "#,
        [
            r#"(position(B'1' IN "flag") > 0)"#,
            r#""active""#,
            r#"(NOT (position(B'1' IN "flag") > 0))"#,
            r#"((position(B'1' IN "flag") > 0) AND "active")"#,
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

/// An involved table of a `boolean` column `flag` and a `bit` column `bits`,
/// each declared `BOOLEAN` and recording its own source type, beside a `uuid`
/// column `uid` and an `id` column that records no notes.
fn flags_table(name: &str) -> Value {
    json!({
        "name": name,
        "columns": [
            {"name": "id", "dataType": id_type()},
            {
                "name": "flag",
                "dataType": boolean_type(),
                "adapterNotes": r#"{"sourceType":"boolean"}"#,
            },
            {
                "name": "bits",
                "dataType": boolean_type(),
                "adapterNotes": r#"{"sourceType":"bit"}"#,
            },
            {
                "name": "uid",
                "dataType": {"type": "VARCHAR", "size": 2000000, "characterSet": "UTF8"},
                "adapterNotes": r#"{"sourceType":"uuid"}"#,
            },
        ],
    })
}

/// A single-group aggregation over `flags` projecting `select_list`.
fn flags_aggregation(select_list: Value, data_types: Value) -> Value {
    json!({
        "type": "pushdown",
        "pushdownRequest": {
            "type": "select",
            "from": {"type": "table", "name": "flags"},
            "selectList": select_list,
            "selectListDataTypes": data_types,
            "aggregationType": "single_group",
        },
        "involvedTables": [flags_table("flags")],
    })
}

/// `vs-adapter/postgresql-pushdown`, scenario "MIN and MAX over a BOOLEAN
/// column render as bool_and and bool_or": over a `boolean` column and a `bit`
/// column, with and without `DISTINCT`, over the read expression of each; and
/// over a column of any other type, as `MIN` and `MAX`.
#[test]
fn min_and_max_over_boolean_render_as_bool_and_and_bool_or() {
    for (select_entry, expected) in [
        (
            fixtures::aggregate_over("MIN", "flag", false),
            r#"bool_and("flag")"#,
        ),
        (
            fixtures::aggregate_over("MAX", "flag", false),
            r#"bool_or("flag")"#,
        ),
        (
            fixtures::aggregate_over("MIN", "flag", true),
            r#"bool_and(DISTINCT "flag")"#,
        ),
        (
            fixtures::aggregate_over("MAX", "flag", true),
            r#"bool_or(DISTINCT "flag")"#,
        ),
        (
            fixtures::aggregate_over("MIN", "bits", false),
            r#"bool_and((position(B'1' IN "bits") > 0))"#,
        ),
        (
            fixtures::aggregate_over("MAX", "bits", false),
            r#"bool_or((position(B'1' IN "bits") > 0))"#,
        ),
        (
            fixtures::aggregate_over("MAX", "bits", true),
            r#"bool_or(DISTINCT (position(B'1' IN "bits") > 0))"#,
        ),
        (
            fixtures::aggregate_over("COUNT", "flag", true),
            r#"COUNT(DISTINCT "flag")"#,
        ),
        (fixtures::aggregate_over("MIN", "id", false), r#"MIN("id")"#),
        (
            fixtures::aggregate_over("MAX", "uid", false),
            r#"MAX(CAST("uid" AS TEXT))"#,
        ),
    ] {
        let data_type = if select_entry["name"] == "COUNT" {
            count_type()
        } else {
            boolean_type()
        };
        let request = flags_aggregation(json!([select_entry]), json!([data_type]));

        assert_eq!(
            render_statement(&request),
            format!(r#"SELECT {expected} FROM "public"."flags""#)
        );
    }
}

/// An aggregate over a select-list predicate carries no declared column type to
/// read, because a predicate is boolean by construction, and PostgreSQL has no
/// `min` or `max` for it either.
#[test]
fn min_and_max_over_a_predicate_render_as_bool_and_and_bool_or() {
    let predicate = less(numeric_literal("5"), column("id"));
    for (function, expected) in [("MIN", "bool_and"), ("MAX", "bool_or")] {
        let entry = json!({
            "type": "function_aggregate",
            "name": function,
            "arguments": [predicate],
        });
        let request = flags_aggregation(json!([entry]), json!([boolean_type()]));

        assert_eq!(
            render_statement(&request),
            format!(r#"SELECT {expected}((5 < "id")) FROM "public"."flags""#)
        );
    }
}

/// The aggregates over a `BOOLEAN` operand that `every_advertised_capability_
/// renders_per_dialect` exercises as their own arm, because this dialect
/// renders `MIN` and `MAX` differently for that one declared type.
pub(crate) fn boolean_operand_exercising(capability: &str) -> Option<(&'static str, Query)> {
    let (fragment, function) = match capability {
        "FN_AGG_MIN" => (r#"bool_and("flag")"#, "MIN"),
        "FN_AGG_MAX" => (r#"bool_or("flag")"#, "MAX"),
        _ => return None,
    };
    let flag = pushdown_query::ColumnRef {
        name: "flag".to_string(),
        declared_type: ExasolType::Boolean,
        source_type: Some("boolean".to_string()),
        table_name: None,
        table_alias: None,
    };
    let query = fixtures::aggregated(
        &namespace(),
        vec![SelectItem {
            expression: Expression::Aggregate(fixtures::aggregate_call(
                function,
                Expression::Column(flag),
                false,
            )),
            declared_type: ExasolType::Boolean,
        }],
    );

    Some((fragment, query))
}

/// `vs-adapter/pushdown-dialect`, scenario "A column reference renders through
/// its read expression in every position", over PostgreSQL's own read
/// expressions: the join keys of a full outer join, each a `uuid` read as text,
/// carry the conversion on both sides, qualified by the alias of their table.
#[test]
fn converted_join_keys_render_through_their_read_expressions_in_a_full_outer_join() {
    let qualified =
        |name: &str, table: &str| json!({"type": "column", "name": name, "tableName": table});
    let request = json!({
        "type": "pushdown",
        "pushdownRequest": {
            "type": "select",
            "from": {
                "type": "join",
                "join_type": "full_outer",
                "left": {"type": "table", "name": "flags"},
                "right": {"type": "table", "name": "other_flags"},
                "condition": equal(qualified("uid", "flags"), qualified("uid", "other_flags")),
            },
            "selectList": [qualified("id", "flags")],
        },
        "involvedTables": [flags_table("flags"), flags_table("other_flags")],
    });

    assert_eq!(
        render_statement(&request),
        r#"SELECT "t1"."id" FROM ("public"."flags" AS "t1" FULL OUTER JOIN "public"."other_flags" AS "t2" ON (CAST("t1"."uid" AS TEXT) = CAST("t2"."uid" AS TEXT)))"#
    );
}
