<!-- DELTA:CHANGED -->
# Feature: result_ordering

Keeps `ORDER BY` in Exasol rather than delegating it to the source, together with every `LIMIT` and `OFFSET` that applies after it, so an ordered query over a virtual table returns the same rows in the same order on every run.
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
## Background

- The adapter's `pushdown` response is a `SELECT` that calls an emitting scalar script over `DUAL`, which Exasol splices into the querying statement as a derived table. Exasol documents no ordering guarantee for a script-emitted derived table.
- Measured against the test Exasol: a bare scan of 20000 and of 500000 emitted rows returned every row in emission order, while the same scan joined to one local Exasol table returned 9760 of 20000 rows out of emission order and started at the 4097th emitted row.
- The cause is the transport, not the source, so no dialect advertises an ordering capability, and each dialect states that withholding in its own set, per `vs-adapter/pushdown-dialect`. `postgresql-virtual-schema` advertises `ORDER_BY_COLUMN` over an `IMPORT` transport that Exasol's Virtual Schema user guide documents as unordered, and this release does not inherit that decision.
- A `LIMIT` or `OFFSET` that follows an `ORDER BY` selects rows by that order, so Exasol applies it together with the ordering. Exasol accepts an `OFFSET` only after an `ORDER BY`, so every `OFFSET` stays in Exasol. `vs-adapter/pushdown-limit` delegates only a `LIMIT` that no ordering precedes, which selects an arbitrary set of rows in any emission order.
- Exasol executes the retained `ORDER BY` over the rows the scan returns, so an ordered query costs a sort in Exasol and transfers rows the source could otherwise discard. That cost is the deliberate price of a stable result.
<!-- /DELTA:CHANGED -->

## Scenarios

<!-- DELTA:REMOVED -->
### Scenario: Capability set withholds ordering and limiting

* *GIVEN* a virtual schema, for each driver this release supports
* *WHEN* Exasol sends a `getCapabilities` request
* *THEN* the `capabilities` array MUST NOT contain `ORDER_BY_COLUMN`
* *AND* it MUST NOT contain `ORDER_BY_EXPRESSION`
* *AND* it MUST NOT contain `LIMIT` or `LIMIT_WITH_OFFSET`
* *AND* every dialect MUST withhold all four, because the emitting-script transport that makes them unsound is the same for every source
<!-- /DELTA:REMOVED -->

<!-- DELTA:NEW -->
### Scenario: Capability set withholds ordering

* *GIVEN* a virtual schema, for each driver this release supports
* *WHEN* Exasol sends a `getCapabilities` request
* *THEN* the `capabilities` array MUST NOT contain `ORDER_BY_COLUMN`
* *AND* it MUST NOT contain `ORDER_BY_EXPRESSION`
* *AND* every dialect MUST withhold both, because the emitting-script transport that makes ordering unsound is the same for every source
<!-- /DELTA:NEW -->

<!-- DELTA:REMOVED -->
### Scenario: Pushed-down statement carries no ordering or limiting clause

* *GIVEN* an Exasol query whose text carries `ORDER BY` and `LIMIT` over a virtual table, for each driver this release supports
* *WHEN* `EXPLAIN VIRTUAL` reports the statement the adapter generated
* *THEN* the generated statement MUST NOT contain `ORDER BY`
* *AND* it MUST NOT contain `LIMIT`
* *AND* it MUST NOT contain `OFFSET`
<!-- /DELTA:REMOVED -->

<!-- DELTA:NEW -->
### Scenario: An ordered query keeps its LIMIT and OFFSET in Exasol

* *GIVEN* a query over a virtual table that carries `ORDER BY` and `LIMIT`, and a second one that carries `ORDER BY` and `LIMIT ... OFFSET`, for each driver this release supports
* *WHEN* `EXPLAIN VIRTUAL` reports the statement the adapter generated for each query
* *THEN* the generated statement MUST NOT contain `ORDER BY`
* *AND* it MUST NOT contain `LIMIT`
* *AND* it MUST NOT contain `OFFSET`
* *AND* each query MUST return the same rows in the same order as the same query over the local Exasol mirror, because a limit applied at the source before Exasol's sort would select rows by emission order rather than by the `ORDER BY`
<!-- /DELTA:NEW -->

<!-- DELTA:REMOVED -->
### Scenario: Request carrying an order-by or limit element fails rather than dropping it

* *GIVEN* a `pushdown` request carrying an `orderBy` or a `limit` element, which Exasol sends only when the matching capability was advertised
* *WHEN* the adapter builds the response
* *THEN* the adapter MUST return an error naming the element it will not delegate
* *AND* the adapter MUST NOT return a statement that silently ignores the element
* *AND* the same MUST hold for each dialect this release supports, because a request reaching a dialect that never advertised the capability is the case this rejection exists for
<!-- /DELTA:REMOVED -->

<!-- DELTA:NEW -->
### Scenario: Request carrying an order-by element fails rather than dropping it

* *GIVEN* a `pushdown` request carrying an `orderBy` element, which Exasol sends only when an ordering capability was advertised
* *WHEN* the adapter builds the response
* *THEN* the adapter MUST return an error naming the `orderBy` element and the withheld ordering capabilities
* *AND* the adapter MUST NOT return a statement that silently ignores the element
* *AND* a request carrying `orderBy` together with `limit` MUST fail the same way rather than render the limit without the ordering it follows
* *AND* the same MUST hold for each dialect this release supports, because a request reaching a dialect that never advertised the capability is the case this rejection exists for
<!-- /DELTA:NEW -->
