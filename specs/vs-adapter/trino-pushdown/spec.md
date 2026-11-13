# Feature: trino_pushdown

States the capability set the Trino dialect advertises and the Trino SQL it renders for each delegated fragment.

## Background

- The Trino dialect states its own advertised set, derived from no other dialect's, and `vs-adapter/pushdown-dialect` owns the all-or-nothing negotiation rule and the invariant that every advertised capability renders in the dialect that advertises it; the list is data in `crates/adbc-vs-scan/drivers/trino.toml`, from which the build generates the table the dialect returns.
- The set holds 40 entries: the PostgreSQL dialect's 40 without `FN_PRED_LIKE_ESCAPE`, which no Trino form renders for every pattern Exasol accepts, plus `AGGREGATE_GROUP_BY_TUPLE`, which caps how many columns a delegated `GROUP BY` names and needs no rendering change in any dialect.
- Every other capability the set omits is withheld by a decision that binds every dialect: `JOIN_CONDITION_ALL` by mission.md's pushdown scope, `AGGREGATE_GROUP_BY_EXPRESSION` by `vs-adapter/pushdown-aggregation-delegation`, `LITERAL_BOOL` by `vs-adapter/type-mapping`, every scalar function by `vs-adapter/pushdown-dialect`, and the ordering and limiting capabilities by `vs-adapter/result-ordering`.
- Identifiers are double-quoted with an embedded double quote doubled, a table is qualified by the catalog and the schema `adapterNotes` records, a string literal renders with every single quote doubled, date and timestamp literals render typed, an exact-numeric literal renders bare and a floating-point one as `DOUBLE '<value>'`, `predicate_notequal` renders as `<>`, and every nested predicate is parenthesized.
- `vs-adapter/trino-collation` states that the Trino dialect renders no collation suffix and why, and `vs-adapter/trino-type-mapping` owns the mapped source types every rendered operand can hold.

## Scenarios

### Scenario: Capability set advertises the fragments this dialect renders

* *GIVEN* a virtual schema whose driver is `trino`
* *WHEN* Exasol sends a `getCapabilities` request
* *THEN* the `capabilities` array MUST contain `SELECTLIST_PROJECTION`, `SELECTLIST_EXPRESSIONS`, `FILTER_EXPRESSIONS`, `AGGREGATE_SINGLE_GROUP`, `AGGREGATE_GROUP_BY_COLUMN`, `AGGREGATE_HAVING`, `JOIN`, `JOIN_TYPE_INNER`, `JOIN_TYPE_LEFT_OUTER`, `JOIN_TYPE_RIGHT_OUTER`, `JOIN_TYPE_FULL_OUTER`, and `JOIN_CONDITION_EQUI`
* *AND* it MUST contain every `FN_PRED_` capability the PostgreSQL dialect advertises except `FN_PRED_LIKE_ESCAPE`, every `FN_AGG_` capability it advertises, and every `LITERAL_` capability it advertises
* *AND* the array MUST hold exactly the 40 entries an independently written constant states, `AGGREGATE_GROUP_BY_TUPLE` among them where no other dialect advertises it, so both the widening and every deliberate absence are pinned by an exact comparison rather than by a prefix rule that admits any name
* *AND* every entry MUST be a `MainCapability` constant, or carry the prefix `FN_PRED_`, `FN_AGG_`, or `LITERAL_`

### Scenario: A table is qualified by its catalog and its schema

* *GIVEN* a virtual schema whose recorded namespace is the catalog `memory` and the schema `adbc_vs`
* *WHEN* the Trino dialect renders a statement over the table `orders`
* *THEN* the generated statement MUST qualify the table as three separately quoted identifiers, the catalog, the schema, and the table name
* *AND* the adapter MUST NOT rely on a catalog or schema the connection selected, because a session default is state the generated statement does not carry
* *AND* an identifier containing a double quote MUST double every double quote it contains and stay enclosed in a single pair of double quotes
* *AND* every identifier MUST be emitted quoted, including one that needs no quoting, because Trino requires quoting for a reserved word and a rule applied per name would have to carry Trino's reserved-word list

### Scenario: String, date, and timestamp literals render in the forms Trino requires

* *GIVEN* a filter carrying a string literal, a date literal, and a timestamp literal
* *WHEN* the Trino dialect renders each
* *THEN* the string literal MUST render as a plain quoted constant with every single quote doubled and nothing else escaped, and an empty string MUST render as two quotes
* *AND* a string literal containing a backslash MUST render that backslash unchanged, because Trino reads no escape sequence inside a quoted string
* *AND* the date literal MUST render as `DATE` followed by the quoted value and the timestamp literal as `TIMESTAMP` followed by the quoted value, because Trino applies no implicit coercion from a character type and a bare quoted value fails the comparison rather than matching no rows
* *AND* a temporal literal value carrying a character outside digits, `-`, `:`, `.`, and a space MUST fail with a named error rather than be rendered, because the value sits inside the quotes the typed literal requires

### Scenario: An exact-numeric literal renders bare and a floating-point literal renders typed

* *GIVEN* a filter carrying a `LITERAL_EXACTNUMERIC` and a filter carrying a `LITERAL_DOUBLE` of the same spelled value
* *WHEN* the Trino dialect renders each
* *THEN* the exact-numeric literal MUST render as the bare numeric constant, unquoted
* *AND* the floating-point literal MUST render as `DOUBLE` followed by the quoted value, so Trino types it as a floating-point value rather than as the `DECIMAL` a bare constant yields
* *AND* the two MUST render differently for the same spelled value, because the typed form is total over every value Exasol can send where a bare constant is not: a value spelled with an exponent is already `double`, `Infinity` and `NaN` have no bare spelling at all, and a constant above 38 significant digits is rejected outright
* *AND* a literal value not spelled as a bare numeric constant MUST fail with a named error rather than be rendered

### Scenario: Delegated full outer join is rendered and executed

* *GIVEN* Trino virtual tables `orders` and `customers` sharing a key column, each holding a row the other does not match, and local Exasol mirrors of both
* *WHEN* Exasol pushes down a full outer join over that key
* *THEN* `EXPLAIN VIRTUAL` MUST report a generated statement rendering `FULL OUTER JOIN`, so the join reaches Trino rather than Exasol
* *AND* the query MUST return the unmatched row of each side, with nulls for the other side's columns
* *AND* the returned rows MUST equal what Exasol returns for the same full outer join executed locally over the mirrors

### Scenario: Projection pushdown reaches Trino rather than Exasol

* *GIVEN* a virtual schema over a Trino table with seven mapped columns
* *WHEN* Exasol runs a `SELECT` naming two of them
* *THEN* `EXPLAIN VIRTUAL` MUST report a generated statement projecting only those two columns
* *AND* every identifier in that statement MUST be enclosed in double quotes
* *AND* the query MUST return those two columns with the values seeded in Trino
* *AND* the generated statement MUST NOT name any other mapped column

### Scenario: Filter pushdown reaches Trino rather than Exasol

* *GIVEN* a virtual schema over a Trino table holding three rows
* *WHEN* Exasol runs a `SELECT` whose `WHERE` clause matches one row
* *THEN* `EXPLAIN VIRTUAL` MUST report a generated statement carrying that `WHERE` clause
* *AND* the query MUST return exactly the matching row
* *AND* the generated statement MUST NOT read the unmatched rows

### Scenario: Aggregation pushdown reaches Trino rather than Exasol

* *GIVEN* a virtual schema over a Trino table holding rows across two groups
* *WHEN* Exasol runs a `SELECT` grouping by one column and counting another
* *THEN* `EXPLAIN VIRTUAL` MUST report a generated statement carrying the aggregate and the `GROUP BY` clause, and the query MUST return one row per group with the correct count
* *AND* the returned values MUST equal what Exasol returns for the same aggregation executed locally over the same data, for `COUNT`, `MIN`, and `MAX` over every mapped operand type
* *AND* a query summing or averaging a mapped exact-numeric column MUST produce a generated statement carrying `SUM`, and MUST return the value Exasol's own aggregate returns over the same data
* *AND* the same query grouping by two columns MUST produce a generated statement carrying both columns in one `GROUP BY` clause, in the order the request gave them, and MUST return the rows Exasol returns for the same two-column grouping executed locally, because this dialect advertises `AGGREGATE_GROUP_BY_TUPLE` and Exasol therefore delegates the grouping rather than withdrawing it

### Scenario: Capability set withholds the fragments the Trino dialect does not delegate

* *GIVEN* a virtual schema whose driver is `trino`
* *WHEN* Exasol sends a `getCapabilities` request
* *THEN* the `capabilities` array MUST NOT contain `FN_PRED_LIKE_ESCAPE`, `AGGREGATE_GROUP_BY_EXPRESSION`, `JOIN_CONDITION_ALL`, or `LITERAL_BOOL`
* *AND* the array MUST NOT contain `ORDER_BY_COLUMN`, `ORDER_BY_EXPRESSION`, `LIMIT`, or `LIMIT_WITH_OFFSET`, which `vs-adapter/result-ordering` withholds for every dialect because the emitting-UDF transport carries no ordering guarantee whatever the source is

### Scenario: The Trino set differs from the PostgreSQL set by two named entries, and each is stated on its own

* *GIVEN* the capability set the Trino dialect advertises and the set the PostgreSQL dialect advertises
* *WHEN* the two sets are compared
* *THEN* the Trino set MUST hold `AGGREGATE_GROUP_BY_TUPLE`, which the PostgreSQL set does not, and MUST NOT hold `FN_PRED_LIKE_ESCAPE`, which the PostgreSQL set does, with no other difference in either direction
* *AND* each set MUST be stated by its own technology's declaration rather than one derived from the other by addition, removal, or reference
* *AND* every entry of the Trino set MUST have its own rendering path in the Trino dialect, proven by rendering each rather than by the comparison
* *AND* a later change to either set MUST fail this comparison rather than be absorbed, because the 39 entries they share are an outcome of two independent decisions and not a rule either dialect is held to

### Scenario: LIKE renders its pattern with no escape clause

* *GIVEN* a `predicate_like` carrying no `escapeChar`, once with the pattern `ab%` and once with the pattern `ab\%`
* *WHEN* the Trino dialect renders each
* *THEN* each MUST render as `LIKE` over its pattern with no `ESCAPE` clause
* *AND* the pattern MUST be rendered through the same string-literal form every other literal in the statement takes, with its backslash unchanged
* *AND* the dialect MUST NOT refuse a pattern for the character that follows a backslash, because Trino reads a backslash in a pattern with no `ESCAPE` clause as an ordinary character
* *AND* a `predicate_like` carrying an `escapeChar` MUST fail with an error naming `FN_PRED_LIKE_ESCAPE` as a capability this dialect does not advertise, rather than render without the escape character

### Scenario: A LIKE carrying an explicit ESCAPE clause is evaluated by Exasol

* *GIVEN* a virtual table over a Trino `varchar` column and a local Exasol table holding the same values
* *WHEN* the same `LIKE` with an explicit `ESCAPE` clause is run over each
* *THEN* `EXPLAIN VIRTUAL` MUST report a generated statement carrying no `LIKE`, because Exasol keeps a predicate whose capability the dialect does not advertise
* *AND* the query over the virtual table MUST return exactly the rows the local query returns

### Scenario: SUM over an exact-numeric column sums in Trino's 38-digit decimal

* *GIVEN* a Trino `bigint` column declared `DECIMAL(19,0)`, holding `9223372036854775807` and `1`, and a local Exasol mirror table holding the same two values
* *WHEN* Exasol runs `SUM` and `AVG` over the column through the virtual schema
* *THEN* the generated statement MUST render the operand inside `SUM` cast to `DECIMAL(38,0)`, and in general an operand declared `DECIMAL(p,s)` cast to `DECIMAL(38,s)`, because Trino's `sum` returns its input's type, so a sum over `bigint` cannot leave the `bigint` range
* *AND* the query MUST return `9223372036854775808` for `SUM`, as the mirror does, rather than a Trino overflow error
* *AND* a `SUM` operand declared `DOUBLE` MUST render without a cast
