# Feature: pushdown_aggregation

Delegates a single-group or `GROUP BY` aggregation and its `HAVING` clause to the source, rendering each aggregate in the source's own SQL.

## Background

- This feature owns the aggregation fragment for every dialect: how a single-group and a grouped aggregation render, how a `having` element renders, and how each aggregate function renders.
- Each `<technology>-collation` feature states what its source's default collation does to a grouping key, a `DISTINCT` operand, and a `MIN` or `MAX` operand.
- `vs-adapter/result-ordering` owns the rejection of `orderBy` and `limit`.
- An aggregation request carries an `aggregationType` of `single_group` or `group_by`, a `groupBy` list of columns, `function_aggregate` select-list entries, one `selectListDataTypes` entry per select-list entry, and an optional `having` predicate.
- `vs-adapter/pushdown-aggregation-types` owns where each aggregate's declared `EMITS` type comes from, and `vs-adapter/pushdown-aggregation-delegation` owns which aggregates, grouping shapes, and `HAVING` clauses a dialect delegates.

## Scenarios

### Scenario: Single-group aggregation renders without a GROUP BY clause

* *GIVEN* a `pushdown` request whose `aggregationType` is `single_group`, which Exasol sends when the query aggregates without grouping, and whose select list is one `COUNT` over a mapped `DECIMAL` column
* *WHEN* each dialect renders the statement
* *THEN* the generated statement MUST project `COUNT` over the quoted column
* *AND* it MUST NOT carry a `GROUP BY` clause, in every dialect
* *AND* the `EMITS` clause MUST declare exactly one column

### Scenario: GROUP BY renders every grouping column the request carries, in request order

* *GIVEN* a constructed `pushdown` request whose `aggregationType` is `group_by`, whose `groupBy` names two columns, and whose select list names those columns and one aggregate
* *WHEN* the dialect renders the statement
* *THEN* the generated statement MUST carry a `GROUP BY` clause naming both columns in the order `groupBy` gives them
* *AND* the select list MUST render in the order the `selectList` gives it, which need not match the `groupBy` order
* *AND* the `EMITS` clause MUST declare one positionally named column per select-list entry
* *AND* this requirement MUST be verified by a unit test over a constructed request for every dialect, because a dialect withholding `AGGREGATE_GROUP_BY_TUPLE` is sent no `groupBy` naming more than one column and a constructed request is the only way to reach its renderer at all, and it MUST additionally be verified end to end under the Trino dialect, because that dialect advertises `AGGREGATE_GROUP_BY_TUPLE` and Exasol therefore sends it a multi-column `groupBy` rather than withdrawing the grouping

### Scenario: COUNT star is told apart from COUNT of a column

* *GIVEN* a `function_aggregate` named `COUNT` carrying no `arguments`
* *WHEN* the dialect renders it
* *THEN* the rendered aggregate MUST be `COUNT(*)`
* *AND* a `function_aggregate` named `COUNT` carrying one column argument MUST render as `COUNT` over that quoted column, which counts only its non-null values

### Scenario: DISTINCT is rendered inside the aggregate

* *GIVEN* a `function_aggregate` whose `distinct` field is true
* *WHEN* the dialect renders it
* *THEN* the rendered aggregate MUST carry `DISTINCT` before its argument
* *AND* a `function_aggregate` carrying no `distinct` field MUST render without `DISTINCT`

### Scenario: Aggregate function name case does not change the rendered statement

* *GIVEN* two `pushdown` requests differing only in the case of their `function_aggregate` names, one carrying `min` and one carrying `MIN`, as Exasol sends them: measured on the test Exasol, one request carried `sum`, `min`, and `max` in lower case beside `AVG` and `COUNT` in upper case
* *WHEN* the dialect renders each
* *THEN* both generated statements MUST be byte-for-byte identical, because the adapter matches a function name case-insensitively
* *AND* the rendered function name MUST be the target's canonical upper-case spelling, which every target this release supports accepts for `COUNT`, `MIN`, `MAX`, and `SUM`
* *AND* a name the dialect does not render MUST fail regardless of its case

### Scenario: Unrenderable aggregate function fails with a named error

* *GIVEN* a `pushdown` request whose select list carries an aggregate function the dialect does not render
* *WHEN* the adapter builds the response
* *THEN* the adapter MUST return an error
* *AND* the error message MUST name the aggregate function it received
* *AND* the adapter MUST NOT render the aggregate's argument alone, because Exasol would then aggregate nothing and return the wrong value

### Scenario: A having element renders as a HAVING clause

* *GIVEN* a `pushdown` request whose `aggregationType` is `group_by` and which carries a `having` element, as Exasol sends for `SELECT "category", COUNT(*) FROM vs."items" GROUP BY "category" HAVING COUNT(*) > 12` once `AGGREGATE_HAVING` is advertised
* *WHEN* each dialect renders the statement
* *THEN* the generated statement MUST carry a `HAVING` clause after the `GROUP BY` clause, rendering the `having` predicate
* *AND* the predicate MUST render through the predicate renderer a `WHERE` filter and a select-list predicate use, and each aggregate inside it through the aggregate renderer a select-list aggregate uses
* *AND* a request whose `aggregationType` is `single_group` and which carries a `having` element MUST render the `HAVING` clause with no `GROUP BY` clause
* *AND* the adapter MUST NOT drop the element, because Exasol reads the result as one the `HAVING` clause was already applied to

### Scenario: SUM renders as the source's SUM over the operand

* *GIVEN* a `function_aggregate` named `SUM` over a column, once without and once with `distinct`
* *WHEN* each dialect renders it
* *THEN* the rendered aggregate MUST be `SUM` over the column's read expression, carrying `DISTINCT` before the operand exactly when the request sets `distinct`
* *AND* the operand MUST be the column as every other aggregate operand renders it, except where the dialect's `<technology>-pushdown` feature widens the operand's type inside `SUM` because its target would otherwise sum in a narrower type, and except for a `BOOLEAN` or character operand, which renders converted as the conversion scenario states

### Scenario: AVG renders as the double-precision quotient of SUM and COUNT

* *GIVEN* a `function_aggregate` named `AVG` over a column, once without and once with `distinct`
* *WHEN* each dialect renders it
* *THEN* the rendered expression MUST be the operand's `SUM`, rendered as the `SUM` scenario states, cast to the dialect's double-precision type and divided by the operand's `COUNT`, the whole quotient in parentheses: the type is `DOUBLE` for MySQL and Trino and `DOUBLE PRECISION` for PostgreSQL
* *AND* when the request sets `distinct`, both the `SUM` and the `COUNT` MUST carry `DISTINCT` before the operand, so the distinct values are taken over the operand's own type and never over its double-precision cast, except for a `BOOLEAN` or character operand, whose `SUM` takes its distinct values over the converted operand, as Exasol does
* *AND* the dialect MUST NOT render the target's own `AVG`, because MySQL's returns a `DECIMAL` at the operand's scale plus 4 digits over an exact-numeric operand and Trino's returns a `decimal` at the operand's own scale while Exasol declares `AVG` as `DOUBLE`, and MUST NOT render `AVG` over the operand cast to a double-precision type, because measured on the test MySQL and the test Exasol that form returns a value wrong by 47 per cent for `AVG(DISTINCT)` over a `DECIMAL(19,0)` column holding values near 1e18
* *AND* a group whose operand values are all null MUST yield null, because the `SUM` of no value is null and every target returns null for null divided by zero

### Scenario: A BOOLEAN or character SUM and AVG operand renders converted as Exasol converts it

* *GIVEN* a `function_aggregate` named `SUM` or `AVG`, once without and once with `distinct`, over a `BOOLEAN` operand, which is a column declared `BOOLEAN` or a predicate, and over a character operand, which is a column declared `VARCHAR` or `CHAR` or a string literal
* *WHEN* each dialect renders it
* *THEN* the `SUM` operand MUST be the value Exasol sums when it computes the aggregate itself: a `BOOLEAN` operand as 1 for true and 0 for false with null kept null, and a character operand as a double-precision number with an empty value read as null, because Exasol converts both implicitly and PostgreSQL and Trino define no `sum` over a boolean or a character type
* *AND* PostgreSQL and Trino MUST render a `BOOLEAN` operand as `CAST(<operand> AS INTEGER)`, and MySQL MUST render it unchanged, because MySQL evaluates a boolean as the integer 1 or 0
* *AND* every dialect MUST render a character operand as `CAST(NULLIF(<operand>, '') AS <double type>)`, with the double type the `AVG` scenario names, because Exasol reads an empty value as null and converts every other value to `DOUBLE`
* *AND* `SUM(DISTINCT)` MUST carry `DISTINCT` before the converted operand, and the `COUNT` inside a rendered `AVG` MUST take the operand before the conversion to a number, a character operand as `NULLIF(<operand>, '')`, because Exasol takes the distinct values after its conversion and divides `AVG(DISTINCT)` by the number of distinct unconverted values: measured on the test Exasol over `'5'`, `'5.0'`, and `'7'`, `SUM(DISTINCT)` returned `12` and `AVG(DISTINCT)` returned `4`, which is `12` divided by `3`
