# Feature: pushdown_filter

Delegates a query's `WHERE` clause to the source, rendering each predicate in the source's own SQL so the source selects the rows, with every character comparison evaluated under the collation the source applies to the column.

<!-- DELTA:CHANGED -->
## Background

- This feature owns what a filter fragment is, for every dialect: predicate nesting and grouping, operand order, a `BOOLEAN` column as a predicate on its own, and a select-list predicate rendered by the `WHERE` renderer.
- Each dialect's literal forms and advertised filter capabilities live in its `<technology>-pushdown` feature, and each `<technology>-collation` feature states what its source's collation does to a delegated character comparison.
- A character operand is Exasol `VARCHAR` or Exasol `CHAR`, and Exasol keeps a `CHAR` value blank-padded.
- `vs-adapter/pushdown-dialect` owns how a column operand renders and how comparison operands are read by position.
<!-- /DELTA:CHANGED -->

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Numeric comparison is rendered as a WHERE clause

* *GIVEN* a virtual table `orders` with a mapped `DECIMAL` column `id`
* *WHEN* Exasol pushes down a filter comparing `id` to the literal 2, for each dialect this release supports
* *THEN* the generated statement MUST carry a `WHERE` clause comparing the quoted column to the literal
* *AND* the operands MUST appear in the order the request gave them
* *AND* the numeric literal MUST be rendered without quotes and with no collation suffix, because a numeric comparison needs none
* *AND* a `predicate_notequal` MUST render as `<>` in every dialect, because every target accepts that standard form and a dialect gains nothing from spelling a shared form differently
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Conjunction, disjunction, and negation nest without losing grouping

* *GIVEN* a filter combining two comparisons under `predicate_or` and negating the result
* *WHEN* the dialect renders that filter
* *THEN* the rendered MySQL clause MUST parenthesize each nested predicate
* *AND* the PostgreSQL and Trino dialects MUST parenthesize each nested predicate the same way, because every target accepts the same standard form
* *AND* the rows MySQL returns MUST equal the rows Exasol returns for the same filter over the same data, where every compared character column declares a code-point collation at the source, as `vs-adapter/mysql-collation` states
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: LIKE renders its pattern and its escape character

* *GIVEN* a `predicate_like` over a character column with the pattern `Wid%`
* *WHEN* each dialect this release supports renders that predicate
* *THEN* a request carrying an `escapeChar` MUST render an explicit `ESCAPE` clause over that character in every dialect that advertises `FN_PRED_LIKE_ESCAPE`
* *AND* the pattern MUST be rendered through the same string-literal form the dialect renders every other literal in, which each `<technology>-pushdown` feature states for its own dialect
* *AND* the compared operand MUST carry no collation suffix in any dialect
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: A LIKE pattern without an ESCAPE clause follows the target's default escape rule

* *GIVEN* a `predicate_like` carrying no `escapeChar`, whose pattern escapes `%` with a backslash, as Exasol spells a pattern when the query gives no `ESCAPE` clause
* *WHEN* each dialect this release supports renders the predicate and the source evaluates it
* *THEN* the dialect MUST render no `ESCAPE` clause, so the target's own escape rule reads the pattern
* *AND* the backslash MUST escape the `%` when the pattern is delegated to MySQL or PostgreSQL, because both apply Exasol's rule that a backslash escapes the character after it
* *AND* the backslash MUST be an ordinary character when the pattern is delegated to Trino, which defines no default escape character, as `vs-adapter/trino-pushdown` states
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: A delegated predicate follows the source's comparison rules

* *GIVEN* a delegated filter over a character column, for each dialect this release supports
* *WHEN* the dialect renders the filter and the source evaluates it
* *THEN* the rows the query returns MUST be the rows the source's evaluation selects, because Exasol never re-applies a delegated filter
* *AND* the dialect MUST NOT add syntax whose purpose is to reproduce an Exasol comparison rule the target does not share
* *AND* a character comparison, `LIKE`, `BETWEEN`, `IN`, and select-list predicate MUST follow the collation the source applies to the column, which can equate values Exasol keeps apart and order values differently, because Exasol compares character values by code point and no dialect renders a collation, as each `<technology>-collation` feature states together with how a user obtains Exasol's comparison
* *AND* a column operand MUST render as its read expression, per `vs-adapter/pushdown-dialect`, so the predicate acts on the value the scan emits
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: A CHAR column compares as the source compares fixed-width values

* *GIVEN* a column mapped to Exasol `CHAR(5)` holding `ab`, which on the test Exasol reports `LENGTH` 5, equals `'ab'`, `'ab '`, and `'ab   '`, is unequal to a `VARCHAR` holding `ab`, and does not match `LIKE 'ab'`, while PostgreSQL and MySQL ignore the trailing blanks of a fixed-width value, as each `<technology>-type-mapping` feature states
* *WHEN* a filter over the column is delegated to the source
* *THEN* a comparison with a string literal MUST select the rows Exasol's own evaluation selects
* *AND* a comparison with a `VARCHAR` operand MUST select the rows the source selects, which can include rows Exasol's own evaluation would not
* *AND* a `LIKE` without a trailing wildcard MUST select the rows the source selects, which can include rows Exasol's own evaluation would not
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: String, date, and timestamp literals render in a form no server setting can change

* *GIVEN* a filter comparing a character column with a string literal, a date column with a date literal, and a timestamp column with a timestamp literal
* *WHEN* each dialect this release supports renders the filter
* *THEN* the string literal MUST render in the dialect's own form that no server setting can change, which its `<technology>-pushdown` feature states, for every string literal rather than only for one that needs it
* *AND* the dialect MUST NOT render a bare quoted string whose meaning a server mode can shift
* *AND* the date and timestamp literals MUST render as typed literals, `DATE '...'` and `TIMESTAMP '...'`, so no source coerces a string by its own rules
* *AND* no literal value MUST change the shape of the generated statement, in any dialect
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: REGEXP_LIKE is evaluated by Exasol rather than delegated

* *GIVEN* the capability set each dialect this release supports advertises
* *WHEN* the sets are read
* *THEN* every set MUST NOT carry `FN_PRED_REGEXP_LIKE`, so Exasol evaluates a `REGEXP_LIKE` itself over the rows the source returns
* *AND* the withholding MUST hold for every dialect, because Exasol's regular-expression dialect is PCRE, MySQL 8's is ICU, PostgreSQL's is POSIX, and Trino's is `RE2J`, so one pattern can match different rows against any of them
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: BOOLEAN column is a predicate on its own

* *GIVEN* a `pushdown` request whose `filter` is a `column` entry whose declared type is `BOOLEAN`, and requests carrying that entry as an operand of `predicate_and`, `predicate_or`, and `predicate_not`
* *WHEN* each dialect parses and renders each request
* *THEN* the rendered `WHERE` clause MUST carry the column's read expression as the predicate, with no comparison added: the bare quoted column for a PostgreSQL `boolean` column, `(position(B'1' IN <column>) > 0)` for a PostgreSQL `bit` column, and `(<column> <> 0)` for a MySQL `BIT` column
* *AND* for each of those three columns, the rows the source returns MUST equal the rows Exasol returns for the same filter over the same data
* *AND* a `column` entry of any other declared type in predicate position MUST fail with an error naming the entry type and the declared type, rather than render a non-boolean operand as a predicate
* *AND* a comparison whose operand is itself a predicate, such as a `predicate_equal` between a `BOOLEAN` column and a `predicate_less`, MUST render that operand parenthesized through the same renderer rather than fail
<!-- /DELTA:NEW -->
