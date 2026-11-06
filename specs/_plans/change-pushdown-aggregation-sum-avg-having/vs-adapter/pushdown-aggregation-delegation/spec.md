# Feature: pushdown_aggregation_delegation

States which aggregation shapes and aggregate functions this release delegates to the source and which Exasol computes itself.

<!-- DELTA:CHANGED -->
## Background

- This feature owns which aggregates, grouping shapes, and `HAVING` clauses this release delegates, which ones Exasol computes over the rows instead, and the rule that an aggregation is delegated whole or not at all.
- Each dialect's advertised capability set lives in its `<technology>-pushdown` feature, and `vs-adapter/pushdown-dialect` owns the `BOOLEAN` declared type and the all-or-nothing negotiation rule.
- Each `<technology>-collation` feature states what its source's default collation does to a grouping key, a `DISTINCT` operand, and a `MIN` or `MAX` operand.
- `vs-adapter/pushdown-aggregation` owns how a single-group and a grouped aggregation, a `having` element, and each aggregate function render, and `vs-adapter/pushdown-aggregation-types` owns each aggregate's declared `EMITS` type.
<!-- /DELTA:CHANGED -->

## Scenarios

<!-- DELTA:REMOVED -->
### Scenario: AVG and SUM are computed by Exasol rather than delegated

* *GIVEN* a virtual schema, a query averaging a mapped numeric column, and a query summing one, for each driver this release supports
* *WHEN* Exasol plans each query
* *THEN* each generated statement MUST carry no `AVG` and no `SUM`, because no dialect advertises `FN_AGG_AVG`, `FN_AGG_AVG_DISTINCT`, `FN_AGG_SUM`, or `FN_AGG_SUM_DISTINCT`, and Exasol withdraws an aggregation it cannot delegate whole
* *AND* each query MUST return the value Exasol's own `AVG` or `SUM` returns, so withholding the four capabilities changes no answer an operator reads, for any operand type
* *AND* a `SUM` over a floating-point column MUST return that same value rather than an adapter error, because the capability is withheld rather than advertised and refused at render time, which would turn `SELECT SUM("price") FROM vs."orders"` into a hard failure where release 0.1 answers it
* *AND* a `pushdown` request nonetheless carrying an `AVG` or a `SUM` MUST fail with an error naming that function, so the withheld capabilities and the renderer cannot disagree
<!-- /DELTA:REMOVED -->

<!-- DELTA:NEW -->
### Scenario: SUM and AVG are delegated by every dialect and match Exasol over exact-numeric operands

* *GIVEN* a virtual schema for each driver this release supports, over a table holding an exact-numeric column, a `DOUBLE` column, and a grouping column across several groups, one group holding only null operands, and a local Exasol mirror table holding the same rows
* *WHEN* Exasol runs `SUM`, `SUM(DISTINCT)`, `AVG`, and `AVG(DISTINCT)` over each numeric column, once without grouping and once grouped by the grouping column
* *THEN* `EXPLAIN VIRTUAL` MUST report a generated statement carrying each delegated `SUM`, and for each `AVG` the `SUM` and `COUNT` that render it
* *AND* over the exact-numeric column the query MUST return exactly the rows Exasol returns for the same query over the mirror table, because every target sums an exact-numeric operand exactly and independently of row order, and the quotient that renders `AVG` reproduced Exasol's `AVG` in every exact-numeric case measured on the test MySQL and the test Exasol
* *AND* over the `DOUBLE` column, whose values were chosen so that every partial sum is exact in double precision, the query MUST return exactly the rows the mirror returns
* *AND* the group holding only null operands MUST return null for both functions, as the mirror does
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: A floating-point SUM or AVG follows the source's accumulation order

* *GIVEN* the capability set of each dialect this release supports, and a query summing or averaging a `DOUBLE` column
* *WHEN* the set is read and Exasol delegates the aggregate
* *THEN* the set MUST advertise `FN_AGG_SUM`, `FN_AGG_SUM_DISTINCT`, `FN_AGG_AVG`, and `FN_AGG_AVG_DISTINCT`, although a floating-point sum at the source depends on the order in which the source adds the values: measured on the test MySQL and the test Exasol over one `DOUBLE` multiset, `1e16` once and `1` a thousand times, seeded under three physical row orders, MySQL's `SUM` returned `1e16`, `1.0000000000001e16`, and `1.0000000000000772e16`, while Exasol returned the correctly rounded `1.0000000000001e16` every time
* *AND* the dialect MUST render the source's own double-precision sum, with no reordering and no compensated summation, so a delegated result MAY differ from Exasol's local result in its low-order digits, MAY differ further where large values cancel, and MAY differ between two runs of one query
* *AND* the dialect MUST NOT refuse a `DOUBLE` operand at render time instead, because a capability is advertised once per session and admits every numeric operand, and Exasol does not compute a delegated fragment itself after the adapter errors, so a refusal would turn `SELECT SUM("price") FROM vs."orders"` into a hard failure
* *AND* a `NaN` in the operand at a source that stores one MUST make the delegated `SUM` and `AVG` null, because the source's sum is `NaN` and Exasol reads a `NaN` as null, where Exasol's local computation reads that one value as null and skips it
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: SUM and AVG over a BOOLEAN or character operand are delegated and match Exasol

* *GIVEN* a virtual schema for each driver this release supports, over a table holding a grouping column, a column declared `BOOLEAN` for MySQL and PostgreSQL, and a column declared `VARCHAR` holding decimal numbers as text, an empty string, and `'5'` beside `'5.0'` in one group, and a local Exasol mirror table holding the same rows
* *WHEN* Exasol runs `SUM`, `SUM(DISTINCT)`, `AVG`, and `AVG(DISTINCT)` over the `BOOLEAN` column, over a predicate for Trino, which maps no `BOOLEAN` column, and over the `VARCHAR` column, once without grouping and once grouped by the grouping column
* *THEN* `EXPLAIN VIRTUAL` MUST report a generated statement carrying each delegated `SUM`, because Exasol delegates `SUM` and `AVG` over a column declared `BOOLEAN`, `VARCHAR`, or `CHAR`, a predicate, and a string literal as the unconverted operand, and rejects a `DATE` or `TIMESTAMP` operand before it sends a request: both measured on the test Exasol with a scratch adapter that advertises the dialects' aggregation capabilities
* *AND* each query MUST return exactly the rows Exasol returns for the same query over the mirror table, because the dialect converts the operand as Exasol converts it before the source sums it
* *AND* the dialect MUST NOT render the source's aggregate over the unconverted operand, because PostgreSQL and Trino define no `sum` over a boolean or a character type, so the query would fail where Exasol answers it
* *AND* at MySQL a character value that is not a number MAY read as its numeric prefix or as 0, where Exasol fails the query, because MySQL's conversion of a string to a number does not fail
<!-- /DELTA:NEW -->

<!-- DELTA:REMOVED -->
### Scenario: HAVING and grouping by an expression are withheld from every dialect

* *GIVEN* the capability set of each dialect this release supports, Trino's included
* *WHEN* the set is read
* *THEN* it MUST NOT advertise `AGGREGATE_HAVING`, so Exasol sends no `having` element and applies the `HAVING` clause itself, even though every target implements `HAVING`
* *AND* it MUST NOT advertise `AGGREGATE_GROUP_BY_EXPRESSION`, so this release groups by columns rather than by expressions
* *AND* both withholdings MUST be identical for every dialect, because they are framework decisions about this release's scope and independent of what any target can express: a target that would execute the withheld shape is withheld from it for the same reason as one that would not, and lifting either is a change to this release's scope rather than a driver addition
* *AND* each MUST stay withheld from the Trino dialect too, unlike `AGGREGATE_GROUP_BY_TUPLE`, because neither is confirmed to need no rendering change, so lifting either would buy a rendering change in three dialects rather than none
<!-- /DELTA:REMOVED -->

<!-- DELTA:NEW -->
### Scenario: HAVING is delegated by every dialect

* *GIVEN* a virtual schema for each driver this release supports, a local Exasol mirror table holding the same rows, and queries grouping by one column with a `HAVING` clause over `COUNT(*)`, over `SUM`, and over `AVG`
* *WHEN* Exasol plans and runs each query
* *THEN* each dialect's capability set MUST advertise `AGGREGATE_HAVING`
* *AND* `EXPLAIN VIRTUAL` MUST report a generated statement carrying the `HAVING` clause, so the source returns only the groups the clause keeps
* *AND* each query MUST return exactly the rows Exasol returns for the same query over the mirror table
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: Grouping by an expression is withheld from every dialect

* *GIVEN* the capability set of each dialect this release supports, Trino's included
* *WHEN* the set is read
* *THEN* it MUST NOT advertise `AGGREGATE_GROUP_BY_EXPRESSION`, so this release groups by columns rather than by expressions
* *AND* the withholding MUST be identical for every dialect, because it is a framework decision about this release's scope and independent of what any target can express
<!-- /DELTA:NEW -->

<!-- DELTA:CHANGED -->
### Scenario: An aggregation Exasol cannot delegate whole is computed by Exasol

* *GIVEN* a query applying a scalar operation to a delegable aggregate, such as `COUNT("quantity") + 1`, and a query mixing `STDDEV`, which no dialect advertises, with `COUNT`, `SUM`, or `AVG`
* *WHEN* Exasol plans each query
* *THEN* each generated statement MUST carry no aggregate, because Exasol delegates an aggregation whole or not at all: measured on the test Exasol, `SUM("c") + 1` withdrew the aggregation entirely rather than delegating `SUM` and adding locally, because no dialect advertises a scalar function
* *AND* each query MUST return the value Exasol computes over the scanned rows, so the withdrawal costs throughput and no correctness
<!-- /DELTA:CHANGED -->

<!-- DELTA:REMOVED -->
### Scenario: AVG stays withheld because no rendering reproduces Exasol's value

* *GIVEN* the capability set of each dialect this release supports
* *WHEN* the set is read
* *THEN* it MUST NOT advertise `FN_AGG_AVG` or `FN_AGG_AVG_DISTINCT`, because bare `AVG(operand)` does not reproduce Exasol's value: measured on the test MySQL and the test Exasol, it returns MySQL's `DECIMAL` at the operand's scale plus 4 digits over an exact-numeric column, an absolute error where a double's error is relative, with a worst measured relative error of 1.0e-04
* *AND* the dialect MUST NOT render `AVG` through a rewrite instead, because the two rewrites measured fail too: `AVG(CAST(operand AS DOUBLE))` returns a value wrong by 47 per cent for `AVG(DISTINCT)` over a `DECIMAL(19,0)` column holding values near 1e18, and `CAST(SUM(operand) AS DOUBLE) / COUNT(operand)` reproduced Exasol exactly in every exact-numeric case measured but cannot help a `DOUBLE` operand
* *AND* the PostgreSQL and Trino dialects MUST withhold both capabilities as well, because PostgreSQL's `avg` returns `numeric` over an exact-numeric operand and `double precision` over a floating-point one, and Trino's returns `double` for every numeric operand, so each carries the same two problems in its own spelling
<!-- /DELTA:REMOVED -->

<!-- DELTA:REMOVED -->
### Scenario: SUM stays withheld because a floating-point sum depends on the source's row order

* *GIVEN* the capability set of each dialect this release supports
* *WHEN* the set is read
* *THEN* it MUST NOT advertise `FN_AGG_SUM` or `FN_AGG_SUM_DISTINCT`, because a floating-point sum at the source depends on the row order: measured on the test MySQL and the test Exasol over one `DOUBLE` multiset, `1e16` once and `1` a thousand times, seeded three times under three physical row orders, MySQL's `SUM` returned `1e16`, `1.0000000000001e16`, and `1.0000000000000772e16`, while Exasol returned one value every time, the correctly rounded one
* *AND* the PostgreSQL and Trino dialects MUST withhold both capabilities as well, because PostgreSQL's `sum` over `double precision` is order-dependent and can combine partial aggregates in a parallel plan, Trino combines partial results across workers, and `vs-adapter/result-ordering` withholds every ordering capability, so no rendering forces a matching order in any target
* *AND* an exact-numeric-only `SUM` MUST NOT be advertised either, even where the target's exact-numeric `SUM` is exact and order-independent, because withholding is all-or-nothing at the capability level: `FN_AGG_SUM` is advertised once per session and admits every numeric operand, and once Exasol delegates a shape an advertised capability covers, it does not compute that fragment itself after the adapter errors
<!-- /DELTA:REMOVED -->

<!-- DELTA:CHANGED -->
### Scenario: Every dialect delegates the same aggregate set

* *GIVEN* the capability set of each dialect this release supports
* *WHEN* its aggregate function capabilities are read
* *THEN* they MUST be `FN_AGG_COUNT`, `FN_AGG_COUNT_STAR`, `FN_AGG_COUNT_DISTINCT`, `FN_AGG_MIN`, `FN_AGG_MAX`, `FN_AGG_SUM`, `FN_AGG_SUM_DISTINCT`, `FN_AGG_AVG`, and `FN_AGG_AVG_DISTINCT` in every dialect, so `COUNT`, `COUNT(*)`, `COUNT(DISTINCT)`, `MIN`, `MAX`, `SUM`, `SUM(DISTINCT)`, `AVG`, and `AVG(DISTINCT)` are the delegated aggregates
* *AND* `COUNT`, `MIN`, and `MAX` MUST be exact over every operand type, because none of them sums, and `SUM` and `AVG` MUST return Exasol's value over every exact-numeric operand, follow the source's accumulation order over a `DOUBLE` operand, and convert a `BOOLEAN` or character operand as Exasol converts it
<!-- /DELTA:CHANGED -->
