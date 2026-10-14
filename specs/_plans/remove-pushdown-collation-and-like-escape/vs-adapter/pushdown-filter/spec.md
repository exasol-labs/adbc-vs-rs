<!-- DELTA:CHANGED -->
# Feature: pushdown_filter

Delegates a query's `WHERE` clause to the source, rendering each predicate in the source's own SQL so the source selects the rows, with every character comparison evaluated under the collation the source applies to the column.
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
## Background

- This feature owns what a filter fragment is, for every dialect: how predicates nest and keep their grouping, that operands appear in the order the request gave them, and that a predicate in select-list position renders as a projected boolean. Each dialect's literal forms and advertised filter capabilities live in its own `<technology>-pushdown` feature; each `<technology>-collation` feature states what its source's collation does to a delegated character comparison.
- Exasol never re-applies a delegated filter, so the source's evaluation of a delegated predicate is the answer the query returns. Each dialect renders a predicate in its target's own form and adds no syntax whose purpose is to reproduce an Exasol semantic the target does not share. Two consequences follow and are stated rather than compensated: a character comparison follows the source's collation, and a `LIKE` pattern follows the target's escape rule.
- Exasol compares character values by code point, and no dialect renders a collation. A delegated character comparison, `LIKE`, `BETWEEN`, `IN`, and select-list predicate is therefore evaluated under the collation the source applies to the column, which can equate values Exasol keeps apart and order values differently. Each `<technology>-collation` feature states what its source's default collation does and how a user obtains Exasol's comparison.
- Every character operand a delegated predicate carries is Exasol `VARCHAR`, and Exasol pads no `VARCHAR`. `vs-adapter/type-mapping` maps a fixed-width source character type to Exasol `VARCHAR` or leaves it unmapped, so no blank-padded Exasol type reaches this path and no separate padding rule is needed here.
- A string literal is rendered in a form no server setting can change, in every dialect, and no dialect renders a bare quoted string whose meaning a mode can shift. The form is rendered unconditionally rather than only for a value that needs it, so no literal can change the shape of the generated statement. Each dialect's own form is stated in its own feature.
- Date and timestamp literals are rendered as typed literals in every dialect — `DATE '...'` and `TIMESTAMP '...'` — so no source coerces a string by its own rules.
- A `LIKE` pattern reaches the adapter spelled under Exasol's own escape rule, which treats a backslash as escaping the character after it when the query gave no `ESCAPE` clause. No dialect renders an `ESCAPE` clause for a request carrying no `escapeChar`, so the target's own rule reads the pattern. MySQL and PostgreSQL apply the same backslash rule, verified on Exasol 2025.2.1 and MySQL 8.4 and expected of PostgreSQL by its documented default. Trino defines no default escape character, so a backslash in a pattern delegated to Trino is an ordinary character, as `vs-adapter/trino-pushdown` states. A request's own `escapeChar` is rendered as an explicit `ESCAPE` clause by every dialect that advertises `FN_PRED_LIKE_ESCAPE`.
- Exasol normalizes `>` and `>=` into `predicate_less` and `predicate_lessequal` with the operands swapped, so no dialect renders a greater-than predicate and every dialect reads the operands by position. Confirmed on Exasol 2025.2.1: `i > 5` arrives as `predicate_less` whose `left` is the literal `5` and whose `right` is the column `i`.
- A predicate also arrives in select-list position, not only in `filter`. `vs-adapter/pushdown-dialect` records the measurement: once `SELECTLIST_EXPRESSIONS` is advertised — which it must be for an aggregate to reach the adapter at all — Exasol places a bare predicate in `selectList` and declares it `BOOLEAN`. Each dialect's predicate renderer serves both positions: a predicate rendered one way inside `WHERE` and another way in the select list would be two answers to one question.
- `REGEXP_LIKE` is not delegated by any dialect: Exasol's regular-expression dialect is PCRE, MySQL 8's is ICU, PostgreSQL's is POSIX, and Trino's is `RE2J`, so an identical pattern can match different rows against any of them.
- Every dialect renders `<>` for `predicate_notequal` and parenthesizes every nested predicate, because every target accepts the same standard forms and a dialect gains nothing from spelling a shared form differently.
<!-- /DELTA:CHANGED -->

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Conjunction, disjunction, and negation nest without losing grouping

* *GIVEN* a filter combining two comparisons under `predicate_or` and negating the result
* *WHEN* the dialect renders that filter
* *THEN* the rendered MySQL clause MUST parenthesize each nested predicate
* *AND* the rows MySQL returns MUST equal the rows Exasol returns for the same filter over the same data, where every compared character column declares a code-point collation at the source, as `vs-adapter/mysql-collation` states
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: LIKE renders its pattern and its escape character

* *GIVEN* a `predicate_like` over a character column with the pattern `Wid%`
* *WHEN* each dialect this release supports renders that predicate
* *THEN* a request carrying an `escapeChar` MUST render an explicit `ESCAPE` clause over that character in every dialect that advertises `FN_PRED_LIKE_ESCAPE`
* *AND* a request carrying no `escapeChar` MUST render no `ESCAPE` clause in any dialect
* *AND* the pattern MUST be rendered through the same string-literal form the dialect renders every other literal in, which each `<technology>-pushdown` feature states for its own dialect
* *AND* the compared operand MUST carry no collation suffix in any dialect
<!-- /DELTA:CHANGED -->

<!-- DELTA:REMOVED -->
### Scenario: A pattern escaping something the target's escape rule does not admit fails rather than matching different rows

* *GIVEN* a `predicate_like` whose pattern carries its escape character followed by a character that is neither `%`, nor `_`, nor the escape character itself
* *WHEN* a dialect whose target rejects that sequence renders the predicate
* *THEN* the dialect MUST return an error naming the pattern and the sequence the target does not admit
* *AND* the dialect MUST NOT emit a statement the source rejects, because the failure reaches an operator as the source's own message about a pattern the operator never typed
* *AND* the dialect MUST NOT drop or rewrite the sequence, because a pattern matching a different set of rows than Exasol asked for is the wrong-rows outcome this release does not ship
* *AND* a dialect whose target admits the sequence MUST render the predicate rather than refusing it, so the exclusion binds the targets that have it rather than every dialect
<!-- /DELTA:REMOVED -->

<!-- DELTA:CHANGED -->
### Scenario: BETWEEN and IN keep their operand order

* *GIVEN* a `predicate_between` over a numeric column and a `predicate_in_constlist` over a character column
* *WHEN* each dialect this release supports renders each predicate
* *THEN* the `BETWEEN` bounds MUST appear in the order the request gave them
* *AND* the `IN` list entries MUST appear in the order the request gave them
* *AND* the character `IN` comparison MUST carry no collation suffix, on the compared operand or on the list
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Predicate in select-list position renders as a projected boolean

* *GIVEN* a `pushdown` request carrying no `group_by` aggregation, whose select list carries a `predicate_less` over a mapped column and whose `selectListDataTypes` declares that entry `BOOLEAN`
* *WHEN* each dialect this release supports renders the statement
* *THEN* the generated statement MUST project the parenthesized comparison the same renderer produces for a `WHERE` clause, so one renderer serves both positions
* *AND* the `EMITS` clause MUST declare that column `BOOLEAN`
* *AND* the request MUST NOT fail, because the query Exasol delegated returns a boolean column today with Exasol computing it, and Exasol does not re-plan after an adapter error
* *AND* a predicate whose operand is null MUST project a null rather than a false, because Exasol and both targets evaluate a comparison against a null operand to null
<!-- /DELTA:CHANGED -->
