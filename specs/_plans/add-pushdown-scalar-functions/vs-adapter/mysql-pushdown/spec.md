# Feature: mysql_pushdown

States the capability set the MySQL dialect advertises and the MySQL SQL it renders for each fragment Exasol delegates to it.

## Background

- The MySQL dialect states its own advertised set, derived from no other dialect's by addition or removal, and `vs-adapter/pushdown-dialect` owns the framework rules the set obeys: negotiation is all-or-nothing at the capability level, and an invariant test renders every advertised capability through the dialect that advertised it.
- The capability list is data: `crates/adbc-vs-scan/drivers/mysql.toml` holds it beside MySQL's driver package row, and the build generates the table `MySqlDialect::capabilities` returns, per `vs-adapter/driver-selection`.
- `vs-adapter/pushdown-join` owns the withheld `JOIN_TYPE_FULL_OUTER`, the one capability MySQL's set and PostgreSQL's differ on, `vs-adapter/pushdown-aggregation-delegation` owns the aggregate, grouping, and `HAVING` capabilities, which leave a MySQL-backed virtual schema grouping by exactly one column, and `vs-adapter/result-ordering` owns the ordering and limiting capabilities.
- Identifiers are quoted with backticks, a string literal renders as `_utf8mb4 X'<hex>'` so it reads the same under every `sql_mode`, date and timestamp literals render as typed literals, and `REGEXP_LIKE` is not delegated because Exasol's regular expressions are PCRE and MySQL 8's are ICU.
- `vs-adapter/mysql-collation` states that the MySQL dialect renders no collation suffix and what MySQL's default collation does to a delegated comparison, and `vs-adapter/mysql-type-mapping` states which MySQL column types reach a rendered fragment.

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Capability set advertises the fragments this dialect renders

* *GIVEN* a virtual schema whose driver is `mysql`
* *WHEN* Exasol sends a `getCapabilities` request
* *THEN* the `capabilities` array MUST contain `SELECTLIST_PROJECTION`, `SELECTLIST_EXPRESSIONS`, `FILTER_EXPRESSIONS`, `AGGREGATE_SINGLE_GROUP`, `AGGREGATE_GROUP_BY_COLUMN`, `AGGREGATE_GROUP_BY_EXPRESSION`, `AGGREGATE_HAVING`, `JOIN`, `JOIN_TYPE_INNER`, `JOIN_TYPE_LEFT_OUTER`, `JOIN_TYPE_RIGHT_OUTER`, and `JOIN_CONDITION_EQUI`
* *AND* the array MUST NOT contain `AGGREGATE_GROUP_BY_TUPLE`, `JOIN_TYPE_FULL_OUTER`, or `JOIN_CONDITION_ALL`
* *AND* the array MUST NOT contain `ORDER_BY_COLUMN`, `ORDER_BY_EXPRESSION`, `LIMIT`, or `LIMIT_WITH_OFFSET`
* *AND* every entry MUST be a `MainCapability` constant, or carry the prefix `FN_PRED_`, `FN_AGG_`, `LITERAL_`, or `FN_`, the last for exactly the scalar functions `vs-adapter/pushdown-scalar-function` names for MySQL, which leave out `FN_TRIM`
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Aggregation capabilities cover single-group and column grouping

* *GIVEN* a virtual schema whose driver is `mysql`
* *WHEN* Exasol sends a `getCapabilities` request
* *THEN* the `capabilities` array MUST contain `AGGREGATE_SINGLE_GROUP`, `AGGREGATE_GROUP_BY_COLUMN`, `AGGREGATE_GROUP_BY_EXPRESSION`, and `SELECTLIST_EXPRESSIONS`, without which Exasol delegates no aggregate at all, and `AGGREGATE_HAVING`
* *AND* it MUST contain `FN_AGG_COUNT`, `FN_AGG_COUNT_STAR`, `FN_AGG_COUNT_DISTINCT`, `FN_AGG_MIN`, `FN_AGG_MAX`, `FN_AGG_SUM`, `FN_AGG_SUM_DISTINCT`, `FN_AGG_AVG`, and `FN_AGG_AVG_DISTINCT`, which are the aggregates this release delegates
* *AND* it MUST NOT contain `AGGREGATE_GROUP_BY_TUPLE`
* *AND* it MUST NOT contain any other `FN_AGG_` capability, because the dialect renders no other aggregate function
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: String functions render with MySQL's character semantics

* *GIVEN* `pushdown` requests carrying `UPPER`, `LOWER`, `LENGTH`, and `SUBSTR` over operands of each declared type Exasol admits for them
* *WHEN* the MySQL dialect renders each request
* *THEN* `LENGTH` MUST render as `CHAR_LENGTH`, because MySQL's `LENGTH` counts bytes
* *AND* every rendered string literal MUST keep the `_utf8mb4 X'<hex>'` form
* *AND* a `function_scalar` node named `TRIM` MUST fail with an error naming `FN_TRIM` as a capability this dialect does not advertise
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: Arithmetic renders with MySQL's day and decimal semantics

* *GIVEN* `pushdown` requests carrying `ADD`, `SUB`, and `MULT` over operands of each declared type Exasol admits for them
* *WHEN* the MySQL dialect renders each request
* *THEN* a number added to or subtracted from a `DATE` or `TIMESTAMP` operand MUST render through `DATE_ADD` or `DATE_SUB` with a `MICROSECOND` interval of the number times 86400000000, so a fractional day keeps its fraction where a `DAY` interval rounds it, and `DATE` minus `DATE` MUST render through `DATEDIFF`, because MySQL's `+` reads a date as the number `YYYYMMDD`
* *AND* a `TIMESTAMP` difference MUST render through `TIMESTAMPDIFF(MICROSECOND, ...)`, and a timestamp plus or minus such a difference through a `MICROSECOND` interval of it
* *AND* an exact-numeric operand of `+`, `-`, or `*` MUST render cast to `DECIMAL(65,s)`, so a `BIGINT` result beyond its range is returned rather than refused with MySQL's out-of-range error
<!-- /DELTA:NEW -->
