# Feature: pushdown_dialect

Separates the query fragment Exasol delegates from the target dialect's SQL text, and gives the advertised capability set one owner — the dialect that renders it — pinned to what that dialect can express by a required invariant test.

<!-- DELTA:CHANGED -->
## Background

- This feature owns the dialect contract: one dialect per virtual schema, the invariant that every advertised capability renders, the all-or-nothing advertising rule, column-reference rendering, and determinism.
- `vs-adapter/driver-selection` resolves one dialect for both `getCapabilities` and `pushdown`, and each technology's advertised set and SQL live in its `<technology>-pushdown` feature.
- Capability names are `virtual-schema-common-java`'s serialized forms, identical across dialects: `MainCapability` constants verbatim, `FN_PRED_<NAME>`, `FN_AGG_<NAME>`, `LITERAL_<NAME>`, and `FN_<NAME>`.
- A `pushdown` request is parsed into a dialect-neutral query model of a select list, an optional filter, an optional aggregation with an optional `having` predicate, an optional limit, and a `from` tree, named in Exasol's protocol vocabulary rather than any target's syntax, which every dialect renders.
- `vs-adapter/pushdown-aggregation` owns the aggregate, grouping, and `HAVING` shapes, `vs-adapter/pushdown-join` the `join_type` discriminator, `vs-adapter/result-ordering` the ordering capabilities, `vs-adapter/pushdown-limit` the advertised `LIMIT`, the withheld `LIMIT_WITH_OFFSET`, and the `limit` element, and `vs-adapter/pushdown-declared-types` the `EMITS` naming and declared-type contract.
<!-- /DELTA:CHANGED -->

## Scenarios

### Scenario: Every advertised capability has a rendering path

* *GIVEN* the capability set a dialect advertises, for each dialect this release supports
* *WHEN* each capability in that dialect's set is rendered through that same dialect against a request fragment that exercises it
* *THEN* every capability MUST produce a statement in that dialect's target SQL, independently of what the other dialects render, rather than reach a fallback that drops the fragment, ignores it, or returns the request unchanged, because nothing in the type system enforces the agreement between a dialect's stated set and its renderer, and an advertised capability the renderer cannot express returns wrong rows rather than an error
* *AND* the dialects the test covers MUST be enumerated from the driver identities rather than named in the test, so a release adding a dialect inherits the invariant without editing the test
* *AND* where a dialect renders one capability differently for one declared operand type, as the PostgreSQL dialect renders `MIN` and `MAX` over a `BOOLEAN` operand, the test MUST exercise that operand type as its own arm
* *AND* a composed shape a dialect cannot render, although each of its fragments renders alone, MUST fail with an error naming what could not be rendered rather than return a statement the source refuses, because this test proves one rendering path per capability, not every composition, so the feature that owns the exclusion states which dialects it binds and tests the shape
