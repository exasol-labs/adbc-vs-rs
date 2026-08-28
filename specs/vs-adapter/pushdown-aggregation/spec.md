# Feature: pushdown_aggregation

Delegates a single-group or `GROUP BY` aggregation to MySQL, declaring for each aggregate the Exasol type Exasol's own compiler computed for it.

## Background

- Aggregation delegation requires `SELECTLIST_EXPRESSIONS`. Measured on Exasol 2025.2.1, the aggregate capabilities alone delegate nothing: `SELECT SUM("c"), COUNT(*) FROM vs."t" GROUP BY "d"` produced a request byte-identical to the projection-only baseline until `SELECTLIST_EXPRESSIONS` was advertised beside them. `vs-adapter/pushdown-dialect` owns that capability and the `BOOLEAN` declared type it brings with it.
- An aggregate select-list entry carries no source column type to copy, so its declared `EMITS` type comes from `pushdownRequest.selectListDataTypes` at the same index, as `vs-adapter/pushdown-dialect` requires. Exasol rejects a declared type that differs from the one it expects, so reading Exasol's own answer is the only source that is correct by construction.
- Measured on Exasol 2025.2.1, those types follow Exasol's aggregate rules: `SUM` over `DECIMAL(p,s)` yields `DECIMAL(min(p + 11, 36), s)`, so `SUM` over `DECIMAL(12,4)` yields `DECIMAL(23,4)` and `SUM` over `DECIMAL(30,2)` yields `DECIMAL(36,2)`; `SUM` over `DOUBLE` yields `DOUBLE`; `SUM(DISTINCT DECIMAL(12,4))` yields `DECIMAL(23,4)`, the same as the non-distinct form; `AVG` yields `DOUBLE` for every numeric operand; `COUNT`, `COUNT(*)`, and `COUNT(DISTINCT)` yield `DECIMAL(18,0)`; `MIN` and `MAX` yield the operand's own type. These values are asserted so a change in what Exasol sends is caught rather than absorbed, not used as a substitute for reading the field.
- Aggregate function names arrive in mixed case. Measured on Exasol 2025.2.1, one request carried `sum`, `min`, and `max` in lower case beside `AVG` and `COUNT` in upper case, so a name MUST be matched case-insensitively and the generated statement MUST spell it in MySQL's canonical upper case whatever case arrived.
- A select-list predicate may wrap an aggregate: `COUNT(*) > 0` arrives as a predicate whose operand is a `function_aggregate`, declared `BOOLEAN`. An aggregate operand is therefore not always the direct child of the select-list entry.
- Whether an aggregate operand is character-typed is read from the aggregate itself: `MIN` and `MAX` carry their argument's type, and `COUNT` and `SUM` are numeric. A select-list predicate's declared type is `BOOLEAN` and carries no type for its operand, so no declared type answers the question. This chooses a collation, not a declared `EMITS` type, so `vs-adapter/pushdown-dialect`'s rule that a declared type is read and never inferred is untouched. Measured valid on MySQL 8.4: `MIN(\`v\` COLLATE utf8mb4_0900_bin) COLLATE utf8mb4_0900_as_cs` compares case-sensitively over the code-point minimum, so the two collations compose on one operand.
- An aggregation is delegated whole or not at all. Measured on Exasol 2025.2.1, `SUM("c") + 1` withdraws the aggregation entirely rather than delegating `SUM` and adding locally, because this release advertises no scalar function. A query mixing an aggregate with a scalar operation therefore scans the source and Exasol aggregates itself, at no correctness cost.
- Every character operand of a delegated `MIN` or `MAX` is Exasol `VARCHAR`, and Exasol pads no `VARCHAR`. `vs-adapter/type-mapping` maps MySQL `CHAR` to Exasol `VARCHAR`, so the padding question is settled upstream and this feature carries no separate `CHAR` rule.
- Exasol compares and orders character values by code point and pads no `VARCHAR`. MySQL 8.4's default `utf8mb4_0900_ai_ci` orders by collation weight, so `MIN` over the values `B`, `a`, `C` returns `a` in MySQL and `B` in Exasol. A character operand of `MIN` or `MAX` is therefore rendered with `COLLATE utf8mb4_0900_bin`, which weighs the `utf8mb4` encoding bytes and so reproduces code-point order, and which MySQL reports as `NO PAD`. This differs from the `utf8mb4_0900_as_cs` a comparison carries, because ordering and equality are different questions: `utf8mb4_0900_as_cs` orders by collation weight and so is wrong for `MIN` and `MAX`.
- `utf8mb4_bin` is not the binary collation used here. `information_schema.COLLATIONS` reports it as `PAD SPACE`, so it ignores a trailing space: measured on MySQL 8.4, `MIN` over the values `'ab '`, `'ab'`, `'ab!'` returns `'ab '` under `utf8mb4_bin`, while Exasol 2025.2.1 returns `'ab'` and reports `'ab' = 'ab '` as false. `utf8mb4_0900_bin` returns `'ab'` for the same values, matching Exasol. It requires MySQL 8.0.17, which `vs-adapter/create-virtual-schema` enforces as the source-server floor.
- `aggregationType` is `single_group` when the query aggregates without grouping and `group_by` when it groups. `single_group` renders no `GROUP BY` clause.
- `AGGREGATE_GROUP_BY_EXPRESSION` and `AGGREGATE_GROUP_BY_TUPLE` are not advertised: this release groups by columns only. `AGGREGATE_HAVING` is not advertised, so Exasol sends no `having` element and applies the `HAVING` clause itself.
- `FN_AGG_AVG` and `FN_AGG_AVG_DISTINCT` are not advertised: this release delegates no `AVG`. Measured on MySQL 8.4.11 and Exasol 2025.2.1, no rendering of `AVG` reproduces Exasol's value. Bare `AVG(operand)` over an exact-numeric column returns MySQL's `DECIMAL` at the operand's scale plus 4 digits, an absolute error where a double's error is relative: worst measured relative error 1.0e-04, over a `DECIMAL(9,0)` column holding values under 100. `AVG(CAST(operand AS DOUBLE))` fixes that case and returns a value wrong by 47 per cent for `AVG(DISTINCT)` over a `DECIMAL(19,0)` identifier column holding values near 1e18, because the cast moves the `DISTINCT` reduction into the double domain, where it reweights rather than rounds. `CAST(SUM(operand) AS DOUBLE) / COUNT(operand)` reproduced Exasol exactly in every exact-numeric case measured, including three physical row orders and both `DISTINCT` cases, but cannot help a `DOUBLE` operand. Exasol therefore computes every `AVG` itself, exactly as it did before this release, so withholding the two capabilities costs one delegated aggregate and no correctness. `vs-adapter/pushdown-dialect` owns the withholding.
- `FN_AGG_SUM` and `FN_AGG_SUM_DISTINCT` are not advertised either: this release delegates no `SUM`. A `SUM` over a `DOUBLE` column is unreproducible at any rendering. Measured on MySQL 8.4.11 and Exasol 2025.2.1 over one `DOUBLE` multiset — `1e16` once and `1` a thousand times — seeded three times under three physical row orders, MySQL's `SUM` returned `1e16`, `1.0000000000001e16`, and `1.0000000000000772e16`, while Exasol returned one value every time, the correctly rounded one. MySQL accumulates left to right, so an addend below the running total's unit in the last place is lost; Exasol's summation preserves it. The delegated value is therefore a function of physical row order rather than of the data alone, and `vs-adapter/result-ordering` withholds every ordering capability, so no rendering forces a matching order. `DISTINCT` does not help: it reduces `DOUBLE` values by exact bit equality and leaves the summation order-dependent.
- Both `SUM` capabilities are withheld rather than advertised behind a render-time operand-type check, because negotiation is all-or-nothing at the capability level. MySQL's `SUM` over an exact-numeric operand was measured exact and order-independent, so the hazard is one operand type wide while a capability is advertised once per session. `vs-adapter/pushdown-dialect` records the consequence: once Exasol delegates a query shape an advertised capability covers it is committed, and it does not compute that fragment itself after the adapter errors. Advertising `FN_AGG_SUM` and rejecting a `DOUBLE` operand in the renderer would therefore make `SELECT SUM("price") FROM vs."orders"` fail where release 0.1 answers it. Withholding both capabilities costs the exact-numeric `SUM` delegation and no answer: Exasol scans the source and sums every operand type itself, exactly as it does against release 0.1. `vs-adapter/pushdown-dialect` owns the withholding.
- This feature makes no query shape fail that release 0.1 answered, apart from the select-list predicate over a grouped character column, which MySQL rejects in every rendering. Every `SUM` query and every `AVG` query returns the value it returned before this release, for every operand type. The cost of the two withholdings is throughput: an aggregation is delegated whole or not at all, so a query mixing `SUM` or `AVG` with `COUNT`, `MIN`, or `MAX` scans the source and Exasol aggregates all of it.
- This release delegates an aggregation grouping by exactly one column. `AGGREGATE_GROUP_BY_TUPLE` gates how many columns a delegated `GROUP BY` may name, not grouping by a tuple-valued expression as its name suggests. Measured on Exasol 2025.2.1 with a throwaway Lua adapter probe advertising `AGGREGATE_GROUP_BY_COLUMN` and this feature's aggregate functions: `GROUP BY "d"` arrived as `aggregationType: group_by` with a `groupBy` naming one column, while `GROUP BY "d", "f"` arrived carrying no `aggregationType` and no `groupBy` at all, its select list degraded to the two plain columns with the `COUNT` entry gone. Adding `AGGREGATE_GROUP_BY_TUPLE` delegated the same query with both columns in `groupBy`, and a three-column `GROUP BY` with all three. A multi-column `GROUP BY` is therefore withdrawn whole and Exasol groups it itself, at no correctness cost. The dialect renders every grouping column a request carries, so the cap is a capability boundary rather than a rendering limit, and no end-to-end scenario claims multi-column grouping is reachable in this release. `vs-adapter/pushdown-dialect` owns the withholding.
- A `GROUP BY` key and a `DISTINCT` aggregate operand each ask the equality question a comparison asks, so a character-typed one carries `COLLATE utf8mb4_0900_as_cs` — the collation `vs-adapter/pushdown-filter` renders on a comparison operand. Measured on MySQL 8.4 over a `VARCHAR(50)` column of collation `utf8mb4_0900_ai_ci` holding `Widget`, `widget`, `Gizmo`, and `Gîzmo`: `GROUP BY` produced 2 groups and `COUNT(DISTINCT)` returned 2 under the column's own collation, and 4 of each under `utf8mb4_0900_as_cs`. Exasol 2025.2.1 returns 4 for both over the same values. An uncollated delegated `GROUP BY` therefore returns fewer groups than Exasol asked for, and an uncollated delegated `COUNT(DISTINCT)` a smaller count — the silent wrong-value failure mode this release does not ship. Both shapes are reachable: measured on the same probe, `SELECT "g", MIN("g") FROM vs."t" GROUP BY "g"` arrived with `groupBy` naming the character column and a select list projecting it beside the aggregate, and `SELECT COUNT(DISTINCT "g") FROM vs."t"` arrived as a `function_aggregate` carrying `"distinct": true` over it.
- The grouping collation is one coordinated rule over two clauses rather than two independent rules. MySQL's `ONLY_FULL_GROUP_BY` mode, which `mysql:8.4` sets by default, rejects a select list whose bare column occurrence does not carry the collation its `GROUP BY` key carries: measured, `SELECT v, COUNT(*) FROM n GROUP BY v COLLATE utf8mb4_0900_as_cs` failed with MySQL error 1055, and the same statement collating both occurrences succeeded and returned Exasol's 4 groups. A grouping key the select list does not project needs no second occurrence, and Exasol rejects a select list naming a non-aggregated column absent from `GROUP BY` exactly as MySQL does, so no other bare column occurrence can arrive. An aggregate is never subject to the mode, so a `MIN` over the grouping column keeps `COLLATE utf8mb4_0900_bin` while the grouping key carries `utf8mb4_0900_as_cs` — measured valid in one MySQL statement.
- One select-list predicate shape is not delegated. A predicate whose operand reads the query's own character-typed `GROUP BY` key outside an aggregate MUST fail with an error naming that column, because MySQL rejects every statement that renders it. Measured on Exasol 2025.2.1 with a throwaway Lua adapter probe over the capability set this release advertises, five predicate kinds arrive in that shape — `"g" > 'x'`, `"g" IS NULL`, `"g" LIKE 'a%'`, `"g" IN ('a','b')`, and `"g" BETWEEN 'a' AND 'z'` — each operand naming the exact column `groupBy` names, each declared `BOOLEAN`, and the comparison also arrives in the variant projecting no bare column at all. Measured on MySQL 8.4 under `ONLY_FULL_GROUP_BY`, each such statement fails with error 1055 at every collation placement, including the one leaving the predicate's operand uncollated: MySQL matches a select-list expression against a `GROUP BY` expression whole and credits no sub-expression of the collated grouping key this feature renders. The failure is a hard MySQL error rather than wrong rows, so an adapter error naming the column is the same outcome diagnosed at the point the operator can read it.
- The hazard is bounded to that one shape in both directions, and every other select-list predicate is delegated. A select-list predicate's operand is always either the query's own grouping column or an aggregate: `SELECT "d", "g" > 'x' … GROUP BY "d"` never reaches the adapter, because Exasol rejects it itself with `not a valid GROUP BY expression`. Measured on MySQL 8.4 over the same four values, a predicate over `COUNT(*)`, a predicate over `MIN` of the character grouping column, and a predicate over a numeric grouping key each returned the 4 rows Exasol returns. A query whose aggregation is `single_group` or absent carries no grouping key to disagree with.
- One question about the grouping collation is open rather than settled, and no scenario here asserts an answer to it. Under `utf8mb4_0900_as_cs` MySQL compares an NFC-composed character and its NFD-decomposed equivalent as equal, measured on MySQL 8.4, while `ß` and `ss` compare unequal under the same collation. Exasol compares character values by code point. A delegated `GROUP BY` over normalization-variant values may therefore still return fewer groups than Exasol asked for, in a narrower form of the divergence the grouping collation removes for case and accent. What Exasol 2025.2.1 returns for such values was not measured, so the gap is unconfirmed rather than known, and the fixture carries no normalization-variant pair.
- `COUNT(*)` arrives as a `function_aggregate` named `COUNT` carrying no `arguments`, which is how it is told apart from `COUNT(column)`.

## Scenarios

### Scenario: Aggregation capabilities cover single-group and column grouping

* *GIVEN* a virtual schema backed by MySQL
* *WHEN* Exasol sends a `getCapabilities` request
* *THEN* the `capabilities` array MUST contain `AGGREGATE_SINGLE_GROUP`, `AGGREGATE_GROUP_BY_COLUMN`, and `SELECTLIST_EXPRESSIONS`, without which Exasol delegates no aggregate at all
* *AND* it MUST contain `FN_AGG_COUNT`, `FN_AGG_COUNT_STAR`, `FN_AGG_COUNT_DISTINCT`, `FN_AGG_MIN`, and `FN_AGG_MAX`, which are the aggregates this release delegates
* *AND* it MUST NOT contain `AGGREGATE_GROUP_BY_EXPRESSION`, `AGGREGATE_GROUP_BY_TUPLE`, or `AGGREGATE_HAVING`
* *AND* it MUST NOT contain `FN_AGG_AVG`, `FN_AGG_AVG_DISTINCT`, `FN_AGG_SUM`, `FN_AGG_SUM_DISTINCT`, or any other `FN_AGG_` capability the dialect does not render: no rendering of `AVG` reproduces the value Exasol's own `AVG` returns, and no rendering of `SUM` over a `DOUBLE` column does either. The two `SUM` capabilities MUST be withheld whole rather than advertised behind a render-time operand-type check, because a capability is advertised once per session and Exasol does not compute a delegated fragment itself after the adapter errors

### Scenario: Request carrying a having element fails rather than dropping it

* *GIVEN* a `pushdown` request carrying a `having` element, which Exasol sends only when `AGGREGATE_HAVING` was advertised
* *WHEN* the adapter builds the response
* *THEN* the adapter MUST return an error naming the `having` element
* *AND* the error MUST name the withheld capability that gates the element, which is the one place a change would make it legal
* *AND* the adapter MUST NOT return a statement that silently ignores the element, because Exasol reads the result as one the `HAVING` clause was already applied to

### Scenario: Single-group aggregation renders without a GROUP BY clause

* *GIVEN* a `pushdown` request whose `aggregationType` is `single_group` and whose select list is one `COUNT` over a mapped `DECIMAL` column
* *WHEN* the dialect renders the statement
* *THEN* the generated MySQL statement MUST project `COUNT` over the quoted column
* *AND* it MUST NOT carry a `GROUP BY` clause
* *AND* the `EMITS` clause MUST declare exactly one column

### Scenario: GROUP BY renders every grouping column the request carries, in request order

* *GIVEN* a constructed `pushdown` request whose `aggregationType` is `group_by`, whose `groupBy` names two columns, and whose select list names those columns and one aggregate
* *WHEN* the dialect renders the statement
* *THEN* the generated statement MUST carry a `GROUP BY` clause naming both columns in the order `groupBy` gives them
* *AND* the select list MUST render in the order the `selectList` gives it, which need not match the `groupBy` order
* *AND* the `EMITS` clause MUST declare one positionally named column per select-list entry
* *AND* this requirement MUST be verified by a unit test over a constructed request rather than end to end, because this release withholds `AGGREGATE_GROUP_BY_TUPLE` and Exasol therefore sends no `groupBy` naming more than one column

### Scenario: Grouped character columns carry the comparison collation in both the grouping list and the select list

* *GIVEN* a `pushdown` request whose `aggregationType` is `group_by`, whose `groupBy` names one character column and one numeric column, and whose select list projects both grouping columns beside a `MIN` over the same character column
* *WHEN* the dialect renders the statement
* *THEN* the character grouping column MUST carry `COLLATE utf8mb4_0900_as_cs` in the `GROUP BY` clause
* *AND* the same suffix MUST appear on that column's bare select-list occurrence, because MySQL rejects the statement with error 1055 under `ONLY_FULL_GROUP_BY` when the two occurrences carry different collations
* *AND* the `MIN` over the character column MUST keep `COLLATE utf8mb4_0900_bin`, because an aggregate is not subject to `ONLY_FULL_GROUP_BY` and the value `MIN` returns is chosen by order rather than by equality

### Scenario: Only a grouped character column carries the grouping collation

* *GIVEN* a `pushdown` request whose `groupBy` names a numeric column, and a second request carrying no `aggregationType` whose select list projects a character column
* *WHEN* the dialect renders each statement
* *THEN* the numeric grouping column MUST render with no collation suffix in the `GROUP BY` clause and none in the select list
* *AND* the character column of the second request MUST render with no collation suffix, so a projection-only statement is byte-identical to what this release renders before the grouping rule exists
* *AND* the collation MUST therefore read off both the operand's declared type and whether the query groups by that column, never off the type alone

### Scenario: A DISTINCT aggregate over a character column carries the comparison collation

* *GIVEN* a `function_aggregate` whose `distinct` field is true over a character column, the shape `COUNT(DISTINCT "name")` arrives in
* *WHEN* the dialect renders it
* *THEN* the operand MUST carry `COLLATE utf8mb4_0900_as_cs`, because `DISTINCT` reduces values by equality and MySQL's default collation equates values Exasol keeps apart
* *AND* the same aggregate carrying no `distinct` field MUST render its operand with no collation suffix, because `COUNT` over a character column reads neither equality nor order
* *AND* a character operand of `MIN` or `MAX` MUST carry `COLLATE utf8mb4_0900_bin` whatever its `distinct` field holds, because the value those two return is chosen by order

### Scenario: Delegated grouping and DISTINCT over case-only variants return Exasol's groups

* *GIVEN* a virtual table and a local Exasol table holding identical character values, two of which differ only by case and two only by an accent
* *WHEN* the same `GROUP BY` over that column, and the same `COUNT(DISTINCT)` over it, are run over each
* *THEN* `EXPLAIN VIRTUAL` MUST report a generated statement whose `GROUP BY` key and whose projected grouping column both carry `COLLATE utf8mb4_0900_as_cs`
* *AND* the virtual `GROUP BY` MUST return exactly the groups the local query returns, so no two values Exasol keeps apart are collapsed into one group
* *AND* the virtual `COUNT(DISTINCT)` MUST return the value the local query returns
* *AND* the query MUST execute at the source rather than failing with MySQL error 1055

### Scenario: Each aggregate declares the type the request supplies for it

* *GIVEN* a `pushdown` request whose select list holds `COUNT`, `COUNT(*)`, `COUNT(DISTINCT)`, `MIN`, and `MAX` entries, and whose `selectListDataTypes` carries one entry per select-list entry
* *WHEN* the adapter builds the `EMITS` clause
* *THEN* each declared type MUST be the `selectListDataTypes` entry at the same index
* *AND* the adapter MUST NOT compute an aggregate's type from its operand's type

### Scenario: Exasol's aggregate types match the values this release was measured against

* *GIVEN* an Exasol query aggregating a mapped `DECIMAL(12,4)` column, a mapped `DECIMAL(30,2)` column, a mapped `DOUBLE` column, and a mapped `VARCHAR(100)` column
* *WHEN* the `selectListDataTypes` Exasol sends is read back
* *THEN* `SUM` over `DECIMAL(12,4)` MUST carry `DECIMAL(23,4)` and `SUM` over `DECIMAL(30,2)` MUST carry `DECIMAL(36,2)`, matching `DECIMAL(min(p + 11, 36), s)`
* *AND* `AVG` MUST carry `DOUBLE`, `SUM` over the `DOUBLE` column MUST carry `DOUBLE`, and `SUM(DISTINCT)` over the `DECIMAL(12,4)` column MUST carry `DECIMAL(23,4)`
* *AND* `COUNT`, `COUNT(*)`, and `COUNT(DISTINCT)` MUST carry `DECIMAL(18,0)`, and `MIN` and `MAX` MUST carry the operand's own type
* *AND* a departure from these values MUST fail the test rather than be absorbed, because the declared type Exasol accepts is the type Exasol sent, and every `AVG` and `SUM` entry MUST be measured through a throwaway adapter probe advertising those capabilities — the shipped capability set delegates neither function, so no such entry is reachable through the shipped dialect, and the values stay asserted as the protocol evidence a later release restoring either fragment would build on

### Scenario: Declared type entries carry the shapes this release measured

* *GIVEN* an Exasol query over a mapped `VARCHAR(100)` column and a mapped `TIMESTAMP` column, projecting one aggregate and one predicate
* *WHEN* the `selectListDataTypes` Exasol sends is read back
* *THEN* a `VARCHAR` entry MUST carry its size and no `characterSet` field, unlike the `involvedTables[].columns[].dataType` entry for the same column
* *AND* a `TIMESTAMP` entry MUST carry `fractionalSecondsPrecision` 3, which the adapter reads past rather than rendering into the `EMITS` clause
* *AND* the predicate's entry MUST carry `BOOLEAN`

### Scenario: COUNT star is told apart from COUNT of a column

* *GIVEN* a `function_aggregate` named `COUNT` carrying no `arguments`
* *WHEN* the dialect renders it
* *THEN* the rendered aggregate MUST be `COUNT(*)`
* *AND* a `function_aggregate` named `COUNT` carrying one column argument MUST render as `COUNT` over that quoted column, which counts only its non-null values

### Scenario: DISTINCT is rendered inside the aggregate

* *GIVEN* a `function_aggregate` whose `distinct` field is true
* *WHEN* the dialect renders it
* *THEN* the rendered aggregate MUST carry `DISTINCT` before its argument
* *AND* a `function_aggregate` carrying no `distinct` field MUST render without `DISTINCT`

### Scenario: MIN and MAX over a character column reproduce Exasol's code-point order

* *GIVEN* a MySQL `VARCHAR` column of collation `utf8mb4_0900_ai_ci` holding the values `B`, `a`, and `C`
* *WHEN* Exasol pushes down `MIN` and `MAX` over that column
* *THEN* the rendered operand MUST carry `COLLATE utf8mb4_0900_bin`
* *AND* the query MUST return `B` for `MIN` and `a` for `MAX`, matching what Exasol returns for the same aggregate over the same values
* *AND* `MIN` over a column holding two values that differ only by a trailing space MUST return the value Exasol returns for the same aggregate over the same values, so the collation MUST NOT be one that pads
* *AND* a numeric or temporal operand MUST render without a collation suffix

### Scenario: Aggregate function name case does not change the rendered statement

* *GIVEN* two `pushdown` requests differing only in the case of their `function_aggregate` names, one carrying `min` and one carrying `MIN`
* *WHEN* the dialect renders each
* *THEN* both generated statements MUST be byte-for-byte identical
* *AND* the rendered function name MUST be MySQL's canonical upper-case spelling
* *AND* a name the dialect does not render MUST fail regardless of its case

### Scenario: Predicate over an aggregate is delegated as a BOOLEAN column

* *GIVEN* a `pushdown` request whose select list carries a predicate whose operand is a `function_aggregate`, as Exasol sends for `COUNT(*) > 0`
* *WHEN* the dialect renders the statement
* *THEN* the generated statement MUST project the predicate over the rendered aggregate
* *AND* the `EMITS` clause MUST declare that column `BOOLEAN`
* *AND* the aggregate MUST be rendered by the same aggregate renderer a bare aggregate entry uses
* *AND* a character-typed aggregate operand of that predicate MUST carry `COLLATE utf8mb4_0900_as_cs`, as a character column operand does, so `MIN("name") = 'x'` compares the way Exasol compares

### Scenario: Select-list predicate over the grouped character column fails with a named error

* *GIVEN* a `pushdown` request whose `aggregationType` is `group_by`, whose `groupBy` names one character column, and whose select list carries a predicate reading that same column outside any aggregate
* *WHEN* the dialect renders the statement
* *THEN* the dialect MUST return an error naming that column
* *AND* the dialect MUST NOT emit a MySQL statement, because MySQL rejects every rendering of this shape with error 1055 under `ONLY_FULL_GROUP_BY` whatever collation the operand carries
* *AND* the same MUST hold for each of the five measured predicate kinds `predicate_less`, `predicate_is_null`, `predicate_like`, `predicate_in_constlist`, and `predicate_between`
* *AND* the same MUST hold for a request whose select list does not project the grouping column at all, because the grouping key is still a collated expression the predicate reads a sub-expression of

### Scenario: Select-list predicate shapes that stay delegated

* *GIVEN* three `pushdown` requests each carrying a select-list predicate: one whose `groupBy` names a numeric column that the predicate reads, one whose `groupBy` names a character column that the predicate reads only through a `MIN` over it, and one carrying no `group_by` aggregation whose predicate reads a character column
* *WHEN* the dialect renders each
* *THEN* each MUST produce a MySQL statement rather than an error
* *AND* the predicate over the `MIN` MUST carry `COLLATE utf8mb4_0900_as_cs` on the aggregate and `COLLATE utf8mb4_0900_bin` on the aggregate's own argument, the composition measured valid on MySQL 8.4 beside a collated grouping key
* *AND* the numeric grouping column MUST carry no collation suffix in either its `GROUP BY` occurrence or its predicate occurrence
* *AND* a predicate over `COUNT(*)` MUST carry no collation suffix at all

### Scenario: Grouped-column select-list predicate fails before reaching MySQL

* *GIVEN* a virtual table whose character column holds values differing only by case and only by accent
* *WHEN* Exasol runs a `SELECT` projecting a comparison over that column while grouping by it
* *THEN* the query MUST fail with an error naming that column
* *AND* the error MUST come from the adapter rather than from MySQL, so no generated statement reaches the source
* *AND* the same query projecting a comparison over `COUNT(*)` instead MUST succeed and MUST return the values the local Exasol mirror returns

### Scenario: AVG and SUM are computed by Exasol rather than delegated

* *GIVEN* a virtual schema backed by MySQL, a query averaging a mapped numeric column, and a query summing one
* *WHEN* Exasol plans each query
* *THEN* each generated statement MUST carry no `AVG` and no `SUM`, because neither function's capabilities are advertised and Exasol withdraws an aggregation it cannot delegate whole
* *AND* each query MUST return the value Exasol's own `AVG` or `SUM` returns, so withholding the four capabilities MUST change no answer an operator reads, for any operand type
* *AND* a `SUM` over a `DOUBLE` column MUST return that same value rather than an adapter error, because the capability is withheld rather than advertised and refused at render time
* *AND* a `pushdown` request nonetheless carrying an `AVG` or a `SUM` MUST fail with an error naming that function, so the withheld capabilities and the renderer cannot disagree

### Scenario: Unrenderable aggregate function fails with a named error

* *GIVEN* a `pushdown` request whose select list carries an aggregate function the dialect does not render
* *WHEN* the adapter builds the response
* *THEN* the adapter MUST return an error
* *AND* the error message MUST name the aggregate function it received
* *AND* the adapter MUST NOT render the aggregate's argument alone, because Exasol would then aggregate nothing and return the wrong value

### Scenario: Aggregate value that overflows its declared type fails rather than truncating

* *GIVEN* a delegated aggregate whose MySQL result exceeds the precision the adapter declared for it
* *WHEN* the scan coerces that value to the declared Exasol type
* *THEN* the scan MUST fail with an error naming the declared type and the value that did not fit
* *AND* the scan MUST NOT emit a truncated or rounded value

### Scenario: Aggregation pushdown reaches MySQL rather than Exasol

* *GIVEN* a virtual schema over a MySQL table holding rows across two groups
* *WHEN* Exasol runs a `SELECT` grouping by one column and counting another
* *THEN* `EXPLAIN VIRTUAL` MUST report a generated statement carrying the aggregate and the `GROUP BY` clause
* *AND* the query MUST return one row per group with the correct count
* *AND* the returned values MUST equal what Exasol returns for the same aggregation executed locally over the same data, unqualified: `COUNT`, `MIN`, and `MAX` are the aggregates this release delegates and each is exact over every operand type, `DOUBLE` included, because none of them sums. No aggregate whose value depends on summation order is delegated at all
* *AND* the same query grouping by two columns MUST produce a generated statement carrying no `GROUP BY` clause and MUST still return the correct rows, because Exasol withdraws that aggregation and groups it itself
