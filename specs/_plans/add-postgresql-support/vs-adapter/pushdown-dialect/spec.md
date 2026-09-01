# Feature: pushdown_dialect

Separates the query fragment Exasol delegates from the target dialect's SQL text, and gives the advertised capability set one owner — the dialect that renders it — pinned to what that dialect can express by a required invariant test.

## Background

<!-- DELTA:CHANGED -->
- This feature owns the dialect contract in the abstract: one dialect per virtual schema, the invariant that every advertised capability renders, the all-or-nothing rule that governs what may be advertised, the `EMITS` naming and declared-type contract, and determinism. Each technology's own advertised set and rendered SQL live in `vs-adapter/mysql-pushdown` and `vs-adapter/postgresql-pushdown`; each technology's collation placement lives in `vs-adapter/mysql-collation` and `vs-adapter/postgresql-collation`.
- Once Exasol receives a capability it delegates that fragment fully and never re-applies it. An advertised capability the renderer cannot express returns wrong rows rather than an error, so the advertised set and the renderer MUST come from one source.
- One dialect answers a virtual schema, and both `getCapabilities` and `pushdown` resolve it from the same field, so the advertised set and the renderer can never belong to different drivers. `vs-adapter/driver-selection` owns that resolution and enumerates the driver identities.
- Each dialect states its own set by hand, so no second capability list exists in the adapter's production modules. Nothing in the type system enforces the agreement between a dialect's stated set and what its renderer can express, so the invariant test that renders every advertised capability is what establishes the agreement.
- The invariant test runs once per dialect, over the identities `vs-adapter/driver-selection` enumerates rather than over a list of its own. A release adding a dialect therefore inherits the invariant without editing the test, and a dialect advertising a capability its own renderer cannot express fails independently of what the other dialects render.
- No dialect's set is derived from another's by addition or removal. A capability one target's SQL expresses and another's does not is advertised by one dialect and withheld by the other, and a difference between two sets is a legitimate consequence of the targets differing rather than a defect to reconcile.
- Withholding is not symmetric with rendering. A dialect withholds a capability whose fragment it cannot reproduce for every operand type the capability admits, even where the target's SQL has a keyword for it. `vs-adapter/pushdown-aggregation` owns the withheld aggregates and their measurement.
- Capability negotiation is all-or-nothing at the capability level, never per operand type within one query. Once Exasol decides a query shape is coverable by an advertised capability it is committed: it delegates the fragment and does not compute that fragment itself after the adapter errors. A capability advertised broadly and refused narrowly at render time therefore converts a query the previous release answered into a hard failure. A capability is either fully renderable or absent from the set; withholding costs throughput alone, because Exasol withdraws the fragment and computes it itself.
- An advertised set can compose two fragments into a shape the target rejects, even where each fragment renders correctly alone, and whether it can is a property of the target rather than of the protocol. A composed shape a dialect cannot render takes the same outcome as an unrenderable fragment — an error naming what could not be rendered — rather than a statement the source refuses. The exclusion belongs to the dialect that has it: `vs-adapter/pushdown-aggregation` owns the shape, its measurement, and which dialects it binds.
- A `pushdown` request is parsed into a dialect-neutral query model the adapter owns: a select list of expressions, an optional filter, an optional aggregation, and a `from` tree. The model names Exasol's protocol vocabulary, not any target's syntax, and every dialect renders the same model.
- The `EMITS` clause names its columns positionally, not by source column name. A join projects two columns that may share a name, and only a `column` select-list entry carries a `name` field at all, so a positional name is the only naming rule that holds for every advertised capability.
- Exasol compares the pushdown result's column types against the virtual table's stored metadata for exact equality, and rejects the statement on any difference — `VARCHAR(50)` offered where `VARCHAR(100)` was expected fails with a data-type-mismatch error. A declared type is therefore not merely a hint to be widened safely; it MUST equal the type Exasol expects.
- `pushdownRequest.selectListDataTypes` carries that expected type for every select-list entry, positionally parallel to `selectList`, computed by Exasol's own compiler. It is the type source for every entry, expression or column alike. Exasol supplies it from version 8.6.0 and from 7.1.14 on the 7.x line, and measured on Exasol 2025.2.1 it arrives on a projection-only request too, so the `involvedTables` fallback serves an older Exasol rather than the common path. Nothing about it is per dialect: the type Exasol expects is Exasol's answer, not the source's.
- `SELECTLIST_EXPRESSIONS` is advertised by every dialect, because it is the only capability under which an aggregate reaches the adapter. Measured on Exasol 2025.2.1: with `AGGREGATE_SINGLE_GROUP`, `AGGREGATE_GROUP_BY_COLUMN`, and nine `FN_AGG_*` advertised and `SELECTLIST_EXPRESSIONS` withheld, `SELECT SUM("c"), COUNT(*) FROM vs."t" GROUP BY "d"` produced a request byte-identical to the projection-only baseline; adding `SELECTLIST_EXPRESSIONS` delegated the aggregation; `SELECTLIST_EXPRESSIONS` without the aggregate capabilities did not.
- The capability is advertised once per session rather than per query, so it also lets a predicate reach the select list. Measured on the same stack with the advertised `FN_PRED_*` set, Exasol places `i > 5`, `i IS NULL`, `i BETWEEN 1 AND 9`, `i LIKE 'a%'`, `i IN (1,2,3)`, an `AND` of two comparisons, and `COUNT(*) > 0` in `selectList` and declares each `{"type":"BOOLEAN"}` in `selectListDataTypes`. `BOOLEAN` is therefore a type the `EMITS` clause MUST be able to declare, and no capability separates a boolean select-list entry from a boolean filter operand.
- Two type sets answer two questions. The set a source column may map to is the five `vs-adapter/type-mapping` produces, which is one set across every driver rather than one per driver. The set an `EMITS` column may declare is those five plus `BOOLEAN`, plus Exasol `CHAR` for a virtual schema release 0.1 created and no operator has refreshed. A `BOOLEAN` column is never a mapped source column, so the two sets stay modeled as two types rather than one.
- The declarable set is closed under the advertised capabilities, and stays closed for every dialect because none advertises a scalar function. Exasol composes a select-list expression only from capabilities the adapter advertises: measured, `SELECTLIST_EXPRESSIONS` alone yields plain columns, and `SUM("c") + 1` withdraws the whole aggregation rather than delegating `SUM` alone. The aggregate and predicate capabilities over the five mapped types therefore produce nothing outside the declarable set, and a declared type outside it still fails with a named error.
- A withheld capability can withhold more than its name says. `AGGREGATE_GROUP_BY_TUPLE` gates how many columns a delegated `GROUP BY` may name, not grouping by a tuple-valued expression: measured on Exasol 2025.2.1 with a throwaway Lua adapter probe, `AGGREGATE_GROUP_BY_COLUMN` alone delegated a `GROUP BY` naming one column and withdrew one naming two, and adding `AGGREGATE_GROUP_BY_TUPLE` delegated both a two-column and a three-column `GROUP BY` in request order. Withholding it therefore caps this release's delegated grouping at one column, for every dialect. It stays withheld, and adding it would need no rendering change in any: each dialect already renders every grouping column a request carries.
- Capability names follow `virtual-schema-common-java`'s serialized forms: `MainCapability` constants verbatim, predicates as `FN_PRED_<NAME>`, aggregate functions as `FN_AGG_<NAME>`, literals as `LITERAL_<NAME>`, and scalar functions as `FN_<NAME>`. The names are protocol vocabulary, so they are identical across dialects and only membership differs.
- Exasol normalizes `>` and `>=` by swapping operands into `predicate_less` and `predicate_lessequal`, so no `predicate_greater` type exists and the `left` operand of a comparison is not reliably the column.
- The join `from` element spells its discriminator `join_type` in snake case while every sibling protocol field is camel case.
<!-- /DELTA:CHANGED -->

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Every advertised capability has a rendering path

* *GIVEN* the capability set a dialect advertises, for each dialect this release supports
* *WHEN* each capability in that dialect's set is rendered through that same dialect against a request fragment that exercises it
* *THEN* every capability MUST produce a statement in that dialect's target SQL
* *AND* no capability MUST reach a fallback that drops the fragment, ignores it, or returns the request unchanged, and the dialects the test covers MUST be enumerated from the driver identities rather than named in the test
* *AND* the test MUST be read as proving that each advertised capability has one rendering path in its own dialect, not that every composition of two capabilities renders, so a composed shape a dialect rejects needs its own test in the feature that owns it
* *AND* an aggregate a dialect cannot render for every operand type its capability would admit MUST be absent from that dialect's set, rather than present with a render-time operand-type check, so the test's per-capability arms need no per-operand-type variants
<!-- /DELTA:CHANGED -->

<!-- DELTA:REMOVED -->
### Scenario: Capability set advertises the fragments this release renders

* *GIVEN* a virtual schema backed by MySQL
* *WHEN* Exasol sends a `getCapabilities` request
* *THEN* the `capabilities` array MUST contain `SELECTLIST_PROJECTION`, `SELECTLIST_EXPRESSIONS`, `FILTER_EXPRESSIONS`, `AGGREGATE_SINGLE_GROUP`, `AGGREGATE_GROUP_BY_COLUMN`, `JOIN`, `JOIN_TYPE_INNER`, `JOIN_TYPE_LEFT_OUTER`, `JOIN_TYPE_RIGHT_OUTER`, and `JOIN_CONDITION_EQUI`
* *AND* the array MUST NOT contain `AGGREGATE_GROUP_BY_EXPRESSION`, `AGGREGATE_GROUP_BY_TUPLE`, `AGGREGATE_HAVING`, `JOIN_TYPE_FULL_OUTER`, or `JOIN_CONDITION_ALL`
* *AND* the array MUST NOT contain `ORDER_BY_COLUMN`, `ORDER_BY_EXPRESSION`, `LIMIT`, or `LIMIT_WITH_OFFSET`
* *AND* every entry MUST be a `MainCapability` constant, or carry the prefix `FN_PRED_`, `FN_AGG_`, or `LITERAL_`
<!-- /DELTA:REMOVED -->

<!-- DELTA:NEW -->
### Scenario: The two dialects' capability sets differ only where their SQL differs

* *GIVEN* the capability set each dialect this release supports advertises
* *WHEN* the sets are compared
* *THEN* each set MUST be stated by its own dialect rather than derived from another by addition or removal, because a derived set makes one dialect's withholding silently govern another's
* *AND* a capability present in one set and absent from another MUST be read as a difference between the targets' SQL rather than as a defect
* *AND* the one such difference in this release MUST be `JOIN_TYPE_FULL_OUTER`, advertised by the PostgreSQL dialect and withheld by the MySQL dialect, which is what makes the invariant observable
<!-- /DELTA:NEW -->

<!-- DELTA:CHANGED -->
### Scenario: Unrepresentable request fragment fails rather than returning wrong rows

* *GIVEN* a `pushdown` request carrying a fragment the resolved dialect cannot render, for each dialect this release supports
* *WHEN* the adapter builds the response
* *THEN* the adapter MUST return an error
* *AND* the error message MUST name the protocol element type it could not render
* *AND* the adapter MUST NOT return a statement that silently omits the fragment
* *AND* the adapter MUST NOT return a statement that reads more rows than the fragment asked for
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Comparison operands are read by position, not by assumed role

* *GIVEN* a `predicate_less` whose `left` operand is a literal and whose `right` operand is a column, as Exasol emits for `column > literal`
* *WHEN* each dialect renders that predicate
* *THEN* the rendered comparison MUST keep the operands in the order the request gave them
* *AND* the adapter MUST NOT assume the `left` operand is a column reference
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Generated statement is deterministic for a given request

* *GIVEN* the same `pushdown` request rendered twice through one dialect
* *WHEN* the adapter builds each response
* *THEN* both generated statements MUST be byte-for-byte identical
* *AND* the ordering of rendered select-list columns, filter operands, and join operands MUST follow the request's own ordering rather than an iteration order of a hash container
* *AND* the same MUST hold for each dialect this release supports
<!-- /DELTA:CHANGED -->
