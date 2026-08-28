# Feature: pushdown_dialect

Separates the query fragment Exasol delegates from the target dialect's SQL text, and gives the advertised capability set one owner — the dialect that renders it — pinned to what that dialect can express by a required invariant test.

## Background

- Once Exasol receives a capability it delegates that fragment fully and never re-applies it. An advertised capability the renderer cannot express returns wrong rows rather than an error, so the advertised set and the renderer MUST come from one source.
- The advertised set is produced by the dialect that renders it, so no second capability list exists in the adapter's production modules. The dialect states its set by hand and nothing in the type system enforces the agreement between that set and what the renderer can express, so the invariant test that renders every advertised capability is what establishes the agreement.
- A `pushdown` request is parsed into a dialect-neutral query model the adapter owns: a select list of expressions, an optional filter, an optional aggregation, and a `from` tree. The model names Exasol's protocol vocabulary, not MySQL's syntax.
- One `SqlDialect` implementation exists in this release, for MySQL. The trait exists so a second driver adds an implementation rather than editing the renderer; mission.md's v1 driver set names PostgreSQL, Snowflake, Databricks, and DuckDB.
- The `EMITS` clause names its columns positionally, not by source column name. A join projects two columns that may share a name, and only a `column` select-list entry carries a `name` field at all, so a positional name is the only naming rule that holds for every advertised capability.
- Exasol compares the pushdown result's column types against the virtual table's stored metadata for exact equality, and rejects the statement on any difference — `VARCHAR(50)` offered where `VARCHAR(100)` was expected fails with a data-type-mismatch error. A declared type is therefore not merely a hint to be widened safely; it MUST equal the type Exasol expects.
- `pushdownRequest.selectListDataTypes` carries that expected type for every select-list entry, positionally parallel to `selectList`, computed by Exasol's own compiler. It is the type source for every entry, expression or column alike. Exasol supplies it from version 8.6.0 and from 7.1.14 on the 7.x line, and measured on Exasol 2025.2.1 it arrives on a projection-only request too, so the `involvedTables` fallback serves an older Exasol rather than the common path.
- `SELECTLIST_EXPRESSIONS` is advertised, because it is the only capability under which an aggregate reaches the adapter. Measured on Exasol 2025.2.1: with `AGGREGATE_SINGLE_GROUP`, `AGGREGATE_GROUP_BY_COLUMN`, and nine `FN_AGG_*` advertised and `SELECTLIST_EXPRESSIONS` withheld, `SELECT SUM("c"), COUNT(*) FROM vs."t" GROUP BY "d"` produced a request byte-identical to the projection-only baseline; adding `SELECTLIST_EXPRESSIONS` delegated the aggregation; `SELECTLIST_EXPRESSIONS` without the aggregate capabilities did not.
- The capability is advertised once per session rather than per query, so it also lets a predicate reach the select list. Measured on the same stack with the advertised `FN_PRED_*` set, Exasol places `i > 5`, `i IS NULL`, `i BETWEEN 1 AND 9`, `i LIKE 'a%'`, `i IN (1,2,3)`, an `AND` of two comparisons, and `COUNT(*) > 0` in `selectList` and declares each `{"type":"BOOLEAN"}` in `selectListDataTypes`. `BOOLEAN` is therefore a type the `EMITS` clause MUST be able to declare, and no capability separates a boolean select-list entry from a boolean filter operand.
- Two type sets answer two questions. The set a MySQL column may map to is the five `vs-adapter/type-mapping` produces. The set an `EMITS` column may declare is those five plus `BOOLEAN`, plus Exasol `CHAR` for a virtual schema release 0.1 created and no operator has refreshed. A `BOOLEAN` column is never a mapped MySQL column, so the two sets are modeled as two types rather than one.
- The declarable set is closed under the advertised capabilities. Exasol composes a select-list expression only from capabilities the adapter advertises: measured, `SELECTLIST_EXPRESSIONS` alone yields plain columns, and `SUM("c") + 1` withdraws the whole aggregation rather than delegating `SUM` alone, because this release advertises no scalar function. The aggregate and predicate capabilities over the five mapped types therefore produce nothing outside the declarable set, and a declared type outside it still fails with a named error.
- A withheld capability can withhold more than its name says. `AGGREGATE_GROUP_BY_TUPLE` gates how many columns a delegated `GROUP BY` may name, not grouping by a tuple-valued expression: measured on Exasol 2025.2.1 with a throwaway Lua adapter probe, `AGGREGATE_GROUP_BY_COLUMN` alone delegated a `GROUP BY` naming one column and withdrew one naming two, and adding `AGGREGATE_GROUP_BY_TUPLE` delegated both a two-column and a three-column `GROUP BY` in request order. Withholding it therefore caps this release's delegated grouping at one column. It stays withheld, and adding it would need no rendering change: the dialect already renders every grouping column a request carries, and measured with `AGGREGATE_GROUP_BY_TUPLE` advertised and `AGGREGATE_GROUP_BY_EXPRESSION` withheld, every `groupBy` entry was still a `column` and `GROUP BY "d" + 1` was withdrawn whole.
- An advertised set can compose two fragments into a shape the target rejects, even where each fragment renders correctly alone. `SELECTLIST_EXPRESSIONS` puts a predicate in the select list and `AGGREGATE_GROUP_BY_COLUMN` puts a character column in `GROUP BY`; measured on MySQL 8.4, a statement carrying both over the same column is rejected with error 1055 under `ONLY_FULL_GROUP_BY` at every collation placement. No capability withholds that composition without withholding one of the two fragments whole. A composed shape the dialect cannot render therefore takes the same outcome as an unrenderable fragment — an error naming what could not be rendered — rather than a statement the source refuses. `vs-adapter/pushdown-aggregation` owns this shape and its measurement.
- Capability negotiation is all-or-nothing at the capability level, never per operand type within one query. Once Exasol decides a query shape is coverable by an advertised capability it is committed: it delegates the fragment and does not compute that fragment itself after the adapter errors. A capability advertised broadly and refused narrowly at render time therefore converts a query the previous release answered into a hard failure. A capability that cannot be rendered safely for every operand type it admits is withheld whole instead, which costs throughput alone, because Exasol withdraws the fragment and computes it itself. `FN_AGG_SUM` is the measured case: MySQL's `SUM` over an exact-numeric operand is exact and order-independent, while over a `DOUBLE` operand it accumulates in physical row order — measured on MySQL 8.4.11, one multiset under three physical row orders returned three different sums where Exasol 2025.2.1 returned one. A capability is advertised once per session, so none admits the exact-numeric operand while refusing the floating-point one, and both `SUM` capabilities are withheld rather than guarded in the renderer. `vs-adapter/pushdown-aggregation` owns the withholding and its measurement. A composed shape two capabilities produce is the one remaining render-time rejection, and the bullet above owns it.
- Capability names follow `virtual-schema-common-java`'s serialized forms: `MainCapability` constants verbatim, predicates as `FN_PRED_<NAME>`, aggregate functions as `FN_AGG_<NAME>`, literals as `LITERAL_<NAME>`, and scalar functions as `FN_<NAME>`.
- Exasol normalizes `>` and `>=` by swapping operands into `predicate_less` and `predicate_lessequal`, so no `predicate_greater` type exists and the `left` operand of a comparison is not reliably the column.
- The join `from` element spells its discriminator `join_type` in snake case while every sibling protocol field is camel case.

## Scenarios

### Scenario: Every advertised capability has a rendering path

* *GIVEN* the capability set the adapter advertises
* *WHEN* each capability in that set is rendered through the MySQL dialect against a request fragment that exercises it
* *THEN* every capability MUST produce a MySQL statement
* *AND* no capability MUST reach a fallback that drops the fragment, ignores it, or returns the request unchanged
* *AND* the test MUST be read as proving that each advertised capability has one rendering path, not that every composition of two capabilities renders: a select-list predicate over a collated `GROUP BY` key is the one composed shape the dialect rejects, so that rule needs its own test in the feature that owns it
* *AND* an aggregate this release cannot render for every operand type its capability would admit MUST be absent from the set the test reads, rather than present with a render-time operand-type check, so the test's per-capability arms need no per-operand-type variants

### Scenario: Capability set advertises the fragments this release renders

* *GIVEN* a virtual schema backed by MySQL
* *WHEN* Exasol sends a `getCapabilities` request
* *THEN* the `capabilities` array MUST contain `SELECTLIST_PROJECTION`, `SELECTLIST_EXPRESSIONS`, `FILTER_EXPRESSIONS`, `AGGREGATE_SINGLE_GROUP`, `AGGREGATE_GROUP_BY_COLUMN`, `JOIN`, `JOIN_TYPE_INNER`, `JOIN_TYPE_LEFT_OUTER`, `JOIN_TYPE_RIGHT_OUTER`, and `JOIN_CONDITION_EQUI`
* *AND* the array MUST NOT contain `AGGREGATE_GROUP_BY_EXPRESSION`, `AGGREGATE_GROUP_BY_TUPLE`, `AGGREGATE_HAVING`, `JOIN_TYPE_FULL_OUTER`, or `JOIN_CONDITION_ALL`
* *AND* the array MUST NOT contain `ORDER_BY_COLUMN`, `ORDER_BY_EXPRESSION`, `LIMIT`, or `LIMIT_WITH_OFFSET`
* *AND* every entry MUST be a `MainCapability` constant, or carry the prefix `FN_PRED_`, `FN_AGG_`, or `LITERAL_`

### Scenario: Unrepresentable request fragment fails rather than returning wrong rows

* *GIVEN* a `pushdown` request carrying a fragment the MySQL dialect cannot render
* *WHEN* the adapter builds the response
* *THEN* the adapter MUST return an error
* *AND* the error message MUST name the protocol element type it could not render
* *AND* the adapter MUST NOT return a statement that silently omits the fragment
* *AND* the adapter MUST NOT return a statement that reads more rows than the fragment asked for

### Scenario: Emitted columns are named positionally

* *GIVEN* a `pushdown` request whose select list projects two columns that share a source column name
* *WHEN* the adapter builds the `EMITS` clause
* *THEN* each declared column name MUST be derived from the column's position in the select list
* *AND* the declared names MUST be pairwise distinct
* *AND* the names the scan specification carries MUST equal the names the `EMITS` clause declares, so the scan UDF's emitted schema matches the declaration

### Scenario: Declared types are read from the request positionally

* *GIVEN* a `pushdown` request carrying `selectListDataTypes`
* *WHEN* the adapter builds the `EMITS` clause
* *THEN* the declared type of each column MUST be the `selectListDataTypes` entry at the same index as its `selectList` entry
* *AND* the adapter MUST NOT infer a type from the expression tree while `selectListDataTypes` supplies one, because Exasol rejects any type that differs from the one it expects
* *AND* a `selectListDataTypes` array whose length differs from the `selectList` length MUST fail with an error naming both lengths

### Scenario: Missing declared types fail rather than being guessed

* *GIVEN* a `pushdown` request carrying a select-list entry that is not a `column`, and carrying no `selectListDataTypes`, as an Exasol older than 8.6.0 sends
* *WHEN* the adapter builds the response
* *THEN* the adapter MUST return an error naming `selectListDataTypes` as the missing field and the minimum Exasol version that supplies it
* *AND* the adapter MUST NOT guess the entry's type

### Scenario: Declared type source for each select-list state

* *GIVEN* a `pushdown` request carrying no `selectListDataTypes`
* *WHEN* the adapter builds the `EMITS` clause
* *THEN* a request carrying no `selectList` field MUST succeed, declaring one column per mapped column of the involved table and taking each declared type from that table's `involvedTables[].columns[].dataType` entry
* *AND* a request whose `selectList` is an empty array MUST succeed, declaring the single constant column `vs-adapter/pushdown-projection` requires with the Exasol type `DECIMAL(1,0)` that scenario names
* *AND* a request whose `selectList` holds `column` entries exclusively MUST succeed, taking each declared type from `involvedTables[].columns[].dataType`

### Scenario: Declared type outside the declarable set fails with a named error

* *GIVEN* a `selectListDataTypes` entry naming an Exasol type outside the set an `EMITS` column may declare, such as `INTERVAL` or `GEOMETRY`
* *WHEN* the adapter builds the `EMITS` clause
* *THEN* the adapter MUST return an error naming the type it received
* *AND* the adapter MUST NOT substitute a type it can render
* *AND* a `BOOLEAN` entry MUST NOT fail, because a delegated predicate in the select list is declared `BOOLEAN`

### Scenario: BOOLEAN is declarable in the EMITS clause

* *GIVEN* a `pushdown` request whose select list carries a predicate and whose `selectListDataTypes` declares that entry `{"type":"BOOLEAN"}`
* *WHEN* the adapter builds the `EMITS` clause and the scan specification
* *THEN* the declared column MUST carry the Exasol type `BOOLEAN`
* *AND* the scan specification MUST carry the same type for that column, so the scan emits a boolean rather than a number
* *AND* the rendered `EMITS` fragment MUST be `BOOLEAN` carrying no size and no precision, because Exasol's `BOOLEAN` takes neither

### Scenario: Select-list entry kinds are recognized by their type field

* *GIVEN* `pushdown` requests whose select list carries a `column` entry, a `function_aggregate` entry, and a `predicate_less` entry
* *WHEN* the adapter parses each request
* *THEN* each entry's kind MUST be taken from its `type` field rather than from which other fields it carries
* *AND* an entry whose `type` this release does not render MUST fail with an error naming that `type`
* *AND* such an entry MUST NOT be read as a column reference, because a select list is no longer columns only once `SELECTLIST_EXPRESSIONS` is advertised

### Scenario: Declared type shapes the request carries are read without loss

* *GIVEN* a `selectListDataTypes` entry of type `TIMESTAMP` carrying `fractionalSecondsPrecision: 3`, and one of type `VARCHAR` carrying a size and no `characterSet`, both shapes measured on Exasol 2025.2.1
* *WHEN* the adapter reads each entry
* *THEN* the `TIMESTAMP` entry MUST declare `TIMESTAMP` and MUST NOT render a fractional-seconds precision Exasol did not ask for
* *AND* the `VARCHAR` entry MUST declare its size with character set `UTF8`, because `selectListDataTypes` omits the `characterSet` field that `involvedTables[].columns[].dataType` carries
* *AND* neither MUST fail for carrying a field the adapter does not use

### Scenario: Comparison operands are read by position, not by assumed role

* *GIVEN* a `predicate_less` whose `left` operand is a literal and whose `right` operand is a column, as Exasol emits for `column > literal`
* *WHEN* the dialect renders that predicate
* *THEN* the rendered MySQL comparison MUST keep the operands in the order the request gave them
* *AND* the adapter MUST NOT assume the `left` operand is a column reference

### Scenario: Generated statement is deterministic for a given request

* *GIVEN* the same `pushdown` request rendered twice
* *WHEN* the adapter builds each response
* *THEN* both generated MySQL statements MUST be byte-for-byte identical
* *AND* the ordering of rendered select-list columns, filter operands, and join operands MUST follow the request's own ordering rather than an iteration order of a hash container
