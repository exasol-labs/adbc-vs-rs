# Feature: pushdown_declared_types

Names each emitted column positionally and reads its declared Exasol type from the request, so the `EMITS` clause carries the types Exasol expects.

## Background

- This feature owns the `EMITS` naming and declared-type contract for every dialect: positional column names, where each declared type is read from, how a missing or unsupported type fails, and the `BOOLEAN` declared type.
- A `pushdown` request carries a `selectListDataTypes` entry for each select-list entry, and each select-list entry has a kind recognized by its type field.
- `vs-adapter/pushdown-dialect` owns the dialect contract: one dialect per virtual schema, the advertised capability invariant, the all-or-nothing rule, column-reference rendering, and determinism.
- `vs-adapter/pushdown-aggregation-types` applies this contract to aggregate select-list entries.


## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Declared type outside the declarable set fails with a named error

* *GIVEN* a `selectListDataTypes` or `involvedTables[].columns[].dataType` entry naming an Exasol type outside the set an `EMITS` column may declare, such as `INTERVAL YEAR TO MONTH`, `GEOMETRY`, or `TIMESTAMP` carrying `withLocalTimeZone: true`
* *WHEN* the adapter reads the entry
* *THEN* the adapter MUST return an error naming the type it received
* *AND* the adapter MUST NOT substitute a type it can render, so a `TIMESTAMP WITH LOCAL TIME ZONE` entry MUST NOT be read as `TIMESTAMP`
* *AND* a `selectListDataTypes` entry declaring `INTERVAL DAY TO SECOND` MUST NOT fail, and MUST be emitted and converted as `vs-adapter/pushdown-scalar-function` states, because a delegated `TIMESTAMP` difference yields it
* *AND* `CHAR` and `BOOLEAN` entries MUST NOT fail, and a `TIMESTAMP` entry MUST be read with the precision its `fractionalSecondsPrecision` field carries, or 3 when the field is absent, and declared in the `EMITS` clause as `TIMESTAMP(p)`
* *AND* the declarable set MUST equal the seven types `vs-adapter/type-mapping` lets a source column map to, which include the `CHAR` a virtual schema release 0.1 created declares, so one type set answers both which types a source column maps to and which types an `EMITS` column declares
<!-- /DELTA:CHANGED -->
