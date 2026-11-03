# Feature: pushdown_dialect

Separates the query fragment Exasol delegates from the target dialect's SQL text, and gives the advertised capability set one owner — the dialect that renders it — pinned to what that dialect can express by a required invariant test.

<!-- DELTA:CHANGED -->
## Background

- This feature owns the dialect contract in the abstract: one dialect per virtual schema, the invariant that every advertised capability renders, the all-or-nothing rule that governs what may be advertised, the `EMITS` naming and declared-type contract, and determinism. `vs-adapter/driver-selection` resolves the dialect for both `getCapabilities` and `pushdown` from one field. Each technology's set, stated by hand in the capability names of `virtual-schema-common-java` (`FN_PRED_<NAME>`, `FN_AGG_<NAME>`, `LITERAL_<NAME>`, `FN_<NAME>`), and its SQL live in its `<technology>-pushdown` feature.
- Exasol delegates an advertised fragment fully and never re-applies it, and it negotiates per capability rather than per operand type, so a capability is either renderable by its dialect for every operand type it admits or absent from that dialect's set, and withholding costs throughput alone. A withheld capability is withheld either because the target cannot reproduce the fragment or by a framework scope decision, and the absent dialect's feature names which. A `pushdown` request is parsed into a dialect-neutral query model whose column references carry their declared Exasol types.
- Exasol compares the result's column types against the virtual table's stored metadata for exact equality. `pushdownRequest.selectListDataTypes`, supplied from Exasol 8.6.0 and from 7.1.14 on the 7.x line, carries the expected type of every select-list entry positionally, and the `EMITS` clause names its columns positionally because only a `column` entry carries a name and a join may project two columns that share one.
- Two type sets answer two questions: the six types `vs-adapter/type-mapping` lets a source column map to, and the set an `EMITS` column may declare, which adds Exasol `CHAR` for a virtual schema release 0.1 created and no operator has refreshed. The declarable set is closed under the advertised capabilities because no dialect advertises a scalar function: measured on the test Exasol, `SUM("c") + 1` withdraws the whole aggregation, a predicate placed in `selectList` is declared `BOOLEAN`, and an aggregate takes a mapped type, so nothing reaches the `EMITS` clause outside the set.
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
* *AND* a `BOOLEAN` entry MUST NOT fail, because a delegated predicate in the select list and a mapped `boolean` column are both declared `BOOLEAN`
<!-- /DELTA:CHANGED -->
