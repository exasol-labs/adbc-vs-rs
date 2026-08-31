# Feature: pushdown_aggregation

Delegates a single-group or `GROUP BY` aggregation to the source, declaring for each aggregate the Exasol type Exasol's own compiler computed for it.

## Background

<!-- DELTA:CHANGED -->
- Aggregation delegation requires `SELECTLIST_EXPRESSIONS`, for every dialect. Measured on Exasol 2025.2.1, the aggregate capabilities alone delegate nothing: `SELECT SUM("c"), COUNT(*) FROM vs."t" GROUP BY "d"` produced a request byte-identical to the projection-only baseline until `SELECTLIST_EXPRESSIONS` was advertised beside them. `vs-adapter/pushdown-dialect` owns that capability and the `BOOLEAN` declared type it brings with it. The measurement is a property of Exasol's negotiation rather than of any source, so it holds for the PostgreSQL dialect without being re-measured against PostgreSQL.
- An aggregate select-list entry carries no source column type to copy, so its declared `EMITS` type comes from `pushdownRequest.selectListDataTypes` at the same index, as `vs-adapter/pushdown-dialect` requires. Exasol rejects a declared type that differs from the one it expects, so reading Exasol's own answer is the only source that is correct by construction — and it is what makes the two dialects need no per-dialect declared-type rule despite their targets computing aggregate types differently.
- Measured on Exasol 2025.2.1, those types follow Exasol's aggregate rules: `SUM` over `DECIMAL(p,s)` yields `DECIMAL(min(p + 11, 36), s)`, so `SUM` over `DECIMAL(12,4)` yields `DECIMAL(23,4)` and `SUM` over `DECIMAL(30,2)` yields `DECIMAL(36,2)`; `SUM` over `DOUBLE` yields `DOUBLE`; `SUM(DISTINCT DECIMAL(12,4))` yields `DECIMAL(23,4)`, the same as the non-distinct form; `AVG` yields `DOUBLE` for every numeric operand; `COUNT`, `COUNT(*)`, and `COUNT(DISTINCT)` yield `DECIMAL(18,0)`; `MIN` and `MAX` yield the operand's own type. These values are asserted so a change in what Exasol sends is caught rather than absorbed, not used as a substitute for reading the field.
- Aggregate function names arrive in mixed case. Measured on Exasol 2025.2.1, one request carried `sum`, `min`, and `max` in lower case beside `AVG` and `COUNT` in upper case, so a name MUST be matched case-insensitively and the generated statement MUST spell it in the target's canonical upper case whatever case arrived. Both targets accept `COUNT`, `MIN`, and `MAX` under that spelling.
- A select-list predicate may wrap an aggregate: `COUNT(*) > 0` arrives as a predicate whose operand is a `function_aggregate`, declared `BOOLEAN`. An aggregate operand is therefore not always the direct child of the select-list entry.
- Whether an aggregate operand is character-typed is read from the aggregate itself: `MIN` and `MAX` carry their argument's type, and `COUNT` and `SUM` are numeric. A select-list predicate's declared type is `BOOLEAN` and carries no type for its operand, so no declared type answers the question. This chooses a collation, not a declared `EMITS` type, so `vs-adapter/pushdown-dialect`'s rule that a declared type is read and never inferred is untouched.
- An aggregation is delegated whole or not at all. Measured on Exasol 2025.2.1, `SUM("c") + 1` withdraws the aggregation entirely rather than delegating `SUM` and adding locally, because neither dialect advertises a scalar function. A query mixing an aggregate with a scalar operation therefore scans the source and Exasol aggregates itself, at no correctness cost.
- Every character operand of a delegated `MIN` or `MAX` is Exasol `VARCHAR`, and Exasol pads no `VARCHAR`. `vs-adapter/type-mapping` maps MySQL `CHAR` to Exasol `VARCHAR` and leaves PostgreSQL `character` unmapped, so no blank-padded source type reaches this path from either driver and this feature carries no padding rule of its own. The two mappings reach that outcome differently — MySQL returns a `CHAR` value with trailing spaces stripped, while PostgreSQL returns a `character(n)` value padded and compares two of them ignoring trailing spaces — and `vs-adapter/type-mapping` owns both arguments.
- Exasol compares and orders character values by code point and pads no `VARCHAR`. Both targets order text differently from that by default, so both dialects render an ordering collation on a character operand of `MIN` or `MAX`, and each renders its own.
- MySQL 8.4's default `utf8mb4_0900_ai_ci` orders by collation weight, so `MIN` over the values `B`, `a`, `C` returns `a` in MySQL and `B` in Exasol. A character operand of `MIN` or `MAX` is therefore rendered with `COLLATE utf8mb4_0900_bin`, which weighs the `utf8mb4` encoding bytes and so reproduces code-point order, and which MySQL reports as `NO PAD`. This differs from the `utf8mb4_0900_as_cs` a MySQL comparison carries, because ordering and equality are different questions.
- PostgreSQL's default ordering is a locale's, not code point: the official `postgres` image sets `LANG=en_US.utf8`, so `initdb` creates the cluster with libc `LC_COLLATE=en_US.utf8` and text sorts by locale weight. A character operand of `MIN` or `MAX` is therefore rendered with `COLLATE "C"`, which sorts by byte value and so reproduces code-point order for UTF-8. `C` is a built-in collation available in every cluster whatever the database encoding, which is why `vs-adapter/create-virtual-schema` asserts no PostgreSQL version floor for it and checks the database encoding instead.
- Where the two dialects genuinely diverge is the equality question, and the divergence removes a whole rule for PostgreSQL rather than renaming one. MySQL's default collation equates values Exasol keeps apart, so `vs-adapter/pushdown-filter` renders `utf8mb4_0900_as_cs` on every MySQL equality operand and this feature renders it on a character `GROUP BY` key and a character `DISTINCT` aggregate operand too. PostgreSQL's built-in collations are all deterministic, and a deterministic collation equates two character values only when their bytes are identical, which is Exasol's equality — so a PostgreSQL `GROUP BY` key and a PostgreSQL `DISTINCT` operand carry no collation suffix at all, and neither does an equality comparison.
- That single difference removes MySQL's coordinated two-clause grouping rule from the PostgreSQL dialect, and with it the composed shape that rule creates. MySQL's grouping collation turns the `GROUP BY` key into an expression, which forces the same suffix onto every bare select-list occurrence of that column — measured, `SELECT v, COUNT(*) FROM n GROUP BY v COLLATE utf8mb4_0900_as_cs` fails with MySQL error 1055 under `ONLY_FULL_GROUP_BY` and collating both occurrences succeeds. The PostgreSQL dialect renders a bare grouping column, so its `GROUP BY` key and its select-list occurrence are the same expression and no coordination is needed. PostgreSQL enforces the full-group-by rule unconditionally rather than under a mode, which makes the absence of a collated key the load-bearing fact rather than a configuration.
- Consequently the one select-list shape MySQL cannot render — a predicate reading its own query's character-typed `GROUP BY` key outside an aggregate — is expected to render on PostgreSQL, because PostgreSQL credits an expression over a bare grouping column. The plan measures that expectation before the PostgreSQL dialect delegates the shape, because the failing direction is a hard source error rather than wrong rows and the measured answer decides whether the exclusion is one dialect's or both dialects'. Until measured, the exclusion binds the MySQL dialect alone and the scenario below states which.
- A `GROUP BY` key and a `DISTINCT` aggregate operand each ask the equality question a comparison asks, so a character-typed one carries the collation its dialect renders for equality — `COLLATE utf8mb4_0900_as_cs` for MySQL, and none for PostgreSQL. Measured on MySQL 8.4 over a `VARCHAR(50)` column of collation `utf8mb4_0900_ai_ci` holding `Widget`, `widget`, `Gizmo`, and `Gîzmo`: `GROUP BY` produced 2 groups and `COUNT(DISTINCT)` returned 2 under the column's own collation, and 4 of each under `utf8mb4_0900_as_cs`. Exasol 2025.2.1 returns 4 for both over the same values. The same fixture is used against PostgreSQL, where the expected result is 4 of each with no suffix rendered, which is what proves the suffix is genuinely unnecessary rather than merely omitted.
- `aggregationType` is `single_group` when the query aggregates without grouping and `group_by` when it groups. `single_group` renders no `GROUP BY` clause, in either dialect.
- `AGGREGATE_GROUP_BY_EXPRESSION` and `AGGREGATE_GROUP_BY_TUPLE` are not advertised by either dialect: this release groups by columns only, and `AGGREGATE_GROUP_BY_TUPLE` gates the delegated grouping column count rather than a tuple-valued grouping form, so withholding it caps delegated grouping at one column for both. `AGGREGATE_HAVING` is not advertised by either, so Exasol sends no `having` element and applies the `HAVING` clause itself, even though both targets implement `HAVING`.
- `FN_AGG_AVG` and `FN_AGG_AVG_DISTINCT` are not advertised by either dialect. Measured on MySQL 8.4.11 and Exasol 2025.2.1, no rendering of `AVG` reproduces Exasol's value: bare `AVG(operand)` over an exact-numeric column returns MySQL's `DECIMAL` at the operand's scale plus 4 digits, an absolute error where a double's error is relative, worst measured relative error 1.0e-04; `AVG(CAST(operand AS DOUBLE))` returns a value wrong by 47 per cent for `AVG(DISTINCT)` over a `DECIMAL(19,0)` column holding values near 1e18; `CAST(SUM(operand) AS DOUBLE) / COUNT(operand)` reproduced Exasol exactly in every exact-numeric case measured but cannot help a `DOUBLE` operand. PostgreSQL's `avg` over an exact-numeric operand returns `numeric` and over a floating-point operand returns `double precision`, so it carries the same two problems in its own spelling: an exact-numeric result at a precision Exasol did not ask for, and a floating-point result whose value depends on accumulation order. Exasol therefore computes every `AVG` itself for both drivers.
- `FN_AGG_SUM` and `FN_AGG_SUM_DISTINCT` are not advertised by either dialect either, and the reason is the floating-point operand rather than the exact-numeric one. Measured on MySQL 8.4.11 and Exasol 2025.2.1 over one `DOUBLE` multiset — `1e16` once and `1` a thousand times — seeded three times under three physical row orders, MySQL's `SUM` returned `1e16`, `1.0000000000001e16`, and `1.0000000000000772e16`, while Exasol returned one value every time, the correctly rounded one. PostgreSQL's `sum` over `double precision` is order-dependent for the same reason floating-point addition is not associative, and PostgreSQL may additionally combine partial aggregates in a parallel plan, so the accumulation order is not even stable between two runs of one query. `vs-adapter/result-ordering` withholds every ordering capability from every dialect, so no rendering forces a matching order in either target.
- PostgreSQL's exact-numeric `sum` is exact and order-independent — `sum(integer)` returns `bigint` and `sum(numeric)` returns `numeric` — so a capability admitting only that operand type would be sound. None exists. Capability negotiation is all-or-nothing at the capability level: `FN_AGG_SUM` is advertised once per session and admits every numeric operand, and once Exasol delegates a shape an advertised capability covers it does not compute that fragment itself after the adapter errors. Advertising `FN_AGG_SUM` for PostgreSQL and refusing a `double precision` operand at render time would therefore turn `SELECT SUM("price") FROM vs."orders"` into a hard failure. Both `SUM` capabilities stay withheld for both dialects, and the cost is throughput alone. `vs-adapter/pushdown-dialect` owns the principle.
- This release's aggregate set is therefore the same for both dialects — `COUNT`, `COUNT(*)`, `COUNT(DISTINCT)`, `MIN`, `MAX` — and each is exact over every operand type because none of them sums.
- One question about MySQL's grouping collation is open rather than settled, and no scenario asserts an answer to it. Under `utf8mb4_0900_as_cs` MySQL compares an NFC-composed character and its NFD-decomposed equivalent as equal, measured on MySQL 8.4, while `ß` and `ss` compare unequal under the same collation. Exasol compares character values by code point. A delegated MySQL `GROUP BY` over normalization-variant values may therefore still return fewer groups than Exasol asked for. PostgreSQL's deterministic byte equality has no such gap, because byte equality is exactly code-point equality. What Exasol 2025.2.1 returns for such values was not measured, so the MySQL gap is unconfirmed rather than known, and no fixture carries a normalization-variant pair.
- `COUNT(*)` arrives as a `function_aggregate` named `COUNT` carrying no `arguments`, which is how it is told apart from `COUNT(column)`.
<!-- /DELTA:CHANGED -->

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Aggregation capabilities cover single-group and column grouping

* *GIVEN* a virtual schema, for each driver this release supports
* *WHEN* Exasol sends a `getCapabilities` request
* *THEN* the `capabilities` array MUST contain `AGGREGATE_SINGLE_GROUP`, `AGGREGATE_GROUP_BY_COLUMN`, and `SELECTLIST_EXPRESSIONS`, without which Exasol delegates no aggregate at all
* *AND* it MUST contain `FN_AGG_COUNT`, `FN_AGG_COUNT_STAR`, `FN_AGG_COUNT_DISTINCT`, `FN_AGG_MIN`, and `FN_AGG_MAX`, which are the aggregates this release delegates, and MUST NOT contain `AGGREGATE_GROUP_BY_EXPRESSION`, `AGGREGATE_GROUP_BY_TUPLE`, or `AGGREGATE_HAVING`
* *AND* it MUST NOT contain `FN_AGG_AVG`, `FN_AGG_AVG_DISTINCT`, `FN_AGG_SUM`, or `FN_AGG_SUM_DISTINCT`, withheld by both dialects — PostgreSQL included, despite its exact-numeric `sum` being exact and order-independent, because a capability is advertised once per session and admits a floating-point operand neither target sums the way Exasol does
* *AND* the two dialects MUST advertise identical aggregate capabilities, so the only capability the two sets differ on stays the join type `vs-adapter/pushdown-join` owns
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: PostgreSQL renders MIN and MAX over a character column with the C collation

* *GIVEN* a PostgreSQL `character varying` column holding the values `B`, `a`, and `C`, in a database whose cluster collation orders by the `en_US.utf8` locale
* *WHEN* Exasol pushes down `MIN` and `MAX` over that column
* *THEN* the rendered operand MUST carry `COLLATE "C"`
* *AND* the query MUST return `B` for `MIN` and `a` for `MAX`, matching what Exasol returns for the same aggregate over the same values
* *AND* a numeric or temporal operand MUST render without a collation suffix
* *AND* the same aggregate rendered without the suffix MUST return a different value against the same data, so the test proves the suffix is load-bearing rather than decorative
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: PostgreSQL renders a grouping key and a DISTINCT operand without a collation

* *GIVEN* a `pushdown` request whose `aggregationType` is `group_by`, whose `groupBy` names one PostgreSQL `character varying` column and one numeric column, and whose select list projects both grouping columns beside a `MIN` over the same character column
* *WHEN* the PostgreSQL dialect renders the statement
* *THEN* the character grouping column MUST carry no collation suffix in the `GROUP BY` clause
* *AND* it MUST carry no collation suffix on its bare select-list occurrence either, so the grouping key and that occurrence are the same expression and PostgreSQL's unconditional full-group-by rule is satisfied without a coordinated suffix
* *AND* the `MIN` over the character column MUST carry `COLLATE "C"`, because the value `MIN` returns is chosen by order rather than by equality
* *AND* a `function_aggregate` whose `distinct` field is true over a character column MUST render its operand with no collation suffix
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: Delegated PostgreSQL grouping and DISTINCT over case-only variants return Exasol's groups

* *GIVEN* a PostgreSQL virtual table and a local Exasol table holding identical character values, two of which differ only by case and two only by an accent
* *WHEN* the same `GROUP BY` over that column, and the same `COUNT(DISTINCT)` over it, are run over each
* *THEN* `EXPLAIN VIRTUAL` MUST report a generated statement whose `GROUP BY` key carries no collation suffix
* *AND* the virtual `GROUP BY` MUST return exactly the groups the local query returns, so no two values Exasol keeps apart are collapsed into one group
* *AND* the virtual `COUNT(DISTINCT)` MUST return the value the local query returns
* *AND* the query MUST execute at the source rather than failing, so the absence of a suffix is proven correct against the same fixture that proves MySQL needs one
<!-- /DELTA:NEW -->

<!-- DELTA:CHANGED -->
### Scenario: Select-list predicate over the grouped character column fails with a named error

* *GIVEN* a `pushdown` request whose `aggregationType` is `group_by`, whose `groupBy` names one character column, and whose select list carries a predicate reading that same column outside any aggregate
* *WHEN* the MySQL dialect renders the statement
* *THEN* the dialect MUST return an error naming that column
* *AND* the dialect MUST NOT emit a MySQL statement, because MySQL rejects every rendering of this shape with error 1055 under `ONLY_FULL_GROUP_BY` whatever collation the operand carries
* *AND* the same MUST hold for each of the five measured predicate kinds `predicate_less`, `predicate_is_null`, `predicate_like`, `predicate_in_constlist`, and `predicate_between`, and for a request whose select list does not project the grouping column at all
* *AND* the exclusion MUST bind the MySQL dialect alone, because it is created by the collation that dialect renders on its grouping key rather than by the capability pair, and the PostgreSQL dialect renders no such collation
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: PostgreSQL delegates a select-list predicate over its grouped character column

* *GIVEN* a `pushdown` request whose `aggregationType` is `group_by`, whose `groupBy` names one PostgreSQL `character varying` column, and whose select list carries a predicate reading that same column outside any aggregate
* *WHEN* the PostgreSQL dialect renders the statement
* *THEN* the dialect MUST produce a statement rather than an error
* *AND* the statement MUST execute at the source rather than being rejected by it
* *AND* the rows it returns MUST equal what Exasol returns for the same query executed over a local mirror of the same values
* *AND* this scenario MUST be verified against a live PostgreSQL source rather than by rendering alone, because what is asserted is that the source accepts an expression over its bare grouping column
<!-- /DELTA:NEW -->

<!-- DELTA:CHANGED -->
### Scenario: Aggregate value that overflows its declared type fails rather than truncating

* *GIVEN* a delegated aggregate whose source result exceeds the precision the adapter declared for it
* *WHEN* the scan coerces that value to the declared Exasol type
* *THEN* the scan MUST fail with an error naming the declared type and the value that did not fit
* *AND* the scan MUST NOT emit a truncated or rounded value
* *AND* the same MUST hold when the source returns the value as a decimal, as an integer, or as a string, because the PostgreSQL driver returns every `numeric` as a string and a `COUNT` as a 64-bit integer
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: Aggregation pushdown reaches PostgreSQL rather than Exasol

* *GIVEN* a virtual schema over a PostgreSQL table holding rows across two groups
* *WHEN* Exasol runs a `SELECT` grouping by one column and counting another
* *THEN* `EXPLAIN VIRTUAL` MUST report a generated statement carrying the aggregate and the `GROUP BY` clause, and the query MUST return one row per group with the correct count
* *AND* the returned values MUST equal what Exasol returns for the same aggregation executed locally over the same data, for `COUNT`, `MIN`, and `MAX` over every mapped operand type
* *AND* a query averaging or summing a mapped numeric column MUST produce a generated statement carrying no `AVG` and no `SUM`, and MUST return the value Exasol's own aggregate returns
* *AND* the same query grouping by two columns MUST produce a generated statement carrying no `GROUP BY` clause and MUST still return the correct rows, because Exasol withdraws that aggregation and groups it itself
<!-- /DELTA:NEW -->
