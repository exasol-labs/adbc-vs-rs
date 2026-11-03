# Feature: pushdown_filter

Delegates a query's `WHERE` clause to the source, rendering each predicate in the source's own SQL so the source selects the rows, with every character comparison evaluated under the collation the source applies to the column.

<!-- DELTA:CHANGED -->
## Background

- This feature owns what a filter fragment is, for every dialect: how predicates nest and keep their grouping, that operands appear in the order the request gave them, that a `BOOLEAN` column is a predicate on its own, and that a predicate in select-list position renders as a projected boolean through the same renderer as in `WHERE`. Each dialect's literal forms and advertised filter capabilities live in its own `<technology>-pushdown` feature.
- Exasol never re-applies a delegated filter, so the source's evaluation of a delegated predicate is the answer the query returns. A dialect adds no syntax whose purpose is to reproduce an Exasol comparison rule the target does not share, so a character comparison, `LIKE`, `BETWEEN`, `IN`, and select-list predicate follow the collation the source applies to the column, and a `LIKE` pattern follows the target's escape rule, as each `<technology>-collation` and `<technology>-pushdown` feature states. A dialect MAY render a column reference converted to the source type that holds its declared Exasol type's value, so the source compares the value the scan emits, as `vs-adapter/postgresql-pushdown` states for PostgreSQL.
- Every character operand a delegated predicate carries is Exasol `VARCHAR`, and Exasol pads no `VARCHAR`, so no blank-padded type reaches this path. A string literal renders in a form no server setting can change, and date and timestamp literals render as typed literals, `DATE '...'` and `TIMESTAMP '...'`, in every dialect.
- A `LIKE` pattern reaches the adapter under Exasol's own escape rule, which treats a backslash as escaping the next character when the query gave no `ESCAPE` clause. No dialect renders an `ESCAPE` clause for a request carrying no `escapeChar`, MySQL and PostgreSQL apply the same backslash rule, and Trino defines no default escape character. A request's own `escapeChar` renders as an explicit `ESCAPE` clause in every dialect that advertises `FN_PRED_LIKE_ESCAPE`.
- Exasol normalizes `>` and `>=` into `predicate_less` and `predicate_lessequal` with the operands swapped, confirmed on the test Exasol, so every dialect reads operands by position. Every dialect renders `<>` for `predicate_notequal` and parenthesizes every nested predicate. `REGEXP_LIKE` is delegated by no dialect, because Exasol's PCRE, MySQL 8's ICU, PostgreSQL's POSIX, and Trino's `RE2J` can match one pattern against different rows.
<!-- /DELTA:CHANGED -->

## Scenarios

<!-- DELTA:NEW -->
### Scenario: BOOLEAN column is a predicate on its own

* *GIVEN* a `pushdown` request whose `filter` is a `column` entry whose declared type is `BOOLEAN`, and requests carrying that entry as an operand of `predicate_and`, `predicate_or`, and `predicate_not`
* *WHEN* each dialect parses and renders each request
* *THEN* the rendered `WHERE` clause MUST carry the quoted column as the predicate, with no comparison added
* *AND* for a PostgreSQL `boolean` column, the rows the source returns MUST equal the rows Exasol returns for the same filter over the same data
* *AND* a `column` entry of any other declared type in predicate position MUST fail with an error naming the entry type and the declared type, rather than render a non-boolean operand as a predicate
<!-- /DELTA:NEW -->
