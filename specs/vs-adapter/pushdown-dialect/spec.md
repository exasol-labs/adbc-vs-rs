# Feature: pushdown_dialect

Separates the query fragment Exasol delegates from the target dialect's SQL text, and gives the advertised capability set one owner — the dialect that renders it — pinned to what that dialect can express by a required invariant test.

## Background

- This feature owns the dialect contract: one dialect per virtual schema, the invariant that every advertised capability renders, the all-or-nothing advertising rule, column-reference rendering, and determinism.
- `vs-adapter/driver-selection` resolves one dialect for both `getCapabilities` and `pushdown`, and each technology's advertised set and SQL live in its `<technology>-pushdown` feature.
- Capability names are `virtual-schema-common-java`'s serialized forms, identical across dialects: `MainCapability` constants verbatim, `FN_PRED_<NAME>`, `FN_AGG_<NAME>`, `LITERAL_<NAME>`, and `FN_<NAME>`.
- A `pushdown` request is parsed into a dialect-neutral query model of a select list, an optional filter, an optional aggregation, and a `from` tree, named in Exasol's protocol vocabulary rather than any target's syntax, which every dialect renders.
- `vs-adapter/pushdown-aggregation` owns the aggregate and grouping shapes, including `AGGREGATE_GROUP_BY_TUPLE`, `vs-adapter/pushdown-join` owns the `join_type` discriminator, and `vs-adapter/result-ordering` owns the ordering and limiting capabilities.
- `vs-adapter/pushdown-declared-types` owns the `EMITS` naming and declared-type contract.

## Scenarios

### Scenario: Every advertised capability has a rendering path

* *GIVEN* the capability set a dialect advertises, for each dialect this release supports
* *WHEN* each capability in that dialect's set is rendered through that same dialect against a request fragment that exercises it
* *THEN* every capability MUST produce a statement in that dialect's target SQL, independently of what the other dialects render, rather than reach a fallback that drops the fragment, ignores it, or returns the request unchanged, because nothing in the type system enforces the agreement between a dialect's stated set and its renderer, and an advertised capability the renderer cannot express returns wrong rows rather than an error
* *AND* the dialects the test covers MUST be enumerated from the driver identities rather than named in the test, so a release adding a dialect inherits the invariant without editing the test
* *AND* where a dialect renders one capability differently for one declared operand type, as the PostgreSQL dialect renders `MIN` and `MAX` over a `BOOLEAN` operand, the test MUST exercise that operand type as its own arm
* *AND* a composed shape a dialect cannot render, although each of its fragments renders alone, MUST fail with an error naming what could not be rendered rather than return a statement the source refuses, because this test proves one rendering path per capability, not every composition, so the feature that owns the exclusion states which dialects it binds and tests the shape

### Scenario: Unrepresentable request fragment fails rather than returning wrong rows

* *GIVEN* a `pushdown` request carrying a fragment the resolved dialect cannot render, for each dialect this release supports
* *WHEN* the adapter builds the response
* *THEN* the adapter MUST return an error
* *AND* the error message MUST name the protocol element type it could not render
* *AND* the adapter MUST NOT return a statement that silently omits the fragment
* *AND* the adapter MUST NOT return a statement that reads more rows than the fragment asked for

### Scenario: Comparison operands are read by position, not by assumed role

* *GIVEN* a `predicate_less` whose `left` operand is a literal and whose `right` operand is a column, as Exasol emits for `column > literal`, and a `predicate_lessequal` of the same shape, as Exasol emits for `column >= literal`
* *WHEN* each dialect renders each predicate
* *THEN* the rendered comparison MUST keep the operands in the order the request gave them
* *AND* the adapter MUST NOT assume the `left` operand is a column reference, because Exasol normalizes `>` and `>=` by swapping the operands, so no `predicate_greater` type exists: measured on the test Exasol, `i > 5` arrives as `predicate_less` whose `left` is the literal `5` and whose `right` is the column `i`

### Scenario: Generated statement is deterministic for a given request

* *GIVEN* the same `pushdown` request rendered twice through one dialect
* *WHEN* the adapter builds each response
* *THEN* both generated statements MUST be byte-for-byte identical
* *AND* the ordering of rendered select-list columns, filter operands, and join operands MUST follow the request's own ordering rather than an iteration order of a hash container
* *AND* the same MUST hold for each dialect this release supports

### Scenario: The dialects' capability sets differ only where their SQL differs

* *GIVEN* the capability set each dialect this release supports advertises
* *WHEN* the sets are compared pairwise
* *THEN* each set MUST be stated by its own dialect rather than derived from another by addition or removal, because a derived set makes one dialect's withholding silently govern another's
* *AND* a capability present in one set and absent from another MUST be read as a difference between the targets' SQL rather than as a defect
* *AND* at least one pair of sets MUST differ, or the invariant that a set is per dialect is not observable in this release
* *AND* every capability present in one set and absent from another MUST be named by the feature that owns the withholding, together with its reason: the absent dialect's own feature where the reason is a property of that target, or the framework feature that withholds it where the reason is this release's scope
* *AND* two sets holding the same entries MUST be read as two decisions that agreed rather than as one derived from the other, and a later change to either MUST fail the comparison rather than be absorbed

### Scenario: A withheld capability carries one of two reasons

* *GIVEN* a capability that a dialect cannot render for every operand type it admits, such as an aggregate, and a capability that a framework decision about this release's scope or transport withholds
* *WHEN* each dialect's advertised capability set is stated
* *THEN* the first MUST be absent from that dialect's set rather than present with a render-time check that refuses an operand type, which would turn a query an earlier release answered into a hard failure, because Exasol negotiates per capability rather than per operand type
* *AND* the second MUST be absent from every dialect's set, whatever its target can express, and each dialect's own feature names which of the two reasons each absence carries
* *AND* a query over a withheld capability MUST return the answer Exasol computes itself, because Exasol delegates an advertised fragment fully, never re-applies it, and does not compute it after the adapter errors, so withholding costs throughput alone

### Scenario: Each dialect's capability set is stated once

* *GIVEN* the adapter's and the scan crate's production modules
* *WHEN* each dialect's advertised capability set is read
* *THEN* it MUST come from that technology's driver declaration alone
* *AND* no production module, the build script included, MUST hold a second capability list, such as a list of valid capability names

### Scenario: The declarable set is closed under the advertised capabilities

* *GIVEN* a virtual schema of any dialect this release supports, none of which advertises a scalar function
* *WHEN* Exasol plans a query over the schema's mapped columns and sends the `selectListDataTypes` of the select list it delegates
* *THEN* every entry MUST name a type of the declarable set, so nothing reaches the `EMITS` clause outside it
* *AND* the delegated entries MUST match these shapes, measured on the test Exasol: `SUM("c") + 1` withdraws the whole aggregation rather than delegating `SUM` alone, a predicate placed in `selectList` is declared `BOOLEAN`, `COUNT` yields `DECIMAL(18,0)`, and `MIN` and `MAX` yield their operand's own type
* *AND* an entry naming a type outside the set MUST still fail with a named error, as the scenario for a declared type outside the declarable set states
* *AND* every dialect MUST advertise `SELECTLIST_EXPRESSIONS`, because Exasol composes a select-list expression only from capabilities the adapter advertises, and measured on the test Exasol, `SELECTLIST_EXPRESSIONS` alone yields plain columns while the aggregate capabilities alone delegate nothing

### Scenario: A column reference renders through its read expression in every position

* *GIVEN* `pushdown` requests referencing one column whose `adapterNotes` record a source type its technology reads through a conversion, such as a PostgreSQL `uuid` column, in the select list, a `WHERE` comparison, a `GROUP BY` key, a `MIN` operand, a `COUNT(DISTINCT)` operand, and an equi-join condition
* *WHEN* the dialect renders each request
* *THEN* each column reference of the dialect-neutral query model MUST carry the column's declared Exasol type and the source type its `adapterNotes` recorded, and every reference MUST render as the one read expression the column's technology names for that source type and declared type, such as `CAST("uid" AS TEXT)`
* *AND* a grouped column MUST render as its read expression in `GROUP BY`, and its select-list occurrence and a select-list predicate reading it MUST render in a form the source accepts under its full-group-by rule, which `vs-adapter/mysql-collation` states for MySQL
* *AND* a column whose source type its technology reads without a conversion, and a column whose `adapterNotes` record no source type, MUST render as the bare quoted column
* *AND* the read expression MUST yield the value the scan emits for the declared type, so a delegated operation over the column acts on that value
