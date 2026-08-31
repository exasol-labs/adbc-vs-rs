# Feature: pushdown_filter

Delegates a query's `WHERE` clause to the source, rendering each predicate so the source selects exactly the rows Exasol would have selected itself.

## Background

<!-- DELTA:CHANGED -->
- Exasol never re-applies a delegated filter, so a predicate the source evaluates differently than Exasol returns wrong rows rather than an error. Every rendered predicate therefore carries whatever syntax its target takes to reproduce Exasol's own semantics, and the two dialects need different amounts of it because their defaults sit at different distances from Exasol's.
- Exasol asks two different questions of a character value. Equality asks whether two values are the same characters; ordering asks which comes first, and Exasol answers it by code point. A dialect needs a collation wherever its target's default answers one of those questions differently from Exasol.
- MySQL's default answers both differently, so the MySQL dialect renders two collations. MySQL 8.4's default column collation `utf8mb4_0900_ai_ci` is case-insensitive and accent-insensitive: measured against the release's own fixture, `name = 'widget'` matches the stored value `Widget`, and `'Gizmo' = 'Gîzmo'` is true. `utf8mb4_0900_as_cs` reproduces Exasol's equality and is the collation a comparison operand carries; `utf8mb4_bin` does not, because MySQL reports it as `PAD SPACE`. `vs-adapter/pushdown-aggregation` renders `utf8mb4_0900_bin` where order decides the answer.
- PostgreSQL's default answers the equality question the way Exasol does and the ordering question differently, so the PostgreSQL dialect renders one collation and renders it only where order decides the answer. Under any deterministic collation PostgreSQL compares two character values equal only when their bytes are identical, which is Exasol's equality; but a locale collation orders by collation weight, so `'B' < 'a'` is true under Exasol's code-point order and false under a collation ordering `a` before `B`. The built-in `C` collation compares UTF-8 bytes, which for UTF-8 is code-point order, and so reproduces Exasol's ordering. One collation answers both of Exasol's questions for PostgreSQL where MySQL needs two, because `C` is byte equality as well as byte order.
- The PostgreSQL dialect therefore carries `COLLATE "C"` on a character operand of `predicate_less`, `predicate_lessequal`, and `predicate_between`, and on nothing else in this feature. An equality, `IN`, `IS NULL`, or `LIKE` operand carries no collation suffix, so those predicates keep the source's index support, which MySQL's character predicates lose. Whether the suffix is needed for `LIKE` in particular is settled by measurement rather than by the collation's definition: PostgreSQL's `LIKE` compares character by character rather than by collation weight, so the plan expects no suffix, and the plan's gate measures a pattern whose matching would differ if that were wrong.
- Relying on PostgreSQL's equality needs the column's collation to be deterministic, which is why `vs-adapter/type-mapping` unmaps a character column declaring a nondeterministic one. A nondeterministic ICU collation makes PostgreSQL equate values Exasol keeps apart — the same silent divergence MySQL's default collation causes — and makes `LIKE` fail outright. Unmapping the column is the same answer `vs-adapter/type-mapping` already gives a MySQL character column of the wrong character set.
- `C` is a built-in collation present in every PostgreSQL cluster rather than one a release added, so the PostgreSQL dialect needs no source-server version floor and `vs-adapter/create-virtual-schema` asserts none. What it does need is a UTF-8 database encoding, which that feature checks, because `C` reproduces code-point order only over UTF-8 bytes.
- A string literal is rendered so that no server setting can change its meaning, in both dialects, and neither renders a bare quoted string. MySQL renders `_utf8mb4 X'<hex>'`, the hex encoding of the value's UTF-8 bytes, because with `NO_BACKSLASH_ESCAPES` set on the source `'a\\b'` is four characters rather than three. PostgreSQL renders an escape-string constant, `E'<body>'`, escaping a backslash as `\\` and a single quote as `''`: the `E` prefix selects backslash-escape processing whatever `standard_conforming_strings` is set to, and doubling the quote rather than backslash-escaping it keeps the literal independent of `backslash_quote` too. Both forms are rendered unconditionally rather than only for a value that needs them, so no literal can change the shape of the generated statement.
- PostgreSQL's hex route is rejected rather than untried, and the reason is typing rather than verbosity. A decoded hex literal — `convert_from(decode('<hex>','hex'),'UTF8')` — is typed `text`, and PostgreSQL applies no implicit coercion from `text` to a numeric or temporal column, so comparing one against a mapped `DECIMAL` or `DATE` column fails outright with an unknown-operator error where MySQL would silently coerce. `E'<body>'` yields an untyped literal PostgreSQL resolves from the other operand, which is the behavior every non-character comparison in this feature depends on. The same strictness is why a literal is never rendered with a cast it does not need.

- MySQL, PostgreSQL, and Exasol all treat a backslash as escaping the following character in a `LIKE` pattern when no `ESCAPE` clause is given, verified on Exasol 2025.2.1 and MySQL 8.4 and expected of PostgreSQL by its documented default. The pattern is passed through unchanged, and the request's `escapeChar` is rendered as an explicit `ESCAPE` clause when present.
- Date and timestamp literals are rendered as typed literals in both dialects — `DATE '...'` and `TIMESTAMP '...'`, which both targets accept — so neither source coerces a string by its own rules.
- Exasol normalizes `>` and `>=` into `predicate_less` and `predicate_lessequal` with the operands swapped, so neither dialect renders a greater-than predicate and both read the operands by position. Confirmed on Exasol 2025.2.1: `i > 5` arrives as `predicate_less` whose `left` is the literal `5` and whose `right` is the column `i`.
- A predicate also arrives in select-list position, not only in `filter`. `vs-adapter/pushdown-dialect` records the measurement: once `SELECTLIST_EXPRESSIONS` is advertised — which it must be for an aggregate to reach the adapter at all — Exasol places a bare predicate in `selectList` and declares it `BOOLEAN`. Each dialect's predicate renderer serves both positions: a predicate rendered one way inside `WHERE` and another way in the select list would be two answers to one question.
- One select-list shape is excepted and fails with a named error rather than rendering: a predicate reading its own query's character-typed `GROUP BY` key outside an aggregate. `vs-adapter/pushdown-aggregation` owns that exclusion, its measurement, and which dialects it binds. Nothing about a predicate in `filter` position changes for either dialect: both targets evaluate `WHERE` before grouping, so no `WHERE` operand is subject to a grouping rule.
- `REGEXP_LIKE` is not delegated by either dialect: Exasol's regular-expression dialect is PCRE, MySQL 8's is ICU, and PostgreSQL's is POSIX, so an identical pattern can match different rows against either source.
- Both dialects render `<>` for `predicate_notequal` and parenthesize every nested predicate, because both targets accept the same standard forms and a dialect gains nothing from spelling a shared form differently.
<!-- /DELTA:CHANGED -->

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Filter capabilities cover the predicates and literals this release renders

* *GIVEN* a virtual schema, for each driver this release supports
* *WHEN* Exasol sends a `getCapabilities` request
* *THEN* the `capabilities` array MUST contain `FILTER_EXPRESSIONS`, and exactly the predicate capabilities `FN_PRED_AND`, `FN_PRED_OR`, `FN_PRED_NOT`, `FN_PRED_EQUAL`, `FN_PRED_NOTEQUAL`, `FN_PRED_LESS`, `FN_PRED_LESSEQUAL`, `FN_PRED_IS_NULL`, `FN_PRED_IS_NOT_NULL`, `FN_PRED_LIKE`, `FN_PRED_LIKE_ESCAPE`, `FN_PRED_BETWEEN`, and `FN_PRED_IN_CONSTLIST`
* *AND* it MUST contain exactly the literal capabilities `LITERAL_NULL`, `LITERAL_EXACTNUMERIC`, `LITERAL_DOUBLE`, `LITERAL_STRING`, `LITERAL_DATE`, and `LITERAL_TIMESTAMP`
* *AND* it MUST NOT contain `FN_PRED_REGEXP_LIKE`, `FN_PRED_IS_JSON`, `FN_PRED_IS_NOT_JSON`, `LITERAL_BOOL`, `LITERAL_INTERVAL`, or `LITERAL_TIMESTAMP_UTC`
* *AND* both dialects MUST advertise the same filter and literal capabilities, because every predicate this feature renders has a form in both targets — `LITERAL_BOOL` included, withheld for PostgreSQL despite its native boolean type because `vs-adapter/type-mapping` maps no source column to Exasol `BOOLEAN` and a boolean literal would have no mapped column to compare against
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: PostgreSQL collates an ordering comparison and leaves equality bare

* *GIVEN* a PostgreSQL `character varying` column and a `pushdown` request carrying `predicate_equal`, `predicate_less`, `predicate_lessequal`, `predicate_between`, `predicate_in_constlist`, and `predicate_like` over it
* *WHEN* the PostgreSQL dialect renders each predicate
* *THEN* the character operand of `predicate_less`, `predicate_lessequal`, and `predicate_between` MUST carry the suffix `COLLATE "C"`
* *AND* the character operand of `predicate_equal`, `predicate_in_constlist`, and `predicate_like` MUST carry no collation suffix
* *AND* a numeric or temporal operand MUST carry no collation suffix in any predicate
* *AND* the suffix MUST be rendered on the compared operand rather than on a literal or on an `IN` list
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: PostgreSQL character comparison reproduces Exasol's case, accent, and code-point order

* *GIVEN* a virtual table over a PostgreSQL `character varying` column and a local Exasol table holding identical values, including two differing only by case, two differing only by an accent, and the values `B`, `a`, and `C`
* *WHEN* the same equality comparison and the same ordering comparison are run over each
* *THEN* the equality comparison over the virtual table MUST return exactly the rows the local query returns, so no two values Exasol keeps apart are matched
* *AND* the ordering comparison over the virtual table MUST return exactly the rows the local query returns, so the delegated comparison follows Exasol's code-point order rather than a locale's collation weight
* *AND* `EXPLAIN VIRTUAL` MUST report `COLLATE "C"` on the ordering comparison's operand and no collation suffix on the equality comparison's operand
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: PostgreSQL string literal is rendered as an escape-string constant

* *GIVEN* a filter carrying a string literal
* *WHEN* the PostgreSQL dialect renders that literal
* *THEN* the emitted literal MUST be the form `E'<body>'`, with a backslash emitted as two backslashes and a single quote as two single quotes
* *AND* a literal containing a single quote, a backslash, or a multi-byte character MUST render to that one form rather than to a second form chosen per value, and an empty string literal MUST render as `E''`
* *AND* the rendered literal MUST carry the value the request gave it whatever the source's `standard_conforming_strings` and `backslash_quote` settings are, because the `E` prefix fixes the escape processing and the doubled quote avoids the setting that governs a backslash-escaped one
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: PostgreSQL renders LIKE, IS NULL, BETWEEN, and IN in their standard forms

* *GIVEN* a `pushdown` request carrying `predicate_like` with and without an `escapeChar`, `predicate_is_null`, `predicate_is_not_null`, `predicate_between`, and `predicate_in_constlist`
* *WHEN* the PostgreSQL dialect renders each
* *THEN* `predicate_like` MUST render its pattern as an escape-string constant, and MUST render an explicit `ESCAPE` clause exactly when the request carries an `escapeChar`
* *AND* `predicate_is_null` and `predicate_is_not_null` MUST render as `IS NULL` and `IS NOT NULL` rather than as a comparison against a null literal
* *AND* the `BETWEEN` bounds and the `IN` list entries MUST appear in the order the request gave them
* *AND* `predicate_notequal` MUST render as `<>`
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: Filter pushdown reaches PostgreSQL rather than Exasol

* *GIVEN* a virtual schema over a PostgreSQL table holding three rows
* *WHEN* Exasol runs a `SELECT` whose `WHERE` clause matches one row
* *THEN* `EXPLAIN VIRTUAL` MUST report a generated statement carrying that `WHERE` clause
* *AND* the query MUST return exactly the matching row
* *AND* the generated statement MUST NOT read the unmatched rows
<!-- /DELTA:NEW -->
