# Feature: trino_pushdown

States the capability set the Trino dialect advertises and the Trino SQL it renders for each delegated fragment.

<!-- DELTA:CHANGED -->
## Background

- The Trino dialect states its own advertised set, derived from no other dialect's, and `vs-adapter/pushdown-dialect` owns the all-or-nothing negotiation rule and the invariant that every advertised capability renders in the dialect that advertises it; the list is data in `crates/adbc-vs-scan/drivers/trino.toml`, from which the build generates the table the dialect returns.
- The set holds 52 entries: the PostgreSQL dialect's 52 without `FN_PRED_LIKE_ESCAPE`, which no Trino form renders for every pattern Exasol accepts, plus `AGGREGATE_GROUP_BY_TUPLE`, which caps how many columns a delegated `GROUP BY` names and needs no rendering change in any dialect.
- Every other capability the set omits is withheld by a decision that binds every dialect: `JOIN_CONDITION_ALL` by mission.md's pushdown scope, `LITERAL_BOOL` by `vs-adapter/type-mapping`, every scalar function outside its list by `vs-adapter/pushdown-scalar-function`, and the ordering and limiting capabilities by `vs-adapter/result-ordering`.
- Identifiers are double-quoted with an embedded double quote doubled, a table is qualified by the catalog and the schema `adapterNotes` records, a string literal renders with every single quote doubled, date and timestamp literals render typed, an exact-numeric literal renders bare and a floating-point one as `DOUBLE '<value>'`, `predicate_notequal` renders as `<>`, and every nested predicate is parenthesized.
- `vs-adapter/trino-collation` states that the Trino dialect renders no collation suffix and why, and `vs-adapter/trino-type-mapping` owns the mapped source types every rendered operand can hold.
<!-- /DELTA:CHANGED -->

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Capability set advertises the fragments this dialect renders

* *GIVEN* a virtual schema whose driver is `trino`
* *WHEN* Exasol sends a `getCapabilities` request
* *THEN* the `capabilities` array MUST contain `SELECTLIST_PROJECTION`, `SELECTLIST_EXPRESSIONS`, `FILTER_EXPRESSIONS`, `AGGREGATE_SINGLE_GROUP`, `AGGREGATE_GROUP_BY_COLUMN`, `AGGREGATE_GROUP_BY_EXPRESSION`, `AGGREGATE_HAVING`, `JOIN`, `JOIN_TYPE_INNER`, `JOIN_TYPE_LEFT_OUTER`, `JOIN_TYPE_RIGHT_OUTER`, `JOIN_TYPE_FULL_OUTER`, and `JOIN_CONDITION_EQUI`
* *AND* it MUST contain every `FN_PRED_` capability the PostgreSQL dialect advertises except `FN_PRED_LIKE_ESCAPE`, every `FN_AGG_` capability it advertises, every `LITERAL_` capability it advertises, and every scalar-function capability it advertises
* *AND* the array MUST hold exactly the 52 entries an independently written constant states, `AGGREGATE_GROUP_BY_TUPLE` among them where no other dialect advertises it, so both the widening and every deliberate absence are pinned by an exact comparison rather than by a prefix rule that admits any name
* *AND* every entry MUST be a `MainCapability` constant, or carry the prefix `FN_PRED_`, `FN_AGG_`, `LITERAL_`, or `FN_`, the last for exactly the scalar functions `vs-adapter/pushdown-scalar-function` names for Trino
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Capability set withholds the fragments the Trino dialect does not delegate

* *GIVEN* a virtual schema whose driver is `trino`
* *WHEN* Exasol sends a `getCapabilities` request
* *THEN* the `capabilities` array MUST NOT contain `FN_PRED_LIKE_ESCAPE`, `JOIN_CONDITION_ALL`, `LITERAL_BOOL`, or `FN_CAST`
* *AND* the array MUST NOT contain `ORDER_BY_COLUMN`, `ORDER_BY_EXPRESSION`, `LIMIT`, or `LIMIT_WITH_OFFSET`, which `vs-adapter/result-ordering` withholds for every dialect because the emitting-UDF transport carries no ordering guarantee whatever the source is
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: Scalar functions render in Trino's forms

* *GIVEN* `pushdown` requests carrying each scalar function `vs-adapter/pushdown-scalar-function` names, over operands of each declared type Exasol admits for it
* *WHEN* the Trino dialect renders each request
* *THEN* `YEAR`, `MONTH`, and `DAY` MUST render as Trino's `year`, `month`, and `day`, and `LENGTH` as `length`, which counts characters
* *AND* a two-argument `TRIM` MUST render as `trim` over the string and the character set, which removes a set of characters as Exasol does
* *AND* an exact-numeric operand of `+` or `-` MUST render cast to `DECIMAL(38,s)`, so a `bigint` result beyond its range is returned rather than refused with Trino's overflow error, and a factor of `*` cast to its own `DECIMAL(p,s)`, because Trino types `DECIMAL(38,4) * DECIMAL(38,4)` as `DECIMAL(38,6)` and drops the scale Exasol keeps
* *AND* a number added to or subtracted from a `DATE` or `TIMESTAMP` operand MUST render through `date_add` with the unit `day` and the number rounded half away from zero, as Exasol rounds it, `DATE` minus `DATE` through `date_diff('day', ...)`, and a `TIMESTAMP` difference as Trino's `interval day to second` subtraction, and as `to_milliseconds(<interval>)` divided by the decimal 1000 where a select-list entry declared `INTERVAL DAY TO SECOND` needs its seconds count
<!-- /DELTA:NEW -->
