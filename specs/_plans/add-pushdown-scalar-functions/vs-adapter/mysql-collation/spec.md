# Feature: mysql_collation

States that the MySQL dialect renders no collation, what MySQL's default collation does to a delegated character comparison, grouping, and `MIN` or `MAX`, and the two ways a user obtains Exasol's code-point comparison.

## Background

- The MySQL dialect renders no collation suffix on any operand, per the framework rule `vs-adapter/pushdown-filter` states, so MySQL applies its own collation to every delegated character operand.
- MySQL's default column collation is `utf8mb4_0900_ai_ci`, and Exasol compares by code point.
- A user obtains Exasol's comparison by declaring `utf8mb4_0900_bin` at the source or by wrapping the operand in an expression the MySQL dialect does not advertise.
- `vs-adapter/pushdown-dialect` owns how a column reference renders, and `vs-adapter/mysql-type-mapping` owns each column's read expression.

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: A comparison over a wrapped operand is evaluated by Exasol

* *GIVEN* a virtual table over a MySQL `VARCHAR` column under the server default collation `utf8mb4_0900_ai_ci`, holding two values that differ only by case, and a local Exasol table holding the same values
* *WHEN* the same equality comparison against one value of the pair, with the column wrapped as `"<column>" || ''`, is run over each
* *THEN* `EXPLAIN VIRTUAL` MUST report a generated statement that carries no comparison over that column, because the MySQL dialect does not advertise `FN_CONCAT`
* *AND* the virtual query MUST return exactly the rows the local query returns, which hold one value of the pair rather than both
<!-- /DELTA:CHANGED -->
