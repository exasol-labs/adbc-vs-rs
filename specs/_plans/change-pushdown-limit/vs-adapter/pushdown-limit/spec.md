# Feature: pushdown_limit

Delegates a `LIMIT` that no `ORDER BY` precedes to the source, so a query such as `SELECT ... LIMIT 100` over a virtual table reads at most 100 rows at the source instead of the whole table. Every `OFFSET` stays in Exasol.

## Background

- This feature owns the `limit` element for every dialect: the advertised `LIMIT` capability, the withheld `LIMIT_WITH_OFFSET` capability, how the adapter reads a `limit` element, and the `LIMIT n` clause every dialect renders as the last clause of its statement, a form MySQL, PostgreSQL, and Trino all accept. `vs-adapter/result-ordering` keeps every `ORDER BY`, and every `LIMIT` or `OFFSET` that follows one, in Exasol.
- A `limit` element carries a `numElements` count, and an `offset` only where `LIMIT_WITH_OFFSET` is advertised. It applies after the `filter`, the aggregation, and the `having` predicate of the same request.
- The adapter cannot see from a request whether Exasol applies an ordering, a filter, a join, an aggregation, or an analytic function of its own above the scan. A delegated limit is therefore correct only where Exasol sends `limit` with nothing it applies itself between the scan and the limit, and the scenarios below state that requirement per query shape.
- A `LIMIT` with no `ORDER BY` selects an arbitrary set of rows. SQL promises no particular set, so the rows the source returns MAY differ from the rows Exasol would return, and MAY differ between runs.
- Exasol's SQL reference accepts an `OFFSET` only together with `ORDER BY`, and every `ORDER BY` stays in Exasol, so Exasol applies every offset itself. No dialect advertises `LIMIT_WITH_OFFSET`, because no query would reach an offset rendering.

## Scenarios

### Scenario: Every dialect advertises LIMIT and withholds LIMIT_WITH_OFFSET

* *GIVEN* a virtual schema, for each driver this release supports
* *WHEN* Exasol sends a `getCapabilities` request
* *THEN* the `capabilities` array MUST contain `LIMIT`
* *AND* it MUST NOT contain `LIMIT_WITH_OFFSET`
* *AND* each dialect MUST state `LIMIT` in its own driver declaration, per `vs-adapter/pushdown-dialect`

### Scenario: A limit renders as the last clause of the statement

* *GIVEN* a `pushdown` request carrying a `filter`, a `group_by` aggregation, a `having` predicate, and a `limit` with `numElements` 5, for each driver this release supports
* *WHEN* the dialect renders the request
* *THEN* the statement MUST end with `LIMIT 5`, after the `WHERE`, `GROUP BY`, and `HAVING` clauses
* *AND* the count MUST render as the decimal integer the request carries
* *AND* the same request without the `limit` element MUST render with no `LIMIT` clause

### Scenario: A limit element the adapter cannot read fails rather than being dropped

* *GIVEN* a `pushdown` request whose `limit` element has no `numElements`, or whose `numElements` is not a non-negative integer
* *WHEN* the adapter parses the request
* *THEN* the adapter MUST return an error naming the `limit` element
* *AND* the adapter MUST NOT return a statement without the limit, because that statement would read every row Exasol believes the source has already cut

### Scenario: A limit carrying an offset fails rather than dropping the offset

* *GIVEN* a `pushdown` request whose `limit` element carries an `offset`, which Exasol sends only when `LIMIT_WITH_OFFSET` was advertised
* *WHEN* the adapter parses the request
* *THEN* the adapter MUST return an error naming the `limit` element and the withheld `LIMIT_WITH_OFFSET` capability
* *AND* the adapter MUST NOT return a statement that carries the count without the offset, because that statement would return the rows the offset skips

### Scenario: A LIMIT over a delegated filter reaches the source with the filter

* *GIVEN* a virtual table and a `SELECT` with no `ORDER BY`, whose `WHERE` clause the dialect delegates and whose `LIMIT` is smaller than the number of matching rows
* *WHEN* Exasol runs the query
* *THEN* `EXPLAIN VIRTUAL` MUST report one generated statement carrying the `WHERE` clause and the limit
* *AND* the query MUST return exactly as many rows as the `LIMIT` names
* *AND* every returned row MUST satisfy the filter and MUST be a row of the source table, as the local Exasol mirror holds it

### Scenario: A LIMIT over a delegated aggregation applies to the groups

* *GIVEN* a virtual table and a `SELECT` with no `ORDER BY` that groups by one column and carries a `HAVING` clause, both delegated, with a `LIMIT` smaller than the number of groups the `HAVING` clause keeps
* *WHEN* Exasol runs the query
* *THEN* `EXPLAIN VIRTUAL` MUST report one generated statement carrying the `GROUP BY` clause, the `HAVING` clause, and the limit after both
* *AND* the query MUST return exactly as many rows as the `LIMIT` names
* *AND* every returned row MUST equal a row of the full grouped result that Exasol computes over the local mirror

### Scenario: A LIMIT over a delegated join applies to the joined rows

* *GIVEN* two virtual tables of one virtual schema joined on an equality, in a `SELECT` with no `ORDER BY` and a `LIMIT` smaller than the number of joined rows
* *WHEN* Exasol runs the query
* *THEN* `EXPLAIN VIRTUAL` MUST report one generated statement carrying the join and the limit
* *AND* the query MUST return exactly as many rows as the `LIMIT` names
* *AND* every returned row MUST equal a row of the full join result that Exasol computes over the local mirror

### Scenario: A LIMIT above a fragment Exasol evaluates itself stays in Exasol

* *GIVEN* four `SELECT` statements, each with a `LIMIT` and no `ORDER BY` over a virtual table: one whose filter compares an arithmetic expression that no dialect advertises, such as `"price" + 0 > 10`; one that joins the virtual table with a local Exasol table; one that groups by an arithmetic expression that no dialect advertises, such as `GROUP BY "price" + 0`, and selects an aggregate; and one whose select list carries an analytic function, such as `COUNT(*) OVER ()`
* *WHEN* Exasol runs each query
* *THEN* no generated statement that `EXPLAIN VIRTUAL` reports MUST carry a limit, because a limit applied at the source before Exasol's own filter, join, grouping, or analytic function would drop rows that the query asked for, or compute the query's values over the limited rows only
* *AND* each query MUST return as many rows as the `LIMIT` names, or every row of Exasol's full result when that result is smaller
* *AND* every returned row MUST be a row of the full result that Exasol computes over the local mirror
* *AND* every analytic value MUST equal the value Exasol computes over the full local mirror

### Scenario: An aggregate over a limited derived table aggregates only the limited rows

* *GIVEN* a virtual table holding more than 3 rows
* *WHEN* Exasol runs `SELECT COUNT(*)` over a derived table that selects one column of the virtual table with `LIMIT 3`
* *THEN* the query MUST return 3
* *AND* no generated statement MUST carry both the aggregate and the limit, because a statement applies its limit after its aggregation and would count the whole table

### Scenario: A LIMIT over DISTINCT returns that many distinct values

* *GIVEN* a virtual table whose column holds more distinct values than the `LIMIT` names
* *WHEN* Exasol runs a `SELECT DISTINCT` of that column with that `LIMIT` and no `ORDER BY`
* *THEN* the query MUST return exactly as many rows as the `LIMIT` names, each with a different value
* *AND* every returned value MUST be a value of the column in the local mirror
* *AND* a generated statement that carries the limit MUST also carry the grouping that makes the values distinct, because a limit applied before the grouping can return fewer distinct values than the `LIMIT` names
