# Feature: trino_pushdown

States the capability set the Trino dialect advertises and the Trino SQL it renders for each delegated fragment.

<!-- DELTA:CHANGED -->
## Background

- The Trino dialect states its own advertised capability set and derives it from no other dialect's. `vs-adapter/pushdown-dialect` owns the all-or-nothing negotiation rule — Exasol delegates a fragment fully once it receives the capability and never re-applies it — and the invariant that every advertised capability has a rendering path in the dialect that advertises it.
- The set holds 35 entries: the PostgreSQL dialect's 35, minus `FN_PRED_LIKE_ESCAPE`, plus `AGGREGATE_GROUP_BY_TUPLE`, which that dialect withholds. Each set is stated independently and `capabilities_tests` renders every entry of each through its own dialect, so the 34 they share is an agreement between two decisions rather than a derivation. `FN_PRED_LIKE_ESCAPE` is the one entry withheld from Trino for a reason about Trino's SQL, stated below. Every other entry the set omits is omitted by a decision that binds every dialect identically, listed below.
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
- An exact-numeric literal and a floating-point literal render differently, which is the one literal rule this dialect does not share with the other two. A bare decimal constant is typed `DECIMAL`: gate 2.3(b) measured `typeof(123.45)` as `decimal(5,2)`. That typing alone returns the right rows, because Trino promotes a decimal operand to `double` in a mixed comparison, and the same gate measured `WHERE price = 19.99` returning the row `WHERE price = DOUBLE '19.99'` returns, over a `double` column and over a `decimal` column alike. The typed form is rendered because it is total where the bare form is not: a value spelled with an exponent is already `double`, `Infinity` and `NaN` have no bare spelling at all, and a constant above 38 significant digits is rejected outright. A floating-point literal therefore renders as `DOUBLE '<value>'`, the form Trino documents, and an exact-numeric literal renders bare.
- The bare form is safe for every exact-numeric literal Exasol can send. Trino documents a maximum `DECIMAL` precision of 38 and states that a decimal literal exceeding it is rejected rather than widened to a floating-point value, and Exasol's own maximum `DECIMAL` precision is 36, so no literal Exasol produces reaches Trino's limit. Gate 2.3(h) measures the rejection, because a Trino that widened instead would make the bare form silently change a literal's type.
- `predicate_notequal` renders as `<>`, which Trino documents as the standard spelling beside a non-standard `!=`, and every nested predicate is parenthesized. Both are the forms the other two dialects render.
- `FN_PRED_LIKE_ESCAPE` is withheld, and `predicate_like` renders as a plain `LIKE` with no `ESCAPE` clause. Trino defines no default escape character and rejects an escape character followed by anything other than `%`, `_`, or itself, so neither Exasol's default backslash rule nor a request's own escape character has a Trino form that accepts every pattern Exasol accepts. A backslash in a delegated pattern is therefore an ordinary character at Trino, where Exasol's default reads it as an escape. A query that needs an escape character writes an explicit `ESCAPE` clause, and Exasol evaluates that predicate itself because the capability is withheld.
- Collation placement is owned by `vs-adapter/trino-collation`, which states that this dialect renders no collation suffix at all and why. The mapped source types every rendered operand can hold are owned by `vs-adapter/trino-type-mapping`.
- The capability list is data rather than code. `crates/adbc-vs-scan/drivers/trino.toml` states it beside the driver package row, and the build generates the table the dialect returns, so adding or withdrawing a capability is a data edit.
<!-- /DELTA:CHANGED -->

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Capability set advertises the fragments this dialect renders

* *GIVEN* a virtual schema whose driver is `trino`
* *WHEN* Exasol sends a `getCapabilities` request
* *THEN* the `capabilities` array MUST contain `SELECTLIST_PROJECTION`, `SELECTLIST_EXPRESSIONS`, `FILTER_EXPRESSIONS`, `AGGREGATE_SINGLE_GROUP`, `AGGREGATE_GROUP_BY_COLUMN`, `JOIN`, `JOIN_TYPE_INNER`, `JOIN_TYPE_LEFT_OUTER`, `JOIN_TYPE_RIGHT_OUTER`, `JOIN_TYPE_FULL_OUTER`, and `JOIN_CONDITION_EQUI`
* *AND* it MUST contain every `FN_PRED_` capability the PostgreSQL dialect advertises except `FN_PRED_LIKE_ESCAPE`, every `FN_AGG_` capability it advertises, and every `LITERAL_` capability it advertises
* *AND* the array MUST hold exactly the 35 entries an independently written constant states, `AGGREGATE_GROUP_BY_TUPLE` among them where no other dialect advertises it, so both the widening and every deliberate absence are pinned by an exact comparison rather than by a prefix rule that admits any name
* *AND* every entry MUST be a `MainCapability` constant, or carry the prefix `FN_PRED_`, `FN_AGG_`, or `LITERAL_`
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: Capability set withholds the fragments the Trino dialect does not delegate

* *GIVEN* a virtual schema whose driver is `trino`
* *WHEN* Exasol sends a `getCapabilities` request
* *THEN* the `capabilities` array MUST NOT contain `FN_PRED_LIKE_ESCAPE`, `AGGREGATE_GROUP_BY_EXPRESSION`, `AGGREGATE_HAVING`, `JOIN_CONDITION_ALL`, or `LITERAL_BOOL`
* *AND* the array MUST NOT contain `ORDER_BY_COLUMN`, `ORDER_BY_EXPRESSION`, `LIMIT`, or `LIMIT_WITH_OFFSET`, which `vs-adapter/result-ordering` withholds for every dialect because the emitting-UDF transport carries no ordering guarantee whatever the source is
<!-- /DELTA:NEW -->

<!-- DELTA:REMOVED -->
### Scenario: The Trino set is the PostgreSQL set plus one entry, and each is stated on its own

* *GIVEN* the capability set the Trino dialect advertises and the set the PostgreSQL dialect advertises
* *WHEN* the two sets are compared
* *THEN* the Trino set MUST hold every entry of the PostgreSQL set, and the one entry it holds that the PostgreSQL set does not MUST be `AGGREGATE_GROUP_BY_TUPLE`, with no other difference in either direction
* *AND* each set MUST be stated by its own technology's declaration rather than one derived from the other by addition, removal, or reference
* *AND* every entry of the Trino set MUST have its own rendering path in the Trino dialect, proven by rendering each rather than by the comparison
* *AND* a later change to either set MUST fail this comparison rather than be absorbed, because the 35 they share is an outcome of two independent decisions and not a rule either dialect is held to
<!-- /DELTA:REMOVED -->

<!-- DELTA:NEW -->
### Scenario: The Trino set differs from the PostgreSQL set by two named entries, and each is stated on its own

* *GIVEN* the capability set the Trino dialect advertises and the set the PostgreSQL dialect advertises
* *WHEN* the two sets are compared
* *THEN* the Trino set MUST hold `AGGREGATE_GROUP_BY_TUPLE`, which the PostgreSQL set does not, and MUST NOT hold `FN_PRED_LIKE_ESCAPE`, which the PostgreSQL set does, with no other difference in either direction
* *AND* each set MUST be stated by its own technology's declaration rather than one derived from the other by addition, removal, or reference
* *AND* every entry of the Trino set MUST have its own rendering path in the Trino dialect, proven by rendering each rather than by the comparison
* *AND* a later change to either set MUST fail this comparison rather than be absorbed, because the 34 entries they share are an outcome of two independent decisions and not a rule either dialect is held to
<!-- /DELTA:NEW -->

<!-- DELTA:REMOVED -->
### Scenario: LIKE renders an explicit escape clause for every pattern

* *GIVEN* a `predicate_like` carrying an `escapeChar` and a `predicate_like` carrying none
* *WHEN* the Trino dialect renders each
* *THEN* the first MUST render an `ESCAPE` clause over the character the request carried
* *AND* the second MUST render an `ESCAPE` clause over a backslash, because Trino defines no default escape character and Exasol's own default is the backslash
* *AND* the rendered rows MUST equal what Exasol returns for the same pattern over the same values, for a pattern carrying an escaped wildcard and for a pattern carrying none
* *AND* the escape character MUST be rendered through the same string-literal form every other literal in the statement takes
<!-- /DELTA:REMOVED -->

<!-- DELTA:NEW -->
### Scenario: LIKE renders its pattern with no escape clause

* *GIVEN* a `predicate_like` carrying no `escapeChar`, once with the pattern `ab%` and once with the pattern `ab\%`
* *WHEN* the Trino dialect renders each
* *THEN* each MUST render as `LIKE` over its pattern with no `ESCAPE` clause
* *AND* the pattern MUST be rendered through the same string-literal form every other literal in the statement takes, with its backslash unchanged
* *AND* the dialect MUST NOT refuse a pattern for the character that follows a backslash, because Trino reads a backslash in a pattern with no `ESCAPE` clause as an ordinary character
* *AND* a `predicate_like` carrying an `escapeChar` MUST fail with an error naming `FN_PRED_LIKE_ESCAPE` as a capability this dialect does not advertise, rather than render without the escape character
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: A LIKE carrying an explicit ESCAPE clause is evaluated by Exasol

* *GIVEN* a virtual table over a Trino `varchar` column and a local Exasol table holding the same values
* *WHEN* the same `LIKE` with an explicit `ESCAPE` clause is run over each
* *THEN* `EXPLAIN VIRTUAL` MUST report a generated statement carrying no `LIKE`, because Exasol keeps a predicate whose capability the dialect does not advertise
* *AND* the query over the virtual table MUST return exactly the rows the local query returns
<!-- /DELTA:NEW -->
