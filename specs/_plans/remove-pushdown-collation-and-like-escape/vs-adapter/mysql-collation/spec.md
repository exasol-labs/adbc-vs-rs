<!-- DELTA:CHANGED -->
# Feature: mysql_collation

States that the MySQL dialect renders no collation, what MySQL's default collation does to a delegated character comparison, grouping, and `MIN` or `MAX`, and the two ways a user obtains Exasol's code-point comparison.
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
## Background

- The MySQL dialect renders no collation suffix on any operand, per the framework rule `vs-adapter/pushdown-filter` states. MySQL therefore evaluates every delegated character comparison, `LIKE`, `IN`, grouping key, `DISTINCT` operand, `MIN` or `MAX` operand, and equi-join key under the collation it applies to the column.
- MySQL 8.4's default column collation `utf8mb4_0900_ai_ci` is case-insensitive and accent-insensitive and orders by collation weight, while Exasol compares by code point. Measured against this release's own fixture: `name = 'widget'` matches the stored value `Widget`, and `'Gizmo' = 'Gîzmo'` is true. Over a `VARCHAR(50)` column holding `Widget`, `widget`, `Gizmo`, and `Gîzmo`, `GROUP BY` produced 2 groups and `COUNT(DISTINCT)` returned 2, where Exasol 2025.2.1 returns 4 for both. A delegated query over such a column returns MySQL's answers.
- A user who needs Exasol's comparison has two remedies. The first declares `utf8mb4_0900_bin` at the source, on the column or in a view the virtual schema exposes. It weighs the `utf8mb4` encoding bytes, so it reproduces code-point order, and MySQL reports it `NO PAD`. `utf8mb4_bin` is not a substitute: `information_schema.COLLATIONS` reports it `PAD SPACE`, and measured on MySQL 8.4, `MIN` over the values `'ab '`, `'ab'`, `'ab!'` returns `'ab '` under `utf8mb4_bin`, where Exasol 2025.2.1 and `utf8mb4_0900_bin` return `'ab'`.
- The second remedy wraps the operand in an expression the MySQL dialect does not advertise, such as `"category" || ''`. The dialect advertises no scalar-function capability, so Exasol keeps a comparison over the wrapped operand and evaluates it itself over the rows MySQL returns.
- A grouping key renders as the bare column, so its select-list occurrence and a select-list predicate reading it are expressions over that same column. Measured on MySQL 8.4, a select-list predicate over a bare numeric grouping key executes under `ONLY_FULL_GROUP_BY`, the mode `mysql:8.4` sets by default.
<!-- /DELTA:CHANGED -->

## Scenarios

<!-- DELTA:REMOVED -->
### Scenario: Character comparison reproduces Exasol's case and accent sensitivity

* *GIVEN* a MySQL `VARCHAR` column whose collation is `utf8mb4_0900_ai_ci` holding the value `Widget`
* *WHEN* Exasol pushes down a filter comparing that column to the literal `widget`
* *THEN* the rendered operand MUST carry the suffix `COLLATE utf8mb4_0900_as_cs`
* *AND* the query MUST return no row, matching what Exasol returns for the same comparison
* *AND* the same MUST hold for a value differing only in an accent
<!-- /DELTA:REMOVED -->

<!-- DELTA:REMOVED -->
### Scenario: A LIKE operand, an IN comparison, and a select-list predicate carry the comparison collation

* *GIVEN* a `predicate_like` over a character column, a `predicate_in_constlist` over a character column, and a `pushdown` request carrying no `group_by` aggregation whose select list carries a character comparison declared `BOOLEAN`
* *WHEN* the MySQL dialect renders each
* *THEN* the `LIKE` operand MUST carry `COLLATE utf8mb4_0900_as_cs`, because MySQL matches a pattern under the operand's collation and its default matches values Exasol keeps apart
* *AND* the `IN` comparison MUST carry that suffix on the compared operand rather than on the list
* *AND* the select-list comparison MUST carry the same suffix its `WHERE` counterpart carries, so the one renderer `vs-adapter/pushdown-filter` requires for both positions gives one answer to the equality question
* *AND* a numeric or temporal operand MUST carry no suffix in any of the three positions
<!-- /DELTA:REMOVED -->

<!-- DELTA:REMOVED -->
### Scenario: Grouped character columns carry the comparison collation in both the grouping list and the select list

* *GIVEN* a `pushdown` request whose `aggregationType` is `group_by`, whose `groupBy` names one character column and one numeric column, and whose select list projects both grouping columns beside a `MIN` over the same character column
* *WHEN* the MySQL dialect renders the statement
* *THEN* the character grouping column MUST carry `COLLATE utf8mb4_0900_as_cs` in the `GROUP BY` clause
* *AND* the same suffix MUST appear on that column's bare select-list occurrence, because MySQL rejects the statement with error 1055 under `ONLY_FULL_GROUP_BY` when the two occurrences carry different collations
* *AND* the `MIN` over the character column MUST keep `COLLATE utf8mb4_0900_bin`, because an aggregate is not subject to `ONLY_FULL_GROUP_BY` and the value `MIN` returns is chosen by order rather than by equality
<!-- /DELTA:REMOVED -->

<!-- DELTA:REMOVED -->
### Scenario: Only a grouped character column carries the grouping collation

* *GIVEN* a `pushdown` request whose `groupBy` names a numeric column, and a second request carrying no `aggregationType` whose select list projects a character column
* *WHEN* the MySQL dialect renders each statement
* *THEN* the numeric grouping column MUST render with no collation suffix in the `GROUP BY` clause and none in the select list
* *AND* the character column of the second request MUST render with no collation suffix, so a projection-only statement is byte-identical to what this release renders before the grouping rule exists
* *AND* the collation MUST therefore read off both the operand's declared type and whether the query groups by that column, never off the type alone
<!-- /DELTA:REMOVED -->

<!-- DELTA:REMOVED -->
### Scenario: A DISTINCT aggregate over a character column carries the comparison collation

* *GIVEN* a `function_aggregate` whose `distinct` field is true over a character column, the shape `COUNT(DISTINCT "name")` arrives in
* *WHEN* the MySQL dialect renders it
* *THEN* the operand MUST carry `COLLATE utf8mb4_0900_as_cs`, because `DISTINCT` reduces values by equality and MySQL's default collation equates values Exasol keeps apart
* *AND* the same aggregate carrying no `distinct` field MUST render its operand with no collation suffix, because `COUNT` over a character column reads neither equality nor order
* *AND* a character operand of `MIN` or `MAX` MUST carry `COLLATE utf8mb4_0900_bin` whatever its `distinct` field holds, because the value those two return is chosen by order
<!-- /DELTA:REMOVED -->

<!-- DELTA:REMOVED -->
### Scenario: Delegated grouping and DISTINCT over case-only variants return Exasol's groups

* *GIVEN* a virtual table and a local Exasol table holding identical character values, two of which differ only by case and two only by an accent
* *WHEN* the same `GROUP BY` over that column, and the same `COUNT(DISTINCT)` over it, are run over each
* *THEN* `EXPLAIN VIRTUAL` MUST report a generated statement whose `GROUP BY` key and whose projected grouping column both carry `COLLATE utf8mb4_0900_as_cs`
* *AND* the virtual `GROUP BY` MUST return exactly the groups the local query returns, so no two values Exasol keeps apart are collapsed into one group
* *AND* the virtual `COUNT(DISTINCT)` MUST return the value the local query returns
* *AND* the query MUST execute at the source rather than failing with MySQL error 1055
<!-- /DELTA:REMOVED -->

<!-- DELTA:REMOVED -->
### Scenario: MIN and MAX over a character column reproduce Exasol's code-point order

* *GIVEN* a MySQL `VARCHAR` column of collation `utf8mb4_0900_ai_ci` holding the values `B`, `a`, and `C`
* *WHEN* Exasol pushes down `MIN` and `MAX` over that column
* *THEN* the rendered operand MUST carry `COLLATE utf8mb4_0900_bin`
* *AND* the query MUST return `B` for `MIN` and `a` for `MAX`, matching what Exasol returns for the same aggregate over the same values
* *AND* `MIN` over a column holding two values that differ only by a trailing space MUST return the value Exasol returns for the same aggregate over the same values, so the collation MUST NOT be one that pads
* *AND* a numeric or temporal operand MUST render without a collation suffix
<!-- /DELTA:REMOVED -->

<!-- DELTA:REMOVED -->
### Scenario: Select-list predicate over the grouped character column fails with a named error

* *GIVEN* a `pushdown` request whose `aggregationType` is `group_by`, whose `groupBy` names one character column, and whose select list carries a predicate reading that same column outside any aggregate
* *WHEN* the MySQL dialect renders the statement
* *THEN* the dialect MUST return an error naming that column
* *AND* the dialect MUST NOT emit a MySQL statement, because MySQL rejects every rendering of this shape with error 1055 under `ONLY_FULL_GROUP_BY` whatever collation the operand carries
* *AND* the same MUST hold for each of the five measured predicate kinds `predicate_less`, `predicate_is_null`, `predicate_like`, `predicate_in_constlist`, and `predicate_between`, and for a request whose select list does not project the grouping column at all
* *AND* the exclusion MUST bind the MySQL dialect alone, because it is created by the collation that dialect renders on its grouping key rather than by the capability pair
<!-- /DELTA:REMOVED -->

<!-- DELTA:REMOVED -->
### Scenario: Select-list predicate shapes that stay delegated

* *GIVEN* three `pushdown` requests each carrying a select-list predicate: one whose `groupBy` names a numeric column that the predicate reads, one whose `groupBy` names a character column that the predicate reads only through a `MIN` over it, and one carrying no `group_by` aggregation whose predicate reads a character column
* *WHEN* the MySQL dialect renders each
* *THEN* each MUST produce a MySQL statement rather than an error
* *AND* the predicate over the `MIN` MUST carry `COLLATE utf8mb4_0900_as_cs` on the aggregate and `COLLATE utf8mb4_0900_bin` on the aggregate's own argument, the composition measured valid on MySQL 8.4 beside a collated grouping key
* *AND* the numeric grouping column MUST carry no collation suffix in either its `GROUP BY` occurrence or its predicate occurrence
* *AND* a predicate over `COUNT(*)` MUST carry no collation suffix at all
<!-- /DELTA:REMOVED -->

<!-- DELTA:REMOVED -->
### Scenario: Grouped-column select-list predicate fails before reaching MySQL

* *GIVEN* a virtual table whose character column holds values differing only by case and only by accent
* *WHEN* Exasol runs a `SELECT` projecting a comparison over that column while grouping by it
* *THEN* the query MUST fail with an error naming that column
* *AND* the error MUST come from the adapter rather than from MySQL, so no generated statement reaches the source
* *AND* the same query projecting a comparison over `COUNT(*)` instead MUST succeed and MUST return the values the local Exasol mirror returns
<!-- /DELTA:REMOVED -->

<!-- DELTA:NEW -->
### Scenario: No rendered operand carries a collation suffix

* *GIVEN* a MySQL `VARCHAR` column and a `pushdown` request carrying `predicate_equal`, `predicate_less`, `predicate_lessequal`, `predicate_between`, `predicate_in_constlist`, and `predicate_like` over it
* *WHEN* the MySQL dialect renders each predicate, in `WHERE` position and in select-list position
* *THEN* the generated statement MUST contain no `COLLATE` keyword
* *AND* the same MUST hold for a `GROUP BY` key and its select-list occurrence, for a `DISTINCT` aggregate operand, for a `MIN` and a `MAX` operand, and for a character equi-join key
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: A delegated comparison and grouping follow the column's own collation

* *GIVEN* a virtual table over a MySQL `VARCHAR` column under the server default collation `utf8mb4_0900_ai_ci`, holding two values that differ only by case and two that differ only by an accent
* *WHEN* an equality comparison against one value of each pair, a `GROUP BY` over the column, and a `COUNT(DISTINCT)` over it are run over the virtual table
* *THEN* `EXPLAIN VIRTUAL` MUST report the comparison and the grouping key in the generated statement with no `COLLATE` keyword
* *AND* each equality comparison MUST return both values of its pair, because MySQL's default collation matches them
* *AND* the `GROUP BY` MUST return one group per pair and the `COUNT(DISTINCT)` MUST count each pair once, which is MySQL's answer rather than the one Exasol computes over a local copy of the same values
* *AND* the value each group reports MUST be one of the values the group folds, and no step requires which one, because MySQL does not specify which member of a folded group it reports
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: A column declaring utf8mb4_0900_bin returns Exasol's answers

* *GIVEN* a virtual table over a MySQL table whose character columns declare `COLLATE utf8mb4_0900_bin`, and a local Exasol table holding the same values, including values that differ only by case, only by an accent, and only by a trailing space
* *WHEN* the same equality comparison, `GROUP BY`, `COUNT(DISTINCT)`, `MIN`, and `MAX` are run over each
* *THEN* each query over the virtual table MUST return exactly what the same query returns over the local table
* *AND* `EXPLAIN VIRTUAL` MUST report each fragment in the generated statement, so the rows come from MySQL rather than from Exasol evaluating the fragment itself
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: A select-list predicate over the grouped character column is delegated

* *GIVEN* a `pushdown` request whose `aggregationType` is `group_by`, whose `groupBy` names one MySQL `VARCHAR` column, and whose select list carries a predicate reading that same column outside any aggregate
* *WHEN* the MySQL dialect renders the statement
* *THEN* the dialect MUST produce a statement rather than an error
* *AND* the statement MUST execute at the source rather than being rejected with MySQL error 1055
* *AND* over a column declaring `COLLATE utf8mb4_0900_bin`, the rows it returns MUST equal what Exasol returns for the same query over a local copy of the same values
* *AND* this scenario MUST be verified against a live MySQL source rather than by rendering alone, because what is asserted is that the source accepts an expression over its bare grouping column
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: A comparison over a wrapped operand is evaluated by Exasol

* *GIVEN* a virtual table over a MySQL `VARCHAR` column under the server default collation `utf8mb4_0900_ai_ci`, holding two values that differ only by case, and a local Exasol table holding the same values
* *WHEN* the same equality comparison against one value of the pair, with the column wrapped as `"<column>" || ''`, is run over each
* *THEN* `EXPLAIN VIRTUAL` MUST report a generated statement that carries no comparison over that column, because the MySQL dialect advertises no scalar-function capability
* *AND* the virtual query MUST return exactly the rows the local query returns, which hold one value of the pair rather than both
<!-- /DELTA:NEW -->
