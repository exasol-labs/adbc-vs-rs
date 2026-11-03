# Feature: mysql_collation

States that the MySQL dialect renders no collation, what MySQL's default collation does to a delegated character comparison, grouping, and `MIN` or `MAX`, and the two ways a user obtains Exasol's code-point comparison.

<!-- DELTA:CHANGED -->
## Background

- The MySQL dialect renders no collation suffix on any operand, per the framework rule `vs-adapter/pushdown-filter` states. MySQL therefore evaluates every delegated character comparison, `LIKE`, `IN`, grouping key, `DISTINCT` operand, `MIN` or `MAX` operand, and equi-join key under the collation it applies to the column. A `latin1` column compares under its own `latin1` collation, `latin1_swedish_ci` by default, and a value the dialect reads as `CAST(<column> AS CHAR)`, such as a `JSON` value, compares under the session's connection collation.
- MySQL's default column collation `utf8mb4_0900_ai_ci` is case-insensitive and accent-insensitive and orders by collation weight, while Exasol compares by code point. Measured against this release's own fixture: `name = 'widget'` matches the stored value `Widget`, and `'Gizmo' = 'Gîzmo'` is true. Over a `VARCHAR(50)` column holding `Widget`, `widget`, `Gizmo`, and `Gîzmo`, `GROUP BY` produced 2 groups and `COUNT(DISTINCT)` returned 2, where Exasol returns 4 for both. A delegated query over such a column returns MySQL's answers.
- A user who needs Exasol's comparison has two remedies. The first declares `utf8mb4_0900_bin` at the source, on the column or in a view the virtual schema exposes. It weighs the `utf8mb4` encoding bytes, so it reproduces code-point order, and MySQL reports it `NO PAD`. `utf8mb4_bin` is not a substitute: `information_schema.COLLATIONS` reports it `PAD SPACE`, and measured on the test MySQL, `MIN` over the values `'ab '`, `'ab'`, `'ab!'` returns `'ab '` under `utf8mb4_bin`, where Exasol and `utf8mb4_0900_bin` return `'ab'`. `latin1_bin` is `PAD SPACE` too, so for a `latin1` column the view exposes `CONVERT(<column> USING utf8mb4) COLLATE utf8mb4_0900_bin`.
- The second remedy wraps the operand in an expression the MySQL dialect does not advertise, such as `"category" || ''`. The dialect advertises no scalar-function capability, so Exasol keeps a comparison over the wrapped operand and evaluates it itself over the rows MySQL returns.
- A grouping key renders as the column's read expression, per `vs-adapter/pushdown-dialect`. A bare key's select-list occurrence and a select-list predicate reading it are expressions over that same column, and measured on the test MySQL, such a predicate executes under `ONLY_FULL_GROUP_BY`, the mode the test MySQL sets by default. MySQL matches a select-list expression against a grouping expression only whole, so a select-list reference to a key read through a conversion renders as `ANY_VALUE(<read expression>)`, which MySQL exempts from that check.
<!-- /DELTA:CHANGED -->

## Scenarios

### Scenario: A select-list predicate over the grouped character column is delegated

* *GIVEN* a `pushdown` request whose `aggregationType` is `group_by`, whose `groupBy` names one MySQL `VARCHAR` column, and whose select list carries a predicate reading that same column outside any aggregate
* *WHEN* the MySQL dialect renders the statement
* *THEN* the dialect MUST produce a statement rather than an error
* *AND* the statement MUST execute at the source rather than being rejected with MySQL error 1055
* *AND* over a column declaring `COLLATE utf8mb4_0900_bin`, the rows it returns MUST equal what Exasol returns for the same query over a local copy of the same values
* *AND* this scenario MUST be verified against a live MySQL source rather than by rendering alone, because what is asserted is that the source accepts an expression over its bare grouping column
