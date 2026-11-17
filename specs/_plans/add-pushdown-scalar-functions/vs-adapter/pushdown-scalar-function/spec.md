# Feature: pushdown_scalar_function

Delegates the date-part, string, and arithmetic scalar functions Exasol applies to a column, so a query that filters, projects, or groups by such an expression reads only the rows and groups it needs from the source.

## Background

- This feature owns which scalar functions every dialect delegates, how a `function_scalar` node enters the query model, and the result each delegated function must return; each `<technology>-pushdown` feature owns its SQL for each function.
- The delegated scalar functions are `YEAR`, `MONTH`, `DAY`, `UPPER`, `LOWER`, `TRIM`, `LENGTH`, `SUBSTR`, `ADD`, `SUB`, and `MULT`, except `TRIM` at MySQL; `CAST` and every other scalar function stay withheld.
- Exasol sends a scalar function as a `function_scalar` node carrying its name and arguments, and an arithmetic operator as the infix `ADD`, `SUB`, or `MULT` node.
- `vs-adapter/pushdown-dialect` owns the advertised-capability invariant and the declarable `EMITS` type set, and `vs-adapter/pushdown-aggregation-delegation` owns grouping by an expression.

## Scenarios

### Scenario: Each dialect advertises the scalar functions it renders

* *GIVEN* a virtual schema for each driver this release supports
* *WHEN* Exasol sends a `getCapabilities` request
* *THEN* the `capabilities` array MUST contain `FN_YEAR`, `FN_MONTH`, `FN_DAY`, `FN_UPPER`, `FN_LOWER`, `FN_LENGTH`, `FN_SUBSTR`, `FN_ADD`, `FN_SUB`, and `FN_MULT`, and the PostgreSQL and Trino arrays MUST also contain `FN_TRIM`
* *AND* the MySQL array MUST NOT contain `FN_TRIM`, as the MySQL JDBC Virtual Schema withholds it, because MySQL's `TRIM` removes a substring rather than a set of characters, so no MySQL form renders a two-argument `TRIM` whose character set is a column
* *AND* no array MUST contain `FN_CAST` or any other scalar-function capability, because no dialect renders another scalar function
* *AND* each dialect MUST state these entries in its own driver declaration, as `vs-adapter/pushdown-dialect` requires of every entry

### Scenario: A scalar function renders in every position Exasol delegates it to

* *GIVEN* `pushdown` requests carrying `YEAR` over a mapped `DATE` column in the select list, in a `WHERE` comparison, as a `GROUP BY` key, inside a `HAVING` predicate, and as the operand of `COUNT(DISTINCT)` and `SUM`
* *WHEN* each dialect renders each request
* *THEN* each position MUST render the function through the one renderer that renders it in the select list, over the column's read expression
* *AND* a scalar function nested inside another, such as `UPPER(SUBSTR("c", 2))` or `YEAR("d") + 1`, MUST render nested in the same order
* *AND* a `function_scalar` node whose name is outside the advertised set MUST fail with an error naming the function, as `vs-adapter/pushdown-dialect` requires of an unrenderable fragment

### Scenario: Date parts match Exasol over DATE and TIMESTAMP columns

* *GIVEN* a virtual table for each driver this release supports, over a `DATE` column and a `TIMESTAMP` column holding values on the first and last day of a year, on 29 February, and null, and a local Exasol mirror table holding the same rows
* *WHEN* Exasol runs `YEAR`, `MONTH`, and `DAY` over each column in the select list and in a `WHERE` comparison against a number
* *THEN* `EXPLAIN VIRTUAL` MUST report a generated statement carrying each function
* *AND* the query MUST return exactly the rows the same query returns over the mirror, a null for a null operand included
* *AND* each select-list entry MUST be declared with the `selectListDataTypes` type the request carries, which is a `DECIMAL` for each date part

### Scenario: Exact-numeric arithmetic is exact and does not overflow before Exasol does

* *GIVEN* a virtual table for each driver this release supports, over a column the source stores as a 64-bit integer holding its maximum value, a `DECIMAL(18,4)` column, and a null, and a local Exasol mirror table holding the same rows
* *WHEN* Exasol runs `"i" + 1`, `"i" - -1`, `"i" * 2`, `"d" * "d"`, and `"d" - "d"` in the select list and in a `WHERE` comparison
* *THEN* `EXPLAIN VIRTUAL` MUST report a generated statement carrying each operation
* *AND* each query MUST return exactly the values the mirror returns, so a result beyond the source's integer range MUST be returned rather than fail with a source overflow error
* *AND* a result whose digits exceed the type Exasol declared for it MUST fail as `vs-adapter/pushdown-aggregation-types` requires for an overflowing value, never truncate
* *AND* an operation over a null operand MUST return null

### Scenario: Floating-point and character arithmetic converts as Exasol converts

* *GIVEN* a virtual table for each driver this release supports, over a `DOUBLE` column, a `DECIMAL` column, and a `VARCHAR` column holding decimal numbers as text, and a local Exasol mirror table holding the same rows
* *WHEN* Exasol runs `+`, `-`, and `*` between the `DOUBLE` and the `DECIMAL` column, and between the `VARCHAR` column and a number
* *THEN* `EXPLAIN VIRTUAL` MUST report a generated statement carrying each delegated operation
* *AND* the `DOUBLE` and mixed operations MUST return the values the mirror returns, computed in double precision
* *AND* the `VARCHAR` operand MUST be converted as the `SUM` operand conversion of `vs-adapter/pushdown-aggregation-delegation` converts it, so the query returns the mirror's values

### Scenario: Date and timestamp arithmetic follows Exasol's day arithmetic

* *GIVEN* a virtual table for each driver this release supports, over a `DATE` column, a `TIMESTAMP` column, and a `DECIMAL` column, and a local Exasol mirror table holding the same rows, including a month end and a leap day
* *AND* a null in each column
* *WHEN* Exasol runs `"dt" + 1`, `1 + "dt"`, `"dt" + 1.5`, `"dt" - 1`, `"dt" - "dt"`, `"ts" + 1.5`, and `"ts" - "n"` with a fractional `"n"`, in the select list and in a `WHERE` comparison
* *THEN* `EXPLAIN VIRTUAL` MUST report a generated statement carrying each operation
* *AND* each query MUST return exactly the values the mirror returns, a null for a null operand included: a number added to or subtracted from a `DATE` or `TIMESTAMP` counts days, a fractional number keeps the fraction Exasol keeps, and `DATE` minus `DATE` is the number of days between them
* *AND* the dialect MUST NOT render a source operator that reads a date as a number, as MySQL's `+` does

### Scenario: A select-list TIMESTAMP difference fails with a named error

* *GIVEN* a `pushdown` request whose select list carries `SUB` over two `TIMESTAMP` operands and whose `selectListDataTypes` declares that entry `INTERVAL DAY TO SECOND`
* *WHEN* the adapter builds the response
* *THEN* the adapter MUST return an error naming `INTERVAL DAY TO SECOND` as a type the `EMITS` clause cannot declare, and naming `SECONDS_BETWEEN` or `DAYS_BETWEEN` as the remedy that Exasol computes
* *AND* a `WHERE` comparison between two such differences, a difference between a `DATE` and a `TIMESTAMP`, and a `TIMESTAMP` plus such a difference MUST render rather than fail, because no `EMITS` column declares an interval there, and MUST return the rows a local Exasol mirror returns, a `DATE` operand reading as midnight of its day
* *AND* the same MUST hold for each dialect this release supports

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
