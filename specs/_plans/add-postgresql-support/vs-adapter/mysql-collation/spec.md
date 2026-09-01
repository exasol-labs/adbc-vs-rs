# Feature: mysql_collation

States where the MySQL dialect renders a collation suffix, which of its two collations it renders at each site, and the one composed shape the grouping collation makes unrenderable.

## Background

- Exasol asks two different questions of a character value, and MySQL's default answers both differently. Equality asks whether two values are the same characters; ordering asks which comes first, and Exasol answers it by code point. MySQL 8.4's default column collation `utf8mb4_0900_ai_ci` is case-insensitive and accent-insensitive and orders by collation weight: measured against the release's own fixture, `name = 'widget'` matches the stored value `Widget` and `'Gizmo' = 'Gîzmo'` is true, and `MIN` over the values `B`, `a`, `C` returns `a` in MySQL where Exasol returns `B`. The MySQL dialect therefore renders a collation on an equality operand and on an ordering operand — two collations, not one.
- `utf8mb4_0900_as_cs` is the equality collation. It reproduces Exasol's case-sensitive, accent-sensitive comparison over unpadded values, and it is rendered wherever the delegated fragment asks the equality question: a character comparison operand of a delegated predicate, per `vs-adapter/pushdown-filter`; a character equi-join key, per `vs-adapter/pushdown-join`; a character `GROUP BY` key; a character `DISTINCT` aggregate operand; and a character-typed aggregate operand of a select-list predicate. One collation per question, chosen by the operand's declared type, is the rule; which sites ask the equality question is settled by the feature that owns each site.
- The equality collation follows a predicate into every position and every predicate kind that compares characters. A `LIKE` operand carries it, because MySQL matches a pattern under the operand's collation and its default matches values Exasol keeps apart. An `IN` comparison carries it on the compared operand rather than on the list, so one suffix governs every list entry. A predicate in select-list position carries whatever suffix the same comparison carries in `WHERE` position, because `vs-adapter/pushdown-filter` renders both positions through one renderer. The pattern's own literal form is a separate question, answered by `vs-adapter/mysql-pushdown`'s hex rendering.
- `utf8mb4_0900_bin` is the ordering collation, rendered on a character operand of `MIN` or `MAX`. It weighs the `utf8mb4` encoding bytes and so reproduces code-point order, and MySQL reports it `NO PAD`. `utf8mb4_0900_as_cs` orders by collation weight and is therefore wrong for `MIN` and `MAX`; the two collations answer different questions and neither substitutes for the other. The two compose on one operand: measured valid on MySQL 8.4, `MIN(\`v\` COLLATE utf8mb4_0900_bin) COLLATE utf8mb4_0900_as_cs` compares case-sensitively over the code-point minimum.
- `utf8mb4_bin` is neither collation. `information_schema.COLLATIONS` reports it as `PAD SPACE`, so it ignores a trailing space: measured on MySQL 8.4, `MIN` over the values `'ab '`, `'ab'`, `'ab!'` returns `'ab '` under `utf8mb4_bin`, while Exasol 2025.2.1 returns `'ab'` and reports `'ab' = 'ab '` as false. `utf8mb4_0900_bin` returns `'ab'` for the same values, matching Exasol.
- Both collations MUST exist on the source server. `utf8mb4_0900_bin` requires MySQL 8.0.17, which `vs-adapter/mysql-type-mapping` enforces as the source-server floor at creation.
- MySQL cannot use an index on a column carrying an explicit non-matching collation, so a character predicate filters at the source without index support. Numeric and temporal operands carry no collation and keep their indexes.
- The equality collation is load-bearing, not decorative. Measured on MySQL 8.4 over a `VARCHAR(50)` column of collation `utf8mb4_0900_ai_ci` holding `Widget`, `widget`, `Gizmo`, and `Gîzmo`: `GROUP BY` produced 2 groups and `COUNT(DISTINCT)` returned 2 under the column's own collation, and 4 of each under `utf8mb4_0900_as_cs`. Exasol 2025.2.1 returns 4 for both over the same values. An uncollated delegated `GROUP BY` therefore returns fewer groups than Exasol asked for, and an uncollated delegated `COUNT(DISTINCT)` a smaller count — the silent wrong-value failure mode this release does not ship.
- Collating the `GROUP BY` key turns the key into an expression, which makes the grouping collation one coordinated rule over two clauses rather than two independent rules. MySQL's `ONLY_FULL_GROUP_BY` mode, which `mysql:8.4` sets by default, rejects a select list whose bare column occurrence does not carry the collation its `GROUP BY` key carries: measured, `SELECT v, COUNT(*) FROM n GROUP BY v COLLATE utf8mb4_0900_as_cs` failed with MySQL error 1055, and the same statement collating both occurrences succeeded and returned Exasol's 4 groups. A grouping key the select list does not project needs no second occurrence, and Exasol rejects a select list naming a non-aggregated column absent from `GROUP BY` exactly as MySQL does, so no other bare column occurrence can arrive. An aggregate is never subject to the mode, so a `MIN` over the grouping column keeps `COLLATE utf8mb4_0900_bin` while the grouping key carries `utf8mb4_0900_as_cs` — measured valid in one MySQL statement.
- That same expression-ness creates the one composed shape MySQL rejects outright. A select-list predicate reading its own query's character-typed `GROUP BY` key outside an aggregate fails with MySQL error 1055 under `ONLY_FULL_GROUP_BY` at every collation placement, including the one leaving the predicate's operand uncollated: MySQL matches a select-list expression against a `GROUP BY` expression whole and credits no sub-expression of the collated grouping key. The dialect therefore fails with an error naming the column rather than emitting a statement the source refuses, which is the same outcome diagnosed at the point the operator can read it. `vs-adapter/pushdown-dialect` owns the framework rule that a composed shape a dialect cannot render takes the same outcome as an unrenderable fragment.
- Measured on Exasol 2025.2.1 with a throwaway Lua adapter probe over the capability set this release advertises, five predicate kinds arrive in that shape — `"g" > 'x'`, `"g" IS NULL`, `"g" LIKE 'a%'`, `"g" IN ('a','b')`, and `"g" BETWEEN 'a' AND 'z'` — each operand naming the exact column `groupBy` names, each declared `BOOLEAN`, and the comparison also arrives in the variant projecting no bare column at all.
- The hazard is bounded to that one shape in both directions, and every other select-list predicate is delegated. A select-list predicate's operand is always either the query's own grouping column or an aggregate: `SELECT "d", "g" > 'x' … GROUP BY "d"` never reaches the adapter, because Exasol rejects it itself with `not a valid GROUP BY expression`. Measured on MySQL 8.4 over the same four values, a predicate over `COUNT(*)`, a predicate over `MIN` of the character grouping column, and a predicate over a numeric grouping key each returned the 4 rows Exasol returns. A query whose aggregation is `single_group` or absent carries no grouping key to disagree with.
- Nothing here binds a predicate in `filter` position: MySQL evaluates `WHERE` before grouping, so no `WHERE` operand is subject to the mode.
- This rule set is MySQL's alone. `vs-adapter/postgresql-collation` states PostgreSQL's, which is ordering-only, because PostgreSQL's deterministic collations already answer Exasol's equality question and so need no equality suffix and create no coordinated grouping rule. Neither dialect's rule set derives from the other's.
- One question about the equality collation is open rather than settled, and no scenario here asserts an answer to it. Under `utf8mb4_0900_as_cs` MySQL compares an NFC-composed character and its NFD-decomposed equivalent as equal, measured on MySQL 8.4, while `ß` and `ss` compare unequal under the same collation. Exasol compares character values by code point. A delegated `GROUP BY` over normalization-variant values may therefore still return fewer groups than Exasol asked for, in a narrower form of the divergence the grouping collation removes for case and accent. What Exasol 2025.2.1 returns for such values was not measured, so the gap is unconfirmed rather than known, and the fixture carries no normalization-variant pair.

## Scenarios

### Scenario: Character comparison reproduces Exasol's case and accent sensitivity

* *GIVEN* a MySQL `VARCHAR` column whose collation is `utf8mb4_0900_ai_ci` holding the value `Widget`
* *WHEN* Exasol pushes down a filter comparing that column to the literal `widget`
* *THEN* the rendered operand MUST carry the suffix `COLLATE utf8mb4_0900_as_cs`
* *AND* the query MUST return no row, matching what Exasol returns for the same comparison
* *AND* the same MUST hold for a value differing only in an accent

### Scenario: A LIKE operand, an IN comparison, and a select-list predicate carry the comparison collation

* *GIVEN* a `predicate_like` over a character column, a `predicate_in_constlist` over a character column, and a `pushdown` request carrying no `group_by` aggregation whose select list carries a character comparison declared `BOOLEAN`
* *WHEN* the MySQL dialect renders each
* *THEN* the `LIKE` operand MUST carry `COLLATE utf8mb4_0900_as_cs`, because MySQL matches a pattern under the operand's collation and its default matches values Exasol keeps apart
* *AND* the `IN` comparison MUST carry that suffix on the compared operand rather than on the list
* *AND* the select-list comparison MUST carry the same suffix its `WHERE` counterpart carries, so the one renderer `vs-adapter/pushdown-filter` requires for both positions gives one answer to the equality question
* *AND* a numeric or temporal operand MUST carry no suffix in any of the three positions

### Scenario: Grouped character columns carry the comparison collation in both the grouping list and the select list

* *GIVEN* a `pushdown` request whose `aggregationType` is `group_by`, whose `groupBy` names one character column and one numeric column, and whose select list projects both grouping columns beside a `MIN` over the same character column
* *WHEN* the MySQL dialect renders the statement
* *THEN* the character grouping column MUST carry `COLLATE utf8mb4_0900_as_cs` in the `GROUP BY` clause
* *AND* the same suffix MUST appear on that column's bare select-list occurrence, because MySQL rejects the statement with error 1055 under `ONLY_FULL_GROUP_BY` when the two occurrences carry different collations
* *AND* the `MIN` over the character column MUST keep `COLLATE utf8mb4_0900_bin`, because an aggregate is not subject to `ONLY_FULL_GROUP_BY` and the value `MIN` returns is chosen by order rather than by equality

### Scenario: Only a grouped character column carries the grouping collation

* *GIVEN* a `pushdown` request whose `groupBy` names a numeric column, and a second request carrying no `aggregationType` whose select list projects a character column
* *WHEN* the MySQL dialect renders each statement
* *THEN* the numeric grouping column MUST render with no collation suffix in the `GROUP BY` clause and none in the select list
* *AND* the character column of the second request MUST render with no collation suffix, so a projection-only statement is byte-identical to what this release renders before the grouping rule exists
* *AND* the collation MUST therefore read off both the operand's declared type and whether the query groups by that column, never off the type alone

### Scenario: A DISTINCT aggregate over a character column carries the comparison collation

* *GIVEN* a `function_aggregate` whose `distinct` field is true over a character column, the shape `COUNT(DISTINCT "name")` arrives in
* *WHEN* the MySQL dialect renders it
* *THEN* the operand MUST carry `COLLATE utf8mb4_0900_as_cs`, because `DISTINCT` reduces values by equality and MySQL's default collation equates values Exasol keeps apart
* *AND* the same aggregate carrying no `distinct` field MUST render its operand with no collation suffix, because `COUNT` over a character column reads neither equality nor order
* *AND* a character operand of `MIN` or `MAX` MUST carry `COLLATE utf8mb4_0900_bin` whatever its `distinct` field holds, because the value those two return is chosen by order

### Scenario: Delegated grouping and DISTINCT over case-only variants return Exasol's groups

* *GIVEN* a virtual table and a local Exasol table holding identical character values, two of which differ only by case and two only by an accent
* *WHEN* the same `GROUP BY` over that column, and the same `COUNT(DISTINCT)` over it, are run over each
* *THEN* `EXPLAIN VIRTUAL` MUST report a generated statement whose `GROUP BY` key and whose projected grouping column both carry `COLLATE utf8mb4_0900_as_cs`
* *AND* the virtual `GROUP BY` MUST return exactly the groups the local query returns, so no two values Exasol keeps apart are collapsed into one group
* *AND* the virtual `COUNT(DISTINCT)` MUST return the value the local query returns
* *AND* the query MUST execute at the source rather than failing with MySQL error 1055

### Scenario: MIN and MAX over a character column reproduce Exasol's code-point order

* *GIVEN* a MySQL `VARCHAR` column of collation `utf8mb4_0900_ai_ci` holding the values `B`, `a`, and `C`
* *WHEN* Exasol pushes down `MIN` and `MAX` over that column
* *THEN* the rendered operand MUST carry `COLLATE utf8mb4_0900_bin`
* *AND* the query MUST return `B` for `MIN` and `a` for `MAX`, matching what Exasol returns for the same aggregate over the same values
* *AND* `MIN` over a column holding two values that differ only by a trailing space MUST return the value Exasol returns for the same aggregate over the same values, so the collation MUST NOT be one that pads
* *AND* a numeric or temporal operand MUST render without a collation suffix

### Scenario: Select-list predicate over the grouped character column fails with a named error

* *GIVEN* a `pushdown` request whose `aggregationType` is `group_by`, whose `groupBy` names one character column, and whose select list carries a predicate reading that same column outside any aggregate
* *WHEN* the MySQL dialect renders the statement
* *THEN* the dialect MUST return an error naming that column
* *AND* the dialect MUST NOT emit a MySQL statement, because MySQL rejects every rendering of this shape with error 1055 under `ONLY_FULL_GROUP_BY` whatever collation the operand carries
* *AND* the same MUST hold for each of the five measured predicate kinds `predicate_less`, `predicate_is_null`, `predicate_like`, `predicate_in_constlist`, and `predicate_between`, and for a request whose select list does not project the grouping column at all
* *AND* the exclusion MUST bind the MySQL dialect alone, because it is created by the collation that dialect renders on its grouping key rather than by the capability pair

### Scenario: Select-list predicate shapes that stay delegated

* *GIVEN* three `pushdown` requests each carrying a select-list predicate: one whose `groupBy` names a numeric column that the predicate reads, one whose `groupBy` names a character column that the predicate reads only through a `MIN` over it, and one carrying no `group_by` aggregation whose predicate reads a character column
* *WHEN* the MySQL dialect renders each
* *THEN* each MUST produce a MySQL statement rather than an error
* *AND* the predicate over the `MIN` MUST carry `COLLATE utf8mb4_0900_as_cs` on the aggregate and `COLLATE utf8mb4_0900_bin` on the aggregate's own argument, the composition measured valid on MySQL 8.4 beside a collated grouping key
* *AND* the numeric grouping column MUST carry no collation suffix in either its `GROUP BY` occurrence or its predicate occurrence
* *AND* a predicate over `COUNT(*)` MUST carry no collation suffix at all

### Scenario: Grouped-column select-list predicate fails before reaching MySQL

* *GIVEN* a virtual table whose character column holds values differing only by case and only by accent
* *WHEN* Exasol runs a `SELECT` projecting a comparison over that column while grouping by it
* *THEN* the query MUST fail with an error naming that column
* *AND* the error MUST come from the adapter rather than from MySQL, so no generated statement reaches the source
* *AND* the same query projecting a comparison over `COUNT(*)` instead MUST succeed and MUST return the values the local Exasol mirror returns
