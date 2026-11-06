# Feature: pushdown_aggregation_types

Declares for each delegated aggregate the Exasol type Exasol's own compiler computed for it, so the scan emits what Exasol expects.

## Background

- This feature owns where each aggregate's declared `EMITS` type comes from, how the declared type entries are shaped, and what happens when a value does not fit its declared type.
- `vs-adapter/pushdown-dialect` and `vs-adapter/pushdown-declared-types` own the `BOOLEAN` declared type and the positional declared-type contract this feature applies to aggregates.
- `vs-adapter/pushdown-aggregation` owns how a single-group and a grouped aggregation render.
- An aggregation request carries an `aggregationType` of `single_group` or `group_by`, a `groupBy` list of columns, `function_aggregate` select-list entries, and one `selectListDataTypes` entry per select-list entry.

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Each aggregate declares the type the request supplies for it

* *GIVEN* a `pushdown` request whose select list holds `COUNT`, `COUNT(*)`, `COUNT(DISTINCT)`, `MIN`, `MAX`, `SUM`, `SUM(DISTINCT)`, `AVG`, and `AVG(DISTINCT)` entries, and whose `selectListDataTypes` carries one entry per select-list entry
* *WHEN* the adapter builds the `EMITS` clause
* *THEN* each declared type MUST be the `selectListDataTypes` entry at the same index, as `vs-adapter/pushdown-dialect` requires, because an aggregate select-list entry carries no source column type to copy
* *AND* the adapter MUST NOT compute an aggregate's type from its operand's type, because Exasol rejects a declared type that differs from the one it expects, so reading Exasol's own answer is the only source that is correct by construction
* *AND* every dialect MUST share this one declared-type rule, although their targets compute aggregate types differently
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Exasol's aggregate types match the values this release was measured against

* *GIVEN* an Exasol query aggregating a mapped `DECIMAL(12,4)` column, a mapped `DECIMAL(30,2)` column, a mapped `DOUBLE` column, and a mapped `VARCHAR(100)` column
* *WHEN* the `selectListDataTypes` Exasol sends is read back
* *THEN* `SUM` over `DECIMAL(12,4)` MUST carry `DECIMAL(23,4)` and `SUM` over `DECIMAL(30,2)` MUST carry `DECIMAL(36,2)`, matching `DECIMAL(min(p + 11, 36), s)`
* *AND* `AVG` MUST carry `DOUBLE`, `SUM` over the `DOUBLE` column MUST carry `DOUBLE`, and `SUM(DISTINCT)` over the `DECIMAL(12,4)` column MUST carry `DECIMAL(23,4)`
* *AND* `COUNT`, `COUNT(*)`, and `COUNT(DISTINCT)` MUST carry `DECIMAL(18,0)`, and `MIN` and `MAX` MUST carry the operand's own type
* *AND* a departure from these values MUST fail the test rather than be absorbed, because the declared type Exasol accepts is the type Exasol sent, and Exasol MUST accept the `EMITS` clause the shipped dialects declare for a delegated `SUM` and `AVG`, so a query delegating either returns its rows rather than a declared-type error
<!-- /DELTA:CHANGED -->
