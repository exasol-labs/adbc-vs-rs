use serde_json::{json, Value};

use super::*;
use crate::exa_type::ExasolType;
use crate::pushdown_query;

/// The namespace a PostgreSQL virtual schema records and every rendered table
/// is qualified by: the `public` schema, never the database the connection
/// address names.
const NAMESPACE: &str = "public";

/// A single-table `pushdown` request over `orders`, rendered through the
/// parser so each assertion below covers the request shape Exasol sends
/// rather than a hand-built model.
fn request(columns: Value, select_list: Option<Value>) -> Value {
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

fn orders_columns() -> Value {
    json!([
        {"name": "id", "dataType": {"type": "DECIMAL", "precision": 19, "scale": 0}},
        {"name": "total", "dataType": {"type": "DECIMAL", "precision": 12, "scale": 4}},
        {"name": "note", "dataType": {"type": "VARCHAR", "size": 100, "characterSet": "UTF8"}},
    ])
}

/// The PostgreSQL statement `request` renders to, parsed first so every
/// assertion covers the request shape Exasol sends rather than a hand-built
/// model.
fn render_statement(request: &Value) -> String {
    let query = pushdown_query::parse(request, NAMESPACE).expect("the request must parse");
    PostgresDialect
        .render(&query)
        .expect("the query must render to PostgreSQL")
}

fn render(columns: Value, select_list: Option<Value>) -> String {
    render_statement(&request(columns, select_list))
}

/// The error rendering `request` fails with, for a request the parser accepts
/// and this dialect has no form for.
fn render_error(request: &Value) -> String {
    let query = pushdown_query::parse(request, NAMESPACE).expect("the request must parse");
    PostgresDialect
        .render(&query)
        .expect_err("the query must fail to render")
        .to_string()
}

fn column(name: &str) -> Value {
    json!({"type": "column", "name": name})
}

/// A `pushdown` request carrying a `filter` element, projecting one column so
/// every assertion below reads the `WHERE` clause rather than the projection
/// around it.
fn filtered_request(columns: Value, filter: Value) -> Value {
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

/// The `WHERE` clause `filter` renders to over `columns`, parsed from a
/// request first so each assertion covers the shape Exasol sends.
fn where_clause_over(columns: Value, filter: Value) -> String {
    let sql = render_statement(&filtered_request(columns, filter));
    let (_, clause) = sql
        .split_once(" WHERE ")
        .unwrap_or_else(|| panic!("the rendered statement must carry a WHERE clause: {sql}"));
    clause.to_string()
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

fn numeric_literal(value: &str) -> Value {
    json!({"type": "literal_exactnumeric", "value": value})
}

fn string_literal(value: &str) -> Value {
    json!({"type": "literal_string", "value": value})
}

fn equal(left: Value, right: Value) -> Value {
    json!({"type": "predicate_equal", "left": left, "right": right})
}

/// A `predicate_less`, the node Exasol normalizes `>` into with the operands
/// swapped, so a literal reaches `left` and the column `right`.
fn less(left: Value, right: Value) -> Value {
    json!({"type": "predicate_less", "left": left, "right": right})
}

/// The columns whose declared types the collation rule reads: one character,
/// one numeric, one temporal.
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
/// over a numeric column so no collation suffix stands between the operator
/// and the assertion. `predicate_notequal` is `<>`, a null test is `IS NULL`
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
    let columns = json!([
        {"name": "shipped", "dataType": {"type": "DATE"}},
        {"name": "logged", "dataType": {"type": "TIMESTAMP"}},
    ]);

    let clause = where_clause_over(
        columns,
        json!({
            "type": "predicate_and",
            "expressions": [
                equal(column("shipped"), json!({"type": "literal_date", "value": "2015-12-01"})),
                equal(
                    column("logged"),
                    json!({"type": "literal_timestamp", "value": "2015-12-01 12:01:01.123"}),
                ),
            ],
        }),
    );

    assert_eq!(
        clause,
        r#"(("shipped" = DATE '2015-12-01') AND ("logged" = TIMESTAMP '2015-12-01 12:01:01.123'))"#
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
/// `vs-adapter/postgresql-collation`, measured by gate 2.3(f): the operand
/// carries **no** collation suffix, where MySQL's does. PostgreSQL's `LIKE`
/// compares character by character rather than by collation weight, and four
/// patterns measured bare and collated returned identical row sets in both
/// directions.
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
/// `vs-adapter/postgresql-collation`: the suffix belongs on the compared
/// operand of an ordering predicate and nowhere else, so `BETWEEN` collates
/// its character operand while its bounds render bare, and the `IN` compared
/// operand renders bare because `IN` asks equality — the position where
/// MySQL does render a suffix and this dialect must not.
#[test]
fn between_and_in_keep_operand_order() {
    let between = where_clause(json!({
        "type": "predicate_between",
        "expression": column("note"),
        "left": string_literal("z"),
        "right": string_literal("a"),
    }));

    assert_eq!(
        between, r#"("note" COLLATE "C" BETWEEN E'z' AND E'a')"#,
        "the bounds keep the request's order and carry no suffix of their own"
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

/// `vs-adapter/postgresql-collation`: `COLLATE "C"` reaches a **character**
/// operand of `predicate_less`, `predicate_lessequal`, and `predicate_between`
/// and nothing else. Gate 2.3(d) measured that the suffix changes which rows
/// an ordering comparison returns — `g < 'b'` returns `B,a,C` collated and
/// `a` bare, where Exasol returns `B,a,C` — and gate 2.3(e) measured that
/// equality already agrees, so a suffix on an equality operand would be
/// decoration that costs the source its index.
#[test]
fn collation_placement_matrix() {
    let ordering_predicate = |operand: &str| json!({"type": "predicate_less", "left": column(operand), "right": column(operand)});

    assert_eq!(
        where_clause_over(typed_columns(), ordering_predicate("note")),
        r#"("note" COLLATE "C" < "note" COLLATE "C")"#,
        "both operands of an ordering comparison are read by position, and each character-typed \
         one carries the suffix"
    );

    for bare in [
        ordering_predicate("id"),
        ordering_predicate("shipped"),
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
    ] {
        let clause = where_clause_over(typed_columns(), bare.clone());

        assert!(
            !clause.contains("COLLATE"),
            "this position carries no collation on PostgreSQL: {clause}, for {bare}"
        );
    }
}

/// `vs-adapter/pushdown-filter`: a predicate delegated into the select list
/// renders as a projected boolean rather than reaching a second renderer.
///
/// `vs-adapter/postgresql-collation`: its character operand carries no
/// suffix — the third of the three sites where MySQL renders one and this
/// dialect must not.
#[test]
fn select_list_predicate_renders_as_projected_boolean() {
    let mut request = request(
        orders_columns(),
        Some(json!([less(string_literal("x"), column("note"))])),
    );
    request["pushdownRequest"]["selectListDataTypes"] = json!([{"type": "BOOLEAN"}]);

    let sql = render_statement(&request);

    assert_eq!(
        sql,
        r#"SELECT (E'x' COLLATE "C" < "note" COLLATE "C") FROM "public"."orders""#
    );

    let mut equality = request.clone();
    equality["pushdownRequest"]["selectList"] = json!([equal(column("note"), string_literal("x"))]);

    assert_eq!(
        render_statement(&equality),
        r#"SELECT ("note" = E'x') FROM "public"."orders""#,
        "a select-list predicate carries no collation on an equality operand"
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
            "the error must name the empty PostgreSQL form: {error}"
        );
    }
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

/// A column node as Exasol sends one inside a join: qualified by the
/// `tableName` of the table it belongs to. Measured on Exasol 2025.2.1, that
/// field arrives on every column node of a join, and `tableAlias` joins it only
/// once the operator's query aliased the table.
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

/// The `from` element joining `orders` to `customers` over the foreign key, in
/// the shape a join whose query aliased no table arrives in: neither table
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
/// involved tables `orders_to_customers` names.
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
        let sql = render_statement(&join_request(
            orders_to_customers(join_type),
            json!([joined_column("id", "orders")]),
        ));

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
    for join_type in ["cross", "left_semi"] {
        let error = render_error(&join_request(
            orders_to_customers(join_type),
            json!([joined_column("id", "orders")]),
        ));

        assert!(
            error.contains(join_type),
            "the error must name the join type it received: {error}"
        );
    }

    let mysql_error = crate::mysql_dialect::MySqlDialect
        .render(
            &pushdown_query::parse(
                &join_request(
                    orders_to_customers("full_outer"),
                    json!([joined_column("id", "orders")]),
                ),
                NAMESPACE,
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
    let mut request = join_request(
        json!({
            "type": "join",
            "join_type": "full_outer",
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
        r#"SELECT "t3"."code" FROM (("public"."orders" AS "t1" INNER JOIN "public"."customers" AS "t2" ON ("t1"."customer_id" = "t2"."id")) FULL OUTER JOIN "public"."regions" AS "t3" ON ("t2"."id" = "t3"."customer_id"))"#
    );
}

/// `vs-adapter/pushdown-join`: every position a column reaches the renderer
/// from carries the alias of the table it belongs to — the select list, the
/// `WHERE` clause, and the join's own condition.
///
/// `vs-adapter/postgresql-collation`: a character join key and a character
/// equality operand each render bare, so aliasing does not turn the
/// type-keyed collation rule into an unconditional suffix.
#[test]
fn joined_tables_are_aliased_and_qualified() {
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
        r#"SELECT "t2"."name" FROM ("public"."orders" AS "t1" INNER JOIN "public"."customers" AS "t2" ON ("t1"."customer_id" = "t2"."id")) WHERE ("t1"."note" = E'x')"#
    );
}

/// A character join key renders bare while an ordering comparison over the
/// same column in the same statement renders collated, so the join condition
/// takes the type-keyed rule rather than an exemption of its own.
#[test]
fn a_character_join_key_renders_bare_beside_a_collated_ordering_comparison() {
    let mut from = orders_to_customers("full_outer");
    from["condition"] = json!({
        "type": "predicate_equal",
        "left": joined_column("note", "orders"),
        "right": joined_column("name", "customers"),
    });
    let mut request = join_request(from, json!([joined_column("name", "customers")]));
    request["pushdownRequest"]["filter"] =
        less(string_literal("m"), joined_column("note", "orders"));

    let sql = render_statement(&request);

    assert_eq!(
        sql,
        r#"SELECT "t2"."name" FROM ("public"."orders" AS "t1" FULL OUTER JOIN "public"."customers" AS "t2" ON ("t1"."note" = "t2"."name")) WHERE (E'm' COLLATE "C" < "t1"."note" COLLATE "C")"#
    );
}

/// `vs-adapter/pushdown-join`: a join carrying both a `filter` and a `groupBy`
/// in the same request renders every clause together, in PostgreSQL's required
/// order — `FROM`, `WHERE`, `GROUP BY`. Each tier renders its own fragment;
/// only this test proves the three compose over a joined `FROM`.
#[test]
fn join_composes_with_filter_and_grouping() {
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
    let self_join = json!({
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

    assert_eq!(
        render_statement(&self_join),
        r#"SELECT "t1"."id", "t2"."id" FROM ("public"."orders" AS "t1" INNER JOIN "public"."orders" AS "t2" ON ("t1"."id" = "t2"."customer_id"))"#
    );

    let mut ambiguous = orders_to_customers("inner");
    ambiguous["right"] = json!({"type": "table", "name": "orders"});
    let error = render_error(&join_request(
        ambiguous,
        json!([joined_column("id", "orders")]),
    ));

    assert!(
        error.contains("`id`") && error.contains("orders") && error.contains("more than one"),
        "the error must name the column and the reference two tables answer to: {error}"
    );
}

/// An operator's own alias resolves the reference and is never rendered.
/// Rendering it would put a user-chosen identifier into the alias namespace
/// this dialect has to keep unique, where `FROM "orders" "t2" JOIN "customers"`
/// would produce `t2` for two different tables.
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

/// A `function_aggregate` select-list entry over at most one argument. `None`
/// is the `COUNT(*)` shape: Exasol sends it carrying no `arguments` at all,
/// which is how it is told apart from `COUNT(column)`.
fn aggregate(function: &str, argument: Option<Value>) -> Value {
    let mut entry = json!({"type": "function_aggregate", "name": function});
    if let Some(argument) = argument {
        entry["arguments"] = json!([argument]);
    }
    entry
}

/// A `pushdown` request over `orders` carrying `select_list` and the
/// `selectListDataTypes` Exasol supplies for it, which is the only type source
/// an aggregate or a select-list predicate entry has.
fn typed_request(select_list: Value, data_types: Value) -> Value {
    let mut request = request(orders_columns(), Some(select_list));
    request["pushdownRequest"]["selectListDataTypes"] = data_types;
    request
}

/// A `pushdown` request whose `aggregationType` is `group_by` over
/// `group_by` — the shape MySQL renders through one coordinated collation rule
/// and this dialect renders through none.
fn grouped_request(select_list: Value, data_types: Value, group_by: Value) -> Value {
    let mut request = typed_request(select_list, data_types);
    request["pushdownRequest"]["aggregationType"] = json!("group_by");
    request["pushdownRequest"]["groupBy"] = group_by;
    request
}

/// The type Exasol supplies for `COUNT` over any operand, measured on Exasol
/// 2025.2.1 over a PostgreSQL-shaped schema by gate 2.3(h).
fn count_type() -> Value {
    json!({"type": "DECIMAL", "precision": 18, "scale": 0})
}

/// The type Exasol supplies for `MIN` or `MAX` over `note`, a `VARCHAR(100)` —
/// the operand's own type, spelled without the `characterSet` field
/// `involvedTables[].columns[].dataType` carries.
fn note_type() -> Value {
    json!({"type": "VARCHAR", "size": 100})
}

fn id_type() -> Value {
    json!({"type": "DECIMAL", "precision": 19, "scale": 0})
}

fn total_type() -> Value {
    json!({"type": "DECIMAL", "precision": 12, "scale": 4})
}

/// The type Exasol supplies for a select-list predicate: one `BOOLEAN` column.
fn boolean_type() -> Value {
    json!({"type": "BOOLEAN"})
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

/// `vs-adapter/postgresql-collation`: a `GROUP BY` key, the bare select-list
/// occurrence of that key, and a `DISTINCT` operand each render with no
/// collation suffix, and `MIN` over the same character column renders with
/// one.
///
/// The absence is what makes the shape legal rather than merely tidy. Gate
/// 2.3(b) measured the PostgreSQL analogue of MySQL error 1055 firing only
/// when the grouping key carries a collation, so a bare key is what leaves the
/// key and its select-list occurrence the same expression. Gate 2.3(e)
/// measured that an uncollated `GROUP BY` and `COUNT(DISTINCT)` over
/// `widget`/`Widget` and `cafe`/`café` return Exasol's four groups and its
/// distinct count of four, so the suffix would change no answer either.
#[test]
fn grouping_and_distinct_render_uncollated() {
    let mut distinct_count = aggregate("COUNT", Some(column("note")));
    distinct_count["distinct"] = json!(true);

    let sql = render_statement(&grouped_request(
        json!([
            column("note"),
            column("id"),
            aggregate("MIN", Some(column("note"))),
            distinct_count,
        ]),
        json!([note_type(), id_type(), note_type(), count_type()]),
        json!([column("note"), column("id")]),
    ));

    assert_eq!(
        sql,
        r#"SELECT "note", "id", MIN("note" COLLATE "C"), COUNT(DISTINCT "note") FROM "public"."orders" GROUP BY "note", "id""#
    );
}

/// `MIN` and `MAX` return the value their operand's collation sorts first or
/// last, so reducing the values by equality first cannot change which one comes
/// back: the ordering suffix stays whatever `distinct` the request carries.
/// The rule that leaves a `DISTINCT` operand bare is about the reduction, and
/// no reduction chooses this value.
#[test]
fn a_distinct_min_keeps_the_ordering_collation() {
    let mut distinct_min = aggregate("MIN", Some(column("note")));
    distinct_min["distinct"] = json!(true);
    let mut distinct_count = aggregate("COUNT", Some(column("note")));
    distinct_count["distinct"] = json!(true);

    let sql = render_statement(&typed_request(
        json!([distinct_min, distinct_count]),
        json!([note_type(), count_type()]),
    ));

    assert_eq!(
        sql,
        r#"SELECT MIN(DISTINCT "note" COLLATE "C"), COUNT(DISTINCT "note") FROM "public"."orders""#
    );
}

/// Exasol orders character values by code point and pads no `VARCHAR`, and the
/// cluster's own `en_US.utf8` locale orders by collation weight, so a character
/// `MIN` or `MAX` carries the collation that reproduces Exasol's answer.
/// `COUNT` reads no order, and a numeric operand has no collation to carry.
#[test]
fn only_min_and_max_over_a_character_operand_carry_the_ordering_collation() {
    let sql = render_statement(&typed_request(
        json!([
            aggregate("MIN", Some(column("note"))),
            aggregate("MAX", Some(column("note"))),
            aggregate("COUNT", Some(column("note"))),
            aggregate("MIN", Some(column("total"))),
        ]),
        json!([note_type(), note_type(), count_type(), total_type()]),
    ));

    assert_eq!(
        sql,
        r#"SELECT MIN("note" COLLATE "C"), MAX("note" COLLATE "C"), COUNT("note"), MIN("total") FROM "public"."orders""#
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
    let entries = |count: &str, min: &str| {
        json!([
            aggregate(count, Some(column("total"))),
            aggregate(min, Some(column("note"))),
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
        r#"SELECT COUNT("total"), MIN("note" COLLATE "C") FROM "public"."orders""#
    );
}

/// Rendering the aggregate's argument alone would leave Exasol aggregating
/// nothing and returning the wrong value, so an unknown function fails. `AVG`
/// and `SUM` fail the same way: both are withheld capabilities, so neither
/// reaches this path from an advertising Exasol, but a request that somehow
/// sends one anyway must still fail naming it.
#[test]
fn unrenderable_aggregate_named_in_error() {
    for (function, named) in [
        ("STDDEV", "STDDEV"),
        ("stddev", "STDDEV"),
        ("AVG", "AVG"),
        ("SUM", "SUM"),
    ] {
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

/// `COUNT(*)` is the only aggregate PostgreSQL writes over no argument, and it
/// counts rows rather than values, so `COUNT(DISTINCT *)` is not a form either.
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
        "the error must name the fragment PostgreSQL has no form for: {count_error}"
    );
}

/// PostgreSQL writes no empty `GROUP BY` clause, and a renderer that dropped
/// the clause instead would aggregate the whole table into one row where Exasol
/// asked for one row per group.
#[test]
fn an_aggregation_grouping_by_no_column_fails_rather_than_rendering_an_empty_clause() {
    let mut request = typed_request(json!([aggregate("COUNT", None)]), json!([count_type()]));
    request["pushdownRequest"]["aggregationType"] = json!("group_by");
    request["pushdownRequest"]["groupBy"] = json!([]);

    let error = render_error(&request);

    assert!(
        error.contains("GROUP BY"),
        "the error must name the empty PostgreSQL clause: {error}"
    );
}

/// `vs-adapter/postgresql-collation`, gate 2.3(b): the composed shape MySQL
/// must refuse — a select-list predicate reading the query's own character
/// `GROUP BY` key outside any aggregate — is rendered here, because the key
/// renders bare and PostgreSQL's full-group-by rule reads column references
/// rather than matching whole expressions. Its negative control is the one
/// that pins the reason: the same query with a *collated* grouping key was
/// rejected with PostgreSQL's exact analogue of MySQL error 1055.
///
/// Five predicate kinds were measured arriving in that position; each must
/// render, with the grouping key bare in both clauses.
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
        r#"SELECT "note", (E'x' COLLATE "C" < "note" COLLATE "C") FROM "public"."orders" GROUP BY "note""#
    );
}

/// `vs-adapter/pushdown-aggregation`: an aggregate reaches the select list
/// inside a predicate as `COUNT(*) > 0`, and the whole entry is delegated as
/// one `BOOLEAN` column. `MIN` returns one of its argument's own values, so it
/// carries that argument's type: the comparison against it is an ordering one,
/// which collates both the aggregate and the literal it is compared against,
/// while the argument keeps the suffix the returned value is chosen by.
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
        r#"SELECT "note", (0 < COUNT(*)), (E'x' COLLATE "C" < MIN("note" COLLATE "C") COLLATE "C") FROM "public"."orders" GROUP BY "note""#
    );
}

/// `vs-adapter/result-ordering`: `orderBy` and `limit` stay in Exasol because
/// their capabilities are withheld, and `having` stays because
/// `AGGREGATE_HAVING` is. An element arriving anyway means the advertised set
/// and the rejection list have drifted apart, and reading past it would hand
/// Exasol a statement it believes carries the clause.
#[test]
fn order_by_and_limit_elements_fail_named() {
    for (element, value) in [
        ("orderBy", json!([column("id")])),
        ("limit", json!({"numElements": 10})),
        (
            "having",
            json!({
                "type": "predicate_less",
                "left": numeric_literal("0"),
                "right": aggregate("COUNT", None),
            }),
        ),
    ] {
        let mut request = request(orders_columns(), Some(json!([column("id")])));
        request["pushdownRequest"][element] = value;

        let error = pushdown_query::parse(&request, NAMESPACE)
            .expect_err("a request carrying an undelegated element must fail");

        assert!(
            error.contains(element),
            "the error must name the element it refused: {error}"
        );
    }
}

/// The other half of the collation-placement matrix: a numeric or temporal
/// operand carries no suffix in *any* position, not only in a comparison. The
/// rule reads the operand's declared type, so grouping, `DISTINCT`, and
/// `MIN`/`MAX` each answer from the same fact a predicate operand answers from,
/// and aliasing or grouping cannot turn it into an unconditional suffix.
#[test]
fn numeric_and_temporal_operands_carry_no_collation_in_any_position() {
    let mut distinct_id = aggregate("COUNT", Some(column("id")));
    distinct_id["distinct"] = json!(true);
    let mut request = grouped_request(
        json!([
            column("id"),
            column("shipped"),
            aggregate("MIN", Some(column("id"))),
            aggregate("MAX", Some(column("shipped"))),
            distinct_id,
        ]),
        json!([
            id_type(),
            {"type": "DATE"},
            id_type(),
            {"type": "DATE"},
            count_type(),
        ]),
        json!([column("id"), column("shipped")]),
    );
    request["involvedTables"][0]["columns"] = typed_columns();

    let sql = render_statement(&request);

    assert_eq!(
        sql,
        r#"SELECT "id", "shipped", MIN("id"), MAX("shipped"), COUNT(DISTINCT "id") FROM "public"."orders" GROUP BY "id", "shipped""#
    );
}

/// A query projecting no column is not valid PostgreSQL, and a renderer that
/// emitted an empty select list would fail at the source rather than here.
///
/// Built as a model rather than parsed from a request: the parser resolves an
/// empty `selectList` into the row-marker constant, so this shape reaches the
/// dialect only from a future model change.
#[test]
fn a_query_projecting_nothing_fails_rather_than_rendering_an_empty_projection() {
    let query = Query {
        select_list: vec![],
        filter: None,
        aggregation: None,
        from: FromClause::Table(SourceTable {
            schema: NAMESPACE.to_string(),
            name: "orders".to_string(),
            alias: None,
        }),
    };

    let error = PostgresDialect
        .render(&query)
        .expect_err("a query projecting no column must fail");

    assert!(
        error.to_string().contains("select list"),
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
        from: FromClause::Table(SourceTable {
            schema: NAMESPACE.to_string(),
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

    let query = pushdown_query::parse(&request, NAMESPACE).expect("the request must parse");
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
    fn decimal_column() -> pushdown_query::ColumnRef {
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

    fn varchar_column() -> pushdown_query::ColumnRef {
        pushdown_query::ColumnRef {
            name: "note".to_string(),
            declared_type: ExasolType::Varchar { size: 100 },
            table_name: None,
            table_alias: None,
        }
    }

    fn date_column() -> pushdown_query::ColumnRef {
        pushdown_query::ColumnRef {
            name: "shipped".to_string(),
            declared_type: ExasolType::Date,
            table_name: None,
            table_alias: None,
        }
    }

    fn timestamp_column() -> pushdown_query::ColumnRef {
        pushdown_query::ColumnRef {
            name: "logged".to_string(),
            declared_type: ExasolType::Timestamp,
            table_name: None,
            table_alias: None,
        }
    }

    fn select_list() -> Vec<SelectItem> {
        let column = decimal_column();
        vec![SelectItem {
            expression: Expression::Column(column.clone()),
            declared_type: column.declared_type,
        }]
    }

    fn from() -> FromClause {
        FromClause::Table(SourceTable {
            schema: NAMESPACE.to_string(),
            name: "orders".to_string(),
            alias: None,
        })
    }

    fn filtered(filter: Predicate) -> Query {
        Query {
            select_list: select_list(),
            filter: Some(filter),
            aggregation: None,
            from: from(),
        }
    }

    fn id_equals(right: Expression) -> Predicate {
        Predicate::Equal(Comparison {
            left: Expression::Column(decimal_column()),
            right,
        })
    }

    fn numeric(value: &str) -> Expression {
        Expression::Literal(Literal::ExactNumeric(value.to_string()))
    }

    fn aggregated(select_list: Vec<SelectItem>) -> Query {
        Query {
            select_list,
            filter: None,
            aggregation: Some(pushdown_query::Aggregation::SingleGroup),
            from: from(),
        }
    }

    fn aggregate_item(call: pushdown_query::AggregateCall) -> SelectItem {
        SelectItem {
            expression: Expression::Aggregate(call),
            declared_type: ExasolType::Decimal {
                precision: 18,
                scale: 0,
            },
        }
    }

    fn count_star() -> pushdown_query::AggregateCall {
        pushdown_query::AggregateCall {
            function: "COUNT".to_string(),
            argument: None,
            distinct: false,
        }
    }

    fn aggregate_call(
        function: &str,
        argument: Expression,
        distinct: bool,
    ) -> pushdown_query::AggregateCall {
        pushdown_query::AggregateCall {
            function: function.to_string(),
            argument: Some(Box::new(argument)),
            distinct,
        }
    }

    fn joined_column_ref(name: &str, table: &str) -> pushdown_query::ColumnRef {
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

    fn joined_query(join_type: &str) -> Query {
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
                    schema: NAMESPACE.to_string(),
                    name: "orders".to_string(),
                    alias: None,
                })),
                right: Box::new(FromClause::Table(SourceTable {
                    schema: NAMESPACE.to_string(),
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

    match capability {
        "SELECTLIST_PROJECTION" => (
            r#"SELECT "id" FROM"#,
            Query {
                select_list: select_list(),
                filter: None,
                aggregation: None,
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
            "LIKE E'Wid%'",
            filtered(Predicate::Like(LikePredicate {
                expression: Expression::Column(varchar_column()),
                pattern: Expression::Literal(Literal::String("Wid%".to_string())),
                escape_char: None,
            })),
        ),
        "FN_PRED_LIKE_ESCAPE" => (
            r"ESCAPE E'\\'",
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
            "E'widget'",
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
            r#""public"."orders" AS "t1" INNER JOIN "public"."customers" AS "t2""#,
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
