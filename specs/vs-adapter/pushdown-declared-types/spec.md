# Feature: pushdown_declared_types

Names each emitted column positionally and reads its declared Exasol type from the request, so the `EMITS` clause carries the types Exasol expects.

## Background

- This feature owns the `EMITS` naming and declared-type contract for every dialect: positional column names, where each declared type is read from, how a missing or unsupported type fails, and the `BOOLEAN` declared type.
- A `pushdown` request carries a `selectListDataTypes` entry for each select-list entry, and each select-list entry has a kind recognized by its type field.
- `vs-adapter/pushdown-dialect` owns the dialect contract: one dialect per virtual schema, the advertised capability invariant, the all-or-nothing rule, column-reference rendering, and determinism.
- `vs-adapter/pushdown-aggregation-types` applies this contract to aggregate select-list entries.

## Scenarios

### Scenario: Emitted columns are named positionally

* *GIVEN* a `pushdown` request whose select list projects two columns that share a source column name
* *WHEN* the adapter builds the `EMITS` clause
* *THEN* each declared column name MUST be derived from the column's position in the select list rather than from a source column name, because only a `column` select-list entry carries a name and a join can project two columns that share one
* *AND* the declared names MUST be pairwise distinct
* *AND* the names the scan specification carries MUST equal the names the `EMITS` clause declares, so the scan UDF's emitted schema matches the declaration

### Scenario: Declared types are read from the request positionally

* *GIVEN* a `pushdown` request carrying `selectListDataTypes`
* *WHEN* the adapter builds the `EMITS` clause
* *THEN* the declared type of each column MUST be the `selectListDataTypes` entry at the same index as its `selectList` entry, in every dialect, because the type Exasol expects is the one its own compiler computed
* *AND* the adapter MUST NOT infer a type from the expression tree while `selectListDataTypes` supplies one, because Exasol rejects any type that differs from the one it expects: measured on the test Exasol, `VARCHAR(50)` offered where `VARCHAR(100)` was expected fails with a data-type-mismatch error
* *AND* a `selectListDataTypes` array whose length differs from the `selectList` length MUST fail with an error naming both lengths, and measured on the test Exasol the array MUST be read from a projection-only request too, so the `involvedTables` fallback serves an older Exasol rather than the common path

### Scenario: Missing declared types fail rather than being guessed

* *GIVEN* a `pushdown` request carrying a select-list entry that is not a `column`, and carrying no `selectListDataTypes`, as an Exasol older than 8.6.0, or older than 7.1.14 on the 7.x line, sends
* *WHEN* the adapter builds the response
* *THEN* the adapter MUST return an error naming `selectListDataTypes` as the missing field and the minimum Exasol version that supplies it, which is 8.6.0, or 7.1.14 on the 7.x line
* *AND* the adapter MUST NOT guess the entry's type

### Scenario: Declared type source for each select-list state

* *GIVEN* a `pushdown` request carrying no `selectListDataTypes`
* *WHEN* the adapter builds the `EMITS` clause
* *THEN* a request carrying no `selectList` field MUST succeed, declaring one column per mapped column of the involved table and taking each declared type from that table's `involvedTables[].columns[].dataType` entry
* *AND* a request whose `selectList` is an empty array MUST succeed, declaring the single constant column `vs-adapter/pushdown-projection` requires with the Exasol type `DECIMAL(1,0)` that scenario names
* *AND* a request whose `selectList` holds `column` entries exclusively MUST succeed, taking each declared type from `involvedTables[].columns[].dataType`

### Scenario: Declared type outside the declarable set fails with a named error

* *GIVEN* a `selectListDataTypes` or `involvedTables[].columns[].dataType` entry naming an Exasol type outside the set an `EMITS` column may declare, such as `INTERVAL`, `GEOMETRY`, or `TIMESTAMP` carrying `withLocalTimeZone: true`
* *WHEN* the adapter reads the entry
* *THEN* the adapter MUST return an error naming the type it received
* *AND* the adapter MUST NOT substitute a type it can render, so a `TIMESTAMP WITH LOCAL TIME ZONE` entry MUST NOT be read as `TIMESTAMP`
* *AND* `CHAR` and `BOOLEAN` entries MUST NOT fail, and a `TIMESTAMP` entry MUST be read with the precision its `fractionalSecondsPrecision` field carries, or 3 when the field is absent, and declared in the `EMITS` clause as `TIMESTAMP(p)`
* *AND* the declarable set MUST equal the seven types `vs-adapter/type-mapping` lets a source column map to, which include the `CHAR` a virtual schema release 0.1 created declares, so one type set answers both which types a source column maps to and which types an `EMITS` column declares

### Scenario: BOOLEAN is declarable in the EMITS clause

* *GIVEN* a `pushdown` request whose select list carries a predicate and whose `selectListDataTypes` declares that entry `{"type":"BOOLEAN"}`
* *WHEN* the adapter builds the `EMITS` clause and the scan specification
* *THEN* the declared column MUST carry the Exasol type `BOOLEAN`, and the scan specification MUST carry the same type for that column, so the scan emits a boolean rather than a number
* *AND* the rendered `EMITS` fragment MUST be `BOOLEAN` carrying no size and no precision, because Exasol's `BOOLEAN` takes neither
* *AND* measured on the test Exasol with the advertised `FN_PRED_*` set, `i > 5`, `i IS NULL`, `i BETWEEN 1 AND 9`, `i LIKE 'a%'`, `i IN (1,2,3)`, an `AND` of two comparisons, and `COUNT(*) > 0` MUST each arrive in `selectList` declared `{"type":"BOOLEAN"}`, because `SELECTLIST_EXPRESSIONS` is advertised once per session and no capability separates a boolean select-list entry from a boolean filter operand

### Scenario: Select-list entry kinds are recognized by their type field

* *GIVEN* `pushdown` requests whose select list carries a `column` entry, a `function_aggregate` entry, and a `predicate_less` entry
* *WHEN* the adapter parses each request
* *THEN* each entry's kind MUST be taken from its `type` field rather than from which other fields it carries
* *AND* an entry whose `type` this release does not render MUST fail with an error naming that `type`
* *AND* such an entry MUST NOT be read as a column reference, because a select list is no longer columns only once `SELECTLIST_EXPRESSIONS` is advertised

### Scenario: Declared type shapes the request carries are read without loss

* *GIVEN* a `selectListDataTypes` entry of type `TIMESTAMP` carrying `fractionalSecondsPrecision: 3`, and one of type `VARCHAR` carrying a size and no `characterSet`, both shapes measured on the test Exasol
* *WHEN* the adapter reads each entry
* *THEN* the `TIMESTAMP` entry MUST declare `TIMESTAMP(3)`, the precision its `fractionalSecondsPrecision` field carries, because Exasol compares the result's column types against the virtual table's stored metadata for exact equality, including a `TIMESTAMP`'s fractional-second precision, and rejects the statement on any difference
* *AND* the `VARCHAR` entry MUST declare its size with character set `UTF8`, because `selectListDataTypes` omits the `characterSet` field that `involvedTables[].columns[].dataType` carries
* *AND* neither MUST fail for carrying a field the adapter does not use
