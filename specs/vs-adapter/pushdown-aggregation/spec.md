# Feature: pushdown_aggregation

Delegates a single-group or `GROUP BY` aggregation to the source, declaring for each aggregate the Exasol type Exasol's own compiler computed for it.

## Background

- This feature owns what an aggregation fragment is, for every dialect: which elements are rejected, how a single-group and a grouped aggregation render, where each aggregate's declared `EMITS` type comes from, and which aggregates this release delegates at all. Which capabilities a dialect advertises is that dialect's own fact and lives in its `<technology>-pushdown` feature. Where a dialect renders a collation suffix — on a character `GROUP BY` key, on a `DISTINCT` operand, on a `MIN` or `MAX` operand — is also that dialect's own fact and lives in its `<technology>-collation` feature.
- Aggregation delegation requires `SELECTLIST_EXPRESSIONS`, for every dialect. Measured on Exasol 2025.2.1, the aggregate capabilities alone delegate nothing: `SELECT SUM("c"), COUNT(*) FROM vs."t" GROUP BY "d"` produced a request byte-identical to the projection-only baseline until `SELECTLIST_EXPRESSIONS` was advertised beside them. `vs-adapter/pushdown-dialect` owns that capability and the `BOOLEAN` declared type it brings with it. The measurement is a property of Exasol's negotiation rather than of any source, so it holds for a further dialect without being re-measured against that source.
- `AGGREGATE_HAVING` is advertised by no dialect, so Exasol sends no `having` element and applies the `HAVING` clause itself, even though every target implements `HAVING`. A request carrying the element nonetheless is rejected rather than answered with a statement that drops it. `vs-adapter/result-ordering` owns the matching rejection for `orderBy` and `limit`.
- `AGGREGATE_GROUP_BY_EXPRESSION` is advertised by no dialect: this release groups by columns rather than by expressions, for every target.
- `AGGREGATE_GROUP_BY_TUPLE` is advertised by the Trino dialect and by no other, and Trino is the one exception to the withholdings this feature states. The capability gates how many columns a delegated `GROUP BY` may name rather than a tuple-valued grouping form, so withholding it caps delegated grouping at one column for the dialect that withholds it. Measured on Exasol 2025.2.1 with a throwaway Lua adapter probe: `GROUP BY "d"` arrived as `aggregationType: group_by` with a `groupBy` naming one column, while `GROUP BY "d", "f"` arrived carrying no `aggregationType` and no `groupBy` at all, its select list degraded to two plain columns. Under the MySQL and PostgreSQL dialects a multi-column `GROUP BY` is therefore withdrawn whole and Exasol groups it itself, at no correctness cost; under the Trino dialect it is delegated.
- The reason for the exception is that this one capability costs no dialect a rendering change. The renderer renders every grouping column a request carries, in the order `groupBy` gives them, in every dialect, so the cap the other two keep is a capability boundary rather than a rendering limit and lifting it for a third dialect adds no renderer code. `vs-adapter/trino-pushdown` states the entry and `vs-adapter/pushdown-dialect` owns the rule that a difference between two sets is legitimate rather than a defect. The two sets that withhold it keep it withheld here: widening this release's shared floor is a scope change rather than a driver addition, and nothing about Trino's own SQL bears on what MySQL or PostgreSQL advertises.
- The remaining withholdings — `AGGREGATE_GROUP_BY_EXPRESSION` and `AGGREGATE_HAVING` — are framework decisions about this release's scope, identical for every dialect including Trino's, and independent of what any target can express. A target that would execute the withheld shape is withheld from it for the same reason as one that would not, and lifting either is a change to this release's scope rather than a driver addition. Neither is in `AGGREGATE_GROUP_BY_TUPLE`'s position: neither is confirmed to need no rendering change, so lifting either would buy a rendering change in three dialects rather than none.
- `aggregationType` is `single_group` when the query aggregates without grouping and `group_by` when it groups. `single_group` renders no `GROUP BY` clause, in every dialect.
- An aggregate select-list entry carries no source column type to copy, so its declared `EMITS` type comes from `pushdownRequest.selectListDataTypes` at the same index, as `vs-adapter/pushdown-dialect` requires. Exasol rejects a declared type that differs from the one it expects, so reading Exasol's own answer is the only source that is correct by construction — and it is what lets every dialect share one declared-type rule despite their targets computing aggregate types differently.
- Measured on Exasol 2025.2.1, those types follow Exasol's aggregate rules: `SUM` over `DECIMAL(p,s)` yields `DECIMAL(min(p + 11, 36), s)`, so `SUM` over `DECIMAL(12,4)` yields `DECIMAL(23,4)` and `SUM` over `DECIMAL(30,2)` yields `DECIMAL(36,2)`; `SUM` over `DOUBLE` yields `DOUBLE`; `SUM(DISTINCT DECIMAL(12,4))` yields `DECIMAL(23,4)`, the same as the non-distinct form; `AVG` yields `DOUBLE` for every numeric operand; `COUNT`, `COUNT(*)`, and `COUNT(DISTINCT)` yield `DECIMAL(18,0)`; `MIN` and `MAX` yield the operand's own type. These values are asserted so a change in what Exasol sends is caught rather than absorbed, not used as a substitute for reading the field.
- Aggregate function names arrive in mixed case. Measured on Exasol 2025.2.1, one request carried `sum`, `min`, and `max` in lower case beside `AVG` and `COUNT` in upper case, so a name MUST be matched case-insensitively and the generated statement MUST spell it in the target's canonical upper case whatever case arrived. Every target this release supports accepts `COUNT`, `MIN`, and `MAX` under that spelling.
- `COUNT(*)` arrives as a `function_aggregate` named `COUNT` carrying no `arguments`, which is how it is told apart from `COUNT(column)`. `DISTINCT` arrives as a `distinct` field on the aggregate and renders inside it, before the argument.
- A select-list predicate may wrap an aggregate: `COUNT(*) > 0` arrives as a predicate whose operand is a `function_aggregate`, declared `BOOLEAN`. An aggregate operand is therefore not always the direct child of the select-list entry, and the aggregate renderer is the same one a bare aggregate entry uses.
- Whether an aggregate operand is character-typed is read from the aggregate itself: `MIN` and `MAX` carry their argument's type, and `COUNT` and `SUM` are numeric. A select-list predicate's declared type is `BOOLEAN` and carries no type for its operand, so no declared type answers the question. The answer chooses a collation rather than a declared `EMITS` type, so `vs-adapter/pushdown-dialect`'s rule that a declared type is read and never inferred is untouched; which collation follows, and whether any does, is per dialect.
- An aggregation is delegated whole or not at all. Measured on Exasol 2025.2.1, `SUM("c") + 1` withdraws the aggregation entirely rather than delegating `SUM` and adding locally, because no dialect advertises a scalar function. A query mixing an aggregate with a scalar operation therefore scans the source and Exasol aggregates itself, at no correctness cost.
- `FN_AGG_AVG` and `FN_AGG_AVG_DISTINCT` are advertised by no dialect. Measured on MySQL 8.4.11 and Exasol 2025.2.1, no rendering of `AVG` reproduces Exasol's value: bare `AVG(operand)` over an exact-numeric column returns MySQL's `DECIMAL` at the operand's scale plus 4 digits, an absolute error where a double's error is relative, worst measured relative error 1.0e-04; `AVG(CAST(operand AS DOUBLE))` returns a value wrong by 47 per cent for `AVG(DISTINCT)` over a `DECIMAL(19,0)` column holding values near 1e18; `CAST(SUM(operand) AS DOUBLE) / COUNT(operand)` reproduced Exasol exactly in every exact-numeric case measured but cannot help a `DOUBLE` operand. PostgreSQL's `avg` returns `numeric` over an exact-numeric operand and `double precision` over a floating-point one, and Trino's returns `double` for every numeric operand, so each carries the same two problems in its own spelling. Exasol therefore computes every `AVG` itself, for every dialect.
- `FN_AGG_SUM` and `FN_AGG_SUM_DISTINCT` are advertised by no dialect either, and the reason is the floating-point operand rather than the exact-numeric one. Measured on MySQL 8.4.11 and Exasol 2025.2.1 over one `DOUBLE` multiset — `1e16` once and `1` a thousand times — seeded three times under three physical row orders, MySQL's `SUM` returned `1e16`, `1.0000000000001e16`, and `1.0000000000000772e16`, while Exasol returned one value every time, the correctly rounded one. PostgreSQL's `sum` over `double precision` is order-dependent for the same reason floating-point addition is not associative, and PostgreSQL may additionally combine partial aggregates in a parallel plan; Trino distributes an aggregation across workers and combines partial results by construction, so its accumulation order for the same operand is not stable between two runs of one query either. `vs-adapter/result-ordering` withholds every ordering capability from every dialect, so no rendering forces a matching order in any target.
- Withholding is all-or-nothing at the capability level, which is why an exact-numeric-only `SUM` is not advertised even where the target's exact-numeric `SUM` is exact and order-independent. `FN_AGG_SUM` is advertised once per session and admits every numeric operand, and once Exasol delegates a shape an advertised capability covers it does not compute that fragment itself after the adapter errors. Advertising it and refusing a floating-point operand at render time would turn `SELECT SUM("price") FROM vs."orders"` into a hard failure where release 0.1 answers it. Both pairs therefore stay withheld for every dialect, and the cost is throughput alone: a query mixing `SUM` or `AVG` with `COUNT`, `MIN`, or `MAX` scans the source and Exasol aggregates all of it. `vs-adapter/pushdown-dialect` owns the principle.
- This release's delegated aggregate set is therefore the same for every dialect — `COUNT`, `COUNT(*)`, `COUNT(DISTINCT)`, `MIN`, and `MAX` — and each is exact over every operand type because none of them sums.
- Every character operand of a delegated `MIN` or `MAX` is Exasol `VARCHAR`, and Exasol pads no `VARCHAR`. `vs-adapter/type-mapping` maps no source column to a blank-padded Exasol type, so the padding question is settled upstream and this feature carries no padding rule of its own.
- An aggregate the dialect does not render fails with an error naming the function rather than rendering the aggregate's argument alone, because Exasol would then aggregate nothing and return the wrong value. A delegated aggregate whose source value exceeds the precision the adapter declared for it fails at coercion rather than being truncated or rounded.

## Scenarios

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
* *AND* this requirement MUST be verified by a unit test over a constructed request for every dialect, because a dialect withholding `AGGREGATE_GROUP_BY_TUPLE` is sent no `groupBy` naming more than one column and a constructed request is the only way to reach its renderer at all, and it MUST additionally be verified end to end under the Trino dialect, because that dialect advertises `AGGREGATE_GROUP_BY_TUPLE` and Exasol therefore sends it a multi-column `groupBy` rather than withdrawing the grouping

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
* *AND* a character-typed aggregate operand of that predicate MUST carry whatever suffix its dialect renders for an equality operand, so `MIN("name") = 'x'` compares the way Exasol compares

### Scenario: AVG and SUM are computed by Exasol rather than delegated

* *GIVEN* a virtual schema, a query averaging a mapped numeric column, and a query summing one, for each driver this release supports
* *WHEN* Exasol plans each query
* *THEN* each generated statement MUST carry no `AVG` and no `SUM`, because no dialect advertises either function's capabilities and Exasol withdraws an aggregation it cannot delegate whole
* *AND* each query MUST return the value Exasol's own `AVG` or `SUM` returns, so withholding the four capabilities MUST change no answer an operator reads, for any operand type
* *AND* a `SUM` over a floating-point column MUST return that same value rather than an adapter error, because the capability is withheld rather than advertised and refused at render time
* *AND* a `pushdown` request nonetheless carrying an `AVG` or a `SUM` MUST fail with an error naming that function, so the withheld capabilities and the renderer cannot disagree

### Scenario: Unrenderable aggregate function fails with a named error

* *GIVEN* a `pushdown` request whose select list carries an aggregate function the dialect does not render
* *WHEN* the adapter builds the response
* *THEN* the adapter MUST return an error
* *AND* the error message MUST name the aggregate function it received
* *AND* the adapter MUST NOT render the aggregate's argument alone, because Exasol would then aggregate nothing and return the wrong value

### Scenario: Aggregate value that overflows its declared type fails rather than truncating

* *GIVEN* a delegated aggregate whose source result exceeds the precision the adapter declared for it
* *WHEN* the scan coerces that value to the declared Exasol type
* *THEN* the scan MUST fail with an error naming the declared type and the value that did not fit
* *AND* the scan MUST NOT emit a truncated or rounded value
* *AND* the same MUST hold when the source returns the value as a decimal, as an integer, or as a string, because the PostgreSQL driver returns every `numeric` as a string, the Trino driver returns a `decimal` as an Arrow decimal, and every driver returns a `COUNT` as a 64-bit integer
