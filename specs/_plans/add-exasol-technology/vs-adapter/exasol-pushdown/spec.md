# Feature: exasol_pushdown

States the capability set the Exasol dialect advertises and the Exasol SQL it renders for each delegated fragment when the source is itself Exasol.

## Background

- The Exasol dialect states its own advertised capability set and derives it from no other dialect's. `vs-adapter/pushdown-dialect` owns the all-or-nothing negotiation rule — Exasol delegates a fragment fully once it receives the capability and never re-applies it — and the invariant that every advertised capability has a rendering path in the dialect that advertises it.
- The set this dialect advertises has the same membership as the PostgreSQL dialect's, and the two are stated independently rather than one derived from the other. That the memberships coincide is a consequence of both targets implementing `FULL OUTER JOIN` directly where MySQL implements none, not of one set being copied.
- `JOIN_CONDITION_ALL` stays withheld, and the reason differs from PostgreSQL's. PostgreSQL withholds it because PostgreSQL refuses a full join whose condition is neither merge-joinable nor hash-joinable; Exasol has no such restriction. It is withheld here because this release's renderer covers an equality condition and nothing else, which is `vs-adapter/pushdown-join`'s scope, and withholding costs throughput alone.
- `FN_AGG_SUM`, `FN_AGG_SUM_DISTINCT`, `FN_AGG_AVG`, and `FN_AGG_AVG_DISTINCT` stay withheld, and the withholding is `vs-adapter/pushdown-aggregation`'s for every dialect: a capability is advertised once per session and admits every numeric operand, including `DOUBLE`, whose sum is order-dependent under a parallel plan. An Exasol source's exact-numeric `SUM` would be exact, and that does not narrow a capability advertised over both operand types.
- `AGGREGATE_GROUP_BY_EXPRESSION`, `AGGREGATE_GROUP_BY_TUPLE`, and `AGGREGATE_HAVING` stay withheld for the framework's reasons, which no property of an Exasol source changes.
- `ORDER_BY_COLUMN`, `ORDER_BY_EXPRESSION`, `LIMIT`, and `LIMIT_WITH_OFFSET` stay withheld, and `vs-adapter/result-ordering` owns why: the emitting-script transport carries no ordering guarantee whatever the source is. The public Exasol dialect of the JVM Virtual Schemas advertises `ORDER_BY_COLUMN` and `LIMIT` over its `IMPORT FROM EXA` transport, and its own documentation states that for the `EXA` and `JDBC` connection variants "the outermost order of the imported result rows is not guaranteed" and prescribes an `ORDER BY FALSE` sub-query to suppress the pushdown. Matching that dialect's capability list would reproduce a known defect rather than its behavior, exactly as `vs-adapter/result-ordering` already records for `postgresql-virtual-schema`.
- Identifier quoting is the double quote, and an embedded double quote is escaped by doubling it. Every identifier is quoted unconditionally, because Exasol folds an unquoted identifier to upper case and a source object created under a quoted lower-case or mixed-case name would then resolve to an object that does not exist.
- A string literal renders as `'<body>'` with a single quote emitted as two single quotes and every other byte emitted verbatim. There is no escape-processing form to defeat: measured against Exasol 2025.2.1, `'a\b'` has length 3, so a backslash inside an Exasol literal is a literal backslash. Neither MySQL's hex route nor PostgreSQL's `E'<body>'` form has an analogue here, and one form covers every value rather than a form chosen per value.
- Date and timestamp literals render as the typed literals `DATE '...'` and `TIMESTAMP '...'`, both measured accepted on Exasol 2025.2.1, so the source coerces no string by its own rules. `predicate_notequal` renders as `<>`, and `LIKE` renders an explicit `ESCAPE` clause exactly when the request carries an escape character; both forms were measured on the same version.
- A table is qualified with the source schema `vs-adapter/exasol-type-mapping` records as the namespace, quoted the same way every other identifier is.
- The capability list is data rather than code. `crates/adbc-vs-scan/drivers/exasol.toml` states it beside the driver package row, and the build generates the table the dialect returns, so adding or withdrawing a capability is a data edit.
- Collation placement is owned by `vs-adapter/exasol-collation`, which renders no suffix anywhere. The mapped source types every rendered operand can hold are owned by `vs-adapter/exasol-type-mapping`.

## Scenarios

### Scenario: Capability set advertises the fragments this dialect renders

* *GIVEN* a virtual schema whose driver is `exasol`
* *WHEN* Exasol sends a `getCapabilities` request
* *THEN* the `capabilities` array MUST contain `SELECTLIST_PROJECTION`, `SELECTLIST_EXPRESSIONS`, `FILTER_EXPRESSIONS`, `AGGREGATE_SINGLE_GROUP`, `AGGREGATE_GROUP_BY_COLUMN`, `JOIN`, `JOIN_TYPE_INNER`, `JOIN_TYPE_LEFT_OUTER`, `JOIN_TYPE_RIGHT_OUTER`, `JOIN_TYPE_FULL_OUTER`, and `JOIN_CONDITION_EQUI`
* *AND* the array MUST NOT contain `AGGREGATE_GROUP_BY_EXPRESSION`, `AGGREGATE_GROUP_BY_TUPLE`, `AGGREGATE_HAVING`, or `JOIN_CONDITION_ALL`
* *AND* the array MUST NOT contain `ORDER_BY_COLUMN`, `ORDER_BY_EXPRESSION`, `LIMIT`, or `LIMIT_WITH_OFFSET`, which `vs-adapter/result-ordering` withholds for every dialect
* *AND* every entry MUST be a `MainCapability` constant, or carry the prefix `FN_PRED_`, `FN_AGG_`, or `LITERAL_`

### Scenario: The advertised set matches the PostgreSQL dialect's membership and is stated independently

* *GIVEN* the capability sets the Exasol and PostgreSQL dialects advertise, and the set the MySQL dialect advertises
* *WHEN* the three are compared
* *THEN* the Exasol set MUST have the same membership as the PostgreSQL set
* *AND* it MUST differ from the MySQL set by exactly `JOIN_TYPE_FULL_OUTER`
* *AND* each set MUST be declared in its own technology's TOML file rather than derived from another's at build time or at run time
* *AND* `JOIN_CONDITION_ALL` MUST be absent from the Exasol set, because this release's renderer expresses an equality join condition and no other, rather than because Exasol restricts a full join's condition as PostgreSQL does

### Scenario: Identifiers are quoted unconditionally and an embedded double quote is doubled

* *GIVEN* an Exasol source table or column whose name contains a double quote, and one whose name is entirely lower case
* *WHEN* the Exasol dialect builds the pushdown statement
* *THEN* the emitted identifier MUST double every double quote it contains
* *AND* it MUST remain enclosed in a single pair of double quotes
* *AND* the adapter MUST NOT interpolate the raw name into the statement
* *AND* the lower-case identifier MUST still be emitted quoted, because Exasol folds an unquoted identifier to upper case and the source object would otherwise resolve to a name that does not exist

### Scenario: String literal is rendered by doubling the single quote and nothing else

* *GIVEN* a filter carrying a string literal
* *WHEN* the Exasol dialect renders that literal
* *THEN* the emitted literal MUST be the form `'<body>'` with a single quote emitted as two single quotes
* *AND* a backslash MUST be emitted verbatim rather than doubled, because an Exasol string literal performs no escape processing
* *AND* a literal containing a single quote, a backslash, or a multi-byte character MUST render to that one form rather than to a second form chosen per value, and an empty string literal MUST render as `''`
* *AND* the dialect MUST NOT render a hex-decoding expression or an escape-string prefix, the forms the MySQL and PostgreSQL dialects render, because neither is Exasol syntax

### Scenario: LIKE, IS NULL, BETWEEN, IN, and notequal render in their standard forms

* *GIVEN* a `pushdown` request carrying `predicate_like` with and without an `escapeChar`, `predicate_is_null`, `predicate_is_not_null`, `predicate_between`, `predicate_in_constlist`, and `predicate_notequal`
* *WHEN* the Exasol dialect renders each
* *THEN* `predicate_like` MUST render an explicit `ESCAPE` clause exactly when the request carries an `escapeChar`
* *AND* `predicate_is_null` and `predicate_is_not_null` MUST render as `IS NULL` and `IS NOT NULL` rather than as a comparison against a null literal
* *AND* the `BETWEEN` bounds and the `IN` list entries MUST appear in the order the request gave them
* *AND* `predicate_notequal` MUST render as `<>`
* *AND* a date literal MUST render as `DATE '...'` and a timestamp literal as `TIMESTAMP '...'`, so the source coerces no string by its own rules

### Scenario: Projection pushdown reaches the source Exasol rather than the querying Exasol

* *GIVEN* a virtual schema over an Exasol source table with eight mapped columns
* *WHEN* Exasol runs a `SELECT` naming two of them
* *THEN* `EXPLAIN VIRTUAL` MUST report a generated statement projecting only those two columns
* *AND* every identifier in that statement MUST be enclosed in double quotes, and the table MUST be qualified with the recorded source schema
* *AND* the query MUST return those two columns with the values seeded at the source
* *AND* the generated statement MUST NOT name any other mapped column

### Scenario: Filter pushdown reaches the source Exasol rather than the querying Exasol

* *GIVEN* a virtual schema over an Exasol source table holding three rows
* *WHEN* Exasol runs a `SELECT` whose `WHERE` clause matches one row
* *THEN* `EXPLAIN VIRTUAL` MUST report a generated statement carrying that `WHERE` clause
* *AND* the query MUST return exactly the matching row
* *AND* the generated statement MUST NOT read the unmatched rows

### Scenario: Aggregation pushdown reaches the source Exasol rather than the querying Exasol

* *GIVEN* a virtual schema over an Exasol source table holding rows across two groups
* *WHEN* Exasol runs a `SELECT` grouping by one column and counting another
* *THEN* `EXPLAIN VIRTUAL` MUST report a generated statement carrying the aggregate and the `GROUP BY` clause, and the query MUST return one row per group with the correct count
* *AND* the returned values MUST equal what Exasol returns for the same aggregation executed over a local mirror of the same data, for `COUNT`, `MIN`, and `MAX` over every mapped operand type
* *AND* a query averaging or summing a mapped numeric column MUST produce a generated statement carrying no `AVG` and no `SUM`, and MUST return the value Exasol's own aggregate returns
* *AND* the same query grouping by two columns MUST produce a generated statement carrying no `GROUP BY` clause and MUST still return the correct rows, because Exasol withdraws that aggregation and groups it itself

### Scenario: Delegated full outer join is rendered and executed

* *GIVEN* Exasol source tables `orders` and `customers` sharing a key column, each holding a row the other does not match, and local Exasol mirrors of both
* *WHEN* Exasol pushes down a full outer join over that key
* *THEN* `EXPLAIN VIRTUAL` MUST report a generated statement rendering `FULL OUTER JOIN`, so the join reaches the source rather than the querying Exasol
* *AND* the query MUST return the unmatched row of each side, with nulls for the other side's columns
* *AND* the returned rows MUST equal what Exasol returns for the same full outer join executed locally over the mirrors

### Scenario: The generated statement carries no ordering or limiting clause

* *GIVEN* a virtual schema over an Exasol source and a query carrying an `ORDER BY` and a `LIMIT`
* *WHEN* Exasol plans the query
* *THEN* the generated statement MUST carry no `ORDER BY` clause and no `LIMIT` clause
* *AND* the query MUST still return the rows in the order it asked for, because Exasol applies both itself over the emitted rows
* *AND* the assertion MUST be made against this dialect's own generated text, because that text is rendered per dialect even though the withholding is the framework's
