# Feature: pushdown_filter

Delegates a query's `WHERE` clause to the source, rendering each predicate so the source selects exactly the rows Exasol would have selected itself.

## Background

- This feature owns what a filter fragment is, for every dialect: how predicates nest and keep their grouping, that operands appear in the order the request gave them, and that a predicate in select-list position renders as a projected boolean. Each dialect's literal forms and advertised filter capabilities live in `vs-adapter/mysql-pushdown` and `vs-adapter/postgresql-pushdown`; each dialect's collation placement lives in `vs-adapter/mysql-collation` and `vs-adapter/postgresql-collation`.
- Exasol never re-applies a delegated filter, so a predicate the source evaluates differently than Exasol returns wrong rows rather than an error. Every rendered predicate therefore carries whatever syntax its target takes to reproduce Exasol's own semantics, and the dialects need different amounts of it because their defaults sit at different distances from Exasol's.
- Exasol asks two different questions of a character value. Equality asks whether two values are the same characters; ordering asks which comes first, and Exasol answers it by code point. A dialect renders a collation wherever its target's default answers one of those questions differently, and which collation on which operand is a fact about the target rather than about the predicate.
- Every character operand a delegated predicate carries is Exasol `VARCHAR`, and Exasol pads no `VARCHAR`. `vs-adapter/type-mapping` maps a fixed-width source character type to Exasol `VARCHAR`, so no blank-padded Exasol type reaches this path and no separate padding rule is needed here.
- A string literal is rendered in a form no server setting can change, in every dialect, and no dialect renders a bare quoted string whose meaning a mode can shift. The form is rendered unconditionally rather than only for a value that needs it, so no literal can change the shape of the generated statement. Each dialect's own form is stated in its own feature.
- Date and timestamp literals are rendered as typed literals in every dialect — `DATE '...'` and `TIMESTAMP '...'`, which both targets accept — so no source coerces a string by its own rules.
- MySQL, PostgreSQL, and Exasol all treat a backslash as escaping the following character in a `LIKE` pattern when no `ESCAPE` clause is given, verified on Exasol 2025.2.1 and MySQL 8.4 and expected of PostgreSQL by its documented default. The pattern is passed through unchanged, and the request's `escapeChar` is rendered as an explicit `ESCAPE` clause when present.
- Exasol normalizes `>` and `>=` into `predicate_less` and `predicate_lessequal` with the operands swapped, so no dialect renders a greater-than predicate and every dialect reads the operands by position. Confirmed on Exasol 2025.2.1: `i > 5` arrives as `predicate_less` whose `left` is the literal `5` and whose `right` is the column `i`.
- A predicate also arrives in select-list position, not only in `filter`. `vs-adapter/pushdown-dialect` records the measurement: once `SELECTLIST_EXPRESSIONS` is advertised — which it must be for an aggregate to reach the adapter at all — Exasol places a bare predicate in `selectList` and declares it `BOOLEAN`. Each dialect's predicate renderer serves both positions: a predicate rendered one way inside `WHERE` and another way in the select list would be two answers to one question.
- One select-list shape is excepted and fails with a named error rather than rendering: a predicate reading its own query's character-typed `GROUP BY` key outside an aggregate. `vs-adapter/pushdown-aggregation` owns that exclusion, its measurement, and which dialects it binds. Nothing about a predicate in `filter` position changes for any dialect: every target evaluates `WHERE` before grouping, so no `WHERE` operand is subject to a grouping rule.
- `REGEXP_LIKE` is not delegated by any dialect: Exasol's regular-expression dialect is PCRE, MySQL 8's is ICU, and PostgreSQL's is POSIX, so an identical pattern can match different rows against either source.
- Every dialect renders `<>` for `predicate_notequal` and parenthesizes every nested predicate, because both targets accept the same standard forms and a dialect gains nothing from spelling a shared form differently.

## Scenarios

### Scenario: Numeric comparison is rendered as a WHERE clause

* *GIVEN* a virtual table `orders` with a mapped `DECIMAL` column `id`
* *WHEN* Exasol pushes down a filter comparing `id` to the literal 2, for each dialect this release supports
* *THEN* the generated statement MUST carry a `WHERE` clause comparing the quoted column to the literal
* *AND* the operands MUST appear in the order the request gave them
* *AND* the numeric literal MUST be rendered without quotes and MUST carry no collation suffix, because a numeric comparison needs none

### Scenario: Conjunction, disjunction, and negation nest without losing grouping

* *GIVEN* a filter combining two comparisons under `predicate_or` and negating the result
* *WHEN* the dialect renders that filter
* *THEN* the rendered MySQL clause MUST parenthesize each nested predicate
* *AND* the rows MySQL returns MUST equal the rows Exasol returns for the same filter over the same data

### Scenario: IS NULL and IS NOT NULL are rendered without a comparison

* *GIVEN* a nullable mapped column
* *WHEN* Exasol pushes down `predicate_is_null` and `predicate_is_not_null` over it
* *THEN* the rendered clauses MUST be `IS NULL` and `IS NOT NULL`
* *AND* neither MUST be rendered as a comparison against a null literal, because that comparison is never true

### Scenario: LIKE renders its pattern and its escape character

* *GIVEN* a `predicate_like` over a character column with the pattern `Wid%`
* *WHEN* each dialect this release supports renders that predicate
* *THEN* a request carrying an `escapeChar` MUST render an explicit `ESCAPE` clause over that character
* *AND* a request carrying no `escapeChar` MUST render no `ESCAPE` clause
* *AND* the pattern MUST be rendered through the same string-literal form the dialect renders every other literal in, which `vs-adapter/mysql-pushdown` and `vs-adapter/postgresql-pushdown` each state for their own dialect
* *AND* whether the compared operand carries a collation suffix MUST follow `vs-adapter/mysql-collation` and `vs-adapter/postgresql-collation`, because the two targets answer Exasol's equality question differently

### Scenario: BETWEEN and IN keep their operand order

* *GIVEN* a `predicate_between` over a numeric column and a `predicate_in_constlist` over a character column
* *WHEN* each dialect this release supports renders each predicate
* *THEN* the `BETWEEN` bounds MUST appear in the order the request gave them
* *AND* the `IN` list entries MUST appear in the order the request gave them
* *AND* whether the character `IN` comparison carries a collation suffix MUST follow the dialect's own collation feature, and a dialect that renders one MUST place it on the compared operand rather than on the list

### Scenario: Predicate in select-list position renders as a projected boolean

* *GIVEN* a `pushdown` request carrying no `group_by` aggregation, whose select list carries a `predicate_less` over a mapped column and whose `selectListDataTypes` declares that entry `BOOLEAN`
* *WHEN* each dialect this release supports renders the statement
* *THEN* the generated statement MUST project the parenthesized comparison the same renderer produces for a `WHERE` clause, including whatever collation suffix that dialect's own feature requires in `WHERE` position, so one renderer serves both positions
* *AND* the `EMITS` clause MUST declare that column `BOOLEAN`
* *AND* the request MUST NOT fail, because the query Exasol delegated returns a boolean column today with Exasol computing it, and Exasol does not re-plan after an adapter error
* *AND* a predicate whose operand is null MUST project a null rather than a false, because Exasol and both targets evaluate a comparison against a null operand to null
