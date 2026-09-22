# Feature: trino_pushdown

States the capability set the Trino dialect advertises and the Trino SQL it renders for each delegated fragment.

## Background

- The Trino dialect states its own advertised capability set and derives it from no other dialect's. `vs-adapter/pushdown-dialect` owns the all-or-nothing negotiation rule — Exasol delegates a fragment fully once it receives the capability and never re-applies it — and the invariant that every advertised capability has a rendering path in the dialect that advertises it.
- The set is the 35 entries the PostgreSQL dialect advertises plus one that dialect withholds, `AGGREGATE_GROUP_BY_TUPLE`, 36 in all. Each set is stated independently and `capabilities_tests` renders every entry of each through its own dialect, so the 35 they share is an agreement between two decisions rather than a derivation. Nothing in this release's capability vocabulary is withheld from Trino for a reason about Trino's SQL. Every entry the set omits is omitted by a decision that binds every dialect identically, listed below.
- `AGGREGATE_GROUP_BY_TUPLE` is advertised here and by no other dialect, which makes Trino's set the only one in this release that exceeds the shared floor. Two facts carry it. Trino executes a multi-column `GROUP BY`, which is what the capability gates: `vs-adapter/pushdown-dialect` records that the capability governs how many columns a delegated `GROUP BY` may name rather than a tuple-valued grouping form. And advertising it needs no rendering change in any dialect, Trino's included — `vs-adapter/pushdown-aggregation` already states that the renderer renders every grouping column a request carries, in the order `groupBy` gives them, so the cap this release otherwise imposes is a capability boundary rather than a rendering limit. The other two grouping capabilities are not in that position and stay withheld, below. Gate 2.3(i) confirms Trino executes the rendered multi-column form before the entry ships.
- `JOIN_TYPE_FULL_OUTER` is advertised. Trino's documented join syntax implements `FULL OUTER JOIN` directly and states no condition restriction on it, so `FULL OUTER JOIN … ON a.k < b.k`, `ON a.k <> b.k`, and `ON true` are each expected to execute and null-extend both sides; gate 2.3(d) measures all three against a live Trino before the capability ships. PostgreSQL's restriction on a full join whose condition is neither merge-joinable nor hash-joinable therefore has no Trino counterpart, and Trino's advertising the capability rests on the keyword alone rather than on the equality that makes it safe for PostgreSQL.
- `JOIN_CONDITION_ALL` stays absent, and here the reason is scope rather than SQL. Trino would execute a non-equi join condition, including under a full outer join, but mission.md places general non-equi joins outside this project's pushdown scope for every dialect. Advertising it for one technology would move that boundary rather than add a driver.
- `FN_AGG_SUM`, `FN_AGG_SUM_DISTINCT`, `FN_AGG_AVG`, and `FN_AGG_AVG_DISTINCT` stay absent, and the reason is the one `vs-adapter/pushdown-aggregation` states for every dialect: a capability is advertised once per session and admits every numeric operand, including a floating-point one whose sum depends on accumulation order. Trino distributes an aggregation across workers and combines partial results, so its accumulation order for a `double` operand is not even stable between two runs of one query.
- `AGGREGATE_GROUP_BY_EXPRESSION`, `AGGREGATE_HAVING`, `LITERAL_BOOL`, and every scalar-function capability stay absent for the same kind of reason: each is withheld by a framework decision identical for every dialect, not by anything Trino cannot express. `vs-adapter/pushdown-aggregation` owns the first two, `vs-adapter/type-mapping` the third through the closed set of Exasol types a mapped column declares, and `vs-adapter/pushdown-dialect` the last through the declarable-set closure argument. `vs-adapter/result-ordering` owns the four ordering and limiting capabilities, withheld because of the adapter's own transport rather than any source. Trino would execute a grouping expression and a `HAVING` clause, and the separation from `AGGREGATE_GROUP_BY_TUPLE` is not about what Trino can do: that one entry is confirmed to need no rendering change in any dialect, and neither of these two is, so lifting either would buy a rendering change in three dialects rather than none.
- Identifier quoting is the double quote, and an embedded double quote is escaped by doubling it. Every identifier is quoted unconditionally, because Trino requires quoting for a reserved word and for any name outside letters, digits, and underscores, and a rule that quoted only those names would have to carry Trino's reserved-word list.
- Quoting buys Trino less than it buys the other two targets, and that difference costs this dialect nothing. Trino resolves every identifier case-insensitively whether it is quoted or not, so a quoted mixed-case name resolves to the same object its lower-case spelling would. Two source objects differing only by case are therefore indistinguishable to Trino, which is a property of Trino's own namespace rather than of this rendering: a name the catalog reported is a name Trino resolves.
- A table is qualified by both segments of the namespace `adapterNotes` records — the catalog and the schema — each quoted separately, so the generated statement names its object in full rather than resolving it against a session default the adapter does not control. A Trino table's fully qualified name is `catalog.schema.table`, and the connection address's path carries the first two.
- A string literal renders as a plain quoted constant with every single quote doubled, and nothing else is escaped. Trino's documented string-literal grammar carries no escape-string constant, so `E'a'` is expected to parse as a typed literal of an unknown type `e` and fail; gate 2.3(f) measures that. No Trino setting changes how a quoted string is read either, so a backslash inside one is a literal backslash and the one form covers every value.
- A `DATE` and a `TIMESTAMP` literal render as typed literals, and for Trino that is mandatory rather than defensive. Trino's type documentation states that it applies no implicit coercion between a character type and any other type, so `DATE '2024-01-01' = '2024-01-01'` is expected to fail with a cannot-apply-operator error over `date` and `varchar(10)`, and `BETWEEN` over two string bounds the same way. Gate 2.3(g) measures both and records the message each returns.
- An exact-numeric literal and a floating-point literal render differently, which is the one literal rule this dialect does not share with the other two. Trino's type documentation states that a bare decimal constant is typed `DECIMAL`, so `typeof(123.45)` is expected to be `decimal(5,2)` and rendering a `LITERAL_DOUBLE` bare would hand Trino an exact-numeric operand where Exasol meant a floating-point one. A floating-point literal therefore renders as `DOUBLE '<value>'`, the form Trino documents, and an exact-numeric literal renders bare. Gate 2.3(b) measures the typing before either form ships.
- The bare form is safe for every exact-numeric literal Exasol can send. Trino documents a maximum `DECIMAL` precision of 38 and states that a decimal literal exceeding it is rejected rather than widened to a floating-point value, and Exasol's own maximum `DECIMAL` precision is 36, so no literal Exasol produces reaches Trino's limit. Gate 2.3(h) measures the rejection, because a Trino that widened instead would make the bare form silently change a literal's type.
- `predicate_notequal` renders as `<>`, which Trino documents as the standard spelling beside a non-standard `!=`, and every nested predicate is parenthesized. Both are the forms the other two dialects render.
- A `LIKE` renders an explicit `ESCAPE` clause for every pattern, carrying the request's own `escapeChar` when it has one and a backslash when it has none. Trino defines no default escape character, so a pattern Exasol built under its own backslash rule would match a different set of rows with no clause. `vs-adapter/pushdown-filter` owns the framework rule and the exclusion for a pattern whose escape character precedes something the target does not admit.
- Collation placement is owned by `vs-adapter/trino-collation`, which states that this dialect renders no collation suffix at all and why. The mapped source types every rendered operand can hold are owned by `vs-adapter/trino-type-mapping`.
- The capability list is data rather than code. `crates/adbc-vs-scan/drivers/trino.toml` states it beside the driver package row, and the build generates the table the dialect returns, so adding or withdrawing a capability is a data edit.

## Scenarios

### Scenario: Capability set advertises the fragments this dialect renders

* *GIVEN* a virtual schema whose driver is `trino`
* *WHEN* Exasol sends a `getCapabilities` request
* *THEN* the `capabilities` array MUST contain `SELECTLIST_PROJECTION`, `SELECTLIST_EXPRESSIONS`, `FILTER_EXPRESSIONS`, `AGGREGATE_SINGLE_GROUP`, `AGGREGATE_GROUP_BY_COLUMN`, `JOIN`, `JOIN_TYPE_INNER`, `JOIN_TYPE_LEFT_OUTER`, `JOIN_TYPE_RIGHT_OUTER`, `JOIN_TYPE_FULL_OUTER`, and `JOIN_CONDITION_EQUI`
* *AND* it MUST contain every `FN_PRED_` capability the PostgreSQL dialect advertises, every `FN_AGG_` capability it advertises, and every `LITERAL_` capability it advertises
* *AND* the array MUST hold exactly the 36 entries an independently written constant states, `AGGREGATE_GROUP_BY_TUPLE` among them where no other dialect advertises it, so both the widening and every deliberate absence are pinned by an exact comparison rather than by a prefix rule that admits any name
* *AND* the array MUST NOT contain `AGGREGATE_GROUP_BY_EXPRESSION`, `AGGREGATE_HAVING`, `JOIN_CONDITION_ALL`, or `LITERAL_BOOL`
* *AND* the array MUST NOT contain `ORDER_BY_COLUMN`, `ORDER_BY_EXPRESSION`, `LIMIT`, or `LIMIT_WITH_OFFSET`, which `vs-adapter/result-ordering` withholds for every dialect because the emitting-UDF transport carries no ordering guarantee whatever the source is
* *AND* every entry MUST be a `MainCapability` constant, or carry the prefix `FN_PRED_`, `FN_AGG_`, or `LITERAL_`

### Scenario: The Trino set is the PostgreSQL set plus one entry, and each is stated on its own

* *GIVEN* the capability set the Trino dialect advertises and the set the PostgreSQL dialect advertises
* *WHEN* the two sets are compared
* *THEN* the Trino set MUST hold every entry of the PostgreSQL set, and the one entry it holds that the PostgreSQL set does not MUST be `AGGREGATE_GROUP_BY_TUPLE`, with no other difference in either direction
* *AND* each set MUST be stated by its own technology's declaration rather than one derived from the other by addition, removal, or reference
* *AND* every entry of the Trino set MUST have its own rendering path in the Trino dialect, proven by rendering each rather than by the comparison
* *AND* a later change to either set MUST fail this comparison rather than be absorbed, because the 35 they share is an outcome of two independent decisions and not a rule either dialect is held to

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
* *AND* the two MUST render differently for the same spelled value, because a bare constant Trino types `DECIMAL` is not the operand Exasol meant when it sent a floating-point literal
* *AND* a literal value not spelled as a bare numeric constant MUST fail with a named error rather than be rendered

### Scenario: LIKE renders an explicit escape clause for every pattern

* *GIVEN* a `predicate_like` carrying an `escapeChar` and a `predicate_like` carrying none
* *WHEN* the Trino dialect renders each
* *THEN* the first MUST render an `ESCAPE` clause over the character the request carried
* *AND* the second MUST render an `ESCAPE` clause over a backslash, because Trino defines no default escape character and Exasol's own default is the backslash
* *AND* the rendered rows MUST equal what Exasol returns for the same pattern over the same values, for a pattern carrying an escaped wildcard and for a pattern carrying none
* *AND* the escape character MUST be rendered through the same string-literal form every other literal in the statement takes

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
* *AND* a query averaging or summing a mapped numeric column MUST produce a generated statement carrying no `AVG` and no `SUM`, and MUST return the value Exasol's own aggregate returns
* *AND* the same query grouping by two columns MUST produce a generated statement carrying both columns in one `GROUP BY` clause, in the order the request gave them, and MUST return the rows Exasol returns for the same two-column grouping executed locally, because this dialect advertises `AGGREGATE_GROUP_BY_TUPLE` and Exasol therefore delegates the grouping rather than withdrawing it
