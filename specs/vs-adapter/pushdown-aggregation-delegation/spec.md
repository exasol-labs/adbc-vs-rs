# Feature: pushdown_aggregation_delegation

States which aggregation shapes and aggregate functions this release delegates to the source and which Exasol computes itself.

## Background

- This feature owns which aggregates and grouping shapes this release delegates, which ones Exasol computes over the rows instead, and the rule that an aggregation is delegated whole or not at all.
- Each dialect's advertised capability set lives in its `<technology>-pushdown` feature, and `vs-adapter/pushdown-dialect` owns the `BOOLEAN` declared type and the all-or-nothing negotiation rule.
- Each `<technology>-collation` feature states what its source's default collation does to a grouping key, a `DISTINCT` operand, and a `MIN` or `MAX` operand.
- `vs-adapter/pushdown-aggregation` owns how a single-group and a grouped aggregation render, and `vs-adapter/pushdown-aggregation-types` owns each aggregate's declared `EMITS` type.

## Scenarios

### Scenario: AVG and SUM are computed by Exasol rather than delegated

* *GIVEN* a virtual schema, a query averaging a mapped numeric column, and a query summing one, for each driver this release supports
* *WHEN* Exasol plans each query
* *THEN* each generated statement MUST carry no `AVG` and no `SUM`, because no dialect advertises `FN_AGG_AVG`, `FN_AGG_AVG_DISTINCT`, `FN_AGG_SUM`, or `FN_AGG_SUM_DISTINCT`, and Exasol withdraws an aggregation it cannot delegate whole
* *AND* each query MUST return the value Exasol's own `AVG` or `SUM` returns, so withholding the four capabilities changes no answer an operator reads, for any operand type
* *AND* a `SUM` over a floating-point column MUST return that same value rather than an adapter error, because the capability is withheld rather than advertised and refused at render time, which would turn `SELECT SUM("price") FROM vs."orders"` into a hard failure where release 0.1 answers it
* *AND* a `pushdown` request nonetheless carrying an `AVG` or a `SUM` MUST fail with an error naming that function, so the withheld capabilities and the renderer cannot disagree

### Scenario: Aggregation is delegated only beside SELECTLIST_EXPRESSIONS

* *GIVEN* the capability set of each dialect this release supports, and the test Exasol, on which, with `AGGREGATE_SINGLE_GROUP`, `AGGREGATE_GROUP_BY_COLUMN`, and nine `FN_AGG_*` advertised, `SELECT SUM("c"), COUNT(*) FROM vs."t" GROUP BY "d"` produced a request byte-identical to the projection-only baseline until `SELECTLIST_EXPRESSIONS` was advertised beside them, and `SELECTLIST_EXPRESSIONS` without the aggregate capabilities delegated no aggregation either
* *WHEN* the set advertises an aggregate capability
* *THEN* the set MUST also advertise `SELECTLIST_EXPRESSIONS`, because Exasol's negotiation, not any source, pairs the two, so a further dialect follows the rule without a new measurement

### Scenario: HAVING and grouping by an expression are withheld from every dialect

* *GIVEN* the capability set of each dialect this release supports, Trino's included
* *WHEN* the set is read
* *THEN* it MUST NOT advertise `AGGREGATE_HAVING`, so Exasol sends no `having` element and applies the `HAVING` clause itself, even though every target implements `HAVING`
* *AND* it MUST NOT advertise `AGGREGATE_GROUP_BY_EXPRESSION`, so this release groups by columns rather than by expressions
* *AND* both withholdings MUST be identical for every dialect, because they are framework decisions about this release's scope and independent of what any target can express: a target that would execute the withheld shape is withheld from it for the same reason as one that would not, and lifting either is a change to this release's scope rather than a driver addition
* *AND* each MUST stay withheld from the Trino dialect too, unlike `AGGREGATE_GROUP_BY_TUPLE`, because neither is confirmed to need no rendering change, so lifting either would buy a rendering change in three dialects rather than none

### Scenario: A multi-column GROUP BY is delegated only by the dialect advertising AGGREGATE_GROUP_BY_TUPLE

* *GIVEN* a query grouping a virtual table by two columns, for each driver this release supports, and the test Exasol, on which a Lua adapter probe received `GROUP BY "d"` as `aggregationType: group_by` with a one-column `groupBy`, received `GROUP BY "d", "f"` with no `aggregationType`, no `groupBy`, and a select list of two plain columns, and, once `AGGREGATE_GROUP_BY_TUPLE` was added, received a two-column and a three-column `GROUP BY` in request order
* *WHEN* Exasol plans the query
* *THEN* under the MySQL and PostgreSQL dialects, which withhold `AGGREGATE_GROUP_BY_TUPLE`, the generated statement MUST carry no `GROUP BY`, and Exasol groups the rows itself at no correctness cost
* *AND* under the Trino dialect the generated statement MUST carry a `GROUP BY` naming both columns
* *AND* the Trino dialect MUST be the only one to advertise `AGGREGATE_GROUP_BY_TUPLE`, because the capability caps how many columns a delegated `GROUP BY` names rather than enabling a tuple-valued grouping form, and every dialect renders every grouping column in the order `groupBy` gives them, so the cap is a capability boundary rather than a rendering limit
* *AND* the MySQL and PostgreSQL dialects MUST keep it withheld, because widening this release's shared floor is a scope change rather than a driver addition, and Trino's own SQL does not bear on what they advertise, as `vs-adapter/trino-pushdown` states the entry and `vs-adapter/pushdown-dialect` allows two capability sets to differ

### Scenario: Character grouping keys and aggregate operands carry no collation suffix

* *GIVEN* a delegated aggregation whose `GROUP BY` key, `DISTINCT` operand, and `MIN` or `MAX` operand are character columns, for each driver this release supports
* *WHEN* the dialect renders the statement
* *THEN* the generated statement MUST NOT carry a collation suffix on the grouping key, the `DISTINCT` operand, or the `MIN` or `MAX` operand
* *AND* the source's collation MUST decide which values form one group, which values are distinct, and which value is the minimum or the maximum

### Scenario: An aggregation Exasol cannot delegate whole is computed by Exasol

* *GIVEN* a query applying a scalar operation to a delegable aggregate, such as `COUNT("quantity") + 1`, and a query mixing `SUM` or `AVG` with `COUNT`, `MIN`, or `MAX`
* *WHEN* Exasol plans each query
* *THEN* each generated statement MUST carry no aggregate, because Exasol delegates an aggregation whole or not at all: measured on the test Exasol, `SUM("c") + 1` withdrew the aggregation entirely rather than delegating `SUM` and adding locally, because no dialect advertises a scalar function
* *AND* each query MUST return the value Exasol computes over the scanned rows, so the withdrawal costs throughput and no correctness

### Scenario: AVG stays withheld because no rendering reproduces Exasol's value

* *GIVEN* the capability set of each dialect this release supports
* *WHEN* the set is read
* *THEN* it MUST NOT advertise `FN_AGG_AVG` or `FN_AGG_AVG_DISTINCT`, because bare `AVG(operand)` does not reproduce Exasol's value: measured on the test MySQL and the test Exasol, it returns MySQL's `DECIMAL` at the operand's scale plus 4 digits over an exact-numeric column, an absolute error where a double's error is relative, with a worst measured relative error of 1.0e-04
* *AND* the dialect MUST NOT render `AVG` through a rewrite instead, because the two rewrites measured fail too: `AVG(CAST(operand AS DOUBLE))` returns a value wrong by 47 per cent for `AVG(DISTINCT)` over a `DECIMAL(19,0)` column holding values near 1e18, and `CAST(SUM(operand) AS DOUBLE) / COUNT(operand)` reproduced Exasol exactly in every exact-numeric case measured but cannot help a `DOUBLE` operand
* *AND* the PostgreSQL and Trino dialects MUST withhold both capabilities as well, because PostgreSQL's `avg` returns `numeric` over an exact-numeric operand and `double precision` over a floating-point one, and Trino's returns `double` for every numeric operand, so each carries the same two problems in its own spelling

### Scenario: SUM stays withheld because a floating-point sum depends on the source's row order

* *GIVEN* the capability set of each dialect this release supports
* *WHEN* the set is read
* *THEN* it MUST NOT advertise `FN_AGG_SUM` or `FN_AGG_SUM_DISTINCT`, because a floating-point sum at the source depends on the row order: measured on the test MySQL and the test Exasol over one `DOUBLE` multiset, `1e16` once and `1` a thousand times, seeded three times under three physical row orders, MySQL's `SUM` returned `1e16`, `1.0000000000001e16`, and `1.0000000000000772e16`, while Exasol returned one value every time, the correctly rounded one
* *AND* the PostgreSQL and Trino dialects MUST withhold both capabilities as well, because PostgreSQL's `sum` over `double precision` is order-dependent and can combine partial aggregates in a parallel plan, Trino combines partial results across workers, and `vs-adapter/result-ordering` withholds every ordering capability, so no rendering forces a matching order in any target
* *AND* an exact-numeric-only `SUM` MUST NOT be advertised either, even where the target's exact-numeric `SUM` is exact and order-independent, because withholding is all-or-nothing at the capability level: `FN_AGG_SUM` is advertised once per session and admits every numeric operand, and once Exasol delegates a shape an advertised capability covers, it does not compute that fragment itself after the adapter errors

### Scenario: Every dialect delegates the same aggregate set

* *GIVEN* the capability set of each dialect this release supports
* *WHEN* its aggregate function capabilities are read
* *THEN* they MUST be `FN_AGG_COUNT`, `FN_AGG_COUNT_STAR`, `FN_AGG_COUNT_DISTINCT`, `FN_AGG_MIN`, and `FN_AGG_MAX` in every dialect, so `COUNT`, `COUNT(*)`, `COUNT(DISTINCT)`, `MIN`, and `MAX` are the delegated aggregates, each exact over every operand type because none of them sums
