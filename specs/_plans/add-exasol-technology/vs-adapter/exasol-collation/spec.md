# Feature: exasol_collation

States that the Exasol dialect renders no collation suffix in any position, why the source could not accept one, and how that absence is proven correct rather than assumed.

## Background

- Exasol asks two questions of a character value. Equality asks whether two values are the same characters; ordering asks which comes first. A dialect renders a collation wherever its target's default answers one of those questions differently from Exasol.
- An Exasol source answers both questions the way Exasol answers them, because it is Exasol. Public Exasol documentation states that Exasol does not support SQL collation and that string comparison, whether explicit through `<`, `>`, `<=`, `>=` or implicit through `ORDER BY`, is by binary character value. There is therefore no default to correct and nothing for a suffix to select.
- The absence is a hard constraint rather than a preference. Exasol's parser has no `COLLATE` clause: measured against Exasol 2025.2.1, `SELECT CAST('a' AS VARCHAR(10)) COLLATE "C"` is refused with a syntax error naming `COLLATE`. A rendered suffix would therefore turn every delegated character comparison into a statement the source rejects, not merely a slower one.
- The identity holds for equality as well as ordering, and both were measured against Exasol 2025.2.1 rather than inferred. `'a' = 'A'` is false, so comparison is case sensitive; `'e' = 'é'` is false, so it is accent sensitive; `LEAST('B','a','C')` returns `B` and `GREATEST('B','a','C')` returns `a`, which is code-point order rather than a locale's collation weight. Those are the same three answers the querying Exasol gives for the same operands.
- No operand carries a suffix in any position: not an equality, `LIKE`, `IN`, `BETWEEN`, or ordering operand, not a `GROUP BY` key or its bare select-list occurrence, not a `DISTINCT` aggregate operand, not a join key, and not a `MIN` or `MAX` operand. Every operand therefore keeps whatever index and zone-map support the source has for it, which no MySQL character operand keeps and which a PostgreSQL ordering operand loses.
- This rule set derives from nothing in `vs-adapter/mysql-collation` or `vs-adapter/postgresql-collation`, and none of the three governs another. MySQL renders two collations because its default answers both of Exasol's questions differently; PostgreSQL renders one because its cluster locale answers the ordering question differently; Exasol renders none because it answers both identically.
- A feature that renders nothing still earns its own scenarios, because what it asserts is that the absence is correct. A suffix silently appearing on an Exasol operand would fail the source outright, and a delegated comparison silently disagreeing with Exasol's own is the wrong-rows failure the two sibling features exist to prevent. Both are observable only against a live Exasol source, so the scenarios below are differential rather than rendering-only.
- The character-column preconditions the sibling features depend on have no counterpart here. `vs-adapter/exasol-type-mapping` states no encoding precondition, because a source `VARCHAR(n) ASCII` column is declared `VARCHAR(n) UTF8` and the ASCII subset compares and orders identically under both. It states no collation precondition either, because Exasol declares no per-column collation to reject.
- What the mapping does refuse for a comparison reason is Exasol `CHAR`. Measured on Exasol 2025.2.1, two `CHAR(10)` values holding `ab` and `ab ` compare equal while the two `VARCHAR(10)` values the adapter would declare do not, so a delegated equality over a `CHAR` source column would match rows the querying Exasol rejects. `vs-adapter/exasol-type-mapping` owns that omission; this feature governs only the columns that mapping exposes, every one of which is `VARCHAR`, `DECIMAL`, `DOUBLE`, `DATE`, or `TIMESTAMP`.

## Scenarios

### Scenario: No rendered operand carries a collation suffix

* *GIVEN* an Exasol `VARCHAR` source column and a `pushdown` request carrying `predicate_equal`, `predicate_notequal`, `predicate_less`, `predicate_lessequal`, `predicate_between`, `predicate_in_constlist`, and `predicate_like` over it, an aggregation whose `groupBy` names it, a `function_aggregate` whose `distinct` field is true over it, `MIN` and `MAX` over it, and a join whose condition compares it
* *WHEN* the Exasol dialect renders each
* *THEN* no rendered operand MUST carry a collation suffix, in any of those positions
* *AND* the generated statement MUST NOT contain the word `COLLATE` anywhere
* *AND* a numeric or temporal operand MUST also carry no suffix, so the rule is one rule rather than one per operand type
* *AND* the dialect MUST NOT render a suffix even where the sibling dialects render one, because Exasol's parser has no `COLLATE` clause and the source would refuse the statement rather than execute it more slowly

### Scenario: Delegated comparison and ordering reproduce Exasol's own answers

* *GIVEN* an Exasol virtual table over a `VARCHAR` column and a local Exasol table holding identical values, including two differing only by case, two differing only by an accent, and the values `B`, `a`, and `C`
* *WHEN* the same equality comparison, the same ordering comparison, and the same `MIN` and `MAX` are run over each
* *THEN* the equality comparison over the virtual table MUST return exactly the rows the local query returns, so no two values the querying Exasol keeps apart are matched
* *AND* the ordering comparison MUST return exactly the rows the local query returns
* *AND* `MIN` MUST return `B` and `MAX` MUST return `a`, matching what the querying Exasol returns for the same aggregate over the same values
* *AND* `EXPLAIN VIRTUAL` MUST report a generated statement carrying no collation suffix on any of those operands, so the agreement is shown to come from the source's own semantics rather than from Exasol re-applying a withdrawn fragment

### Scenario: Delegated grouping and DISTINCT return Exasol's own groups

* *GIVEN* an Exasol virtual table and a local Exasol table holding identical `VARCHAR` values, two of which differ only by case and two only by an accent
* *WHEN* the same `GROUP BY` over that column, and the same `COUNT(DISTINCT)` over it, are run over each
* *THEN* the virtual `GROUP BY` MUST return exactly the groups the local query returns, so no two values the querying Exasol keeps apart are collapsed into one group
* *AND* the virtual `COUNT(DISTINCT)` MUST return the value the local query returns
* *AND* `EXPLAIN VIRTUAL` MUST report a `GROUP BY` key and a `DISTINCT` operand carrying no collation suffix
* *AND* the grouping key and its bare select-list occurrence MUST be the same expression, so no coordinated suffix is needed to keep them one expression

### Scenario: A suffix rendered by mistake fails the source rather than returning wrong rows

* *GIVEN* an Exasol source and a generated statement carrying a collation suffix on a character operand
* *WHEN* that statement is executed at the source
* *THEN* the source MUST refuse it with a syntax error naming the collation clause
* *AND* the refusal MUST be asserted against a live Exasol source rather than predicted, because it is what makes an accidental suffix a loud failure rather than a silent one
* *AND* the dialect's own rendering MUST NOT produce such a statement for any request the advertised capability set admits
