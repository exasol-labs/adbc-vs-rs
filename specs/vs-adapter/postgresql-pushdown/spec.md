# Feature: postgresql_pushdown

States the capability set the PostgreSQL dialect advertises and the PostgreSQL SQL it renders for each delegated fragment.

## Background

- The PostgreSQL dialect states its own advertised capability set and derives it from no other dialect's. `vs-adapter/pushdown-dialect` owns the all-or-nothing negotiation rule — Exasol delegates a fragment fully once it receives the capability and never re-applies it — and the invariant that every advertised capability has a rendering path in the dialect that advertises it.
- The set holds one capability the MySQL dialect withholds, `JOIN_TYPE_FULL_OUTER`, and that is the one capability those two targets genuinely disagree on. MySQL implements no `FULL OUTER JOIN` and emulating one requires the union rewrite mission.md places out of scope; PostgreSQL implements the keyword directly. Every other capability is advertised or withheld identically by the two dialects, and the two sets are stated independently rather than one derived from the other. The Trino dialect's set holds every entry of this one except `FN_PRED_LIKE_ESCAPE`, plus `AGGREGATE_GROUP_BY_TUPLE`, which this dialect withholds by the framework scope decision `vs-adapter/pushdown-aggregation` owns rather than by anything about PostgreSQL's SQL; that third set is an independent statement whose overlap with this one is an outcome rather than a rule binding either.
- Advertising `JOIN_TYPE_FULL_OUTER` rests on PostgreSQL accepting a full join whose condition is an equality. PostgreSQL rejects a full join whose condition is neither merge-joinable nor hash-joinable — measured, `ON "l"."k" < "r"."k"` is refused on 18.6 — but only `JOIN_CONDITION_EQUI` is advertised and an equality over one common type is both, so the restriction does not bind. The restriction is PostgreSQL's own: neither of the other targets has it.
- `FN_AGG_SUM`, `FN_AGG_SUM_DISTINCT`, `FN_AGG_AVG`, and `FN_AGG_AVG_DISTINCT` stay withheld, despite PostgreSQL's exact-numeric `sum` being exact and order-independent. A capability is advertised once per session and admits every numeric operand, including `double precision`, whose sum is order-dependent because floating-point addition is not associative and whose accumulation order is not even stable between two runs under a parallel plan. `vs-adapter/pushdown-aggregation` owns the withholding, for this dialect and for every other.
- Identifier quoting is the double quote, and an embedded double quote is escaped by doubling it. Every identifier is quoted unconditionally, because an unquoted PostgreSQL identifier folds to lower case and a mixed-case source name would then resolve to an object that does not exist.
- The capability list is data rather than code. `crates/adbc-vs-scan/drivers/postgresql.toml` states it beside the driver package row, and the build generates the table the dialect returns, so adding or withdrawing a capability is a data edit.
- A string literal renders as an escape-string constant, `E'<body>'`, with a backslash emitted as two backslashes and a single quote as two single quotes. The `E` prefix selects backslash-escape processing whatever `standard_conforming_strings` is set to, and doubling the quote rather than backslash-escaping it keeps the literal independent of `backslash_quote`. The form is rendered unconditionally rather than only for a value that needs it, so no literal can change the shape of the generated statement.
- The hex route the MySQL dialect uses is rejected here for typing rather than verbosity. A decoded hex literal — `convert_from(decode('<hex>','hex'),'UTF8')` — is typed `text`, and PostgreSQL applies no implicit coercion from `text` to a numeric or temporal column, so comparing one against a mapped `DECIMAL` or `DATE` column fails with an unknown-operator error where MySQL would silently coerce. `E'<body>'` yields an untyped literal PostgreSQL resolves from the other operand.
- Date and timestamp literals render as typed literals, `DATE '...'` and `TIMESTAMP '...'`, so the source coerces no string by its own rules. `predicate_notequal` renders as `<>` and every nested predicate is parenthesized, both of which are the standard forms every other dialect also renders.
- `vs-adapter/postgresql-collation` states that the PostgreSQL dialect renders no collation suffix and what PostgreSQL's column collation does to a delegated comparison. The mapped source types every rendered operand can hold are owned by `vs-adapter/postgresql-type-mapping`.

## Scenarios

### Scenario: Capability set advertises the fragments this dialect renders

* *GIVEN* a virtual schema whose driver is `postgresql`
* *WHEN* Exasol sends a `getCapabilities` request
* *THEN* the `capabilities` array MUST contain `SELECTLIST_PROJECTION`, `SELECTLIST_EXPRESSIONS`, `FILTER_EXPRESSIONS`, `AGGREGATE_SINGLE_GROUP`, `AGGREGATE_GROUP_BY_COLUMN`, `JOIN`, `JOIN_TYPE_INNER`, `JOIN_TYPE_LEFT_OUTER`, `JOIN_TYPE_RIGHT_OUTER`, `JOIN_TYPE_FULL_OUTER`, and `JOIN_CONDITION_EQUI`
* *AND* the array MUST NOT contain `AGGREGATE_GROUP_BY_EXPRESSION`, `AGGREGATE_GROUP_BY_TUPLE`, `AGGREGATE_HAVING`, or `JOIN_CONDITION_ALL`
* *AND* the array MUST NOT contain `ORDER_BY_COLUMN`, `ORDER_BY_EXPRESSION`, `LIMIT`, or `LIMIT_WITH_OFFSET`, which `vs-adapter/result-ordering` withholds for every dialect because the emitting-UDF transport carries no ordering guarantee whatever the source is
* *AND* every entry MUST be a `MainCapability` constant, or carry the prefix `FN_PRED_`, `FN_AGG_`, or `LITERAL_`

### Scenario: Join capabilities add the full outer join

* *GIVEN* a virtual schema whose driver is `postgresql`
* *WHEN* Exasol sends a `getCapabilities` request
* *THEN* the `capabilities` array MUST contain `JOIN`, `JOIN_TYPE_INNER`, `JOIN_TYPE_LEFT_OUTER`, `JOIN_TYPE_RIGHT_OUTER`, `JOIN_TYPE_FULL_OUTER`, and `JOIN_CONDITION_EQUI`
* *AND* it MUST NOT contain `JOIN_CONDITION_ALL`
* *AND* `JOIN_TYPE_FULL_OUTER` MUST be the only capability present here and absent from the MySQL dialect's set

### Scenario: Double quote in an identifier is escaped

* *GIVEN* a PostgreSQL table or column whose name contains a double quote
* *WHEN* the PostgreSQL dialect builds the pushdown statement
* *THEN* the emitted identifier MUST double every double quote it contains
* *AND* the emitted identifier MUST remain enclosed in a single pair of double quotes
* *AND* the adapter MUST NOT interpolate the raw name into the statement
* *AND* a lower-case identifier MUST still be emitted quoted, because an unquoted PostgreSQL identifier folds to lower case and a mixed-case source name would then resolve to a table that does not exist

### Scenario: String literal is rendered as an escape-string constant

* *GIVEN* a filter carrying a string literal
* *WHEN* the PostgreSQL dialect renders that literal
* *THEN* the emitted literal MUST be the form `E'<body>'`, with a backslash emitted as two backslashes and a single quote as two single quotes
* *AND* a literal containing a single quote, a backslash, or a multi-byte character MUST render to that one form rather than to a second form chosen per value, and an empty string literal MUST render as `E''`
* *AND* the rendered literal MUST carry the value the request gave it whatever the source's `standard_conforming_strings` and `backslash_quote` settings are, because the `E` prefix fixes the escape processing and the doubled quote avoids the setting that governs a backslash-escaped one

### Scenario: LIKE, IS NULL, BETWEEN, and IN render in their standard forms

* *GIVEN* a `pushdown` request carrying `predicate_like` with and without an `escapeChar`, `predicate_is_null`, `predicate_is_not_null`, `predicate_between`, and `predicate_in_constlist`
* *WHEN* the PostgreSQL dialect renders each
* *THEN* `predicate_like` MUST render its pattern as an escape-string constant, and MUST render an explicit `ESCAPE` clause exactly when the request carries an `escapeChar`
* *AND* `predicate_is_null` and `predicate_is_not_null` MUST render as `IS NULL` and `IS NOT NULL` rather than as a comparison against a null literal
* *AND* the `BETWEEN` bounds and the `IN` list entries MUST appear in the order the request gave them
* *AND* `predicate_notequal` MUST render as `<>`

### Scenario: Delegated full outer join is rendered and executed

* *GIVEN* PostgreSQL virtual tables `orders` and `customers` sharing a key column, each holding a row the other does not match, and local Exasol mirrors of both
* *WHEN* Exasol pushes down a full outer join over that key
* *THEN* `EXPLAIN VIRTUAL` MUST report a generated statement rendering `FULL OUTER JOIN`, so the join reaches PostgreSQL rather than Exasol
* *AND* the query MUST return the unmatched row of each side, with nulls for the other side's columns
* *AND* the returned rows MUST equal what Exasol returns for the same full outer join executed locally over the mirrors
* *AND* PostgreSQL MUST NOT reject the statement over its restriction on full-join conditions, because the delegated condition is an equality

### Scenario: Projection pushdown reaches PostgreSQL rather than Exasol

* *GIVEN* a virtual schema over a PostgreSQL table with eight mapped columns
* *WHEN* Exasol runs a `SELECT` naming two of them
* *THEN* `EXPLAIN VIRTUAL` MUST report a generated statement projecting only those two columns
* *AND* every identifier in that statement MUST be enclosed in double quotes
* *AND* the query MUST return those two columns with the values seeded in PostgreSQL
* *AND* the generated statement MUST NOT name any other mapped column

### Scenario: Filter pushdown reaches PostgreSQL rather than Exasol

* *GIVEN* a virtual schema over a PostgreSQL table holding three rows
* *WHEN* Exasol runs a `SELECT` whose `WHERE` clause matches one row
* *THEN* `EXPLAIN VIRTUAL` MUST report a generated statement carrying that `WHERE` clause
* *AND* the query MUST return exactly the matching row
* *AND* the generated statement MUST NOT read the unmatched rows

### Scenario: Aggregation pushdown reaches PostgreSQL rather than Exasol

* *GIVEN* a virtual schema over a PostgreSQL table holding rows across two groups
* *WHEN* Exasol runs a `SELECT` grouping by one column and counting another
* *THEN* `EXPLAIN VIRTUAL` MUST report a generated statement carrying the aggregate and the `GROUP BY` clause, and the query MUST return one row per group with the correct count
* *AND* the returned values MUST equal what Exasol returns for the same aggregation executed locally over the same data, for `COUNT`, `MIN`, and `MAX` over every mapped non-character operand type and over a character column declaring `COLLATE "C"`, grouped by such a column
* *AND* a query averaging or summing a mapped numeric column MUST produce a generated statement carrying no `AVG` and no `SUM`, and MUST return the value Exasol's own aggregate returns
* *AND* the same query grouping by two columns MUST produce a generated statement carrying no `GROUP BY` clause and MUST still return the correct rows, because Exasol withdraws that aggregation and groups it itself
