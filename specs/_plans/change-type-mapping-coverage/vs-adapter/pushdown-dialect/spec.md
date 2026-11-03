# Feature: pushdown_dialect

Separates the query fragment Exasol delegates from the target dialect's SQL text, and gives the advertised capability set one owner — the dialect that renders it — pinned to what that dialect can express by a required invariant test.

<!-- DELTA:CHANGED -->
## Background

- This feature owns the dialect contract in the abstract: one dialect per virtual schema, the invariant that every advertised capability renders, the all-or-nothing rule that governs what may be advertised, the `EMITS` naming and declared-type contract, how a column reference renders, and determinism. `vs-adapter/driver-selection` resolves the dialect for both `getCapabilities` and `pushdown` from one field. Each technology's set, stated by hand in the capability names of `virtual-schema-common-java` (`FN_PRED_<NAME>`, `FN_AGG_<NAME>`, `LITERAL_<NAME>`, `FN_<NAME>`), and its SQL live in its `<technology>-pushdown` feature.
- Exasol delegates an advertised fragment fully and never re-applies it, and it negotiates per capability rather than per operand type, so a capability is either renderable by its dialect for every operand type it admits or absent from that dialect's set, and withholding costs throughput alone. A composed shape a dialect cannot render fails with an error naming what could not be rendered, rather than a statement the source refuses. A `pushdown` request is parsed into a dialect-neutral query model whose column references carry their declared Exasol type and the source type their column's `adapterNotes` recorded.
- Exasol compares the result's column types against the virtual table's stored metadata for exact equality, including a `TIMESTAMP`'s fractional-second precision. `pushdownRequest.selectListDataTypes`, supplied from Exasol 8.6.0 and from 7.1.14 on the 7.x line, carries the expected type of every select-list entry positionally, and the `EMITS` clause names its columns positionally because only a `column` entry carries a name and a join may project two columns that share one.
- One type set answers both questions: the seven types `vs-adapter/type-mapping` lets a source column map to are the types an `EMITS` column may declare, and they include the `CHAR` a virtual schema release 0.1 created declares. The set is closed under the advertised capabilities because no dialect advertises a scalar function: measured on the test Exasol, `SUM("c") + 1` withdraws the whole aggregation, a predicate placed in `selectList` is declared `BOOLEAN`, `COUNT` yields `DECIMAL(18,0)`, and `MIN` and `MAX` yield their operand's own type, so nothing reaches the `EMITS` clause outside the set.
- Request shapes measured on the test Exasol: `>` and `>=` arrive as `predicate_less` and `predicate_lessequal` with the operands swapped, so the `left` operand is not reliably the column; the join `from` element spells its discriminator `join_type` in snake case; and `AGGREGATE_GROUP_BY_TUPLE` gates how many columns a delegated `GROUP BY` may name rather than grouping by a tuple-valued expression.
<!-- /DELTA:CHANGED -->

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Every advertised capability has a rendering path

* *GIVEN* the capability set a dialect advertises, for each dialect this release supports
* *WHEN* each capability in that dialect's set is rendered through that same dialect against a request fragment that exercises it
* *THEN* every capability MUST produce a statement in that dialect's target SQL
* *AND* no capability MUST reach a fallback that drops the fragment, ignores it, or returns the request unchanged, and the dialects the test covers MUST be enumerated from the driver identities rather than named in the test
* *AND* the test MUST be read as proving that each advertised capability has one rendering path in its own dialect, not that every composition of two capabilities renders, so a composed shape a dialect rejects needs its own test in the feature that owns it
* *AND* an aggregate a dialect cannot render for every operand type its capability would admit MUST be absent from that dialect's set, rather than present with a render-time check that refuses an operand type, and where a dialect renders one capability differently for one declared operand type, as the PostgreSQL dialect renders `MIN` and `MAX` over a `BOOLEAN` operand, the test MUST exercise that operand type as its own arm
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Declared type outside the declarable set fails with a named error

* *GIVEN* a `selectListDataTypes` or `involvedTables[].columns[].dataType` entry naming an Exasol type outside the set an `EMITS` column may declare, such as `INTERVAL`, `GEOMETRY`, or `TIMESTAMP` carrying `withLocalTimeZone: true`
* *WHEN* the adapter reads the entry
* *THEN* the adapter MUST return an error naming the type it received
* *AND* the adapter MUST NOT substitute a type it can render, so a `TIMESTAMP WITH LOCAL TIME ZONE` entry MUST NOT be read as `TIMESTAMP`
* *AND* `CHAR` and `BOOLEAN` entries MUST NOT fail, and a `TIMESTAMP` entry MUST be read with the precision its `fractionalSecondsPrecision` field carries, or 3 when the field is absent, and declared in the `EMITS` clause as `TIMESTAMP(p)`
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Declared type shapes the request carries are read without loss

* *GIVEN* a `selectListDataTypes` entry of type `TIMESTAMP` carrying `fractionalSecondsPrecision: 3`, and one of type `VARCHAR` carrying a size and no `characterSet`, both shapes measured on the test Exasol
* *WHEN* the adapter reads each entry
* *THEN* the `TIMESTAMP` entry MUST declare `TIMESTAMP(3)`, the precision its `fractionalSecondsPrecision` field carries
* *AND* the `VARCHAR` entry MUST declare its size with character set `UTF8`, because `selectListDataTypes` omits the `characterSet` field that `involvedTables[].columns[].dataType` carries
* *AND* neither MUST fail for carrying a field the adapter does not use
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: A column reference renders through its read expression in every position

* *GIVEN* `pushdown` requests referencing one column whose `adapterNotes` record a source type its technology reads through a conversion, such as a PostgreSQL `uuid` column, in the select list, a `WHERE` comparison, a `GROUP BY` key, a `MIN` operand, a `COUNT(DISTINCT)` operand, and an equi-join condition
* *WHEN* the dialect renders each request
* *THEN* every reference MUST render as the one read expression the column's technology names for that source type and declared type, such as `CAST("uid" AS TEXT)`
* *AND* a grouped column MUST render as its read expression in `GROUP BY`, and its select-list occurrence and a select-list predicate reading it MUST render in a form the source accepts under its full-group-by rule, which `vs-adapter/mysql-collation` states for MySQL
* *AND* a column whose source type its technology reads without a conversion, and a column whose `adapterNotes` record no source type, MUST render as the bare quoted column
* *AND* the read expression MUST yield the value the scan emits for the declared type, so a delegated operation over the column acts on that value
<!-- /DELTA:NEW -->
