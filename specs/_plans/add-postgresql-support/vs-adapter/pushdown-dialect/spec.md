# Feature: pushdown_dialect

Separates the query fragment Exasol delegates from the target dialect's SQL text, and gives the advertised capability set one owner — the dialect that renders it — pinned to what that dialect can express by a required invariant test.

## Background

<!-- DELTA:CHANGED -->
- Once Exasol receives a capability it delegates that fragment fully and never re-applies it. An advertised capability the renderer cannot express returns wrong rows rather than an error, so the advertised set and the renderer MUST come from one source.
- The advertised set is produced by the dialect that renders it, so no second capability list exists in the adapter's production modules. The dialect states its set by hand and nothing in the type system enforces the agreement between that set and what the renderer can express, so the invariant test that renders every advertised capability is what establishes the agreement.
- Two dialects exist in this release, for MySQL and for PostgreSQL, and each states its own set. The set is therefore per dialect rather than per release: a capability one target's SQL expresses and the other's does not is advertised by one dialect and withheld by the other, and neither dialect's set is derived from the other's. `vs-adapter/driver-selection` resolves which dialect answers a given request, and both `getCapabilities` and `pushdown` resolve it from one field so the advertised set and the renderer can never belong to different drivers.
- The invariant test runs once per dialect, over the identities `vs-adapter/driver-selection` enumerates rather than over a list of its own. A release adding a third dialect therefore inherits the invariant without editing the test, and a dialect advertising a capability its own renderer cannot express fails independently of what the other dialects render.
- `JOIN_TYPE_FULL_OUTER` is the capability the two dialects disagree on, and it is the only one. PostgreSQL implements `FULL OUTER JOIN` directly; MySQL implements none and emulating one requires the union rewrite mission.md places out of scope. `vs-adapter/pushdown-join` owns both decisions. Every other capability is advertised or withheld identically by both, which the per-dialect scenarios state as two explicit sets rather than as one set and a diff.
- Withholding is not symmetric with rendering. A dialect withholds a capability whose fragment it cannot reproduce for every operand type the capability admits, even where the target's SQL has a keyword for it: `FN_AGG_SUM` and `FN_AGG_AVG` are withheld by both dialects, because both targets accumulate a floating-point sum in physical row order while Exasol does not, and a capability is advertised once per session rather than per operand type. `vs-adapter/pushdown-aggregation` owns that withholding for both.
- A `pushdown` request is parsed into a dialect-neutral query model the adapter owns: a select list of expressions, an optional filter, an optional aggregation, and a `from` tree. The model names Exasol's protocol vocabulary, not any target's syntax, and both dialects render the same model.
- The `EMITS` clause names its columns positionally, not by source column name. A join projects two columns that may share a name, and only a `column` select-list entry carries a `name` field at all, so a positional name is the only naming rule that holds for every advertised capability.
- Exasol compares the pushdown result's column types against the virtual table's stored metadata for exact equality, and rejects the statement on any difference — `VARCHAR(50)` offered where `VARCHAR(100)` was expected fails with a data-type-mismatch error. A declared type is therefore not merely a hint to be widened safely; it MUST equal the type Exasol expects.
- `pushdownRequest.selectListDataTypes` carries that expected type for every select-list entry, positionally parallel to `selectList`, computed by Exasol's own compiler. It is the type source for every entry, expression or column alike. Exasol supplies it from version 8.6.0 and from 7.1.14 on the 7.x line, and measured on Exasol 2025.2.1 it arrives on a projection-only request too, so the `involvedTables` fallback serves an older Exasol rather than the common path. Nothing about it is per dialect: the type Exasol expects is Exasol's answer, not the source's.
- `SELECTLIST_EXPRESSIONS` is advertised by both dialects, because it is the only capability under which an aggregate reaches the adapter. Measured on Exasol 2025.2.1: with `AGGREGATE_SINGLE_GROUP`, `AGGREGATE_GROUP_BY_COLUMN`, and nine `FN_AGG_*` advertised and `SELECTLIST_EXPRESSIONS` withheld, `SELECT SUM("c"), COUNT(*) FROM vs."t" GROUP BY "d"` produced a request byte-identical to the projection-only baseline; adding `SELECTLIST_EXPRESSIONS` delegated the aggregation; `SELECTLIST_EXPRESSIONS` without the aggregate capabilities did not.
- The capability is advertised once per session rather than per query, so it also lets a predicate reach the select list. Measured on the same stack with the advertised `FN_PRED_*` set, Exasol places `i > 5`, `i IS NULL`, `i BETWEEN 1 AND 9`, `i LIKE 'a%'`, `i IN (1,2,3)`, an `AND` of two comparisons, and `COUNT(*) > 0` in `selectList` and declares each `{"type":"BOOLEAN"}` in `selectListDataTypes`. `BOOLEAN` is therefore a type the `EMITS` clause MUST be able to declare, and no capability separates a boolean select-list entry from a boolean filter operand.
- Two type sets answer two questions. The set a source column may map to is the five `vs-adapter/type-mapping` produces, which is one set across both drivers rather than one per driver. The set an `EMITS` column may declare is those five plus `BOOLEAN`, plus Exasol `CHAR` for a virtual schema release 0.1 created and no operator has refreshed. A `BOOLEAN` column is never a mapped source column — PostgreSQL's `boolean` is unmapped in this release, as `vs-adapter/type-mapping` records — so the two sets stay modeled as two types rather than one.
- The declarable set is closed under the advertised capabilities, and stays closed for both dialects because neither advertises a scalar function. Exasol composes a select-list expression only from capabilities the adapter advertises: measured, `SELECTLIST_EXPRESSIONS` alone yields plain columns, and `SUM("c") + 1` withdraws the whole aggregation rather than delegating `SUM` alone. The aggregate and predicate capabilities over the five mapped types therefore produce nothing outside the declarable set, and a declared type outside it still fails with a named error.
- A withheld capability can withhold more than its name says. `AGGREGATE_GROUP_BY_TUPLE` gates how many columns a delegated `GROUP BY` may name, not grouping by a tuple-valued expression: measured on Exasol 2025.2.1 with a throwaway Lua adapter probe, `AGGREGATE_GROUP_BY_COLUMN` alone delegated a `GROUP BY` naming one column and withdrew one naming two, and adding `AGGREGATE_GROUP_BY_TUPLE` delegated both a two-column and a three-column `GROUP BY` in request order. Withholding it therefore caps this release's delegated grouping at one column, for both dialects. It stays withheld, and adding it would need no rendering change in either: each dialect already renders every grouping column a request carries.
- An advertised set can compose two fragments into a shape the target rejects, even where each fragment renders correctly alone, and whether it can is a property of the target rather than of the protocol. `SELECTLIST_EXPRESSIONS` puts a predicate in the select list and `AGGREGATE_GROUP_BY_COLUMN` puts a character column in `GROUP BY`; measured on MySQL 8.4, a statement carrying both over the same column is rejected with error 1055 under `ONLY_FULL_GROUP_BY`, because the MySQL dialect collates the grouping key and MySQL credits no sub-expression of it. A composed shape a dialect cannot render takes the same outcome as an unrenderable fragment — an error naming what could not be rendered. The exclusion belongs to the dialect that has it rather than to the capability pair: `vs-adapter/pushdown-aggregation` owns the shape, its measurement, and which dialects it binds.
- Capability negotiation is all-or-nothing at the capability level, never per operand type within one query. Once Exasol decides a query shape is coverable by an advertised capability it is committed: it delegates the fragment and does not compute that fragment itself after the adapter errors. A capability advertised broadly and refused narrowly at render time therefore converts a query the previous release answered into a hard failure. A capability that cannot be rendered safely for every operand type it admits is withheld whole instead, which costs throughput alone, because Exasol withdraws the fragment and computes it itself. `vs-adapter/pushdown-aggregation` owns the withholding and its measurement for both dialects.
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
* *AND* the test MUST be read as proving that each advertised capability has one rendering path in its own dialect, not that every composition of two capabilities renders: a select-list predicate over a collated `GROUP BY` key is the one composed shape the MySQL dialect rejects, so that rule needs its own test in the feature that owns it
* *AND* an aggregate a dialect cannot render for every operand type its capability would admit MUST be absent from that dialect's set, rather than present with a render-time operand-type check, so the test's per-capability arms need no per-operand-type variants
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Capability set advertises the fragments this release renders

* *GIVEN* a virtual schema whose driver is `mysql`
* *WHEN* Exasol sends a `getCapabilities` request
* *THEN* the `capabilities` array MUST contain `SELECTLIST_PROJECTION`, `SELECTLIST_EXPRESSIONS`, `FILTER_EXPRESSIONS`, `AGGREGATE_SINGLE_GROUP`, `AGGREGATE_GROUP_BY_COLUMN`, `JOIN`, `JOIN_TYPE_INNER`, `JOIN_TYPE_LEFT_OUTER`, `JOIN_TYPE_RIGHT_OUTER`, and `JOIN_CONDITION_EQUI`
* *AND* the array MUST NOT contain `AGGREGATE_GROUP_BY_EXPRESSION`, `AGGREGATE_GROUP_BY_TUPLE`, `AGGREGATE_HAVING`, `JOIN_TYPE_FULL_OUTER`, or `JOIN_CONDITION_ALL`
* *AND* the array MUST NOT contain `ORDER_BY_COLUMN`, `ORDER_BY_EXPRESSION`, `LIMIT`, or `LIMIT_WITH_OFFSET`
* *AND* every entry MUST be a `MainCapability` constant, or carry the prefix `FN_PRED_`, `FN_AGG_`, or `LITERAL_`
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: PostgreSQL advertises its own capability set

* *GIVEN* a virtual schema whose driver is `postgresql`
* *WHEN* Exasol sends a `getCapabilities` request
* *THEN* the `capabilities` array MUST contain `SELECTLIST_PROJECTION`, `SELECTLIST_EXPRESSIONS`, `FILTER_EXPRESSIONS`, `AGGREGATE_SINGLE_GROUP`, `AGGREGATE_GROUP_BY_COLUMN`, `JOIN`, `JOIN_TYPE_INNER`, `JOIN_TYPE_LEFT_OUTER`, `JOIN_TYPE_RIGHT_OUTER`, `JOIN_TYPE_FULL_OUTER`, and `JOIN_CONDITION_EQUI`
* *AND* the array MUST NOT contain `AGGREGATE_GROUP_BY_EXPRESSION`, `AGGREGATE_GROUP_BY_TUPLE`, `AGGREGATE_HAVING`, or `JOIN_CONDITION_ALL`
* *AND* the array MUST NOT contain `ORDER_BY_COLUMN`, `ORDER_BY_EXPRESSION`, `LIMIT`, or `LIMIT_WITH_OFFSET`, which `vs-adapter/result-ordering` withholds for every dialect because the emitting-UDF transport carries no ordering guarantee whatever the source is
* *AND* every entry MUST be a `MainCapability` constant, or carry the prefix `FN_PRED_`, `FN_AGG_`, or `LITERAL_`
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: The two dialects' capability sets differ only where their SQL differs

* *GIVEN* the capability sets the MySQL dialect and the PostgreSQL dialect advertise
* *WHEN* the two sets are compared
* *THEN* the difference MUST be exactly `JOIN_TYPE_FULL_OUTER`, present in the PostgreSQL set and absent from the MySQL set
* *AND* every other capability MUST be present in both sets or absent from both
* *AND* the two sets MUST be stated independently by their own dialects rather than one derived from the other by addition or removal, because a derived set makes one dialect's withholding silently govern the other's
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
