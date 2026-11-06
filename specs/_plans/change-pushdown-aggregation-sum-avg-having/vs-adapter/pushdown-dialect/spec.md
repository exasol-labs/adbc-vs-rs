# Feature: pushdown_dialect

Separates the query fragment Exasol delegates from the target dialect's SQL text, and gives the advertised capability set one owner — the dialect that renders it — pinned to what that dialect can express by a required invariant test.

<!-- DELTA:CHANGED -->
## Background

- This feature owns the dialect contract: one dialect per virtual schema, the invariant that every advertised capability renders, the all-or-nothing advertising rule, column-reference rendering, and determinism.
- `vs-adapter/driver-selection` resolves one dialect for both `getCapabilities` and `pushdown`, and each technology's advertised set and SQL live in its `<technology>-pushdown` feature.
- Capability names are `virtual-schema-common-java`'s serialized forms, identical across dialects: `MainCapability` constants verbatim, `FN_PRED_<NAME>`, `FN_AGG_<NAME>`, `LITERAL_<NAME>`, and `FN_<NAME>`.
- A `pushdown` request is parsed into a dialect-neutral query model of a select list, an optional filter, an optional aggregation with an optional `having` predicate, and a `from` tree, named in Exasol's protocol vocabulary rather than any target's syntax, which every dialect renders.
- `vs-adapter/pushdown-aggregation` owns the aggregate, grouping, and `HAVING` shapes, `vs-adapter/pushdown-join` the `join_type` discriminator, `vs-adapter/result-ordering` the ordering and limiting capabilities, and `vs-adapter/pushdown-declared-types` the `EMITS` naming and declared-type contract.
<!-- /DELTA:CHANGED -->

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: The declarable set is closed under the advertised capabilities

* *GIVEN* a virtual schema of any dialect this release supports, none of which advertises a scalar function
* *WHEN* Exasol plans a query over the schema's mapped columns and sends the `selectListDataTypes` of the select list it delegates
* *THEN* every entry MUST name a type of the declarable set, so nothing reaches the `EMITS` clause outside it
* *AND* the delegated entries MUST match these shapes, measured on the test Exasol: `SUM("c") + 1` withdraws the whole aggregation rather than delegating `SUM` alone, a predicate placed in `selectList` is declared `BOOLEAN`, `COUNT` yields `DECIMAL(18,0)`, `MIN` and `MAX` yield their operand's own type, `SUM` yields a `DECIMAL` over a `DECIMAL` operand and `DOUBLE` over a `DOUBLE` operand, and `AVG` yields `DOUBLE`
* *AND* an entry naming a type outside the set MUST still fail with a named error, as the scenario for a declared type outside the declarable set states
* *AND* every dialect MUST advertise `SELECTLIST_EXPRESSIONS`, because Exasol composes a select-list expression only from capabilities the adapter advertises, and measured on the test Exasol, `SELECTLIST_EXPRESSIONS` alone yields plain columns while the aggregate capabilities alone delegate nothing
<!-- /DELTA:CHANGED -->
