<!-- DELTA:CHANGED -->
# Feature: postgresql_collation

States that the PostgreSQL dialect renders no collation, what PostgreSQL's column collation does to a delegated character comparison, grouping, and `MIN` or `MAX`, and the two ways a user obtains Exasol's code-point order.
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
## Background

- The PostgreSQL dialect renders no collation suffix on any operand, per the framework rule `vs-adapter/pushdown-filter` states. PostgreSQL therefore evaluates every delegated character comparison, `LIKE`, `IN`, grouping key, `DISTINCT` operand, `MIN` or `MAX` operand, and equi-join key under the collation it applies to the column, which is the database default unless the column declares its own.
- PostgreSQL's built-in collations are all deterministic, and a deterministic collation equates two character values only when their bytes are identical, which is Exasol's equality. Over such a column a delegated equality, `LIKE`, `IN`, `GROUP BY`, and `DISTINCT` return the answer Exasol computes.
- Ordering follows the collation's sort order. The official `postgres` image sets `LANG=en_US.utf8`, so `initdb` creates the cluster with libc `LC_COLLATE=en_US.utf8` and text sorts by locale weight: `MIN` over the values `B`, `a`, `C` returns `a` where Exasol returns `B`, and `'a' < 'B'` holds where Exasol 2025.2.1 finds it false. A delegated ordering comparison, `BETWEEN`, `MIN`, and `MAX` over such a column return PostgreSQL's answer.
- An operator-created nondeterministic collation equates values Exasol keeps apart. A delegated comparison over such a column returns that answer, and `vs-adapter/postgresql-type-mapping` maps the column like any other.
- A user who needs Exasol's code-point order has two remedies. The first declares `COLLATE "C"` at the source, on the column or in a view the virtual schema exposes. `C` sorts by byte value, which for UTF-8 is code-point order, and compares by byte equality, so it answers both of Exasol's questions.
- The second remedy wraps the operand in an expression the PostgreSQL dialect does not advertise, such as `"sort_key" || ''`. The dialect advertises no scalar-function capability, so Exasol keeps a comparison, `MIN`, or `MAX` over the wrapped operand and evaluates it itself over the rows PostgreSQL returns.
- A grouping key renders as the bare column, so its select-list occurrence is the same expression and PostgreSQL's full-group-by rule is satisfied without coordination.
<!-- /DELTA:CHANGED -->

## Scenarios

<!-- DELTA:REMOVED -->
### Scenario: An ordering comparison is collated and equality is left bare

* *GIVEN* a PostgreSQL `character varying` column and a `pushdown` request carrying `predicate_equal`, `predicate_less`, `predicate_lessequal`, `predicate_between`, `predicate_in_constlist`, and `predicate_like` over it
* *WHEN* the PostgreSQL dialect renders each predicate
* *THEN* the character operand of `predicate_less`, `predicate_lessequal`, and `predicate_between` MUST carry the suffix `COLLATE "C"`
* *AND* the character operand of `predicate_equal`, `predicate_in_constlist`, and `predicate_like` MUST carry no collation suffix
* *AND* a numeric or temporal operand MUST carry no collation suffix in any predicate
* *AND* the suffix MUST be rendered on the compared operand rather than on a literal or on an `IN` list
<!-- /DELTA:REMOVED -->

<!-- DELTA:REMOVED -->
### Scenario: Character comparison reproduces Exasol's case, accent, and code-point order

* *GIVEN* a virtual table over a PostgreSQL `character varying` column and a local Exasol table holding identical values, including two differing only by case, two differing only by an accent, and the values `B`, `a`, and `C`
* *WHEN* the same equality comparison and the same ordering comparison are run over each
* *THEN* the equality comparison over the virtual table MUST return exactly the rows the local query returns, so no two values Exasol keeps apart are matched
* *AND* the ordering comparison over the virtual table MUST return exactly the rows the local query returns, so the delegated comparison follows Exasol's code-point order rather than a locale's collation weight
* *AND* `EXPLAIN VIRTUAL` MUST report `COLLATE "C"` on the ordering comparison's operand and no collation suffix on the equality comparison's operand
<!-- /DELTA:REMOVED -->

<!-- DELTA:REMOVED -->
### Scenario: MIN and MAX over a character column render the C collation

* *GIVEN* a PostgreSQL `character varying` column holding the values `B`, `a`, and `C`, in a database whose cluster collation orders by the `en_US.utf8` locale
* *WHEN* Exasol pushes down `MIN` and `MAX` over that column
* *THEN* the rendered operand MUST carry `COLLATE "C"`
* *AND* the query MUST return `B` for `MIN` and `a` for `MAX`, matching what Exasol returns for the same aggregate over the same values
* *AND* a numeric or temporal operand MUST render without a collation suffix
* *AND* the same aggregate rendered without the suffix MUST return a different value against the same data, so the test proves the suffix is load-bearing rather than decorative
<!-- /DELTA:REMOVED -->

<!-- DELTA:REMOVED -->
### Scenario: A grouping key and a DISTINCT operand render without a collation

* *GIVEN* a `pushdown` request whose `aggregationType` is `group_by`, whose `groupBy` names one PostgreSQL `character varying` column and one numeric column, and whose select list projects both grouping columns beside a `MIN` over the same character column
* *WHEN* the PostgreSQL dialect renders the statement
* *THEN* the character grouping column MUST carry no collation suffix in the `GROUP BY` clause
* *AND* it MUST carry no collation suffix on its bare select-list occurrence either, so the grouping key and that occurrence are the same expression and PostgreSQL's unconditional full-group-by rule is satisfied without a coordinated suffix
* *AND* the `MIN` over the character column MUST carry `COLLATE "C"`, because the value `MIN` returns is chosen by order rather than by equality
* *AND* a `function_aggregate` whose `distinct` field is true over a character column MUST render its operand with no collation suffix
<!-- /DELTA:REMOVED -->

<!-- DELTA:CHANGED -->
### Scenario: Delegated grouping and DISTINCT over case-only variants return Exasol's groups

* *GIVEN* a PostgreSQL virtual table and a local Exasol table holding identical character values, two of which differ only by case and two only by an accent
* *WHEN* the same `GROUP BY` over that column, and the same `COUNT(DISTINCT)` over it, are run over each
* *THEN* `EXPLAIN VIRTUAL` MUST report a generated statement whose `GROUP BY` key carries no collation suffix
* *AND* the virtual `GROUP BY` MUST return exactly the groups the local query returns, so no two values Exasol keeps apart are collapsed into one group
* *AND* the virtual `COUNT(DISTINCT)` MUST return the value the local query returns
* *AND* the query MUST execute at the source rather than failing
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: No rendered operand carries a collation suffix

* *GIVEN* a PostgreSQL `character varying` column and a `pushdown` request carrying `predicate_equal`, `predicate_less`, `predicate_lessequal`, `predicate_between`, `predicate_in_constlist`, and `predicate_like` over it
* *WHEN* the PostgreSQL dialect renders each predicate, in `WHERE` position and in select-list position
* *THEN* the generated statement MUST contain no `COLLATE` keyword
* *AND* the same MUST hold for a `GROUP BY` key and its select-list occurrence, for a `DISTINCT` aggregate operand, for a `MIN` and a `MAX` operand, and for a character equi-join key
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: A delegated ordering comparison and MIN or MAX follow the column's own collation

* *GIVEN* a virtual table over a PostgreSQL `character varying` column under the database default collation `en_US.utf8`, holding the values `B`, `a`, and `C`
* *WHEN* a comparison selecting the values ordered before `B`, a `MIN`, and a `MAX` over the column are run over the virtual table
* *THEN* `EXPLAIN VIRTUAL` MUST report the comparison and both aggregates in the generated statement with no `COLLATE` keyword
* *AND* the comparison MUST return the rows holding `a`, which the locale orders before `B` and Exasol's code-point order does not
* *AND* `MIN` MUST return `a` and `MAX` MUST return `C`, which is PostgreSQL's answer rather than the `B` and `a` Exasol computes over a local copy of the same values
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: A column declaring the C collation returns Exasol's answers

* *GIVEN* a virtual table over a PostgreSQL table whose character columns declare `COLLATE "C"`, and a local Exasol table holding the same values, including the values `B`, `a`, and `C` and values that differ only by case or only by an accent
* *WHEN* the same ordering comparison, `BETWEEN`, `MIN`, `MAX`, `GROUP BY`, and `COUNT(DISTINCT)` are run over each
* *THEN* each query over the virtual table MUST return exactly what the same query returns over the local table
* *AND* `EXPLAIN VIRTUAL` MUST report each fragment in the generated statement, so the rows come from PostgreSQL rather than from Exasol evaluating the fragment itself
<!-- /DELTA:NEW -->

<!-- DELTA:CHANGED -->
### Scenario: A select-list predicate over the grouped character column is delegated

* *GIVEN* a `pushdown` request whose `aggregationType` is `group_by`, whose `groupBy` names one PostgreSQL `character varying` column, and whose select list carries a predicate reading that same column outside any aggregate
* *WHEN* the PostgreSQL dialect renders the statement
* *THEN* the dialect MUST produce a statement rather than an error
* *AND* the statement MUST execute at the source rather than being rejected by it
* *AND* over a column declaring `COLLATE "C"`, the rows it returns MUST equal what Exasol returns for the same query executed over a local mirror of the same values
* *AND* this scenario MUST be verified against a live PostgreSQL source rather than by rendering alone, because what is asserted is that the source accepts an expression over its bare grouping column
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: An ordering comparison over a wrapped operand is evaluated by Exasol

* *GIVEN* a virtual table over a PostgreSQL `character varying` column under the database default collation `en_US.utf8`, holding the values `B`, `a`, and `C`, and a local Exasol table holding the same values
* *WHEN* the same comparison selecting the values ordered before `B`, and the same `MIN`, each with the column wrapped as `"<column>" || ''`, are run over each
* *THEN* `EXPLAIN VIRTUAL` MUST report a generated statement that carries neither the comparison nor the aggregate, because the PostgreSQL dialect advertises no scalar-function capability
* *AND* each virtual query MUST return exactly what the local query returns: no row holding `a` for the comparison, and `B` for `MIN`
<!-- /DELTA:NEW -->
