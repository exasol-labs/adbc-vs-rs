# Feature: pushdown_aggregation

Delegates a single-group or `GROUP BY` aggregation and its `HAVING` clause to the source, rendering each aggregate in the source's own SQL.

<!-- DELTA:CHANGED -->
## Background

- This feature owns the aggregation fragment for every dialect: how a single-group and a grouped aggregation render, how a `having` element renders, and how each aggregate function renders.
- Each `<technology>-collation` feature states what its source's default collation does to a grouping key, a `DISTINCT` operand, and a `MIN` or `MAX` operand.
- `vs-adapter/result-ordering` owns the rejection of `orderBy`, and `vs-adapter/pushdown-limit` owns the `limit` that renders after the `GROUP BY` and `HAVING` clauses.
- An aggregation request carries an `aggregationType` of `single_group` or `group_by`, a `groupBy` list of columns, `function_aggregate` select-list entries, one `selectListDataTypes` entry per select-list entry, and an optional `having` predicate.
- `vs-adapter/pushdown-aggregation-types` owns where each aggregate's declared `EMITS` type comes from, and `vs-adapter/pushdown-aggregation-delegation` owns which aggregates, grouping shapes, and `HAVING` clauses a dialect delegates.
<!-- /DELTA:CHANGED -->

## Scenarios

### Scenario: Single-group aggregation renders without a GROUP BY clause

* *GIVEN* a `pushdown` request whose `aggregationType` is `single_group`, which Exasol sends when the query aggregates without grouping, and whose select list is one `COUNT` over a mapped `DECIMAL` column
* *WHEN* each dialect renders the statement
* *THEN* the generated statement MUST project `COUNT` over the quoted column
* *AND* it MUST NOT carry a `GROUP BY` clause, in every dialect
* *AND* the `EMITS` clause MUST declare exactly one column
