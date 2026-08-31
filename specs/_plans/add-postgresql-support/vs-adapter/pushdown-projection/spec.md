# Feature: pushdown_projection

Reports the query fragments this release can delegate to the source, and rewrites the projection fragment into a `SELECT` the scan UDF executes at that source.

## Background

<!-- DELTA:CHANGED -->
- This release advertises projection alongside filter, aggregation, equi-join, and select-list expression delegation. `vs-adapter/pushdown-dialect` owns the advertised set per dialect and the requirement that every advertised capability has a rendering path in the dialect that advertises it; `vs-adapter/result-ordering` owns what stays withheld for every dialect.
- A select list is no longer columns only. `SELECTLIST_EXPRESSIONS` is advertised, so an entry may be an aggregate or a delegated predicate, and each entry's kind is read from its `type` field. A select list of plain columns still parses and renders exactly as it did before.
- Once Exasol receives a capability it delegates that fragment fully and never re-applies it. Advertising a capability the adapter does not render returns wrong rows rather than an error, so the advertised set lives in the dialect that renders it and an invariant test asserts that every entry renders, once per dialect.
- The SQL is assembled by the resolved dialect's own renderer over a dialect-neutral query model. `polyglot-sql` is not a dependency in this release.
- Identifier quoting is per dialect and is the smallest difference between them. MySQL quotes with backticks and escapes an embedded backtick by doubling it; PostgreSQL quotes with double quotes and escapes an embedded double quote by doubling it. Both dialects quote every identifier unconditionally, which is also what keeps `vs-adapter/create-virtual-schema`'s rule that source identifiers keep their original case true against PostgreSQL, where an unquoted identifier folds to lower case rather than being preserved.
- The pushdown response's `sql` field invokes the scan UDF directly with a query-side dynamic `EMITS` clause. There is no `IMPORT FROM SCRIPT` step, for any dialect.
- The `EMITS` clause names its columns positionally rather than by source column name, because a join projects columns that may share a name and an aggregate expression has no source column name.
- The scan specification passed to the UDF carries the driver identity, the driver path, the `CONNECTION` object's name, the generated source statement, and the declared Exasol output columns. It carries no credential. The identity is what lets the scan load the library with the right ADBC entry-point symbol, per `vs-adapter/driver-selection`.
<!-- /DELTA:CHANGED -->

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Absent select list projects every mapped column

* *GIVEN* a virtual table `orders` with three mapped columns
* *WHEN* Exasol pushes down a select carrying no `selectList` field
* *THEN* the generated statement MUST project all three columns in the virtual table's declared order
* *AND* the generated statement MUST NOT use `SELECT *`, because a source column added after creation would then break the declared `EMITS` arity
* *AND* the same MUST hold for each dialect this release supports
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Backtick in an identifier is escaped

* *GIVEN* a MySQL table or column whose name contains a backtick
* *WHEN* the MySQL dialect builds the pushdown statement
* *THEN* the emitted identifier MUST double every backtick it contains
* *AND* the emitted identifier MUST remain enclosed in a single pair of backticks
* *AND* the adapter MUST NOT interpolate the raw name into the statement
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: Double quote in a PostgreSQL identifier is escaped

* *GIVEN* a PostgreSQL table or column whose name contains a double quote
* *WHEN* the PostgreSQL dialect builds the pushdown statement
* *THEN* the emitted identifier MUST double every double quote it contains
* *AND* the emitted identifier MUST remain enclosed in a single pair of double quotes
* *AND* the adapter MUST NOT interpolate the raw name into the statement
* *AND* a lower-case identifier MUST still be emitted quoted, because an unquoted PostgreSQL identifier folds to lower case and a mixed-case source name would then resolve to a table that does not exist
<!-- /DELTA:NEW -->

<!-- DELTA:CHANGED -->
### Scenario: Pushdown response carries no credential

* *GIVEN* a pushdown request against a virtual schema, for each driver this release supports
* *WHEN* the adapter returns its `sql` response
* *THEN* the returned SQL text MUST NOT contain the source password, the source user, or any BucketFS credential
* *AND* the embedded scan specification MUST reference the `CONNECTION` object by name only
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: Projection pushdown reaches PostgreSQL rather than Exasol

* *GIVEN* a virtual schema over a PostgreSQL table with eight mapped columns
* *WHEN* Exasol runs a `SELECT` naming two of them
* *THEN* `EXPLAIN VIRTUAL` MUST report a generated statement projecting only those two columns
* *AND* every identifier in that statement MUST be enclosed in double quotes
* *AND* the query MUST return those two columns with the values seeded in PostgreSQL
* *AND* the generated statement MUST NOT name any other mapped column
<!-- /DELTA:NEW -->
