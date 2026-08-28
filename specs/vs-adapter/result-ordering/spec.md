# Feature: result_ordering

Keeps `ORDER BY` and `LIMIT` in Exasol rather than delegating them to the source, so an ordered query over a virtual table returns the same rows in the same order on every run.

## Background

- The adapter's `pushdown` response is a `SELECT` that calls an emitting scalar script over `DUAL`. Exasol splices that statement into the querying statement as a derived table.
- Exasol preserves the script's row emission order only while the derived table is scanned and nothing else. Measured against Exasol 2025.2.1: a bare scan of 20000 and of 500000 emitted rows returned every row in emission order, while the same scan joined to one local Exasol table returned 9760 of 20000 rows out of emission order and started at the 4097th emitted row.
- Exasol documents no ordering guarantee for a script-emitted derived table. The `ORDER BY FALSE` sub-select idiom exists because a sub-select's `ORDER BY` is not an ordering barrier, which is the same boundary a pushdown response sits on.
- `virtual-schema-common-document`, the Exasol Virtual Schema framework built on the same emitting-UDF mechanism, permits exactly `SELECTLIST_PROJECTION` and `FILTER_EXPRESSIONS` and rejects a dialect that declares more.
- `postgresql-virtual-schema` advertises `ORDER_BY_COLUMN` and `LIMIT` over an `IMPORT` transport Exasol documents as unordered. Exasol's Virtual Schema user guide states that for virtual schemas using `IMPORT`, "the outermost order of the imported result rows is therefore not guaranteed". Matching that dialect's capability list would reproduce a known defect rather than its behavior.
- `LIMIT` and `ORDER BY` are one decision, not two. `LIMIT` pushed down while Exasol retains its own sort would return an arbitrary N rows for Exasol to sort, so the delegated limit is only sound alongside the delegated order it shapes.
- Exasol executes the retained `ORDER BY` over the rows the scan returns, so ordering costs a sort in Exasol and transfers rows the source could otherwise have discarded. That cost is the deliberate price of a stable result.

## Scenarios

### Scenario: Capability set withholds ordering and limiting

* *GIVEN* a virtual schema backed by MySQL
* *WHEN* Exasol sends a `getCapabilities` request
* *THEN* the `capabilities` array MUST NOT contain `ORDER_BY_COLUMN`
* *AND* it MUST NOT contain `ORDER_BY_EXPRESSION`
* *AND* it MUST NOT contain `LIMIT`
* *AND* it MUST NOT contain `LIMIT_WITH_OFFSET`

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

* *GIVEN* an Exasol query whose text carries `ORDER BY` and `LIMIT` over a virtual table
* *WHEN* `EXPLAIN VIRTUAL` reports the statement the adapter generated
* *THEN* the generated MySQL statement MUST NOT contain `ORDER BY`
* *AND* it MUST NOT contain `LIMIT`
* *AND* it MUST NOT contain `OFFSET`

### Scenario: Request carrying an order-by or limit element fails rather than dropping it

* *GIVEN* a `pushdown` request carrying an `orderBy` or a `limit` element, which Exasol sends only when the matching capability was advertised
* *WHEN* the adapter builds the response
* *THEN* the adapter MUST return an error naming the element it will not delegate
* *AND* the adapter MUST NOT return a statement that silently ignores the element
