# Feature: mysql_pushdown

States the capability set the MySQL dialect advertises and the MySQL SQL it renders for each fragment Exasol delegates to it.

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

## Scenarios

### Scenario: Capability set advertises the fragments this dialect renders

* *GIVEN* a virtual schema whose driver is `mysql`
* *WHEN* Exasol sends a `getCapabilities` request
* *THEN* the `capabilities` array MUST contain `SELECTLIST_PROJECTION`, `SELECTLIST_EXPRESSIONS`, `FILTER_EXPRESSIONS`, `AGGREGATE_SINGLE_GROUP`, `AGGREGATE_GROUP_BY_COLUMN`, `JOIN`, `JOIN_TYPE_INNER`, `JOIN_TYPE_LEFT_OUTER`, `JOIN_TYPE_RIGHT_OUTER`, and `JOIN_CONDITION_EQUI`
* *AND* the array MUST NOT contain `AGGREGATE_GROUP_BY_EXPRESSION`, `AGGREGATE_GROUP_BY_TUPLE`, `AGGREGATE_HAVING`, `JOIN_TYPE_FULL_OUTER`, or `JOIN_CONDITION_ALL`
* *AND* the array MUST NOT contain `ORDER_BY_COLUMN`, `ORDER_BY_EXPRESSION`, `LIMIT`, or `LIMIT_WITH_OFFSET`
* *AND* every entry MUST be a `MainCapability` constant, or carry the prefix `FN_PRED_`, `FN_AGG_`, or `LITERAL_`

### Scenario: Filter capabilities cover the predicates and literals this release renders

* *GIVEN* a virtual schema whose driver is `mysql`
* *WHEN* Exasol sends a `getCapabilities` request
* *THEN* the `capabilities` array MUST contain `FILTER_EXPRESSIONS`, and exactly the predicate capabilities `FN_PRED_AND`, `FN_PRED_OR`, `FN_PRED_NOT`, `FN_PRED_EQUAL`, `FN_PRED_NOTEQUAL`, `FN_PRED_LESS`, `FN_PRED_LESSEQUAL`, `FN_PRED_IS_NULL`, `FN_PRED_IS_NOT_NULL`, `FN_PRED_LIKE`, `FN_PRED_LIKE_ESCAPE`, `FN_PRED_BETWEEN`, and `FN_PRED_IN_CONSTLIST`
* *AND* it MUST contain exactly the literal capabilities `LITERAL_NULL`, `LITERAL_EXACTNUMERIC`, `LITERAL_DOUBLE`, `LITERAL_STRING`, `LITERAL_DATE`, and `LITERAL_TIMESTAMP`
* *AND* it MUST NOT contain `FN_PRED_REGEXP_LIKE`, `FN_PRED_IS_JSON`, `FN_PRED_IS_NOT_JSON`, `LITERAL_BOOL`, `LITERAL_INTERVAL`, or `LITERAL_TIMESTAMP_UTC`

### Scenario: Aggregation capabilities cover single-group and column grouping

* *GIVEN* a virtual schema whose driver is `mysql`
* *WHEN* Exasol sends a `getCapabilities` request
* *THEN* the `capabilities` array MUST contain `AGGREGATE_SINGLE_GROUP`, `AGGREGATE_GROUP_BY_COLUMN`, and `SELECTLIST_EXPRESSIONS`, without which Exasol delegates no aggregate at all
* *AND* it MUST contain `FN_AGG_COUNT`, `FN_AGG_COUNT_STAR`, `FN_AGG_COUNT_DISTINCT`, `FN_AGG_MIN`, and `FN_AGG_MAX`, which are the aggregates this release delegates
* *AND* it MUST NOT contain `AGGREGATE_GROUP_BY_EXPRESSION`, `AGGREGATE_GROUP_BY_TUPLE`, or `AGGREGATE_HAVING`
* *AND* it MUST NOT contain `FN_AGG_AVG`, `FN_AGG_AVG_DISTINCT`, `FN_AGG_SUM`, `FN_AGG_SUM_DISTINCT`, or any other `FN_AGG_` capability the dialect does not render: no rendering of `AVG` reproduces the value Exasol's own `AVG` returns, and no rendering of `SUM` over a `DOUBLE` column does either. The two `SUM` capabilities MUST be withheld whole rather than advertised behind a render-time operand-type check, because a capability is advertised once per session and Exasol does not compute a delegated fragment itself after the adapter errors

### Scenario: Join capabilities cover inner and one-sided outer joins only

* *GIVEN* a virtual schema whose driver is `mysql`
* *WHEN* Exasol sends a `getCapabilities` request
* *THEN* the `capabilities` array MUST contain `JOIN`, `JOIN_TYPE_INNER`, `JOIN_TYPE_LEFT_OUTER`, `JOIN_TYPE_RIGHT_OUTER`, and `JOIN_CONDITION_EQUI`
* *AND* it MUST NOT contain `JOIN_TYPE_FULL_OUTER`, because MySQL implements no `FULL OUTER JOIN`
* *AND* it MUST NOT contain `JOIN_CONDITION_ALL`, because a non-equi join is out of scope

### Scenario: Backtick in an identifier is escaped

* *GIVEN* a MySQL table or column whose name contains a backtick
* *WHEN* the MySQL dialect builds the pushdown statement
* *THEN* the emitted identifier MUST double every backtick it contains
* *AND* the emitted identifier MUST remain enclosed in a single pair of backticks
* *AND* the adapter MUST NOT interpolate the raw name into the statement

### Scenario: String literal is rendered as a hex literal

* *GIVEN* a filter carrying a string literal, and a `predicate_like` carrying a pattern
* *WHEN* the MySQL dialect renders each
* *THEN* the emitted literal MUST be the form `_utf8mb4 X'<hex>'` over the UTF-8 bytes of the value
* *AND* a literal containing a single quote, a backslash, or a multi-byte character MUST render to hex without an escape sequence
* *AND* the emitted statement MUST NOT contain the literal's raw characters
* *AND* an empty string literal MUST render as `_utf8mb4 X''`
* *AND* the `LIKE` pattern MUST be rendered in that same hex form, and so MUST the `escapeChar` a request carrying one supplies, because a pattern is a string literal and no rendering position exempts one

### Scenario: Temporal literals are rendered as typed literals

* *GIVEN* a filter comparing a mapped `DATE` column and a mapped `TIMESTAMP` column to literals
* *WHEN* the MySQL dialect renders those literals
* *THEN* the date literal MUST be rendered as `DATE '<value>'`
* *AND* the timestamp literal MUST be rendered as `TIMESTAMP '<value>'`
* *AND* neither MUST be rendered as a bare string, because MySQL would then apply its own coercion rules

### Scenario: Projection pushdown reaches MySQL rather than Exasol

* *GIVEN* a virtual schema over a MySQL table with eight mapped columns
* *WHEN* Exasol runs a `SELECT` naming two of them
* *THEN* `EXPLAIN VIRTUAL` MUST report a generated statement projecting only those two columns
* *AND* the query MUST return those two columns with the values seeded in MySQL
* *AND* the generated statement MUST NOT name any other mapped column

### Scenario: Filter pushdown reaches MySQL rather than Exasol

* *GIVEN* a virtual schema over a MySQL table holding three rows
* *WHEN* Exasol runs a `SELECT` whose `WHERE` clause matches one row
* *THEN* `EXPLAIN VIRTUAL` MUST report a generated statement carrying that `WHERE` clause
* *AND* the query MUST return exactly the matching row
* *AND* the generated statement MUST NOT read the unmatched rows

### Scenario: Aggregation pushdown reaches MySQL rather than Exasol

* *GIVEN* a virtual schema over a MySQL table holding rows across two groups
* *WHEN* Exasol runs a `SELECT` grouping by one column and counting another
* *THEN* `EXPLAIN VIRTUAL` MUST report a generated statement carrying the aggregate and the `GROUP BY` clause
* *AND* the query MUST return one row per group with the correct count
* *AND* the returned values MUST equal what Exasol returns for the same aggregation executed locally over the same data, unqualified: `COUNT`, `MIN`, and `MAX` are the aggregates this release delegates and each is exact over every operand type, `DOUBLE` included, because none of them sums. No aggregate whose value depends on summation order is delegated at all
* *AND* the same query grouping by two columns MUST produce a generated statement carrying no `GROUP BY` clause and MUST still return the correct rows, because Exasol withdraws that aggregation and groups it itself
