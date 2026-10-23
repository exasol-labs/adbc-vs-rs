# Feature: mysql_collation

States that the MySQL dialect renders no collation, what MySQL's default collation does to a delegated character comparison, grouping, and `MIN` or `MAX`, and the two ways a user obtains Exasol's code-point comparison.

## Background

- The MySQL dialect renders no collation suffix on any operand, per the framework rule `vs-adapter/pushdown-filter` states. MySQL therefore evaluates every delegated character comparison, `LIKE`, `IN`, grouping key, `DISTINCT` operand, `MIN` or `MAX` operand, and equi-join key under the collation it applies to the column.
- MySQL's default column collation `utf8mb4_0900_ai_ci` is case-insensitive and accent-insensitive and orders by collation weight, while Exasol compares by code point. Measured against this release's own fixture: `name = 'widget'` matches the stored value `Widget`, and `'Gizmo' = 'Gîzmo'` is true. Over a `VARCHAR(50)` column holding `Widget`, `widget`, `Gizmo`, and `Gîzmo`, `GROUP BY` produced 2 groups and `COUNT(DISTINCT)` returned 2, where Exasol returns 4 for both. A delegated query over such a column returns MySQL's answers.
- A user who needs Exasol's comparison has two remedies. The first declares `utf8mb4_0900_bin` at the source, on the column or in a view the virtual schema exposes. It weighs the `utf8mb4` encoding bytes, so it reproduces code-point order, and MySQL reports it `NO PAD`. `utf8mb4_bin` is not a substitute: `information_schema.COLLATIONS` reports it `PAD SPACE`, and measured on the test MySQL, `MIN` over the values `'ab '`, `'ab'`, `'ab!'` returns `'ab '` under `utf8mb4_bin`, where Exasol and `utf8mb4_0900_bin` return `'ab'`.
- The second remedy wraps the operand in an expression the MySQL dialect does not advertise, such as `"category" || ''`. The dialect advertises no scalar-function capability, so Exasol keeps a comparison over the wrapped operand and evaluates it itself over the rows MySQL returns.
- A grouping key renders as the bare column, so its select-list occurrence and a select-list predicate reading it are expressions over that same column. Measured on the test MySQL, a select-list predicate over a bare numeric grouping key executes under `ONLY_FULL_GROUP_BY`, the mode `mysql:8.4` sets by default.

## Scenarios

### Scenario: No rendered operand carries a collation suffix

* *GIVEN* a MySQL `VARCHAR` column and a `pushdown` request carrying `predicate_equal`, `predicate_less`, `predicate_lessequal`, `predicate_between`, `predicate_in_constlist`, and `predicate_like` over it
* *WHEN* the MySQL dialect renders each predicate, in `WHERE` position and in select-list position
* *THEN* the generated statement MUST contain no `COLLATE` keyword
* *AND* the same MUST hold for a `GROUP BY` key and its select-list occurrence, for a `DISTINCT` aggregate operand, for a `MIN` and a `MAX` operand, and for a character equi-join key

### Scenario: A delegated comparison and grouping follow the column's own collation

* *GIVEN* a virtual table over a MySQL `VARCHAR` column under the server default collation `utf8mb4_0900_ai_ci`, holding two values that differ only by case and two that differ only by an accent
* *WHEN* an equality comparison against one value of each pair, a `GROUP BY` over the column, and a `COUNT(DISTINCT)` over it are run over the virtual table
* *THEN* `EXPLAIN VIRTUAL` MUST report the comparison and the grouping key in the generated statement with no `COLLATE` keyword
* *AND* each equality comparison MUST return both values of its pair, because MySQL's default collation matches them
* *AND* the `GROUP BY` MUST return one group per pair and the `COUNT(DISTINCT)` MUST count each pair once, which is MySQL's answer rather than the one Exasol computes over a local copy of the same values
* *AND* the value each group reports MUST be one of the values the group folds, and no step requires which one, because MySQL does not specify which member of a folded group it reports

### Scenario: A column declaring utf8mb4_0900_bin returns Exasol's answers

* *GIVEN* a virtual table over a MySQL table whose character columns declare `COLLATE utf8mb4_0900_bin`, and a local Exasol table holding the same values, including values that differ only by case, only by an accent, and only by a trailing space
* *WHEN* the same equality comparison, `GROUP BY`, `COUNT(DISTINCT)`, `MIN`, and `MAX` are run over each
* *THEN* each query over the virtual table MUST return exactly what the same query returns over the local table
* *AND* `EXPLAIN VIRTUAL` MUST report each fragment in the generated statement, so the rows come from MySQL rather than from Exasol evaluating the fragment itself

### Scenario: A select-list predicate over the grouped character column is delegated

* *GIVEN* a `pushdown` request whose `aggregationType` is `group_by`, whose `groupBy` names one MySQL `VARCHAR` column, and whose select list carries a predicate reading that same column outside any aggregate
* *WHEN* the MySQL dialect renders the statement
* *THEN* the dialect MUST produce a statement rather than an error
* *AND* the statement MUST execute at the source rather than being rejected with MySQL error 1055
* *AND* over a column declaring `COLLATE utf8mb4_0900_bin`, the rows it returns MUST equal what Exasol returns for the same query over a local copy of the same values
* *AND* this scenario MUST be verified against a live MySQL source rather than by rendering alone, because what is asserted is that the source accepts an expression over its bare grouping column

### Scenario: A comparison over a wrapped operand is evaluated by Exasol

* *GIVEN* a virtual table over a MySQL `VARCHAR` column under the server default collation `utf8mb4_0900_ai_ci`, holding two values that differ only by case, and a local Exasol table holding the same values
* *WHEN* the same equality comparison against one value of the pair, with the column wrapped as `"<column>" || ''`, is run over each
* *THEN* `EXPLAIN VIRTUAL` MUST report a generated statement that carries no comparison over that column, because the MySQL dialect advertises no scalar-function capability
* *AND* the virtual query MUST return exactly the rows the local query returns, which hold one value of the pair rather than both
