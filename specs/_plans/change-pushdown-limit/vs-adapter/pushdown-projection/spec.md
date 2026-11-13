# Feature: pushdown_projection

Reports the query fragments this release can delegate to the source, and rewrites the projection fragment into a `SELECT` the scan UDF executes at that source.

<!-- DELTA:CHANGED -->
## Background

- This feature owns what a projection fragment is, for every dialect: which columns a select list projects and which are pruned, what an absent and an empty select list mean, how the `EMITS` clause names its columns, and that the response carries no credential. Every dialect quotes every identifier unconditionally, so a source identifier keeps its case, and each `<technology>-pushdown` feature states its quote character and escape rule.
- `vs-adapter/pushdown-dialect` owns the capability set each dialect advertises, which delegates projection, filters, aggregation, equi-joins, limits, and select-list expressions, and the rule that every advertised capability renders, because Exasol never re-applies a delegated fragment. `vs-adapter/result-ordering` owns the withheld ordering capabilities.
- `SELECTLIST_EXPRESSIONS` is advertised, so a select-list entry is a column, an aggregate, or a delegated predicate, and the adapter reads each entry's kind from its `type` field.
- The resolved dialect renders the SQL over a dialect-neutral query model, and the pushdown response's `sql` field invokes the scan UDF directly with a query-side dynamic `EMITS` clause, with no `IMPORT FROM SCRIPT` step for any dialect.
- The `EMITS` clause names its columns by position, because a join can project two columns of one name and an aggregate has no source column name. The scan specification carries the driver identity, the driver path, the `CONNECTION` object's name, the generated statement, and the declared output columns, and no credential.
<!-- /DELTA:CHANGED -->

## Scenarios

### Scenario: Named columns are projected and unnamed columns are pruned

* *GIVEN* a virtual table `orders` whose columns are `id`, `total`, and `note`
* *WHEN* Exasol pushes down a select whose `selectList` names `id` and `note`
* *THEN* the generated MySQL statement MUST project `id` and `note` from the qualified source table and MUST NOT project `total`
* *AND* the columns MUST appear in the order the `selectList` gives them
* *AND* the `EMITS` clause MUST declare two positionally named columns in that same order, carrying the Exasol types `id` and `note` are mapped to
