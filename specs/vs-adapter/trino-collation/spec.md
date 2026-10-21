# Feature: trino_collation

States that the Trino dialect renders no collation suffix anywhere, why Trino's own comparison already answers both of Exasol's questions, and the one deployment condition under which a delegated comparison can still disagree.

## Background

- The Trino dialect renders no collation suffix on any operand, per the framework rule `vs-adapter/pushdown-filter` states, so Trino evaluates every delegated character comparison under its own rule. Exasol asks two questions of a character value: equality asks whether two values are the same characters, and ordering asks which comes first, which Exasol answers by code point. Trino's own rule answers both the way Exasol does.
- Trino's documentation states that it compares and orders `varchar` values by Unicode code point, which for valid UTF-8 is byte order. Over the values `a`, `B`, `Z`, `z`, `A`, `ä`, `Ä`, `é`, `e` that rule yields `A B Z a e z Ä ä é` for `ORDER BY`, every ASCII upper-case value before every ASCII lower-case one and every Latin-1 accented value after both, which is what Exasol returns; a locale-aware collation would interleave them. It yields `B` for `min` and `ä` for `max` over `a`, `B`, `Z`, `ä`, matching Exasol's answers over the same values, where a locale collation would return `a` and `Z`.
- Trino has no collation mechanism to render a suffix through even if one were needed. `COLLATE` appears in no part of Trino's documented SQL grammar and in no reserved-word list it publishes, so it is expected to be usable as an ordinary column alias and every syntactic position a suffix could occupy is expected to be a parse error whose reported set of legal continuations does not include it. Trino declares no collation on a column, exposes no collation catalog, and documents no setting that changes how a `varchar` compares.
- Equality and ordering are therefore one answer rather than two, and it is the answer this release needs without a suffix. The same code-point rule makes `'a' < 'B'` false and `'Z' < 'a'` true, which is code-point order rather than collation weight, and makes a value and its case-only or accent-only variant compare unequal.
- Because no operand carries a suffix, a grouping key and its bare select-list occurrence are the same expression, and Trino's `GROUP BY` rule — every non-aggregated select-list expression must appear in `GROUP BY` — is satisfied with nothing to coordinate.
- Every operand keeps whatever the source's own execution can do with it, because no operand is wrapped.
- One condition breaks the claim, and it is a property of the Trino deployment rather than of Trino's SQL. Trino is a federation engine: a catalog backed by a remote database may push a comparison down to that database, which then applies its own collation. Trino's own connector documentation states the hazard and the default protection. The PostgreSQL connector's documentation says it does not push range predicates such as `<`, `>`, or `BETWEEN` down on character columns, "to ensure correctness of results since the remote data source may sort strings differently than Trino", while equality and inequality predicates on textual columns are pushed down. The MySQL connector pushes no predicate down on a textual column at all, because MySQL's default collations are case-insensitive. The SQL Server connector pushes down on a character column only where that column's collation is case-sensitive.
- The protection is a per-connector convention rather than an engine guarantee, and Trino documents no engine-level statement that a pushed-down comparison preserves Trino's own semantics. The PostgreSQL connector documents an opt-in property that removes the protection for string range predicates. Equality pushdown to a case-insensitive remote collation is a filed, reproducible defect in Trino's own issue tracker rather than a theoretical gap, and whether a connector pushes `MIN` or `MAX` over a character column down under a foreign collation is documented nowhere.
- adbc-vs cannot see that condition and does not check it. No catalog Trino exposes reports which comparisons its connector pushes down or which collation the remote source applies, so there is no query the metadata source could run. As for every source, the collation that applies is the source's own: a Trino catalog whose connector pushes a string comparison to a source that does not compare by code point can return rows Exasol would not, for a delegated fragment and for a locally computed one alike. The operator who configures the catalog owns that condition.
- The condition is bounded, not unlimited. A comparison Trino evaluates itself is exactly Exasol's, whatever catalog the rows came from, so the exposure is to pushdown alone. The gate measurements behind this feature's claims are taken against a Trino catalog that reaches no remote system, so they measure Trino's own comparison rather than a connector's.

## Scenarios

### Scenario: No rendered operand carries a collation suffix

* *GIVEN* a Trino `varchar` column and a `pushdown` request carrying `predicate_equal`, `predicate_less`, `predicate_lessequal`, `predicate_between`, `predicate_in_constlist`, and `predicate_like` over it
* *WHEN* the Trino dialect renders each predicate
* *THEN* no rendered operand MUST carry a collation suffix
* *AND* the same MUST hold for a `GROUP BY` key, for a `DISTINCT` aggregate operand, for a `MIN` and a `MAX` operand, and for a character equi-join key
* *AND* the generated statement MUST contain no `COLLATE` keyword in any position, because Trino has no such keyword and would reject the statement rather than ignore it

### Scenario: Character comparison reproduces Exasol's case, accent, and code-point order

* *GIVEN* a virtual table over a Trino `varchar` column and a local Exasol table holding identical values, including two differing only by case, two differing only by an accent, and the values `B`, `a`, and `C`
* *WHEN* the same equality comparison and the same ordering comparison are run over each
* *THEN* the equality comparison over the virtual table MUST return exactly the rows the local query returns, so no two values Exasol keeps apart are matched
* *AND* the ordering comparison over the virtual table MUST return exactly the rows the local query returns, so the delegated comparison follows Exasol's code-point order
* *AND* `EXPLAIN VIRTUAL` MUST report a generated statement carrying no collation suffix on either comparison

### Scenario: MIN and MAX over a character column return Exasol's values with no suffix

* *GIVEN* a Trino `varchar` column holding the values `B`, `a`, and `C`
* *WHEN* Exasol pushes down `MIN` and `MAX` over that column
* *THEN* the rendered operand MUST carry no collation suffix
* *AND* the query MUST return `B` for `MIN` and `a` for `MAX`, matching what Exasol returns for the same aggregate over the same values
* *AND* a value differing from another only by a trailing space MUST order as Exasol orders it, so no padding rule separates the two answers

### Scenario: Delegated grouping and DISTINCT over case-only variants return Exasol's groups

* *GIVEN* a Trino virtual table and a local Exasol table holding identical character values, two of which differ only by case and two only by an accent
* *WHEN* the same `GROUP BY` over that column, and the same `COUNT(DISTINCT)` over it, are run over each
* *THEN* the virtual `GROUP BY` MUST return exactly the groups the local query returns, so no two values Exasol keeps apart are collapsed into one group
* *AND* the virtual `COUNT(DISTINCT)` MUST return the value the local query returns
* *AND* the generated statement MUST carry no collation suffix on the grouping key or on the `DISTINCT` operand

### Scenario: A select-list predicate over the grouped character column is delegated

* *GIVEN* a `pushdown` request whose `aggregationType` is `group_by`, whose `groupBy` names one Trino `varchar` column, and whose select list carries a predicate reading that same column outside any aggregate
* *WHEN* the Trino dialect renders the statement
* *THEN* the dialect MUST produce a statement rather than an error
* *AND* the statement MUST execute at the source rather than being rejected by it
* *AND* the rows it returns MUST equal what Exasol returns for the same query executed over a local mirror of the same values
* *AND* this scenario MUST be verified against a live Trino source rather than by rendering alone, because what is asserted is that the source accepts an expression over its bare grouping column

### Scenario: The connector condition is left to the operator rather than checked

* *GIVEN* a virtual schema over a Trino catalog whose connector reaches a remote database
* *WHEN* the adapter runs its source precondition check
* *THEN* the check MUST succeed
* *AND* the adapter MUST NOT query Trino for which comparisons that connector pushes down or which collation the remote source applies, because no catalog Trino exposes reports either
* *AND* the adapter MUST NOT refuse the virtual schema, because refusing every catalog that reaches a remote system would refuse the federation Trino exists for
* *AND* the adapter MUST render the same statement it renders over a catalog that reaches no remote system, because nothing in the rendering depends on the condition
