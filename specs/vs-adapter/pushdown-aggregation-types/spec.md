# Feature: pushdown_aggregation_types

Declares for each delegated aggregate the Exasol type Exasol's own compiler computed for it, so the scan emits what Exasol expects.

## Background

- This feature owns where each aggregate's declared `EMITS` type comes from, how the declared type entries are shaped, and what happens when a value does not fit its declared type.
- `vs-adapter/pushdown-dialect` and `vs-adapter/pushdown-declared-types` own the `BOOLEAN` declared type and the positional declared-type contract this feature applies to aggregates.
- `vs-adapter/pushdown-aggregation` owns how a single-group and a grouped aggregation render.
- An aggregation request carries an `aggregationType` of `single_group` or `group_by`, a `groupBy` list of columns, `function_aggregate` select-list entries, and one `selectListDataTypes` entry per select-list entry.

## Scenarios

### Scenario: Each aggregate declares the type the request supplies for it

* *GIVEN* a `pushdown` request whose select list holds `COUNT`, `COUNT(*)`, `COUNT(DISTINCT)`, `MIN`, and `MAX` entries, and whose `selectListDataTypes` carries one entry per select-list entry
* *WHEN* the adapter builds the `EMITS` clause
* *THEN* each declared type MUST be the `selectListDataTypes` entry at the same index, as `vs-adapter/pushdown-dialect` requires, because an aggregate select-list entry carries no source column type to copy
* *AND* the adapter MUST NOT compute an aggregate's type from its operand's type, because Exasol rejects a declared type that differs from the one it expects, so reading Exasol's own answer is the only source that is correct by construction
* *AND* every dialect MUST share this one declared-type rule, although their targets compute aggregate types differently

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
* *AND* a `TIMESTAMP` entry MUST carry `fractionalSecondsPrecision` 3, which the adapter renders into the `EMITS` clause as `TIMESTAMP(3)`
* *AND* the predicate's entry MUST carry `BOOLEAN`

### Scenario: Predicate over an aggregate is delegated as a BOOLEAN column

* *GIVEN* a `pushdown` request whose select list carries a predicate whose operand is a `function_aggregate`, as Exasol sends for `COUNT(*) > 0`
* *WHEN* the dialect renders the statement
* *THEN* the generated statement MUST project the predicate over the rendered aggregate
* *AND* the `EMITS` clause MUST declare that column `BOOLEAN`
* *AND* the aggregate MUST be rendered by the same aggregate renderer a bare aggregate entry uses
* *AND* a character-typed aggregate operand of that predicate MUST carry no collation suffix, so `MIN("name") = 'x'` compares under the collation the source applies to `name`

### Scenario: Aggregate value that overflows its declared type fails rather than truncating

* *GIVEN* a delegated aggregate whose source result exceeds the precision the adapter declared for it
* *WHEN* the scan coerces that value to the declared Exasol type
* *THEN* the scan MUST fail with an error naming the declared type and the value that did not fit
* *AND* the scan MUST NOT emit a truncated or rounded value
* *AND* the same MUST hold when the source returns the value as a decimal, as an integer, or as a string, because the PostgreSQL driver returns every `numeric` as a string, the Trino driver returns a `decimal` as an Arrow decimal, and every driver returns a `COUNT` as a 64-bit integer

### Scenario: MIN and MAX over a CHAR operand are declared CHAR

* *GIVEN* a PostgreSQL `character(5)` column holding `ab` and `abc`, mapped to Exasol `CHAR(5)`
* *WHEN* `MIN` and `MAX` over the column, a `GROUP BY` over it, and a `COUNT(DISTINCT)` over it are delegated
* *THEN* `MIN` and `MAX` MUST be declared `CHAR(5)` in the `EMITS` clause, as `selectListDataTypes` carries them, and return `ab` and `abc` padded to five characters
* *AND* the `GROUP BY` and the `COUNT(DISTINCT)` MUST return the groups and the count Exasol computes over a local `CHAR(5)` copy of the same values, because the source and Exasol both ignore trailing blanks between two fixed-width values
