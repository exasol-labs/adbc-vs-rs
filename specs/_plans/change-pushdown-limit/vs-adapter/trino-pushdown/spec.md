# Feature: trino_pushdown

States the capability set the Trino dialect advertises and the Trino SQL it renders for each delegated fragment.

<!-- DELTA:CHANGED -->
## Background

- The Trino dialect states its own advertised set, derived from no other dialect's, and `vs-adapter/pushdown-dialect` owns the all-or-nothing negotiation rule and the invariant that every advertised capability renders in the dialect that advertises it; the list is data in `crates/adbc-vs-scan/drivers/trino.toml`, from which the build generates the table the dialect returns.
- The set holds 41 entries: the PostgreSQL dialect's 41 without `FN_PRED_LIKE_ESCAPE`, which no Trino form renders for every pattern Exasol accepts, plus `AGGREGATE_GROUP_BY_TUPLE`, which caps how many columns a delegated `GROUP BY` names and needs no rendering change in any dialect.
- Every other capability the set omits is withheld by a decision that binds every dialect: `JOIN_CONDITION_ALL` by mission.md's pushdown scope, `AGGREGATE_GROUP_BY_EXPRESSION` by `vs-adapter/pushdown-aggregation-delegation`, `LITERAL_BOOL` by `vs-adapter/type-mapping`, every scalar function by `vs-adapter/pushdown-dialect`, the ordering capabilities by `vs-adapter/result-ordering`, and `LIMIT_WITH_OFFSET` by `vs-adapter/pushdown-limit`.
- Identifiers are double-quoted with an embedded double quote doubled, a table is qualified by the catalog and the schema `adapterNotes` records, a string literal renders with every single quote doubled, date and timestamp literals render typed, an exact-numeric literal renders bare and a floating-point one as `DOUBLE '<value>'`, `predicate_notequal` renders as `<>`, and every nested predicate is parenthesized.
- `vs-adapter/trino-collation` states that the Trino dialect renders no collation suffix and why, and `vs-adapter/trino-type-mapping` owns the mapped source types every rendered operand can hold.
<!-- /DELTA:CHANGED -->

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Capability set advertises the fragments this dialect renders

* *GIVEN* a virtual schema whose driver is `trino`
* *WHEN* Exasol sends a `getCapabilities` request
* *THEN* the `capabilities` array MUST contain `SELECTLIST_PROJECTION`, `SELECTLIST_EXPRESSIONS`, `FILTER_EXPRESSIONS`, `AGGREGATE_SINGLE_GROUP`, `AGGREGATE_GROUP_BY_COLUMN`, `AGGREGATE_HAVING`, `JOIN`, `JOIN_TYPE_INNER`, `JOIN_TYPE_LEFT_OUTER`, `JOIN_TYPE_RIGHT_OUTER`, `JOIN_TYPE_FULL_OUTER`, `JOIN_CONDITION_EQUI`, and `LIMIT`
* *AND* it MUST contain every `FN_PRED_` capability the PostgreSQL dialect advertises except `FN_PRED_LIKE_ESCAPE`, every `FN_AGG_` capability it advertises, and every `LITERAL_` capability it advertises
* *AND* the array MUST hold exactly the 41 entries an independently written constant states, `AGGREGATE_GROUP_BY_TUPLE` among them where no other dialect advertises it, so both the widening and every deliberate absence are pinned by an exact comparison rather than by a prefix rule that admits any name
* *AND* every entry MUST be a `MainCapability` constant, or carry the prefix `FN_PRED_`, `FN_AGG_`, or `LITERAL_`
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Capability set withholds the fragments the Trino dialect does not delegate

* *GIVEN* a virtual schema whose driver is `trino`
* *WHEN* Exasol sends a `getCapabilities` request
* *THEN* the `capabilities` array MUST NOT contain `FN_PRED_LIKE_ESCAPE`, `AGGREGATE_GROUP_BY_EXPRESSION`, `JOIN_CONDITION_ALL`, or `LITERAL_BOOL`
* *AND* the array MUST NOT contain `ORDER_BY_COLUMN` or `ORDER_BY_EXPRESSION`, which `vs-adapter/result-ordering` withholds for every dialect because the emitting-UDF transport carries no ordering guarantee whatever the source is
* *AND* the array MUST NOT contain `LIMIT_WITH_OFFSET`, which `vs-adapter/pushdown-limit` withholds for every dialect because Exasol sends an offset only after an `ORDER BY`
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: The Trino set differs from the PostgreSQL set by two named entries, and each is stated on its own

* *GIVEN* the capability set the Trino dialect advertises and the set the PostgreSQL dialect advertises
* *WHEN* the two sets are compared
* *THEN* the Trino set MUST hold `AGGREGATE_GROUP_BY_TUPLE`, which the PostgreSQL set does not, and MUST NOT hold `FN_PRED_LIKE_ESCAPE`, which the PostgreSQL set does, with no other difference in either direction
* *AND* each set MUST be stated by its own technology's declaration rather than one derived from the other by addition, removal, or reference
* *AND* every entry of the Trino set MUST have its own rendering path in the Trino dialect, proven by rendering each rather than by the comparison
* *AND* a later change to either set MUST fail this comparison rather than be absorbed, because the 40 entries they share are an outcome of two independent decisions and not a rule either dialect is held to
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: LIMIT pushdown reaches Trino rather than Exasol

* *GIVEN* a virtual schema over a Trino table holding more rows than a `LIMIT` names
* *WHEN* Exasol runs a `SELECT` with that `LIMIT` and no `ORDER BY`
* *THEN* `EXPLAIN VIRTUAL` MUST report a generated statement that ends with that `LIMIT`
* *AND* the query MUST return exactly as many rows as the `LIMIT` names
* *AND* every returned row MUST be a row of the Trino table, as the local Exasol mirror holds it
<!-- /DELTA:NEW -->
