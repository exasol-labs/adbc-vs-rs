# Feature: pushdown_dialect

Separates the query fragment Exasol delegates from the target dialect's SQL text, and gives the advertised capability set one owner — the dialect that renders it — pinned to what that dialect can express by a required invariant test.

<!-- DELTA:CHANGED -->
## Background

- This feature owns the dialect contract: one dialect per virtual schema, the invariant that every advertised capability renders, the all-or-nothing advertising rule, the `EMITS` naming and declared-type contract, column-reference rendering, and determinism.
- `vs-adapter/driver-selection` resolves one dialect for both `getCapabilities` and `pushdown`, and each technology's advertised set and SQL live in its `<technology>-pushdown` feature.
- Capability names are `virtual-schema-common-java`'s serialized forms, identical across dialects: `MainCapability` constants verbatim, `FN_PRED_<NAME>`, `FN_AGG_<NAME>`, `LITERAL_<NAME>`, and `FN_<NAME>`.
- A `pushdown` request is parsed into a dialect-neutral query model of a select list, an optional filter, an optional aggregation, and a `from` tree, named in Exasol's protocol vocabulary rather than any target's syntax, which every dialect renders.
- `vs-adapter/pushdown-aggregation` owns the aggregate and grouping shapes, including `AGGREGATE_GROUP_BY_TUPLE`, `vs-adapter/pushdown-join` owns the `join_type` discriminator, and `vs-adapter/result-ordering` owns the ordering and limiting capabilities.
<!-- /DELTA:CHANGED -->

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Every advertised capability has a rendering path

* *GIVEN* the capability set a dialect advertises, for each dialect this release supports
* *WHEN* each capability in that dialect's set is rendered through that same dialect against a request fragment that exercises it
* *THEN* every capability MUST produce a statement in that dialect's target SQL, independently of what the other dialects render, rather than reach a fallback that drops the fragment, ignores it, or returns the request unchanged, because nothing in the type system enforces the agreement between a dialect's stated set and its renderer, and an advertised capability the renderer cannot express returns wrong rows rather than an error
* *AND* the dialects the test covers MUST be enumerated from the driver identities rather than named in the test, so a release adding a dialect inherits the invariant without editing the test
* *AND* where a dialect renders one capability differently for one declared operand type, as the PostgreSQL dialect renders `MIN` and `MAX` over a `BOOLEAN` operand, the test MUST exercise that operand type as its own arm
* *AND* a composed shape a dialect cannot render, although each of its fragments renders alone, MUST fail with an error naming what could not be rendered rather than return a statement the source refuses, because this test proves one rendering path per capability, not every composition, so the feature that owns the exclusion states which dialects it binds and tests the shape
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: A withheld capability carries one of two reasons

* *GIVEN* a capability that a dialect cannot render for every operand type it admits, such as an aggregate, and a capability that a framework decision about this release's scope or transport withholds
* *WHEN* each dialect's advertised capability set is stated
* *THEN* the first MUST be absent from that dialect's set rather than present with a render-time check that refuses an operand type, which would turn a query an earlier release answered into a hard failure, because Exasol negotiates per capability rather than per operand type
* *AND* the second MUST be absent from every dialect's set, whatever its target can express, and each dialect's own feature names which of the two reasons each absence carries
* *AND* a query over a withheld capability MUST return the answer Exasol computes itself, because Exasol delegates an advertised fragment fully, never re-applies it, and does not compute it after the adapter errors, so withholding costs throughput alone
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: Each dialect's capability set is stated once

* *GIVEN* the adapter's and the scan crate's production modules
* *WHEN* each dialect's advertised capability set is read
* *THEN* it MUST come from that technology's driver declaration alone
* *AND* no production module, the build script included, MUST hold a second capability list, such as a list of valid capability names
<!-- /DELTA:NEW -->

<!-- DELTA:CHANGED -->
### Scenario: Emitted columns are named positionally

* *GIVEN* a `pushdown` request whose select list projects two columns that share a source column name
* *WHEN* the adapter builds the `EMITS` clause
* *THEN* each declared column name MUST be derived from the column's position in the select list rather than from a source column name, because only a `column` select-list entry carries a name and a join can project two columns that share one
* *AND* the declared names MUST be pairwise distinct
* *AND* the names the scan specification carries MUST equal the names the `EMITS` clause declares, so the scan UDF's emitted schema matches the declaration
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Missing declared types fail rather than being guessed

* *GIVEN* a `pushdown` request carrying a select-list entry that is not a `column`, and carrying no `selectListDataTypes`, as an Exasol older than 8.6.0, or older than 7.1.14 on the 7.x line, sends
* *WHEN* the adapter builds the response
* *THEN* the adapter MUST return an error naming `selectListDataTypes` as the missing field and the minimum Exasol version that supplies it, which is 8.6.0, or 7.1.14 on the 7.x line
* *AND* the adapter MUST NOT guess the entry's type
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Declared type outside the declarable set fails with a named error

* *GIVEN* a `selectListDataTypes` or `involvedTables[].columns[].dataType` entry naming an Exasol type outside the set an `EMITS` column may declare, such as `INTERVAL`, `GEOMETRY`, or `TIMESTAMP` carrying `withLocalTimeZone: true`
* *WHEN* the adapter reads the entry
* *THEN* the adapter MUST return an error naming the type it received
* *AND* the adapter MUST NOT substitute a type it can render, so a `TIMESTAMP WITH LOCAL TIME ZONE` entry MUST NOT be read as `TIMESTAMP`
* *AND* `CHAR` and `BOOLEAN` entries MUST NOT fail, and a `TIMESTAMP` entry MUST be read with the precision its `fractionalSecondsPrecision` field carries, or 3 when the field is absent, and declared in the `EMITS` clause as `TIMESTAMP(p)`
* *AND* the declarable set MUST equal the seven types `vs-adapter/type-mapping` lets a source column map to, which include the `CHAR` a virtual schema release 0.1 created declares, so one type set answers both which types a source column maps to and which types an `EMITS` column declares
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Declared types are read from the request positionally

* *GIVEN* a `pushdown` request carrying `selectListDataTypes`
* *WHEN* the adapter builds the `EMITS` clause
* *THEN* the declared type of each column MUST be the `selectListDataTypes` entry at the same index as its `selectList` entry, in every dialect, because the type Exasol expects is the one its own compiler computed
* *AND* the adapter MUST NOT infer a type from the expression tree while `selectListDataTypes` supplies one, because Exasol rejects any type that differs from the one it expects: measured on the test Exasol, `VARCHAR(50)` offered where `VARCHAR(100)` was expected fails with a data-type-mismatch error
* *AND* a `selectListDataTypes` array whose length differs from the `selectList` length MUST fail with an error naming both lengths, and measured on the test Exasol the array MUST be read from a projection-only request too, so the `involvedTables` fallback serves an older Exasol rather than the common path
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: BOOLEAN is declarable in the EMITS clause

* *GIVEN* a `pushdown` request whose select list carries a predicate and whose `selectListDataTypes` declares that entry `{"type":"BOOLEAN"}`
* *WHEN* the adapter builds the `EMITS` clause and the scan specification
* *THEN* the declared column MUST carry the Exasol type `BOOLEAN`, and the scan specification MUST carry the same type for that column, so the scan emits a boolean rather than a number
* *AND* the rendered `EMITS` fragment MUST be `BOOLEAN` carrying no size and no precision, because Exasol's `BOOLEAN` takes neither
* *AND* measured on the test Exasol with the advertised `FN_PRED_*` set, `i > 5`, `i IS NULL`, `i BETWEEN 1 AND 9`, `i LIKE 'a%'`, `i IN (1,2,3)`, an `AND` of two comparisons, and `COUNT(*) > 0` MUST each arrive in `selectList` declared `{"type":"BOOLEAN"}`, because `SELECTLIST_EXPRESSIONS` is advertised once per session and no capability separates a boolean select-list entry from a boolean filter operand
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: The declarable set is closed under the advertised capabilities

* *GIVEN* a virtual schema of any dialect this release supports, none of which advertises a scalar function
* *WHEN* Exasol plans a query over the schema's mapped columns and sends the `selectListDataTypes` of the select list it delegates
* *THEN* every entry MUST name a type of the declarable set, so nothing reaches the `EMITS` clause outside it
* *AND* the delegated entries MUST match these shapes, measured on the test Exasol: `SUM("c") + 1` withdraws the whole aggregation rather than delegating `SUM` alone, a predicate placed in `selectList` is declared `BOOLEAN`, `COUNT` yields `DECIMAL(18,0)`, and `MIN` and `MAX` yield their operand's own type
* *AND* an entry naming a type outside the set MUST still fail with a named error, as the scenario for a declared type outside the declarable set states
* *AND* every dialect MUST advertise `SELECTLIST_EXPRESSIONS`, because Exasol composes a select-list expression only from capabilities the adapter advertises, and measured on the test Exasol, `SELECTLIST_EXPRESSIONS` alone yields plain columns while the aggregate capabilities alone delegate nothing
<!-- /DELTA:NEW -->

<!-- DELTA:CHANGED -->
### Scenario: Declared type shapes the request carries are read without loss

* *GIVEN* a `selectListDataTypes` entry of type `TIMESTAMP` carrying `fractionalSecondsPrecision: 3`, and one of type `VARCHAR` carrying a size and no `characterSet`, both shapes measured on the test Exasol
* *WHEN* the adapter reads each entry
* *THEN* the `TIMESTAMP` entry MUST declare `TIMESTAMP(3)`, the precision its `fractionalSecondsPrecision` field carries, because Exasol compares the result's column types against the virtual table's stored metadata for exact equality, including a `TIMESTAMP`'s fractional-second precision, and rejects the statement on any difference
* *AND* the `VARCHAR` entry MUST declare its size with character set `UTF8`, because `selectListDataTypes` omits the `characterSet` field that `involvedTables[].columns[].dataType` carries
* *AND* neither MUST fail for carrying a field the adapter does not use
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Comparison operands are read by position, not by assumed role

* *GIVEN* a `predicate_less` whose `left` operand is a literal and whose `right` operand is a column, as Exasol emits for `column > literal`, and a `predicate_lessequal` of the same shape, as Exasol emits for `column >= literal`
* *WHEN* each dialect renders each predicate
* *THEN* the rendered comparison MUST keep the operands in the order the request gave them
* *AND* the adapter MUST NOT assume the `left` operand is a column reference, because Exasol normalizes `>` and `>=` by swapping the operands, so no `predicate_greater` type exists: measured on the test Exasol, `i > 5` arrives as `predicate_less` whose `left` is the literal `5` and whose `right` is the column `i`
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: A column reference renders through its read expression in every position

* *GIVEN* `pushdown` requests referencing one column whose `adapterNotes` record a source type its technology reads through a conversion, such as a PostgreSQL `uuid` column, in the select list, a `WHERE` comparison, a `GROUP BY` key, a `MIN` operand, a `COUNT(DISTINCT)` operand, and an equi-join condition
* *WHEN* the dialect renders each request
* *THEN* each column reference of the dialect-neutral query model MUST carry the column's declared Exasol type and the source type its `adapterNotes` recorded, and every reference MUST render as the one read expression the column's technology names for that source type and declared type, such as `CAST("uid" AS TEXT)`
* *AND* a grouped column MUST render as its read expression in `GROUP BY`, and its select-list occurrence and a select-list predicate reading it MUST render in a form the source accepts under its full-group-by rule, which `vs-adapter/mysql-collation` states for MySQL
* *AND* a column whose source type its technology reads without a conversion, and a column whose `adapterNotes` record no source type, MUST render as the bare quoted column
* *AND* the read expression MUST yield the value the scan emits for the declared type, so a delegated operation over the column acts on that value
<!-- /DELTA:NEW -->
