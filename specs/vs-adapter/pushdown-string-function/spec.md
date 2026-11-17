# Feature: pushdown_string_function

States the result each delegated string function returns, `UPPER`, `LOWER`, `TRIM`, `LENGTH`, and `SUBSTR`, and how an operand Exasol converts implicitly reaches a delegated function.

## Background

- `vs-adapter/pushdown-scalar-function` owns which scalar functions every dialect delegates and how a `function_scalar` node enters the query model, and each `<technology>-pushdown` feature owns its SQL for each function.
- MySQL does not delegate `TRIM`, so a `TRIM` step applies only under the PostgreSQL and Trino dialects.
- Exasol reads an empty string as null, and each source keeps the empty string apart from null.

## Scenarios

### Scenario: String functions match Exasol over VARCHAR text

* *GIVEN* a virtual table for each driver this release supports, over a `VARCHAR` column holding ASCII text with leading and trailing blanks, a multi-byte character, and a null, and a local Exasol mirror table holding the same rows
* *WHEN* Exasol runs `UPPER`, `LOWER`, `TRIM` with one and with two arguments where the dialect advertises it, `LENGTH`, and `SUBSTR` with two and with three arguments over the column, in the select list and in a `WHERE` comparison
* *THEN* `EXPLAIN VIRTUAL` MUST report a generated statement carrying each function
* *AND* each query MUST return exactly the rows the mirror returns, a null for a null operand included
* *AND* `LENGTH` MUST count characters rather than bytes, so the multi-byte character counts once
* *AND* two-argument `TRIM` MUST remove every leading and trailing character contained in its second argument, as Exasol does

### Scenario: SUBSTR positions follow Exasol's rule

* *GIVEN* a virtual table for each driver this release supports, over a `VARCHAR` column holding `abcdef` and a null, and a local Exasol mirror table holding the same rows
* *WHEN* Exasol runs `SUBSTR` with position 0, 1, a negative position, a position beyond the end, and a length running past the end
* *THEN* `EXPLAIN VIRTUAL` MUST report a generated statement carrying each `SUBSTR`
* *AND* each query MUST return the value the mirror returns, because each source counts a position of 0 or below differently from Exasol and the dialect renders Exasol's rule

### Scenario: An empty string function result is null, as in Exasol

* *GIVEN* a virtual table for each driver this release supports, over a `VARCHAR` column holding blanks only, `abc`, and a null, and a local Exasol mirror table holding the same rows
* *WHEN* Exasol runs `TRIM` over the column as a `GROUP BY` key, as a `COUNT(DISTINCT)` operand, and in an `IS NULL` filter, and `SUBSTR("c", 10) IS NULL`, under each dialect that advertises the function
* *THEN* each dialect MUST render an empty result of `TRIM`, `SUBSTR`, `UPPER`, and `LOWER` as null at the source, because the source keeps the empty string apart from null while Exasol reads it as null
* *AND* each query MUST return exactly the rows the mirror returns, so the blank row and the null row form one group and match `IS NULL`

### Scenario: String function results follow the source for padding and case mapping

* *GIVEN* a virtual table for each driver this release supports, over a `CHAR(5)` column holding `ab` and a `VARCHAR` column holding a non-ASCII letter with a locale-dependent upper case
* *WHEN* Exasol runs `LENGTH` over the `CHAR` column and `UPPER` over the non-ASCII column
* *THEN* each result MUST be the source's own result rather than Exasol's, because the dialect renders the source function without padding the `CHAR` operand and without fixing a locale
* *AND* `docs/pushdown.md` MUST state both differences per database

### Scenario: An implicitly converted operand converts as Exasol converts it

* *GIVEN* a virtual table for each driver this release supports, over a `DECIMAL(18,2)` column, a `DATE` column, a `VARCHAR` column holding a date in Exasol's default format, and a `BOOLEAN` column where the source maps one, and a local Exasol mirror table holding the same rows
* *WHEN* Exasol runs `LENGTH` and `SUBSTR` over the `DECIMAL` and the `DATE` column, `YEAR` over the `VARCHAR` column, and `+ 1` over the `BOOLEAN` column, each where Exasol accepts the query
* *THEN* each query Exasol delegates MUST return exactly the rows the mirror returns, because the dialect renders the conversion Exasol applies under its default `NLS_DATE_FORMAT` and `NLS_TIMESTAMP_FORMAT`
* *AND* `docs/pushdown.md` MUST state that a session that changes either format is not followed
