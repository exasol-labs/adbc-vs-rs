# Feature: mysql_pushdown

States the capability set the MySQL dialect advertises and the MySQL SQL it renders for each fragment Exasol delegates to it.

<!-- DELTA:CHANGED -->
## Background

- The MySQL dialect states its own advertised set, derived from no other dialect's by addition or removal, and `vs-adapter/pushdown-dialect` owns the framework rules the set obeys: negotiation is all-or-nothing at the capability level, and an invariant test renders every advertised capability through the dialect that advertised it.
- The capability list is data: `crates/adbc-vs-scan/drivers/mysql.toml` holds it beside MySQL's driver package row, and the build generates the table `MySqlDialect::capabilities` returns, per `vs-adapter/driver-selection`.
- `vs-adapter/pushdown-join` owns the withheld `JOIN_TYPE_FULL_OUTER`, the one capability MySQL's set and PostgreSQL's differ on, `vs-adapter/pushdown-aggregation-delegation` owns the aggregate, grouping, and `HAVING` capabilities, which leave a MySQL-backed virtual schema grouping by exactly one column, `vs-adapter/result-ordering` owns the withheld ordering capabilities, and `vs-adapter/pushdown-limit` owns the limiting capabilities.
- Identifiers are quoted with backticks, a string literal renders as `_utf8mb4 X'<hex>'` so it reads the same under every `sql_mode`, date and timestamp literals render as typed literals, and `REGEXP_LIKE` is not delegated because Exasol's regular expressions are PCRE and MySQL 8's are ICU.
- `vs-adapter/mysql-collation` states that the MySQL dialect renders no collation suffix and what MySQL's default collation does to a delegated comparison, and `vs-adapter/mysql-type-mapping` states which MySQL column types reach a rendered fragment.
<!-- /DELTA:CHANGED -->

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Capability set advertises the fragments this dialect renders

* *GIVEN* a virtual schema whose driver is `mysql`
* *WHEN* Exasol sends a `getCapabilities` request
* *THEN* the `capabilities` array MUST contain `SELECTLIST_PROJECTION`, `SELECTLIST_EXPRESSIONS`, `FILTER_EXPRESSIONS`, `AGGREGATE_SINGLE_GROUP`, `AGGREGATE_GROUP_BY_COLUMN`, `AGGREGATE_HAVING`, `JOIN`, `JOIN_TYPE_INNER`, `JOIN_TYPE_LEFT_OUTER`, `JOIN_TYPE_RIGHT_OUTER`, `JOIN_CONDITION_EQUI`, `LIMIT`, and `LIMIT_WITH_OFFSET`
* *AND* the array MUST NOT contain `AGGREGATE_GROUP_BY_EXPRESSION`, `AGGREGATE_GROUP_BY_TUPLE`, `JOIN_TYPE_FULL_OUTER`, or `JOIN_CONDITION_ALL`
* *AND* the array MUST NOT contain `ORDER_BY_COLUMN` or `ORDER_BY_EXPRESSION`
* *AND* every entry MUST be a `MainCapability` constant, or carry the prefix `FN_PRED_`, `FN_AGG_`, or `LITERAL_`
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: A limit renders as LIMIT with its offset after the count

* *GIVEN* a `pushdown` request carrying a `limit` with `numElements` 5, and a second one carrying `numElements` 5 and `offset` 2
* *WHEN* the MySQL dialect renders each request
* *THEN* the first statement MUST end with `LIMIT 5`
* *AND* the second statement MUST end with `LIMIT 5 OFFSET 2`, a form MySQL's `SELECT` syntax accepts
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: LIMIT pushdown reaches MySQL rather than Exasol

* *GIVEN* a virtual schema over a MySQL table holding more rows than a `LIMIT` names
* *WHEN* Exasol runs a `SELECT` with that `LIMIT` and no `ORDER BY`
* *THEN* `EXPLAIN VIRTUAL` MUST report a generated statement that ends with that `LIMIT`
* *AND* the query MUST return exactly as many rows as the `LIMIT` names
* *AND* every returned row MUST be a row of the MySQL table, as the local Exasol mirror holds it
<!-- /DELTA:NEW -->
