# Feature: mysql_pushdown

States the capability set the MySQL dialect advertises and the MySQL SQL it renders for each fragment Exasol delegates to it.

## Background

- The MySQL dialect states its own advertised set, derived from no other dialect's by addition or removal, and `vs-adapter/pushdown-dialect` owns the framework rules the set obeys: negotiation is all-or-nothing at the capability level, and an invariant test renders every advertised capability through the dialect that advertised it.
- The capability list is data: `crates/adbc-vs-scan/drivers/mysql.toml` holds it beside MySQL's driver package row, and the build generates the table `MySqlDialect::capabilities` returns, per `vs-adapter/driver-selection`.
- `vs-adapter/pushdown-join` owns the withheld `JOIN_TYPE_FULL_OUTER`, the one capability MySQL's set and PostgreSQL's differ on, `vs-adapter/pushdown-aggregation-delegation` owns the aggregate, grouping, and `HAVING` capabilities, which leave a MySQL-backed virtual schema grouping by exactly one column, and `vs-adapter/result-ordering` owns the ordering and limiting capabilities.
- Identifiers are quoted with backticks, a string literal renders as `_utf8mb4 X'<hex>'` so it reads the same under every `sql_mode`, date and timestamp literals render as typed literals, and `REGEXP_LIKE` is not delegated because Exasol's regular expressions are PCRE and MySQL 8's are ICU.
- `vs-adapter/mysql-collation` states that the MySQL dialect renders no collation suffix and what MySQL's default collation does to a delegated comparison, and `vs-adapter/mysql-type-mapping` states which MySQL column types reach a rendered fragment.

## Scenarios

### Scenario: Capability set advertises the fragments this dialect renders

* *GIVEN* a virtual schema whose driver is `mysql`
* *WHEN* Exasol sends a `getCapabilities` request
* *THEN* the `capabilities` array MUST contain `SELECTLIST_PROJECTION`, `SELECTLIST_EXPRESSIONS`, `FILTER_EXPRESSIONS`, `AGGREGATE_SINGLE_GROUP`, `AGGREGATE_GROUP_BY_COLUMN`, `AGGREGATE_HAVING`, `JOIN`, `JOIN_TYPE_INNER`, `JOIN_TYPE_LEFT_OUTER`, `JOIN_TYPE_RIGHT_OUTER`, and `JOIN_CONDITION_EQUI`
* *AND* the array MUST NOT contain `AGGREGATE_GROUP_BY_EXPRESSION`, `AGGREGATE_GROUP_BY_TUPLE`, `JOIN_TYPE_FULL_OUTER`, or `JOIN_CONDITION_ALL`
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
* *THEN* the `capabilities` array MUST contain `AGGREGATE_SINGLE_GROUP`, `AGGREGATE_GROUP_BY_COLUMN`, and `SELECTLIST_EXPRESSIONS`, without which Exasol delegates no aggregate at all, and `AGGREGATE_HAVING`
* *AND* it MUST contain `FN_AGG_COUNT`, `FN_AGG_COUNT_STAR`, `FN_AGG_COUNT_DISTINCT`, `FN_AGG_MIN`, `FN_AGG_MAX`, `FN_AGG_SUM`, `FN_AGG_SUM_DISTINCT`, `FN_AGG_AVG`, and `FN_AGG_AVG_DISTINCT`, which are the aggregates this release delegates
* *AND* it MUST NOT contain `AGGREGATE_GROUP_BY_EXPRESSION` or `AGGREGATE_GROUP_BY_TUPLE`
* *AND* it MUST NOT contain any other `FN_AGG_` capability, because the dialect renders no other aggregate function

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
* *AND* the returned values MUST equal what Exasol returns for the same aggregation executed locally over the same data, when the grouping key and every aggregate operand is either a non-character column or a character column declaring `utf8mb4_0900_bin`: for `COUNT`, `MIN`, and `MAX` over every non-character operand type, `DOUBLE` included, and for `SUM` and `AVG` over every exact-numeric operand and over a `DOUBLE` or `FLOAT` operand whose partial sums are exact in double precision, because MySQL sums a `FLOAT` operand in double precision
* *AND* the same query grouping by two columns MUST produce a generated statement carrying no `GROUP BY` clause and MUST still return the correct rows, because Exasol withdraws that aggregation and groups it itself

### Scenario: A HAVING clause reads a grouped converted column the way the select list does

* *GIVEN* a constructed `pushdown` request grouping by a MySQL column that has a read expression, such as a `TIME` column, and carrying a `having` predicate that reads that column
* *WHEN* the MySQL dialect renders the statement
* *THEN* the `GROUP BY` clause MUST name the column's read expression
* *AND* the `HAVING` clause MUST read the column inside `ANY_VALUE(<read expression>)`, as a select-list use of the grouped column does, because MySQL evaluates `HAVING` over grouped rows and rejects an expression over a grouped column that does not match a `GROUP BY` expression as a whole
* *AND* a grouped column without a read expression MUST render bare in the `HAVING` clause, as it does in the select list
