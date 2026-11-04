# Feature: pushdown_join_aliasing

Aliases every joined table by its position and qualifies every column, so a join statement names each column unambiguously.

## Background

- This feature owns positional aliasing, column qualification, and the error for a column that matches no table in the `from` tree, for every dialect.
- `vs-adapter/create-virtual-schema` owns the shape of the source namespace `adapterNotes` records, and each technology's own feature states its value.
- `vs-adapter/pushdown-join` owns the join types, outer-join null semantics, nesting, and composition with a delegated filter and aggregation.

## Scenarios

### Scenario: Every joined table is aliased and every column is qualified

* *GIVEN* a join whose two tables both declare a column named `id`, for each driver this release supports
* *WHEN* the dialect renders the statement
* *THEN* each of the join's tables in the `FROM` clause MUST carry an alias
* *AND* every column reference MUST be qualified by the alias of the table it belongs to
* *AND* every alias MUST be quoted with the dialect's own quoting character
* *AND* the `EMITS` clause MUST declare two distinct positional column names for the two `id` columns

### Scenario: Column that matches no table in the from tree fails with a named error

* *GIVEN* a join whose select list names a column whose `tableName` and `tableAlias` match no table in the `from` tree
* *WHEN* the adapter builds the response
* *THEN* the adapter MUST return an error
* *AND* the error message MUST name the column and the table reference it could not resolve
* *AND* the adapter MUST NOT emit an unqualified column reference, because every source this release supports would then resolve it against the wrong table or reject the statement

### Scenario: Aliases derive from table position, never from the request

* *GIVEN* a `pushdown` request whose joined tables carry request aliases, for each driver this release supports
* *WHEN* the dialect renders the statement
* *THEN* each table's alias MUST derive from its position in the `from` tree rather than from the request's own `alias`, because that alias is the namespace a column node's `tableAlias` or `tableName` resolves against, and rendering it would put a name the dialect does not control into the namespace it has to keep unambiguous
* *AND* every dialect MUST alias positionally, because every target rejects a duplicate alias, as the test MySQL does with error 1066 for a request alias colliding with a positional one, while MySQL with `lower_case_table_names=1` and Trino, which resolves every identifier case-insensitively, also collapse two aliases differing only in case, which Exasol sends for a pair of quoted aliases
* *AND* a single-table statement MUST alias and qualify nothing, even when the request assigned an alias, because one table's column reference cannot be ambiguous
