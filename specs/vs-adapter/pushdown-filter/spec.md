# Feature: pushdown_filter

Delegates a query's `WHERE` clause to MySQL, rendering each predicate so MySQL selects exactly the rows Exasol would have selected itself.

## Background

- Exasol never re-applies a delegated filter, so a predicate MySQL evaluates differently than Exasol returns wrong rows rather than an error. Every rendered predicate therefore carries whatever MySQL syntax it takes to reproduce Exasol's own semantics.
- Every character operand a delegated predicate carries is Exasol `VARCHAR`, and Exasol pads no `VARCHAR`. `vs-adapter/type-mapping` maps MySQL `CHAR` to Exasol `VARCHAR`, so no blank-padded Exasol type reaches this path and no separate `CHAR` padding rule is needed here.
- Exasol compares character values case-sensitively and accent-sensitively, and pads no `VARCHAR`. MySQL 8.4's default column collation `utf8mb4_0900_ai_ci` is case-insensitive and accent-insensitive: measured against the release's own fixture, `name = 'widget'` matches the stored value `Widget`, and `'Gizmo' = 'Gîzmo'` is true. `utf8mb4_0900_as_cs` reproduces Exasol's semantics on all three counts and is the collation a comparison operand carries. `utf8mb4_bin` does not, because MySQL reports it as `PAD SPACE`. The `utf8mb4_0900_bin` collation `vs-adapter/pushdown-aggregation` renders for `MIN` and `MAX` is not used here: the measured comparison evidence covers `utf8mb4_0900_as_cs` only.
- A character-typed operand is therefore rendered with an explicit `COLLATE utf8mb4_0900_as_cs` suffix. MySQL cannot use an index on a column carrying an explicit non-matching collation, so a character predicate filters at the source without index support. Numeric and temporal predicates carry no collation and keep their indexes.
- `utf8mb4_0900_as_cs` is the collation of every equality question this release delegates, not of comparisons alone. `vs-adapter/pushdown-aggregation` renders the same suffix on a character `GROUP BY` key and on a character `DISTINCT` aggregate operand, and on a character-typed aggregate operand of a select-list predicate. One collation per question, chosen by the operand's declared type, is the rule; which sites ask the equality question is settled per feature.
- A string literal is rendered as `_utf8mb4 X'<hex>'`, the hex encoding of its UTF-8 bytes. Backslash-escaped quoting is not mode-independent: with `NO_BACKSLASH_ESCAPES` set on the source, `'a\\b'` is four characters rather than three, while `_utf8mb4 X'615C62'` is three characters in both modes. Hex rendering also removes the escaping decision entirely, so no literal can change the shape of the generated statement.
- MySQL and Exasol agree that a backslash escapes the following character in a `LIKE` pattern when no `ESCAPE` clause is given, verified on Exasol 2025.2.1 and MySQL 8.4. The pattern is passed through unchanged, and the request's `escapeChar` is rendered as an explicit `ESCAPE` clause when present.
- Date and timestamp literals are rendered as MySQL typed literals, `DATE '...'` and `TIMESTAMP '...'`, so MySQL does not coerce a string by its own rules.
- Exasol normalizes `>` and `>=` into `predicate_less` and `predicate_lessequal` with the operands swapped, so this release renders no greater-than predicate and reads both operands by position. Confirmed on Exasol 2025.2.1: `i > 5` arrives as `predicate_less` whose `left` is the literal `5` and whose `right` is the column `i`.
- A predicate also arrives in select-list position, not only in `filter`. `vs-adapter/pushdown-dialect` records the measurement: once `SELECTLIST_EXPRESSIONS` is advertised — which it must be for an aggregate to reach the adapter at all — Exasol places a bare predicate in `selectList` and declares it `BOOLEAN`. The same predicate renderer serves both positions: a predicate rendered one way inside `WHERE` and another way in the select list would be two answers to one question.
- One select-list shape is excepted and fails with a named error rather than rendering: a predicate reading its own query's character-typed `GROUP BY` key outside an aggregate, which MySQL rejects with error 1055 under `ONLY_FULL_GROUP_BY` at every collation placement. `vs-adapter/pushdown-aggregation` owns that exclusion and its measurement, because the grouping key is what creates it. Nothing about a predicate in `filter` position changes: MySQL evaluates `WHERE` before grouping, so no `WHERE` operand is subject to the mode.
- `REGEXP_LIKE` is not delegated: Exasol's regular-expression dialect is PCRE and MySQL 8's is ICU, so an identical pattern can match different rows.

## Scenarios

### Scenario: Filter capabilities cover the predicates and literals this release renders

* *GIVEN* a virtual schema backed by MySQL
* *WHEN* Exasol sends a `getCapabilities` request
* *THEN* the `capabilities` array MUST contain `FILTER_EXPRESSIONS`, and exactly the predicate capabilities `FN_PRED_AND`, `FN_PRED_OR`, `FN_PRED_NOT`, `FN_PRED_EQUAL`, `FN_PRED_NOTEQUAL`, `FN_PRED_LESS`, `FN_PRED_LESSEQUAL`, `FN_PRED_IS_NULL`, `FN_PRED_IS_NOT_NULL`, `FN_PRED_LIKE`, `FN_PRED_LIKE_ESCAPE`, `FN_PRED_BETWEEN`, and `FN_PRED_IN_CONSTLIST`
* *AND* it MUST contain exactly the literal capabilities `LITERAL_NULL`, `LITERAL_EXACTNUMERIC`, `LITERAL_DOUBLE`, `LITERAL_STRING`, `LITERAL_DATE`, and `LITERAL_TIMESTAMP`
* *AND* it MUST NOT contain `FN_PRED_REGEXP_LIKE`, `FN_PRED_IS_JSON`, `FN_PRED_IS_NOT_JSON`, `LITERAL_BOOL`, `LITERAL_INTERVAL`, or `LITERAL_TIMESTAMP_UTC`

### Scenario: Numeric comparison is rendered as a WHERE clause

* *GIVEN* a virtual table `orders` with a mapped `DECIMAL` column `id`
* *WHEN* Exasol pushes down a filter comparing `id` to the literal 2
* *THEN* the generated MySQL statement MUST carry a `WHERE` clause comparing the quoted column to the literal
* *AND* the numeric literal MUST be rendered without quotes
* *AND* the statement MUST carry no collation suffix, because a numeric comparison needs none

### Scenario: Character comparison reproduces Exasol's case and accent sensitivity

* *GIVEN* a MySQL `VARCHAR` column whose collation is `utf8mb4_0900_ai_ci` holding the value `Widget`
* *WHEN* Exasol pushes down a filter comparing that column to the literal `widget`
* *THEN* the rendered operand MUST carry the suffix `COLLATE utf8mb4_0900_as_cs`
* *AND* the query MUST return no row, matching what Exasol returns for the same comparison
* *AND* the same MUST hold for a value differing only in an accent

### Scenario: String literal is rendered as a hex literal

* *GIVEN* a filter carrying a string literal
* *WHEN* the dialect renders that literal
* *THEN* the emitted literal MUST be the form `_utf8mb4 X'<hex>'` over the UTF-8 bytes of the value
* *AND* a literal containing a single quote, a backslash, or a multi-byte character MUST render to hex without an escape sequence
* *AND* the emitted statement MUST NOT contain the literal's raw characters
* *AND* an empty string literal MUST render as `_utf8mb4 X''`

### Scenario: Conjunction, disjunction, and negation nest without losing grouping

* *GIVEN* a filter combining two comparisons under `predicate_or` and negating the result
* *WHEN* the dialect renders that filter
* *THEN* the rendered MySQL clause MUST parenthesize each nested predicate
* *AND* the rows MySQL returns MUST equal the rows Exasol returns for the same filter over the same data

### Scenario: LIKE renders its pattern and its escape character

* *GIVEN* a `predicate_like` over a character column with the pattern `Wid%`
* *WHEN* the dialect renders that predicate
* *THEN* the rendered clause MUST apply `COLLATE utf8mb4_0900_as_cs` to the compared operand
* *AND* the pattern MUST be rendered as a hex string literal
* *AND* a request carrying an `escapeChar` MUST render an explicit `ESCAPE` clause over that character
* *AND* a request carrying no `escapeChar` MUST render no `ESCAPE` clause

### Scenario: IS NULL and IS NOT NULL are rendered without a comparison

* *GIVEN* a nullable mapped column
* *WHEN* Exasol pushes down `predicate_is_null` and `predicate_is_not_null` over it
* *THEN* the rendered clauses MUST be `IS NULL` and `IS NOT NULL`
* *AND* neither MUST be rendered as a comparison against a null literal, because that comparison is never true

### Scenario: BETWEEN and IN keep their operand order

* *GIVEN* a `predicate_between` over a numeric column and a `predicate_in_constlist` over a character column
* *WHEN* the dialect renders each predicate
* *THEN* the `BETWEEN` bounds MUST appear in the order the request gave them
* *AND* the `IN` list entries MUST appear in the order the request gave them
* *AND* the character `IN` comparison MUST carry the collation suffix on the compared operand rather than on the list

### Scenario: Temporal literals are rendered as typed MySQL literals

* *GIVEN* a filter comparing a mapped `DATE` column and a mapped `TIMESTAMP` column to literals
* *WHEN* the dialect renders those literals
* *THEN* the date literal MUST be rendered as `DATE '<value>'`
* *AND* the timestamp literal MUST be rendered as `TIMESTAMP '<value>'`
* *AND* neither MUST be rendered as a bare string, because MySQL would then apply its own coercion rules

### Scenario: Predicate in select-list position renders as a projected boolean

* *GIVEN* a `pushdown` request carrying no `group_by` aggregation, whose select list carries a `predicate_less` over a mapped column and whose `selectListDataTypes` declares that entry `BOOLEAN`
* *WHEN* the dialect renders the statement
* *THEN* the generated statement MUST project the parenthesized comparison the same renderer produces for a `WHERE` clause, including the collation suffix on a character operand
* *AND* the `EMITS` clause MUST declare that column `BOOLEAN`
* *AND* the request MUST NOT fail, because the query Exasol delegated returns a boolean column today with Exasol computing it, and Exasol does not re-plan after an adapter error
* *AND* a predicate whose operand is null MUST project a null rather than a false, because MySQL and Exasol both evaluate such a comparison to null

### Scenario: Filter pushdown reaches MySQL rather than Exasol

* *GIVEN* a virtual schema over a MySQL table holding three rows
* *WHEN* Exasol runs a `SELECT` whose `WHERE` clause matches one row
* *THEN* `EXPLAIN VIRTUAL` MUST report a generated statement carrying that `WHERE` clause
* *AND* the query MUST return exactly the matching row
* *AND* the generated statement MUST NOT read the unmatched rows
