# Feature: postgresql_pushdown

States the capability set the PostgreSQL dialect advertises and the PostgreSQL SQL it renders for each delegated fragment.

## Background

- The PostgreSQL dialect states its own advertised set, derived from no other dialect's, and `vs-adapter/pushdown-dialect` owns the all-or-nothing negotiation rule and the invariant that every advertised capability renders in the dialect that advertises it.
- The capability list is data: `crates/adbc-vs-scan/drivers/postgresql.toml` holds it beside the driver package row, and the build generates the table the dialect returns.
- The set holds `JOIN_TYPE_FULL_OUTER`, the one capability the MySQL dialect withholds, which PostgreSQL executes for the equality condition that is the only join condition advertised; the Trino set holds every entry of this one except `FN_PRED_LIKE_ESCAPE`, plus `AGGREGATE_GROUP_BY_TUPLE`; and `vs-adapter/pushdown-aggregation-delegation` owns the aggregate, grouping, and `HAVING` capabilities.
- Identifiers are double-quoted with an embedded double quote doubled, a string literal renders as the escape-string constant `E'<body>'`, date and timestamp literals render as typed literals, `predicate_notequal` renders as `<>`, and every nested predicate is parenthesized.
- `vs-adapter/postgresql-collation` states that the PostgreSQL dialect renders no collation suffix and what PostgreSQL's column collation does to a delegated comparison, and `vs-adapter/postgresql-type-mapping` owns the mapped source types every rendered operand can hold.

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Capability set advertises the fragments this dialect renders

* *GIVEN* a virtual schema whose driver is `postgresql`
* *WHEN* Exasol sends a `getCapabilities` request
* *THEN* the `capabilities` array MUST contain `SELECTLIST_PROJECTION`, `SELECTLIST_EXPRESSIONS`, `FILTER_EXPRESSIONS`, `AGGREGATE_SINGLE_GROUP`, `AGGREGATE_GROUP_BY_COLUMN`, `AGGREGATE_HAVING`, `JOIN`, `JOIN_TYPE_INNER`, `JOIN_TYPE_LEFT_OUTER`, `JOIN_TYPE_RIGHT_OUTER`, `JOIN_TYPE_FULL_OUTER`, `JOIN_CONDITION_EQUI`, `LIMIT`, and `LIMIT_WITH_OFFSET`
* *AND* the array MUST NOT contain `AGGREGATE_GROUP_BY_EXPRESSION`, `AGGREGATE_GROUP_BY_TUPLE`, or `JOIN_CONDITION_ALL`
* *AND* the array MUST NOT contain `ORDER_BY_COLUMN` or `ORDER_BY_EXPRESSION`, which `vs-adapter/result-ordering` withholds for every dialect because the emitting-UDF transport carries no ordering guarantee whatever the source is
* *AND* every entry MUST be a `MainCapability` constant, or carry the prefix `FN_PRED_`, `FN_AGG_`, or `LITERAL_`
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: A limit renders as LIMIT with its offset after the count

* *GIVEN* a `pushdown` request carrying a `limit` with `numElements` 5, and a second one carrying `numElements` 5 and `offset` 2
* *WHEN* the PostgreSQL dialect renders each request
* *THEN* the first statement MUST end with `LIMIT 5`
* *AND* the second statement MUST end with `LIMIT 5 OFFSET 2`, a form PostgreSQL's `SELECT` syntax accepts
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: LIMIT pushdown reaches PostgreSQL rather than Exasol

* *GIVEN* a virtual schema over a PostgreSQL table holding more rows than a `LIMIT` names
* *WHEN* Exasol runs a `SELECT` with that `LIMIT` and no `ORDER BY`
* *THEN* `EXPLAIN VIRTUAL` MUST report a generated statement that ends with that `LIMIT`
* *AND* the query MUST return exactly as many rows as the `LIMIT` names
* *AND* every returned row MUST be a row of the PostgreSQL table, as the local Exasol mirror holds it
<!-- /DELTA:NEW -->
