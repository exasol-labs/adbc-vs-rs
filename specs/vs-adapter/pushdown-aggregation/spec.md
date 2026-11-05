# Feature: pushdown_aggregation

Delegates a single-group or `GROUP BY` aggregation to the source, rendering each aggregate in the source's own SQL.

## Background

- This feature owns the aggregation fragment for every dialect: the elements it rejects, how a single-group and a grouped aggregation render, and how each aggregate function renders.
- Each `<technology>-collation` feature states what its source's default collation does to a grouping key, a `DISTINCT` operand, and a `MIN` or `MAX` operand.
- `vs-adapter/result-ordering` owns the rejection of `orderBy` and `limit`.
- An aggregation request carries an `aggregationType` of `single_group` or `group_by`, a `groupBy` list of columns, `function_aggregate` select-list entries, and one `selectListDataTypes` entry per select-list entry.
- `vs-adapter/pushdown-aggregation-types` owns where each aggregate's declared `EMITS` type comes from, and `vs-adapter/pushdown-aggregation-delegation` owns which aggregates and grouping shapes a dialect delegates.

## Scenarios

### Scenario: Request carrying a having element fails rather than dropping it

* *GIVEN* a `pushdown` request carrying a `having` element, which Exasol sends only when `AGGREGATE_HAVING` was advertised
* *WHEN* the adapter builds the response
* *THEN* the adapter MUST return an error naming the `having` element
* *AND* the error MUST name the withheld capability that gates the element, which is the one place a change would make it legal
* *AND* the adapter MUST NOT return a statement that silently ignores the element, because Exasol reads the result as one the `HAVING` clause was already applied to

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
* *AND* the rendered function name MUST be the target's canonical upper-case spelling, which every target this release supports accepts for `COUNT`, `MIN`, and `MAX`
* *AND* a name the dialect does not render MUST fail regardless of its case

### Scenario: Unrenderable aggregate function fails with a named error

* *GIVEN* a `pushdown` request whose select list carries an aggregate function the dialect does not render
* *WHEN* the adapter builds the response
* *THEN* the adapter MUST return an error
* *AND* the error message MUST name the aggregate function it received
* *AND* the adapter MUST NOT render the aggregate's argument alone, because Exasol would then aggregate nothing and return the wrong value
