# Feature: postgresql_collation

States where the PostgreSQL dialect renders a collation suffix, and why it needs one in fewer places than MySQL does.

## Background

- Exasol asks two different questions of a character value. Equality asks whether two values are the same characters; ordering asks which comes first, and Exasol answers it by code point. A dialect renders a collation wherever its target's default answers one of those questions differently from Exasol.
- PostgreSQL's built-in collations are all deterministic, and a deterministic collation equates two character values only when their bytes are identical — which is Exasol's equality exactly. PostgreSQL therefore needs NO collation suffix on an equality operand, a `LIKE` operand, an `IN` operand, a `GROUP BY` key, a bare select-list occurrence of a grouped column, a `DISTINCT` aggregate operand, or a join key. Those operands keep the source's index support, which every MySQL character operand loses.
- PostgreSQL needs a collation only on an ordering operand. The official `postgres` image sets `LANG=en_US.utf8`, so `initdb` creates the cluster with libc `LC_COLLATE=en_US.utf8` and text sorts by locale weight: `MIN` over the values `B`, `a`, `C` returns `a` where Exasol returns `B`. `COLLATE "C"` sorts by byte value, which for UTF-8 is code-point order, so it reproduces Exasol's answer.
- `COLLATE "C"` is rendered on a character operand of `predicate_less`, `predicate_lessequal`, `predicate_between`, `MIN`, and `MAX`, and nowhere else. A numeric or temporal operand carries no suffix in any position. The suffix is rendered on the compared operand rather than on a literal or on an `IN` list.
- Whether `LIKE` needs the suffix is settled by measurement rather than by the collation's definition: PostgreSQL's `LIKE` compares character by character rather than by collation weight, so no suffix is rendered, and gate 2.3(f) measures a pattern whose matching would differ if that were wrong.
- Because the grouping key renders bare, the `GROUP BY` key and its select-list occurrence are the same expression, so PostgreSQL's unconditional full-group-by rule is satisfied without a coordinated suffix. MySQL's composed-shape exclusion — a select-list predicate reading its own query's character `GROUP BY` key, rejected with error 1055 because the collated key is an expression MySQL credits no sub-expression of — therefore has no analogue here. Gate 2.3(b) measures that the shape executes before the PostgreSQL dialect delegates it, because the failing direction is a hard source error rather than wrong rows.
- This rule set derives from nothing in `vs-adapter/mysql-collation`, and neither feature governs the other. MySQL renders two collations because its default answers both of Exasol's questions differently; PostgreSQL renders one because its default answers only the ordering question differently. One collation answers both questions here, since `C` is byte equality as well as byte order.
- `C` is a built-in collation present in every PostgreSQL cluster rather than one a release added, so no source-server version floor follows from it. What does follow is a UTF-8 database encoding, because `C` reproduces code-point order only over UTF-8 bytes; `vs-adapter/postgresql-type-mapping` owns that precondition and its check.
- The dependence on deterministic collations is paid for in `vs-adapter/postgresql-type-mapping`, which unmaps a character column declaring a nondeterministic collation. A nondeterministic ICU collation equates values Exasol keeps apart — the same silent wrong-rows failure MySQL's default collation causes — and makes `LIKE` fail outright, so the column is refused rather than exposed.

## Scenarios

### Scenario: An ordering comparison is collated and equality is left bare

* *GIVEN* a PostgreSQL `character varying` column and a `pushdown` request carrying `predicate_equal`, `predicate_less`, `predicate_lessequal`, `predicate_between`, `predicate_in_constlist`, and `predicate_like` over it
* *WHEN* the PostgreSQL dialect renders each predicate
* *THEN* the character operand of `predicate_less`, `predicate_lessequal`, and `predicate_between` MUST carry the suffix `COLLATE "C"`
* *AND* the character operand of `predicate_equal`, `predicate_in_constlist`, and `predicate_like` MUST carry no collation suffix
* *AND* a numeric or temporal operand MUST carry no collation suffix in any predicate
* *AND* the suffix MUST be rendered on the compared operand rather than on a literal or on an `IN` list

### Scenario: Character comparison reproduces Exasol's case, accent, and code-point order

* *GIVEN* a virtual table over a PostgreSQL `character varying` column and a local Exasol table holding identical values, including two differing only by case, two differing only by an accent, and the values `B`, `a`, and `C`
* *WHEN* the same equality comparison and the same ordering comparison are run over each
* *THEN* the equality comparison over the virtual table MUST return exactly the rows the local query returns, so no two values Exasol keeps apart are matched
* *AND* the ordering comparison over the virtual table MUST return exactly the rows the local query returns, so the delegated comparison follows Exasol's code-point order rather than a locale's collation weight
* *AND* `EXPLAIN VIRTUAL` MUST report `COLLATE "C"` on the ordering comparison's operand and no collation suffix on the equality comparison's operand

### Scenario: MIN and MAX over a character column render the C collation

* *GIVEN* a PostgreSQL `character varying` column holding the values `B`, `a`, and `C`, in a database whose cluster collation orders by the `en_US.utf8` locale
* *WHEN* Exasol pushes down `MIN` and `MAX` over that column
* *THEN* the rendered operand MUST carry `COLLATE "C"`
* *AND* the query MUST return `B` for `MIN` and `a` for `MAX`, matching what Exasol returns for the same aggregate over the same values
* *AND* a numeric or temporal operand MUST render without a collation suffix
* *AND* the same aggregate rendered without the suffix MUST return a different value against the same data, so the test proves the suffix is load-bearing rather than decorative

### Scenario: A grouping key and a DISTINCT operand render without a collation

* *GIVEN* a `pushdown` request whose `aggregationType` is `group_by`, whose `groupBy` names one PostgreSQL `character varying` column and one numeric column, and whose select list projects both grouping columns beside a `MIN` over the same character column
* *WHEN* the PostgreSQL dialect renders the statement
* *THEN* the character grouping column MUST carry no collation suffix in the `GROUP BY` clause
* *AND* it MUST carry no collation suffix on its bare select-list occurrence either, so the grouping key and that occurrence are the same expression and PostgreSQL's unconditional full-group-by rule is satisfied without a coordinated suffix
* *AND* the `MIN` over the character column MUST carry `COLLATE "C"`, because the value `MIN` returns is chosen by order rather than by equality
* *AND* a `function_aggregate` whose `distinct` field is true over a character column MUST render its operand with no collation suffix

### Scenario: Delegated grouping and DISTINCT over case-only variants return Exasol's groups

* *GIVEN* a PostgreSQL virtual table and a local Exasol table holding identical character values, two of which differ only by case and two only by an accent
* *WHEN* the same `GROUP BY` over that column, and the same `COUNT(DISTINCT)` over it, are run over each
* *THEN* `EXPLAIN VIRTUAL` MUST report a generated statement whose `GROUP BY` key carries no collation suffix
* *AND* the virtual `GROUP BY` MUST return exactly the groups the local query returns, so no two values Exasol keeps apart are collapsed into one group
* *AND* the virtual `COUNT(DISTINCT)` MUST return the value the local query returns
* *AND* the query MUST execute at the source rather than failing, so the absence of a suffix is proven correct against the same fixture that proves MySQL needs one

### Scenario: A select-list predicate over the grouped character column is delegated

* *GIVEN* a `pushdown` request whose `aggregationType` is `group_by`, whose `groupBy` names one PostgreSQL `character varying` column, and whose select list carries a predicate reading that same column outside any aggregate
* *WHEN* the PostgreSQL dialect renders the statement
* *THEN* the dialect MUST produce a statement rather than an error
* *AND* the statement MUST execute at the source rather than being rejected by it
* *AND* the rows it returns MUST equal what Exasol returns for the same query executed over a local mirror of the same values
* *AND* this scenario MUST be verified against a live PostgreSQL source rather than by rendering alone, because what is asserted is that the source accepts an expression over its bare grouping column
