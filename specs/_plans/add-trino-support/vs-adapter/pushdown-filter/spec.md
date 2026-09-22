# Feature: pushdown_filter

Delegates a query's `WHERE` clause to the source, rendering each predicate so the source selects exactly the rows Exasol would have selected itself.

<!-- DELTA:CHANGED -->
## Background

- This feature owns what a filter fragment is, for every dialect: how predicates nest and keep their grouping, that operands appear in the order the request gave them, and that a predicate in select-list position renders as a projected boolean. Each dialect's literal forms and advertised filter capabilities live in its own `<technology>-pushdown` feature; each dialect's collation placement lives in its own `<technology>-collation` feature.
- Exasol never re-applies a delegated filter, so a predicate the source evaluates differently than Exasol returns wrong rows rather than an error. Every rendered predicate therefore carries whatever syntax its target takes to reproduce Exasol's own semantics, and the dialects need different amounts of it because their defaults sit at different distances from Exasol's.
- Exasol asks two different questions of a character value. Equality asks whether two values are the same characters; ordering asks which comes first, and Exasol answers it by code point. A dialect renders a collation wherever its target's default answers one of those questions differently, and which collation on which operand is a fact about the target rather than about the predicate. A target whose default already answers both questions Exasol's way renders none.
- Every character operand a delegated predicate carries is Exasol `VARCHAR`, and Exasol pads no `VARCHAR`. `vs-adapter/type-mapping` maps a fixed-width source character type to Exasol `VARCHAR` or leaves it unmapped, so no blank-padded Exasol type reaches this path and no separate padding rule is needed here.
- A string literal is rendered in a form no server setting can change, in every dialect, and no dialect renders a bare quoted string whose meaning a mode can shift. The form is rendered unconditionally rather than only for a value that needs it, so no literal can change the shape of the generated statement. Each dialect's own form is stated in its own feature.
- Date and timestamp literals are rendered as typed literals in every dialect — `DATE '...'` and `TIMESTAMP '...'` — so no source coerces a string by its own rules.
- A `LIKE` pattern reaches the adapter spelled under Exasol's own escape rule, which treats a backslash as escaping the character after it when the query gave no `ESCAPE` clause. Reproducing that rule is per dialect, because the targets' defaults differ: MySQL and PostgreSQL apply the same backslash rule, verified on Exasol 2025.2.1 and MySQL 8.4 and expected of PostgreSQL by its documented default, so those two dialects render the pattern unchanged with no `ESCAPE` clause. Trino defines no default escape character at all and requires the clause to name one, so its dialect renders one for every `LIKE`. The request's own `escapeChar` is rendered as an explicit `ESCAPE` clause in every dialect when the request carries one.
- Exasol normalizes `>` and `>=` into `predicate_less` and `predicate_lessequal` with the operands swapped, so no dialect renders a greater-than predicate and every dialect reads the operands by position. Confirmed on Exasol 2025.2.1: `i > 5` arrives as `predicate_less` whose `left` is the literal `5` and whose `right` is the column `i`.
- A predicate also arrives in select-list position, not only in `filter`. `vs-adapter/pushdown-dialect` records the measurement: once `SELECTLIST_EXPRESSIONS` is advertised — which it must be for an aggregate to reach the adapter at all — Exasol places a bare predicate in `selectList` and declares it `BOOLEAN`. Each dialect's predicate renderer serves both positions: a predicate rendered one way inside `WHERE` and another way in the select list would be two answers to one question.
- One select-list shape is excepted and fails with a named error rather than rendering: a predicate reading its own query's character-typed `GROUP BY` key outside an aggregate. `vs-adapter/pushdown-aggregation` owns that exclusion, its measurement, and which dialects it binds. Nothing about a predicate in `filter` position changes for any dialect: every target evaluates `WHERE` before grouping, so no `WHERE` operand is subject to a grouping rule.
- `REGEXP_LIKE` is not delegated by any dialect: Exasol's regular-expression dialect is PCRE, MySQL 8's is ICU, PostgreSQL's is POSIX, and Trino's is `RE2J`, so an identical pattern can match different rows against any of them.
- Every dialect renders `<>` for `predicate_notequal` and parenthesizes every nested predicate, because every target accepts the same standard forms and a dialect gains nothing from spelling a shared form differently.
<!-- /DELTA:CHANGED -->

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: LIKE renders its pattern and its escape character

* *GIVEN* a `predicate_like` over a character column with the pattern `Wid%`
* *WHEN* each dialect this release supports renders that predicate
* *THEN* a request carrying an `escapeChar` MUST render an explicit `ESCAPE` clause over that character
* *AND* a request carrying no `escapeChar` MUST render the `ESCAPE` clause a dialect needs to reproduce Exasol's own backslash rule, and no clause where the target already applies that rule
* *AND* the pattern MUST be rendered through the same string-literal form the dialect renders every other literal in, which each `<technology>-pushdown` feature states for its own dialect
* *AND* whether the compared operand carries a collation suffix MUST follow the dialect's own `<technology>-collation` feature, because the targets answer Exasol's equality question differently
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: A pattern escaping something the target's escape rule does not admit fails rather than matching different rows

* *GIVEN* a `predicate_like` whose pattern carries its escape character followed by a character that is neither `%`, nor `_`, nor the escape character itself
* *WHEN* a dialect whose target rejects that sequence renders the predicate
* *THEN* the dialect MUST return an error naming the pattern and the sequence the target does not admit
* *AND* the dialect MUST NOT emit a statement the source rejects, because the failure reaches an operator as the source's own message about a pattern the operator never typed
* *AND* the dialect MUST NOT drop or rewrite the sequence, because a pattern matching a different set of rows than Exasol asked for is the wrong-rows outcome this release does not ship
* *AND* a dialect whose target admits the sequence MUST render the predicate rather than refusing it, so the exclusion binds the targets that have it rather than every dialect
<!-- /DELTA:NEW -->
