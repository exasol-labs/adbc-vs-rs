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
* *THEN* the `capabilities` array MUST contain `SELECTLIST_PROJECTION`, `SELECTLIST_EXPRESSIONS`, `FILTER_EXPRESSIONS`, `AGGREGATE_SINGLE_GROUP`, `AGGREGATE_GROUP_BY_COLUMN`, `AGGREGATE_GROUP_BY_EXPRESSION`, `AGGREGATE_HAVING`, `JOIN`, `JOIN_TYPE_INNER`, `JOIN_TYPE_LEFT_OUTER`, `JOIN_TYPE_RIGHT_OUTER`, `JOIN_TYPE_FULL_OUTER`, and `JOIN_CONDITION_EQUI`
* *AND* the array MUST NOT contain `AGGREGATE_GROUP_BY_TUPLE` or `JOIN_CONDITION_ALL`
* *AND* the array MUST NOT contain `ORDER_BY_COLUMN`, `ORDER_BY_EXPRESSION`, `LIMIT`, or `LIMIT_WITH_OFFSET`, which `vs-adapter/result-ordering` withholds for every dialect because the emitting-UDF transport carries no ordering guarantee whatever the source is
* *AND* every entry MUST be a `MainCapability` constant, or carry the prefix `FN_PRED_`, `FN_AGG_`, `LITERAL_`, or `FN_`, the last for exactly the scalar functions `vs-adapter/pushdown-scalar-function` names for PostgreSQL
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: Scalar functions render in PostgreSQL's forms

* *GIVEN* `pushdown` requests carrying each scalar function `vs-adapter/pushdown-scalar-function` names, over operands of each declared type Exasol admits for it
* *WHEN* the PostgreSQL dialect renders each request
* *THEN* `YEAR`, `MONTH`, and `DAY` MUST render through `EXTRACT`, and `LENGTH` as `char_length`
* *AND* a two-argument `TRIM` MUST render as `btrim`, which removes a set of characters as Exasol does
* *AND* an exact-numeric operand of `+`, `-`, or `*` MUST render cast to `numeric`, so a `bigint` result beyond its range is returned rather than refused
* *AND* a number added to or subtracted from a `DATE` or `TIMESTAMP` operand MUST render as an addition of that many whole days, the number rounded half away from zero as Exasol rounds it, a `DATE` result MUST yield a `date`, and a `TIMESTAMP` difference MUST render as PostgreSQL's `interval` subtraction truncated to milliseconds, and as `EXTRACT(EPOCH FROM <interval>)` where a select-list entry declared `INTERVAL DAY TO SECOND` needs its seconds count
<!-- /DELTA:NEW -->
