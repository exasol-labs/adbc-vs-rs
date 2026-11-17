# Feature: pushdown_aggregation_delegation

States which aggregation shapes and aggregate functions this release delegates to the source and which Exasol computes itself.

## Background

- This feature owns which aggregates, grouping shapes, and `HAVING` clauses this release delegates, which ones Exasol computes over the rows instead, and the rule that an aggregation is delegated whole or not at all.
- Each dialect's advertised capability set lives in its `<technology>-pushdown` feature, and `vs-adapter/pushdown-dialect` owns the `BOOLEAN` declared type and the all-or-nothing negotiation rule.
- Each `<technology>-collation` feature states what its source's default collation does to a grouping key, a `DISTINCT` operand, and a `MIN` or `MAX` operand.
- `vs-adapter/pushdown-aggregation` owns how a single-group and a grouped aggregation, a `having` element, and each aggregate function render, and `vs-adapter/pushdown-aggregation-types` owns each aggregate's declared `EMITS` type.

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: An aggregation Exasol cannot delegate whole is computed by Exasol

* *GIVEN* a query applying a scalar function no dialect advertises to a delegable aggregate, such as `ROUND(AVG("quantity"))`, and a query mixing `STDDEV`, which no dialect advertises, with `COUNT`, `SUM`, or `AVG`
* *WHEN* Exasol plans each query
* *THEN* each generated statement MUST carry no aggregate, because Exasol delegates an aggregation whole or not at all: measured on the test Exasol before any scalar function was advertised, `SUM("c") + 1` withdrew the aggregation entirely rather than delegating `SUM` and adding locally
* *AND* each query MUST return the value Exasol computes over the scanned rows, so the withdrawal costs throughput and no correctness
<!-- /DELTA:CHANGED -->

<!-- DELTA:REMOVED -->
### Scenario: Grouping by an expression is withheld from every dialect

* *GIVEN* the capability set of each dialect this release supports, Trino's included
* *WHEN* the set is read
* *THEN* it MUST NOT advertise `AGGREGATE_GROUP_BY_EXPRESSION`, so this release groups by columns rather than by expressions
* *AND* the withholding MUST be identical for every dialect, because it is a framework decision about this release's scope and independent of what any target can express
<!-- /DELTA:REMOVED -->

<!-- DELTA:NEW -->
### Scenario: Grouping by a scalar expression is delegated by every dialect

* *GIVEN* a virtual schema for each driver this release supports, over a table holding a `DATE` column across several years and months and a null, and a local Exasol mirror table holding the same rows
* *WHEN* Exasol runs `SELECT YEAR("d"), MONTH("d"), COUNT(*) ... GROUP BY YEAR("d"), MONTH("d")` and `SELECT YEAR("d"), COUNT(*) ... GROUP BY YEAR("d")`
* *THEN* each dialect's capability set MUST advertise `AGGREGATE_GROUP_BY_EXPRESSION`, and the withholding of `AGGREGATE_GROUP_BY_TUPLE` by the MySQL and PostgreSQL dialects MUST stay unchanged
* *AND* `EXPLAIN VIRTUAL` MUST report a generated statement whose `GROUP BY` clause carries `YEAR` for the one-key query in every dialect, and both keys for the two-key query under the Trino dialect
* *AND* the select-list occurrence of a grouped expression MUST render as the same text as its `GROUP BY` key, so MySQL accepts it under `ONLY_FULL_GROUP_BY`
* *AND* each query MUST return exactly the rows the same query returns over the mirror, the null group included
<!-- /DELTA:NEW -->
