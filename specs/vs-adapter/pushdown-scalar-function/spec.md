# Feature: pushdown_scalar_function

Delegates the date-part, string, and arithmetic scalar functions Exasol applies to a column, so a query that filters, projects, or groups by such an expression reads only the rows and groups it needs from the source, and states the result each date-part and arithmetic function returns.

## Background

- This feature owns which scalar functions every dialect delegates, how a `function_scalar` node enters the query model, and the result each delegated function must return; each `<technology>-pushdown` feature owns its SQL for each function.
- The delegated scalar functions are `YEAR`, `MONTH`, `DAY`, `UPPER`, `LOWER`, `TRIM`, `LENGTH`, `SUBSTR`, `ADD`, `SUB`, and `MULT`, except `TRIM` at MySQL; `CAST` and every other scalar function stay withheld.
- Exasol sends a scalar function as a `function_scalar` node carrying its name and arguments, and an arithmetic operator as the infix `ADD`, `SUB`, or `MULT` node.
- `vs-adapter/pushdown-dialect` owns the advertised-capability invariant and the declarable `EMITS` type set, `vs-adapter/pushdown-aggregation-delegation` owns grouping by an expression, and `vs-adapter/pushdown-string-function` owns the result each string function returns and how an operand converts to text.

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
* *AND* each query MUST return exactly the values the mirror returns, a null for a null operand included: a number added to or subtracted from a `DATE` or `TIMESTAMP` counts days, a fractional number rounds half away from zero to whole days as Exasol rounds it, and `DATE` minus `DATE` is the number of days between them
* *AND* the dialect MUST NOT render a source operator that reads a date as a number, as MySQL's `+` does

### Scenario: A select-list TIMESTAMP difference is converted to an interval in Exasol

* *GIVEN* a virtual table for each driver this release supports, over two `TIMESTAMP` columns and a `DATE` column holding values a fraction of a second, several days, and more than a year apart, and a null, and a local Exasol mirror table holding the same rows
* *WHEN* Exasol runs `SELECT "t1" - "t2"`, `SELECT "t1" - "d"`, `SELECT MIN("t1" - "t2")`, and `SELECT "t1" + ("t1" - "t2")`, and a `WHERE` comparison between two such differences
* *THEN* the scan MUST emit each select-list entry declared `INTERVAL DAY(p) TO SECOND(f)` as the count of seconds the source computes for the whole entry, declared `DECIMAL(p + 5 + f, f)`, so every value of the declared interval fits with its fractional seconds, and the pushdown response MUST convert it in an outer Exasol `SELECT` through `NUMTODSINTERVAL`, cast to the declared type, because the scan cannot emit an `INTERVAL`
* *AND* every other select-list entry MUST pass through the outer `SELECT` unchanged, and a response with no such entry MUST carry no outer `SELECT`
* *AND* each query MUST return exactly the rows the mirror returns, a null for a null operand included, a `DATE` operand reading as midnight of its day
* *AND* `EXPLAIN VIRTUAL` MUST report a generated statement carrying the subtraction, so the query fails in no dialect where Exasol computed it before
