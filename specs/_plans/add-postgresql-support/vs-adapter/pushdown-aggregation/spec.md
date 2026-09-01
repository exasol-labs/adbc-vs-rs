# Feature: pushdown_aggregation

Delegates a single-group or `GROUP BY` aggregation to the source, declaring for each aggregate the Exasol type Exasol's own compiler computed for it.

## Background

<!-- DELTA:CHANGED -->
- This feature owns what an aggregation fragment is, for every dialect: which elements are rejected, how a single-group and a grouped aggregation render, where each aggregate's declared `EMITS` type comes from, and which aggregates this release delegates at all. Which capabilities a dialect advertises is that dialect's own fact and lives in `vs-adapter/mysql-pushdown` and `vs-adapter/postgresql-pushdown`. Where a dialect renders a collation suffix — on a character `GROUP BY` key, on a `DISTINCT` operand, on a `MIN` or `MAX` operand — is also that dialect's own fact and lives in `vs-adapter/mysql-collation` and `vs-adapter/postgresql-collation`.
- Aggregation delegation requires `SELECTLIST_EXPRESSIONS`, for every dialect. Measured on Exasol 2025.2.1, the aggregate capabilities alone delegate nothing: `SELECT SUM("c"), COUNT(*) FROM vs."t" GROUP BY "d"` produced a request byte-identical to the projection-only baseline until `SELECTLIST_EXPRESSIONS` was advertised beside them. `vs-adapter/pushdown-dialect` owns that capability and the `BOOLEAN` declared type it brings with it. The measurement is a property of Exasol's negotiation rather than of any source, so it holds for a second dialect without being re-measured against that source.
- `AGGREGATE_HAVING` is advertised by no dialect, so Exasol sends no `having` element and applies the `HAVING` clause itself, even though every target implements `HAVING`. A request carrying the element nonetheless is rejected rather than answered with a statement that drops it. `vs-adapter/result-ordering` owns the matching rejection for `orderBy` and `limit`.
- `AGGREGATE_GROUP_BY_EXPRESSION` and `AGGREGATE_GROUP_BY_TUPLE` are advertised by no dialect: this release groups by columns only. `AGGREGATE_GROUP_BY_TUPLE` gates how many columns a delegated `GROUP BY` may name rather than a tuple-valued grouping form, so withholding it caps delegated grouping at one column for every dialect. Measured on Exasol 2025.2.1 with a throwaway Lua adapter probe: `GROUP BY "d"` arrived as `aggregationType: group_by` with a `groupBy` naming one column, while `GROUP BY "d", "f"` arrived carrying no `aggregationType` and no `groupBy` at all, its select list degraded to two plain columns. A multi-column `GROUP BY` is therefore withdrawn whole and Exasol groups it itself, at no correctness cost.
- The renderer nonetheless renders every grouping column a request carries, in the order `groupBy` gives them, so the cap is a capability boundary rather than a rendering limit and no end-to-end scenario claims multi-column grouping is reachable in this release.
- `aggregationType` is `single_group` when the query aggregates without grouping and `group_by` when it groups. `single_group` renders no `GROUP BY` clause, in every dialect.
- An aggregate select-list entry carries no source column type to copy, so its declared `EMITS` type comes from `pushdownRequest.selectListDataTypes` at the same index, as `vs-adapter/pushdown-dialect` requires. Exasol rejects a declared type that differs from the one it expects, so reading Exasol's own answer is the only source that is correct by construction — and it is what lets every dialect share one declared-type rule despite their targets computing aggregate types differently.
- Measured on Exasol 2025.2.1, those types follow Exasol's aggregate rules: `SUM` over `DECIMAL(p,s)` yields `DECIMAL(min(p + 11, 36), s)`, so `SUM` over `DECIMAL(12,4)` yields `DECIMAL(23,4)` and `SUM` over `DECIMAL(30,2)` yields `DECIMAL(36,2)`; `SUM` over `DOUBLE` yields `DOUBLE`; `SUM(DISTINCT DECIMAL(12,4))` yields `DECIMAL(23,4)`, the same as the non-distinct form; `AVG` yields `DOUBLE` for every numeric operand; `COUNT`, `COUNT(*)`, and `COUNT(DISTINCT)` yield `DECIMAL(18,0)`; `MIN` and `MAX` yield the operand's own type. These values are asserted so a change in what Exasol sends is caught rather than absorbed, not used as a substitute for reading the field.
- Aggregate function names arrive in mixed case. Measured on Exasol 2025.2.1, one request carried `sum`, `min`, and `max` in lower case beside `AVG` and `COUNT` in upper case, so a name MUST be matched case-insensitively and the generated statement MUST spell it in the target's canonical upper case whatever case arrived. Every target this release supports accepts `COUNT`, `MIN`, and `MAX` under that spelling.
- `COUNT(*)` arrives as a `function_aggregate` named `COUNT` carrying no `arguments`, which is how it is told apart from `COUNT(column)`. `DISTINCT` arrives as a `distinct` field on the aggregate and renders inside it, before the argument.
- A select-list predicate may wrap an aggregate: `COUNT(*) > 0` arrives as a predicate whose operand is a `function_aggregate`, declared `BOOLEAN`. An aggregate operand is therefore not always the direct child of the select-list entry, and the aggregate renderer is the same one a bare aggregate entry uses.
- Whether an aggregate operand is character-typed is read from the aggregate itself: `MIN` and `MAX` carry their argument's type, and `COUNT` and `SUM` are numeric. A select-list predicate's declared type is `BOOLEAN` and carries no type for its operand, so no declared type answers the question. The answer chooses a collation rather than a declared `EMITS` type, so `vs-adapter/pushdown-dialect`'s rule that a declared type is read and never inferred is untouched; which collation follows, and whether any does, is per dialect.
- An aggregation is delegated whole or not at all. Measured on Exasol 2025.2.1, `SUM("c") + 1` withdraws the aggregation entirely rather than delegating `SUM` and adding locally, because no dialect advertises a scalar function. A query mixing an aggregate with a scalar operation therefore scans the source and Exasol aggregates itself, at no correctness cost.
- `FN_AGG_AVG` and `FN_AGG_AVG_DISTINCT` are advertised by no dialect. Measured on MySQL 8.4.11 and Exasol 2025.2.1, no rendering of `AVG` reproduces Exasol's value: bare `AVG(operand)` over an exact-numeric column returns MySQL's `DECIMAL` at the operand's scale plus 4 digits, an absolute error where a double's error is relative, worst measured relative error 1.0e-04; `AVG(CAST(operand AS DOUBLE))` returns a value wrong by 47 per cent for `AVG(DISTINCT)` over a `DECIMAL(19,0)` column holding values near 1e18; `CAST(SUM(operand) AS DOUBLE) / COUNT(operand)` reproduced Exasol exactly in every exact-numeric case measured but cannot help a `DOUBLE` operand. PostgreSQL's `avg` returns `numeric` over an exact-numeric operand and `double precision` over a floating-point one, so it carries the same two problems in its own spelling. Exasol therefore computes every `AVG` itself, for every dialect.
- `FN_AGG_SUM` and `FN_AGG_SUM_DISTINCT` are advertised by no dialect either, and the reason is the floating-point operand rather than the exact-numeric one. Measured on MySQL 8.4.11 and Exasol 2025.2.1 over one `DOUBLE` multiset — `1e16` once and `1` a thousand times — seeded three times under three physical row orders, MySQL's `SUM` returned `1e16`, `1.0000000000001e16`, and `1.0000000000000772e16`, while Exasol returned one value every time, the correctly rounded one. PostgreSQL's `sum` over `double precision` is order-dependent for the same reason floating-point addition is not associative, and PostgreSQL may additionally combine partial aggregates in a parallel plan, so its accumulation order is not even stable between two runs of one query. `vs-adapter/result-ordering` withholds every ordering capability from every dialect, so no rendering forces a matching order in any target.
- Withholding is all-or-nothing at the capability level, which is why an exact-numeric-only `SUM` is not advertised even where the target's exact-numeric `SUM` is exact and order-independent. `FN_AGG_SUM` is advertised once per session and admits every numeric operand, and once Exasol delegates a shape an advertised capability covers it does not compute that fragment itself after the adapter errors. Advertising it and refusing a floating-point operand at render time would turn `SELECT SUM("price") FROM vs."orders"` into a hard failure where release 0.1 answers it. Both pairs therefore stay withheld for every dialect, and the cost is throughput alone: a query mixing `SUM` or `AVG` with `COUNT`, `MIN`, or `MAX` scans the source and Exasol aggregates all of it. `vs-adapter/pushdown-dialect` owns the principle.
- This release's delegated aggregate set is therefore the same for every dialect — `COUNT`, `COUNT(*)`, `COUNT(DISTINCT)`, `MIN`, and `MAX` — and each is exact over every operand type because none of them sums.
- Every character operand of a delegated `MIN` or `MAX` is Exasol `VARCHAR`, and Exasol pads no `VARCHAR`. `vs-adapter/type-mapping` maps no source column to a blank-padded Exasol type, so the padding question is settled upstream and this feature carries no padding rule of its own.
- An aggregate the dialect does not render fails with an error naming the function rather than rendering the aggregate's argument alone, because Exasol would then aggregate nothing and return the wrong value. A delegated aggregate whose source value exceeds the precision the adapter declared for it fails at coercion rather than being truncated or rounded.
<!-- /DELTA:CHANGED -->

## Scenarios

<!-- DELTA:REMOVED -->
### Scenario: Aggregation capabilities cover single-group and column grouping

* *GIVEN* a virtual schema backed by MySQL
* *WHEN* Exasol sends a `getCapabilities` request
* *THEN* the `capabilities` array MUST contain `AGGREGATE_SINGLE_GROUP`, `AGGREGATE_GROUP_BY_COLUMN`, and `SELECTLIST_EXPRESSIONS`, without which Exasol delegates no aggregate at all
* *AND* it MUST contain `FN_AGG_COUNT`, `FN_AGG_COUNT_STAR`, `FN_AGG_COUNT_DISTINCT`, `FN_AGG_MIN`, and `FN_AGG_MAX`, which are the aggregates this release delegates
* *AND* it MUST NOT contain `AGGREGATE_GROUP_BY_EXPRESSION`, `AGGREGATE_GROUP_BY_TUPLE`, or `AGGREGATE_HAVING`
* *AND* it MUST NOT contain `FN_AGG_AVG`, `FN_AGG_AVG_DISTINCT`, `FN_AGG_SUM`, `FN_AGG_SUM_DISTINCT`, or any other `FN_AGG_` capability the dialect does not render: no rendering of `AVG` reproduces the value Exasol's own `AVG` returns, and no rendering of `SUM` over a `DOUBLE` column does either. The two `SUM` capabilities MUST be withheld whole rather than advertised behind a render-time operand-type check, because a capability is advertised once per session and Exasol does not compute a delegated fragment itself after the adapter errors
<!-- /DELTA:REMOVED -->

<!-- DELTA:REMOVED -->
### Scenario: Grouped character columns carry the comparison collation in both the grouping list and the select list

* *GIVEN* a `pushdown` request whose `aggregationType` is `group_by`, whose `groupBy` names one character column and one numeric column, and whose select list projects both grouping columns beside a `MIN` over the same character column
* *WHEN* the dialect renders the statement
* *THEN* the character grouping column MUST carry `COLLATE utf8mb4_0900_as_cs` in the `GROUP BY` clause
* *AND* the same suffix MUST appear on that column's bare select-list occurrence, because MySQL rejects the statement with error 1055 under `ONLY_FULL_GROUP_BY` when the two occurrences carry different collations
* *AND* the `MIN` over the character column MUST keep `COLLATE utf8mb4_0900_bin`, because an aggregate is not subject to `ONLY_FULL_GROUP_BY` and the value `MIN` returns is chosen by order rather than by equality
<!-- /DELTA:REMOVED -->

<!-- DELTA:REMOVED -->
### Scenario: Only a grouped character column carries the grouping collation

* *GIVEN* a `pushdown` request whose `groupBy` names a numeric column, and a second request carrying no `aggregationType` whose select list projects a character column
* *WHEN* the dialect renders each statement
* *THEN* the numeric grouping column MUST render with no collation suffix in the `GROUP BY` clause and none in the select list
* *AND* the character column of the second request MUST render with no collation suffix, so a projection-only statement is byte-identical to what this release renders before the grouping rule exists
* *AND* the collation MUST therefore read off both the operand's declared type and whether the query groups by that column, never off the type alone
<!-- /DELTA:REMOVED -->

<!-- DELTA:REMOVED -->
### Scenario: A DISTINCT aggregate over a character column carries the comparison collation

* *GIVEN* a `function_aggregate` whose `distinct` field is true over a character column, the shape `COUNT(DISTINCT "name")` arrives in
* *WHEN* the dialect renders it
* *THEN* the operand MUST carry `COLLATE utf8mb4_0900_as_cs`, because `DISTINCT` reduces values by equality and MySQL's default collation equates values Exasol keeps apart
* *AND* the same aggregate carrying no `distinct` field MUST render its operand with no collation suffix, because `COUNT` over a character column reads neither equality nor order
* *AND* a character operand of `MIN` or `MAX` MUST carry `COLLATE utf8mb4_0900_bin` whatever its `distinct` field holds, because the value those two return is chosen by order
<!-- /DELTA:REMOVED -->

<!-- DELTA:REMOVED -->
### Scenario: Delegated grouping and DISTINCT over case-only variants return Exasol's groups

* *GIVEN* a virtual table and a local Exasol table holding identical character values, two of which differ only by case and two only by an accent
* *WHEN* the same `GROUP BY` over that column, and the same `COUNT(DISTINCT)` over it, are run over each
* *THEN* `EXPLAIN VIRTUAL` MUST report a generated statement whose `GROUP BY` key and whose projected grouping column both carry `COLLATE utf8mb4_0900_as_cs`
* *AND* the virtual `GROUP BY` MUST return exactly the groups the local query returns, so no two values Exasol keeps apart are collapsed into one group
* *AND* the virtual `COUNT(DISTINCT)` MUST return the value the local query returns
* *AND* the query MUST execute at the source rather than failing with MySQL error 1055
<!-- /DELTA:REMOVED -->

<!-- DELTA:REMOVED -->
### Scenario: MIN and MAX over a character column reproduce Exasol's code-point order

* *GIVEN* a MySQL `VARCHAR` column of collation `utf8mb4_0900_ai_ci` holding the values `B`, `a`, and `C`
* *WHEN* Exasol pushes down `MIN` and `MAX` over that column
* *THEN* the rendered operand MUST carry `COLLATE utf8mb4_0900_bin`
* *AND* the query MUST return `B` for `MIN` and `a` for `MAX`, matching what Exasol returns for the same aggregate over the same values
* *AND* `MIN` over a column holding two values that differ only by a trailing space MUST return the value Exasol returns for the same aggregate over the same values, so the collation MUST NOT be one that pads
* *AND* a numeric or temporal operand MUST render without a collation suffix
<!-- /DELTA:REMOVED -->

<!-- DELTA:CHANGED -->
### Scenario: Predicate over an aggregate is delegated as a BOOLEAN column

* *GIVEN* a `pushdown` request whose select list carries a predicate whose operand is a `function_aggregate`, as Exasol sends for `COUNT(*) > 0`
* *WHEN* the dialect renders the statement
* *THEN* the generated statement MUST project the predicate over the rendered aggregate
* *AND* the `EMITS` clause MUST declare that column `BOOLEAN`
* *AND* the aggregate MUST be rendered by the same aggregate renderer a bare aggregate entry uses
* *AND* a character-typed aggregate operand of that predicate MUST carry whatever suffix its dialect renders for an equality operand, so `MIN("name") = 'x'` compares the way Exasol compares
<!-- /DELTA:CHANGED -->

<!-- DELTA:REMOVED -->
### Scenario: Select-list predicate over the grouped character column fails with a named error

* *GIVEN* a `pushdown` request whose `aggregationType` is `group_by`, whose `groupBy` names one character column, and whose select list carries a predicate reading that same column outside any aggregate
* *WHEN* the dialect renders the statement
* *THEN* the dialect MUST return an error naming that column
* *AND* the dialect MUST NOT emit a MySQL statement, because MySQL rejects every rendering of this shape with error 1055 under `ONLY_FULL_GROUP_BY` whatever collation the operand carries
* *AND* the same MUST hold for each of the five measured predicate kinds `predicate_less`, `predicate_is_null`, `predicate_like`, `predicate_in_constlist`, and `predicate_between`
* *AND* the same MUST hold for a request whose select list does not project the grouping column at all, because the grouping key is still a collated expression the predicate reads a sub-expression of
<!-- /DELTA:REMOVED -->

<!-- DELTA:REMOVED -->
### Scenario: Select-list predicate shapes that stay delegated

* *GIVEN* three `pushdown` requests each carrying a select-list predicate: one whose `groupBy` names a numeric column that the predicate reads, one whose `groupBy` names a character column that the predicate reads only through a `MIN` over it, and one carrying no `group_by` aggregation whose predicate reads a character column
* *WHEN* the dialect renders each
* *THEN* each MUST produce a MySQL statement rather than an error
* *AND* the predicate over the `MIN` MUST carry `COLLATE utf8mb4_0900_as_cs` on the aggregate and `COLLATE utf8mb4_0900_bin` on the aggregate's own argument, the composition measured valid on MySQL 8.4 beside a collated grouping key
* *AND* the numeric grouping column MUST carry no collation suffix in either its `GROUP BY` occurrence or its predicate occurrence
* *AND* a predicate over `COUNT(*)` MUST carry no collation suffix at all
<!-- /DELTA:REMOVED -->

<!-- DELTA:REMOVED -->
### Scenario: Grouped-column select-list predicate fails before reaching MySQL

* *GIVEN* a virtual table whose character column holds values differing only by case and only by accent
* *WHEN* Exasol runs a `SELECT` projecting a comparison over that column while grouping by it
* *THEN* the query MUST fail with an error naming that column
* *AND* the error MUST come from the adapter rather than from MySQL, so no generated statement reaches the source
* *AND* the same query projecting a comparison over `COUNT(*)` instead MUST succeed and MUST return the values the local Exasol mirror returns
<!-- /DELTA:REMOVED -->

<!-- DELTA:CHANGED -->
### Scenario: AVG and SUM are computed by Exasol rather than delegated

* *GIVEN* a virtual schema, a query averaging a mapped numeric column, and a query summing one, for each driver this release supports
* *WHEN* Exasol plans each query
* *THEN* each generated statement MUST carry no `AVG` and no `SUM`, because no dialect advertises either function's capabilities and Exasol withdraws an aggregation it cannot delegate whole
* *AND* each query MUST return the value Exasol's own `AVG` or `SUM` returns, so withholding the four capabilities MUST change no answer an operator reads, for any operand type
* *AND* a `SUM` over a floating-point column MUST return that same value rather than an adapter error, because the capability is withheld rather than advertised and refused at render time
* *AND* a `pushdown` request nonetheless carrying an `AVG` or a `SUM` MUST fail with an error naming that function, so the withheld capabilities and the renderer cannot disagree
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Aggregate value that overflows its declared type fails rather than truncating

* *GIVEN* a delegated aggregate whose source result exceeds the precision the adapter declared for it
* *WHEN* the scan coerces that value to the declared Exasol type
* *THEN* the scan MUST fail with an error naming the declared type and the value that did not fit
* *AND* the scan MUST NOT emit a truncated or rounded value
* *AND* the same MUST hold when the source returns the value as a decimal, as an integer, or as a string, because the PostgreSQL driver returns every `numeric` as a string and a `COUNT` as a 64-bit integer
<!-- /DELTA:CHANGED -->

<!-- DELTA:REMOVED -->
### Scenario: Aggregation pushdown reaches MySQL rather than Exasol

* *GIVEN* a virtual schema over a MySQL table holding rows across two groups
* *WHEN* Exasol runs a `SELECT` grouping by one column and counting another
* *THEN* `EXPLAIN VIRTUAL` MUST report a generated statement carrying the aggregate and the `GROUP BY` clause
* *AND* the query MUST return one row per group with the correct count
* *AND* the returned values MUST equal what Exasol returns for the same aggregation executed locally over the same data, unqualified: `COUNT`, `MIN`, and `MAX` are the aggregates this release delegates and each is exact over every operand type, `DOUBLE` included, because none of them sums. No aggregate whose value depends on summation order is delegated at all
* *AND* the same query grouping by two columns MUST produce a generated statement carrying no `GROUP BY` clause and MUST still return the correct rows, because Exasol withdraws that aggregation and groups it itself
<!-- /DELTA:REMOVED -->
