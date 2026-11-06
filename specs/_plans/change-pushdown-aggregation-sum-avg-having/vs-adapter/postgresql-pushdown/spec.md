# Feature: postgresql_pushdown

States the capability set the PostgreSQL dialect advertises and the PostgreSQL SQL it renders for each delegated fragment.

<!-- DELTA:CHANGED -->
## Background

- The PostgreSQL dialect states its own advertised set, derived from no other dialect's, and `vs-adapter/pushdown-dialect` owns the all-or-nothing negotiation rule and the invariant that every advertised capability renders in the dialect that advertises it.
- The capability list is data: `crates/adbc-vs-scan/drivers/postgresql.toml` holds it beside the driver package row, and the build generates the table the dialect returns.
- The set holds `JOIN_TYPE_FULL_OUTER`, the one capability the MySQL dialect withholds, which PostgreSQL executes for the equality condition that is the only join condition advertised; the Trino set holds every entry of this one except `FN_PRED_LIKE_ESCAPE`, plus `AGGREGATE_GROUP_BY_TUPLE`; and `vs-adapter/pushdown-aggregation-delegation` owns the aggregate, grouping, and `HAVING` capabilities.
- Identifiers are double-quoted with an embedded double quote doubled, a string literal renders as the escape-string constant `E'<body>'`, date and timestamp literals render as typed literals, `predicate_notequal` renders as `<>`, and every nested predicate is parenthesized.
- `vs-adapter/postgresql-collation` states that the PostgreSQL dialect renders no collation suffix and what PostgreSQL's column collation does to a delegated comparison, and `vs-adapter/postgresql-type-mapping` owns the mapped source types every rendered operand can hold.
<!-- /DELTA:CHANGED -->

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Capability set advertises the fragments this dialect renders

* *GIVEN* a virtual schema whose driver is `postgresql`
* *WHEN* Exasol sends a `getCapabilities` request
* *THEN* the `capabilities` array MUST contain `SELECTLIST_PROJECTION`, `SELECTLIST_EXPRESSIONS`, `FILTER_EXPRESSIONS`, `AGGREGATE_SINGLE_GROUP`, `AGGREGATE_GROUP_BY_COLUMN`, `AGGREGATE_HAVING`, `JOIN`, `JOIN_TYPE_INNER`, `JOIN_TYPE_LEFT_OUTER`, `JOIN_TYPE_RIGHT_OUTER`, `JOIN_TYPE_FULL_OUTER`, and `JOIN_CONDITION_EQUI`
* *AND* the array MUST NOT contain `AGGREGATE_GROUP_BY_EXPRESSION`, `AGGREGATE_GROUP_BY_TUPLE`, or `JOIN_CONDITION_ALL`
* *AND* the array MUST NOT contain `ORDER_BY_COLUMN`, `ORDER_BY_EXPRESSION`, `LIMIT`, or `LIMIT_WITH_OFFSET`, which `vs-adapter/result-ordering` withholds for every dialect because the emitting-UDF transport carries no ordering guarantee whatever the source is
* *AND* every entry MUST be a `MainCapability` constant, or carry the prefix `FN_PRED_`, `FN_AGG_`, or `LITERAL_`
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Aggregation pushdown reaches PostgreSQL rather than Exasol

* *GIVEN* a virtual schema over a PostgreSQL table holding rows across two groups
* *WHEN* Exasol runs a `SELECT` grouping by one column and counting another
* *THEN* `EXPLAIN VIRTUAL` MUST report a generated statement carrying the aggregate and the `GROUP BY` clause, and the query MUST return one row per group with the correct count
* *AND* the returned values MUST equal what Exasol returns for the same aggregation executed locally over the same data, for `COUNT`, `MIN`, and `MAX` over every mapped non-character operand type and over a character column declaring `COLLATE "C"`, grouped by such a column
* *AND* a query summing or averaging a mapped exact-numeric column MUST produce a generated statement carrying `SUM`, and MUST return the value Exasol's own aggregate returns over the same data
* *AND* the same query grouping by two columns MUST produce a generated statement carrying no `GROUP BY` clause and MUST still return the correct rows, because Exasol withdraws that aggregation and groups it itself
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: SUM over a real column accumulates in double precision

* *GIVEN* a PostgreSQL `real` column declared `DOUBLE`, holding `16777216` and `1`, and a local Exasol mirror table holding the same two values as `DOUBLE`
* *WHEN* Exasol runs `SUM` and `AVG` over the column through the virtual schema
* *THEN* the generated statement MUST render the operand inside `SUM` cast to `DOUBLE PRECISION`, as it does for every `SUM` operand declared `DOUBLE`, because PostgreSQL's `sum` over `real` returns `real` and adds in single precision, where `16777216 + 1` rounds back to `16777216`
* *AND* the query MUST return `16777217` for `SUM` and `8388608.5` for `AVG`, as the mirror does
* *AND* a `SUM` operand declared `DECIMAL` MUST render without a cast, because PostgreSQL's `sum` over an integer or `numeric` operand is already exact
<!-- /DELTA:NEW -->
