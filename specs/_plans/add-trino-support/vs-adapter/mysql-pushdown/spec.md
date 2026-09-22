# Feature: mysql_pushdown

States the capability set the MySQL dialect advertises and the MySQL SQL it renders for each fragment Exasol delegates to it.

<!-- DELTA:CHANGED -->
## Background

- The MySQL dialect states its own advertised set, and that set derives from no other dialect's by addition or removal. `vs-adapter/pushdown-dialect` owns the framework rules the set obeys — negotiation is all-or-nothing at the capability level, and an invariant test renders every advertised capability through the dialect that advertised it — and this feature states MySQL's membership.
- The capability list itself is data rather than code. `crates/adbc-vs-scan/drivers/mysql.toml` holds it beside MySQL's driver package row, and the build generates the table `MySqlDialect::capabilities` returns, per `vs-adapter/driver-selection`. Changing MySQL's advertised set is therefore a data edit; the invariant test is what still binds the set to what the renderer can express.
- `JOIN_TYPE_FULL_OUTER` is withheld because MySQL implements no `FULL OUTER JOIN`, and emulating one requires rewriting it as a union of two outer joins, which mission.md places out of scope. `vs-adapter/pushdown-join` owns that decision. It is the one capability MySQL's set and PostgreSQL's differ on.
- `FN_AGG_SUM`, `FN_AGG_SUM_DISTINCT`, `FN_AGG_AVG`, and `FN_AGG_AVG_DISTINCT` are withheld under the framework's all-or-nothing rule rather than advertised behind a render-time operand-type check. MySQL's `SUM` over a `DOUBLE` operand accumulates in physical row order and no rendering of `AVG` reproduces Exasol's value, while a capability is advertised once per session and admits every numeric operand. `vs-adapter/pushdown-aggregation` owns both withholdings and their measurements.
- `AGGREGATE_GROUP_BY_EXPRESSION`, `AGGREGATE_GROUP_BY_TUPLE`, and `AGGREGATE_HAVING` are withheld from this dialect, so a MySQL-backed virtual schema groups by exactly one column and Exasol applies every `HAVING` clause itself. That one-column cap is this dialect's rather than the release's: the Trino dialect advertises `AGGREGATE_GROUP_BY_TUPLE` and is sent a `groupBy` naming more than one column. `vs-adapter/pushdown-aggregation` owns that exception, and decision `[16]` of the `add-trino-support` plan records why it is Trino's alone — the capability costs no dialect a rendering change, so lifting it for one dialect buys no renderer work, while widening this dialect's released set is a scope change nothing about adding a third driver motivates. `vs-adapter/result-ordering` withholds every ordering and limiting capability, for every dialect.
- MySQL identifiers are quoted with backticks, and a backtick inside an identifier is escaped by doubling it. Every identifier is quoted unconditionally rather than only where a raw name would be ambiguous.
- A string literal is rendered as `_utf8mb4 X'<hex>'`, the hex encoding of its UTF-8 bytes. Backslash-escaped quoting is not mode-independent: with `NO_BACKSLASH_ESCAPES` set on the source, `'a\\b'` is four characters rather than three, while `_utf8mb4 X'615C62'` is three characters in both modes. Hex rendering also removes the escaping decision entirely, so no literal can change the shape of the generated statement. A `LIKE` pattern is a string literal like any other and renders in the same form; no rendering position exempts one.
- Date and timestamp literals are rendered as MySQL typed literals, `DATE '...'` and `TIMESTAMP '...'`, so MySQL does not coerce a string by its own rules.
- `REGEXP_LIKE` is not delegated: Exasol's regular-expression dialect is PCRE and MySQL 8's is ICU, so an identical pattern can match different rows.
- Where the MySQL dialect renders a collation suffix, and which of its two collations it renders at each site, is owned by `vs-adapter/mysql-collation`. Which MySQL column types reach a rendered fragment at all is owned by `vs-adapter/mysql-type-mapping`.
<!-- /DELTA:CHANGED -->

## Scenarios

### Scenario: Aggregation capabilities cover single-group and column grouping

* *GIVEN* a virtual schema whose driver is `mysql`
* *WHEN* Exasol sends a `getCapabilities` request
* *THEN* the `capabilities` array MUST contain `AGGREGATE_SINGLE_GROUP`, `AGGREGATE_GROUP_BY_COLUMN`, and `SELECTLIST_EXPRESSIONS`, without which Exasol delegates no aggregate at all
* *AND* it MUST contain `FN_AGG_COUNT`, `FN_AGG_COUNT_STAR`, `FN_AGG_COUNT_DISTINCT`, `FN_AGG_MIN`, and `FN_AGG_MAX`, which are the aggregates this release delegates
* *AND* it MUST NOT contain `AGGREGATE_GROUP_BY_EXPRESSION`, `AGGREGATE_GROUP_BY_TUPLE`, or `AGGREGATE_HAVING`
* *AND* it MUST NOT contain `FN_AGG_AVG`, `FN_AGG_AVG_DISTINCT`, `FN_AGG_SUM`, `FN_AGG_SUM_DISTINCT`, or any other `FN_AGG_` capability the dialect does not render: no rendering of `AVG` reproduces the value Exasol's own `AVG` returns, and no rendering of `SUM` over a `DOUBLE` column does either. The two `SUM` capabilities MUST be withheld whole rather than advertised behind a render-time operand-type check, because a capability is advertised once per session and Exasol does not compute a delegated fragment itself after the adapter errors
