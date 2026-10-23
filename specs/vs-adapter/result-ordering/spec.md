# Feature: result_ordering

Keeps `ORDER BY` and `LIMIT` in Exasol rather than delegating them to the source, so an ordered query over a virtual table returns the same rows in the same order on every run.

## Background

- The adapter's `pushdown` response is a `SELECT` that calls an emitting scalar script over `DUAL`. Exasol splices that statement into the querying statement as a derived table.
- Exasol preserves the script's row emission order only while the derived table is scanned and nothing else. Measured against the test Exasol: a bare scan of 20000 and of 500000 emitted rows returned every row in emission order, while the same scan joined to one local Exasol table returned 9760 of 20000 rows out of emission order and started at the 4097th emitted row.
- Exasol documents no ordering guarantee for a script-emitted derived table. The `ORDER BY FALSE` sub-select idiom exists because a sub-select's `ORDER BY` is not an ordering barrier, which is the same boundary a pushdown response sits on.
- The cause is the transport, not the source. Every dialect's pushdown response is the same emitting-script derived table, so the reason ordering cannot be delegated is identical for every driver and no dialect may advertise an ordering capability. Unlike `JOIN_TYPE_FULL_OUTER`, which one target's SQL expresses and the other's does not, this withholding is a property of how the adapter returns rows and cannot become a per-dialect decision. Each dialect nonetheless states its own withheld set, per `vs-adapter/pushdown-dialect`, and the scenario below asserts every dialect withholds the same four.
- Because the cause is the transport, the end-to-end stability evidence does not multiply per driver. The three scenarios measuring a stable order, a tie-broken order, and an order surviving a join against a local Exasol table exercise Exasol's own retained sort over emitted rows, which no source-side behavior reaches; they stay measured against one source rather than repeated per driver. What each new dialect does add is the assertion that its own generated statement carries no ordering or limiting clause, because that text is rendered per dialect.
- `virtual-schema-common-document`, the Exasol Virtual Schema framework built on the same emitting-UDF mechanism, permits exactly `SELECTLIST_PROJECTION` and `FILTER_EXPRESSIONS` and rejects a dialect that declares more.
- `postgresql-virtual-schema` advertises `ORDER_BY_COLUMN` and `LIMIT` over an `IMPORT` transport Exasol documents as unordered. Exasol's Virtual Schema user guide states that for virtual schemas using `IMPORT`, "the outermost order of the imported result rows is therefore not guaranteed". Matching that dialect's capability list would reproduce a known defect rather than its behavior, and this release renders PostgreSQL SQL without inheriting that decision.
- `LIMIT` and `ORDER BY` are one decision, not two. `LIMIT` pushed down while Exasol retains its own sort would return an arbitrary N rows for Exasol to sort, so the delegated limit is only sound alongside the delegated order it shapes.
- Exasol executes the retained `ORDER BY` over the rows the scan returns, so ordering costs a sort in Exasol and transfers rows the source could otherwise have discarded. That cost is the deliberate price of a stable result.

## Scenarios

### Scenario: Capability set withholds ordering and limiting

* *GIVEN* a virtual schema, for each driver this release supports
* *WHEN* Exasol sends a `getCapabilities` request
* *THEN* the `capabilities` array MUST NOT contain `ORDER_BY_COLUMN`
* *AND* it MUST NOT contain `ORDER_BY_EXPRESSION`
* *AND* it MUST NOT contain `LIMIT` or `LIMIT_WITH_OFFSET`
* *AND* every dialect MUST withhold all four, because the emitting-script transport that makes them unsound is the same for every source

### Scenario: Ordered query over a virtual table returns a stable order

* *GIVEN* a MySQL table seeded with rows whose insertion order differs from the order the query asks for
* *AND* a virtual schema over that database
* *WHEN* Exasol runs the same `SELECT` with a top-level `ORDER BY` three times
* *THEN* every run MUST return the rows in the order the `ORDER BY` asks for
* *AND* the three runs MUST return byte-identical row sequences

### Scenario: Ordering is stable when the sort key holds ties

* *GIVEN* a MySQL table whose `ORDER BY` key repeats across rows
* *WHEN* Exasol runs the same ordered `SELECT` three times with a second sort key breaking the tie
* *THEN* every run MUST return the rows in the order both keys jointly define
* *AND* the three runs MUST return byte-identical row sequences

### Scenario: Ordering survives a join against a local Exasol table

* *GIVEN* a virtual table and a local Exasol table sharing a join key
* *WHEN* Exasol runs an ordered `SELECT` joining the two
* *THEN* the returned rows MUST be in the order the top-level `ORDER BY` asks for
* *AND* the query MUST NOT rely on the scan's emission order, because an operator above the scan reorders it

### Scenario: Pushed-down statement carries no ordering or limiting clause

* *GIVEN* an Exasol query whose text carries `ORDER BY` and `LIMIT` over a virtual table, for each driver this release supports
* *WHEN* `EXPLAIN VIRTUAL` reports the statement the adapter generated
* *THEN* the generated statement MUST NOT contain `ORDER BY`
* *AND* it MUST NOT contain `LIMIT`
* *AND* it MUST NOT contain `OFFSET`

### Scenario: Request carrying an order-by or limit element fails rather than dropping it

* *GIVEN* a `pushdown` request carrying an `orderBy` or a `limit` element, which Exasol sends only when the matching capability was advertised
* *WHEN* the adapter builds the response
* *THEN* the adapter MUST return an error naming the element it will not delegate
* *AND* the adapter MUST NOT return a statement that silently ignores the element
* *AND* the same MUST hold for each dialect this release supports, because a request reaching a dialect that never advertised the capability is the case this rejection exists for
