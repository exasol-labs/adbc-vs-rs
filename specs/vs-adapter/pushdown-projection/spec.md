# Feature: pushdown_projection

Reports the query fragments this release can delegate to the source, and rewrites the projection fragment into a `SELECT` the scan UDF executes at that source.

## Background

- This feature owns what a projection fragment is, for every dialect: which columns a select list projects and which are pruned, what an absent and an empty select list mean, how the `EMITS` clause names its columns, and that the response carries no credential. Identifier quoting is per technology — `vs-adapter/mysql-pushdown` and `vs-adapter/postgresql-pushdown` state each dialect's quote character and its escape rule.
- This release advertises projection alongside filter, aggregation, equi-join, and select-list expression delegation. `vs-adapter/pushdown-dialect` owns the advertised set per dialect and the requirement that every advertised capability has a rendering path in the dialect that advertises it; `vs-adapter/result-ordering` owns what stays withheld for every dialect.
- A select list is no longer columns only. `SELECTLIST_EXPRESSIONS` is advertised, so an entry may be an aggregate or a delegated predicate, and each entry's kind is read from its `type` field. A select list of plain columns still parses and renders exactly as it did before.
- Once Exasol receives a capability it delegates that fragment fully and never re-applies it. Advertising a capability the adapter does not render returns wrong rows rather than an error, so the advertised set lives in the dialect that renders it and an invariant test asserts that every entry renders, once per dialect.
- The SQL is assembled by the resolved dialect's own renderer over a dialect-neutral query model. `polyglot-sql` is not a dependency in this release.
- Every dialect quotes every identifier unconditionally, whatever its quote character. That is what keeps `vs-adapter/create-virtual-schema`'s rule that source identifiers keep their original case true against a target that folds an unquoted identifier to one case rather than preserving it.
- The pushdown response's `sql` field invokes the scan UDF directly with a query-side dynamic `EMITS` clause. There is no `IMPORT FROM SCRIPT` step, for any dialect.
- The `EMITS` clause names its columns positionally rather than by source column name, because a join projects columns that may share a name and an aggregate expression has no source column name.
- The scan specification passed to the UDF carries the driver identity, the driver path, the `CONNECTION` object's name, the generated source statement, and the declared Exasol output columns. It carries no credential. The identity is what lets the scan load the library with the right ADBC entry-point symbol, per `vs-adapter/driver-selection`.

## Scenarios

### Scenario: Named columns are projected and unnamed columns are pruned

* *GIVEN* a virtual table `orders` whose columns are `id`, `total`, and `note`
* *WHEN* Exasol pushes down a select whose `selectList` names `id` and `note`
* *THEN* the generated MySQL statement MUST project `id` and `note` from the qualified source table and MUST NOT project `total`
* *AND* the columns MUST appear in the order the `selectList` gives them
* *AND* the `EMITS` clause MUST declare two positionally named columns in that same order, carrying the Exasol types `id` and `note` are mapped to

### Scenario: Absent select list projects every mapped column

* *GIVEN* a virtual table `orders` with three mapped columns
* *WHEN* Exasol pushes down a select carrying no `selectList` field
* *THEN* the generated statement MUST project all three columns in the virtual table's declared order
* *AND* the generated statement MUST NOT use `SELECT *`, because a source column added after creation would then break the declared `EMITS` arity
* *AND* the same MUST hold for each dialect this release supports

### Scenario: Empty select list projects a single constant

* *GIVEN* a virtual table `orders`
* *WHEN* Exasol pushes down a select whose `selectList` is an empty array, which the protocol permits whenever only the row count matters
* *THEN* the generated MySQL statement MUST project a single constant column rather than no column
* *AND* the statement MUST still read from the source table, so the row count is preserved
* *AND* the `EMITS` clause MUST declare exactly one column, named positionally
* *AND* that column MUST declare the Exasol type `DECIMAL(1,0)`, which is the type of the constant the statement projects, because an empty `selectList` carries no `selectListDataTypes` entry and no involved column to take a type from

### Scenario: Pushdown response carries no credential

* *GIVEN* a pushdown request against a virtual schema, for each driver this release supports
* *WHEN* the adapter returns its `sql` response
* *THEN* the returned SQL text MUST NOT contain the source password, the source user, or any BucketFS credential
* *AND* the embedded scan specification MUST reference the `CONNECTION` object by name only
