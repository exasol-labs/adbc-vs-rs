# Feature: mysql_collation

States that the MySQL dialect renders no collation, what MySQL's default collation does to a delegated character comparison, grouping, and `MIN` or `MAX`, and the two ways a user obtains Exasol's code-point comparison.

<!-- DELTA:CHANGED -->
## Background

- The MySQL dialect renders no collation suffix on any operand, per the framework rule `vs-adapter/pushdown-filter` states, so MySQL applies its own collation to every delegated character operand.
- MySQL's default column collation is `utf8mb4_0900_ai_ci`, and Exasol compares by code point.
- A user obtains Exasol's comparison by declaring `utf8mb4_0900_bin` at the source or by wrapping the operand in an expression the MySQL dialect does not advertise.
- `vs-adapter/pushdown-dialect` owns how a column reference renders, and `vs-adapter/mysql-type-mapping` owns each column's read expression.
<!-- /DELTA:CHANGED -->

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: No rendered operand carries a collation suffix

* *GIVEN* a MySQL `VARCHAR` column and a `pushdown` request carrying `predicate_equal`, `predicate_less`, `predicate_lessequal`, `predicate_between`, `predicate_in_constlist`, and `predicate_like` over it
* *WHEN* the MySQL dialect renders each predicate, in `WHERE` position and in select-list position
* *THEN* the generated statement MUST contain no `COLLATE` keyword
* *AND* the same MUST hold for a `GROUP BY` key and its select-list occurrence, for a `DISTINCT` aggregate operand, for a `MIN` and a `MAX` operand, and for a character equi-join key
* *AND* the same MUST hold for a `latin1` column and for a column the dialect reads as `CAST(<column> AS CHAR)`, such as a `JSON` column, so MySQL compares the first under its own `latin1` collation, `latin1_swedish_ci` by default, and the second under the session's connection collation
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: A delegated comparison and grouping follow the column's own collation

* *GIVEN* a virtual table over a MySQL `VARCHAR(50)` column under the server default collation `utf8mb4_0900_ai_ci`, holding `Widget`, `widget`, `Gizmo`, and `Gîzmo`: two values that differ only by case and two that differ only by an accent
* *WHEN* an equality comparison against one value of each pair, a `GROUP BY` over the column, and a `COUNT(DISTINCT)` over it are run over the virtual table
* *THEN* `EXPLAIN VIRTUAL` MUST report the comparison and the grouping key in the generated statement with no `COLLATE` keyword
* *AND* each equality comparison MUST return both values of its pair, so an equality with `'widget'` matches the stored value `Widget` and `'Gizmo' = 'Gîzmo'` is true, because `utf8mb4_0900_ai_ci` is case-insensitive and accent-insensitive
* *AND* the `GROUP BY` MUST return 2 groups and the `COUNT(DISTINCT)` MUST return 2, which is MySQL's answer rather than the 4 Exasol returns for both over a local copy of the same values, because `utf8mb4_0900_ai_ci` orders by collation weight while Exasol compares by code point
* *AND* the value each group reports MUST be one of the values the group folds, and no step requires which one, because MySQL does not specify which member of a folded group it reports
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: A column declaring utf8mb4_0900_bin returns Exasol's answers

* *GIVEN* a virtual table over a MySQL table, or over a view the virtual schema exposes, whose character columns declare `COLLATE utf8mb4_0900_bin`, and a local Exasol table holding the same values, including values that differ only by case, only by an accent, and only by a trailing space
* *WHEN* the same equality comparison, `GROUP BY`, `COUNT(DISTINCT)`, `MIN`, and `MAX` are run over each
* *THEN* each query over the virtual table MUST return exactly what the same query returns over the local table, because `utf8mb4_0900_bin` weighs the `utf8mb4` encoding bytes and so reproduces code-point order
* *AND* `EXPLAIN VIRTUAL` MUST report each fragment in the generated statement, so the rows come from MySQL rather than from Exasol evaluating the fragment itself
* *AND* `MIN` over the values `'ab '`, `'ab'`, and `'ab!'` MUST return `'ab'`, as Exasol does, because MySQL reports `utf8mb4_0900_bin` `NO PAD`; `utf8mb4_bin` is no substitute, because `information_schema.COLLATIONS` reports it `PAD SPACE` and, measured on the test MySQL, the same `MIN` returns `'ab '` under it
* *AND* a `latin1` column that a view exposes as `CONVERT(<column> USING utf8mb4) COLLATE utf8mb4_0900_bin` MUST return what the same query returns over the local table, because `latin1_bin` is `PAD SPACE` too and this conversion is the remedy for a `latin1` column
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: A select-list predicate over the grouped character column is delegated

* *GIVEN* a `pushdown` request whose `aggregationType` is `group_by`, whose `groupBy` names one MySQL `VARCHAR` column, and whose select list carries a predicate reading that same column outside any aggregate
* *WHEN* the MySQL dialect renders the statement
* *THEN* the dialect MUST produce a statement rather than an error
* *AND* the statement MUST execute at the source under `ONLY_FULL_GROUP_BY`, which the test MySQL sets by default, rather than being rejected with MySQL error 1055, because the bare grouping key's select-list occurrence and the predicate reading it are expressions over that same column, as the test MySQL also showed for a bare numeric grouping key
* *AND* over a column declaring `COLLATE utf8mb4_0900_bin`, the rows it returns MUST equal what Exasol returns for the same query over a local copy of the same values
* *AND* this scenario MUST be verified against a live MySQL source rather than by rendering alone, because what is asserted is that the source accepts an expression over its bare grouping column
<!-- /DELTA:CHANGED -->
