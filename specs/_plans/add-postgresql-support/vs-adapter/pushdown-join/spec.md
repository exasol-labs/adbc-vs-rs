# Feature: pushdown_join

Delegates an equi-join between two virtual tables of one virtual schema to the source, so the source joins the rows instead of shipping both tables to Exasol.

## Background

<!-- DELTA:CHANGED -->
- Exasol pushes a join down only when the adapter advertises `JOIN` together with at least one join type and one join condition capability. It then delegates the whole join and never re-joins the rows itself.
- The join element spells its discriminator `join_type` in snake case, unlike every sibling protocol field, and its value is `inner`, `left_outer`, `right_outer`, or `full_outer`. Its `left` and `right` are each a table element or a nested join element.
- `JOIN_TYPE_INNER`, `JOIN_TYPE_LEFT_OUTER`, `JOIN_TYPE_RIGHT_OUTER`, and `JOIN_CONDITION_EQUI` are advertised by both dialects, and each renders the same standard keyword in both targets.
- `JOIN_TYPE_FULL_OUTER` is the one capability the two dialects disagree on, and the disagreement is a property of the targets rather than a scope choice. MySQL implements no `FULL OUTER JOIN`: its own worklog for the feature is unimplemented, and emulating one requires rewriting it as a union of two outer joins, which mission.md places out of scope. PostgreSQL implements `FULL OUTER JOIN` directly, so the PostgreSQL dialect advertises the capability and renders the keyword, and the MySQL dialect withholds it. This is the capability that makes the per-dialect advertised set `vs-adapter/pushdown-dialect` requires observable rather than merely architectural.
- Advertising `JOIN_TYPE_FULL_OUTER` is gated on measurement rather than on PostgreSQL implementing the keyword, because PostgreSQL restricts which join conditions a full join accepts: it has historically rejected a full join whose condition is neither merge-joinable nor hash-joinable, with an error naming that restriction. Only `JOIN_CONDITION_EQUI` is advertised, and an equality over one common type is both merge- and hash-joinable, so the restriction is expected not to bind. It is measured before the capability is advertised, because an advertised capability whose fragment the source refuses converts a query Exasol would have answered into a hard failure — `vs-adapter/pushdown-dialect` records why a capability is withheld whole rather than refused at render time.
- `JOIN_CONDITION_ALL` is not advertised by either dialect. mission.md scopes pushdown to equi and outer joins and places general non-equi joins out of scope.
- Both joined tables live in the source namespace `adapterNotes` records, because Exasol pushes a join down only when every table belongs to one virtual schema. That namespace is the MySQL database or the PostgreSQL `public` schema, per `vs-adapter/create-virtual-schema`.
- Every table in a rendered join carries an alias derived from the table's position in the `from` tree, in both dialects. The request's own `alias` is never rendered — it is the namespace a column node's `tableAlias` or `tableName` resolves against, and rendering it would put a name the dialect does not control into the namespace it must keep unambiguous: measured on MySQL 8.4, a request alias colliding with a positional one is rejected with error 1066, and a server with `lower_case_table_names=1` collapses two aliases differing only in case, which Exasol sends for a pair of quoted aliases. PostgreSQL rejects a duplicate alias too, and its quoted aliases are always case-distinct, so the case-collapsing half of that argument does not apply to it while the collision half does. A single-table statement aliases and qualifies nothing, because one table's column reference cannot be ambiguous.
- Every character join key is Exasol `VARCHAR`, and Exasol pads no `VARCHAR`. `vs-adapter/type-mapping` maps MySQL `CHAR` to Exasol `VARCHAR` and leaves PostgreSQL `character` unmapped, so an equi-join key is never a blank-padded type from either source and no padding mismatch can arise between the two sides of a condition.
- A join condition asks the equality question, so a character join key carries whatever collation its dialect renders for equality: `COLLATE utf8mb4_0900_as_cs` for MySQL, per `vs-adapter/pushdown-filter`, and no suffix for PostgreSQL, whose deterministic collations already compare byte-identical values equal. A join over character keys therefore returns Exasol's rows in both dialects for different reasons, and the differential test proves each.
- An outer join returns nulls for columns of the unmatched side, including columns the source declares `NOT NULL`, so the scan coerces a null into any declared column type rather than treating the source's nullability as a guarantee. A full outer join returns nulls for either side, which is the same coercion rather than a new one.
<!-- /DELTA:CHANGED -->

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Join capabilities cover inner and one-sided outer joins only

* *GIVEN* a virtual schema whose driver is `mysql`
* *WHEN* Exasol sends a `getCapabilities` request
* *THEN* the `capabilities` array MUST contain `JOIN`, `JOIN_TYPE_INNER`, `JOIN_TYPE_LEFT_OUTER`, `JOIN_TYPE_RIGHT_OUTER`, and `JOIN_CONDITION_EQUI`
* *AND* it MUST NOT contain `JOIN_TYPE_FULL_OUTER`, because MySQL implements no `FULL OUTER JOIN`
* *AND* it MUST NOT contain `JOIN_CONDITION_ALL`, because a non-equi join is out of scope
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: PostgreSQL join capabilities add the full outer join

* *GIVEN* a virtual schema whose driver is `postgresql`
* *WHEN* Exasol sends a `getCapabilities` request
* *THEN* the `capabilities` array MUST contain `JOIN`, `JOIN_TYPE_INNER`, `JOIN_TYPE_LEFT_OUTER`, `JOIN_TYPE_RIGHT_OUTER`, `JOIN_TYPE_FULL_OUTER`, and `JOIN_CONDITION_EQUI`
* *AND* it MUST NOT contain `JOIN_CONDITION_ALL`
* *AND* `JOIN_TYPE_FULL_OUTER` MUST be the only capability present here and absent from the MySQL dialect's set
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: PostgreSQL renders and executes a delegated full outer join

* *GIVEN* PostgreSQL virtual tables `orders` and `customers` sharing a key column, each holding a row the other does not match, and local Exasol mirrors of both
* *WHEN* Exasol pushes down a full outer join over that key
* *THEN* `EXPLAIN VIRTUAL` MUST report a generated statement rendering `FULL OUTER JOIN`, so the join reaches PostgreSQL rather than Exasol
* *AND* the query MUST return the unmatched row of each side, with nulls for the other side's columns
* *AND* the returned rows MUST equal what Exasol returns for the same full outer join executed locally over the mirrors
* *AND* PostgreSQL MUST NOT reject the statement over its restriction on full-join conditions, because the delegated condition is an equality
<!-- /DELTA:NEW -->

<!-- DELTA:REMOVED -->
### Scenario: Each join type renders its MySQL counterpart

* *GIVEN* a `pushdown` request whose `from` element carries `join_type`
* *WHEN* the dialect renders the join
* *THEN* `inner` MUST render as `INNER JOIN`
* *AND* `left_outer` MUST render as `LEFT OUTER JOIN`
* *AND* `right_outer` MUST render as `RIGHT OUTER JOIN`
* *AND* the operand order MUST match the request's `left` and `right`, because reversing an outer join changes its result
<!-- /DELTA:REMOVED -->

<!-- DELTA:NEW -->
### Scenario: Each join type renders its counterpart

* *GIVEN* a `pushdown` request whose `from` element carries `join_type`
* *WHEN* each dialect renders the join
* *THEN* `inner` MUST render as `INNER JOIN`
* *AND* `left_outer` MUST render as `LEFT OUTER JOIN`, and `right_outer` as `RIGHT OUTER JOIN`
* *AND* `full_outer` MUST render as `FULL OUTER JOIN` in the PostgreSQL dialect
* *AND* the operand order MUST match the request's `left` and `right`, because reversing an outer join changes its result
<!-- /DELTA:NEW -->

<!-- DELTA:CHANGED -->
### Scenario: Unsupported join type fails with a named error

* *GIVEN* a `pushdown` request whose `join_type` is a value the resolved dialect does not render
* *WHEN* the adapter builds the response
* *THEN* the adapter MUST return an error
* *AND* the error message MUST name the join type it received
* *AND* the adapter MUST NOT substitute a different join type
* *AND* a `join_type` of `full_outer` MUST fail this way in the MySQL dialect, which never advertised it, and MUST render in the PostgreSQL dialect, which does
<!-- /DELTA:CHANGED -->

<!-- DELTA:REMOVED -->
### Scenario: Inner join of two virtual tables renders one MySQL statement

* *GIVEN* virtual tables `orders` and `customers` sharing a key column
* *WHEN* Exasol pushes down an inner join over that key
* *THEN* the generated MySQL statement MUST join both tables in a single `SELECT`
* *AND* both tables MUST be qualified by the database `adapterNotes` records
* *AND* the join condition MUST render as an equality between the two qualified columns
* *AND* the statement MUST NOT read either table in a separate statement
<!-- /DELTA:REMOVED -->

<!-- DELTA:NEW -->
### Scenario: Inner join of two virtual tables renders one statement

* *GIVEN* virtual tables `orders` and `customers` sharing a key column, for each driver this release supports
* *WHEN* Exasol pushes down an inner join over that key
* *THEN* the generated statement MUST join both tables in a single `SELECT`, reading neither in a separate statement
* *AND* both tables MUST be qualified by the source namespace `adapterNotes` records
* *AND* the join condition MUST render as an equality between the two qualified columns
* *AND* a character join key MUST carry its dialect's equality collation, which is `COLLATE utf8mb4_0900_as_cs` for MySQL and no suffix for PostgreSQL
<!-- /DELTA:NEW -->

<!-- DELTA:CHANGED -->
### Scenario: Outer join emits nulls for the unmatched side

* *GIVEN* a left outer join whose right table has no matching row for one left row, for each driver this release supports
* *WHEN* the query runs through the virtual schema
* *THEN* the unmatched row MUST come back with nulls for every right-table column
* *AND* the scan MUST NOT fail on a null in a column the source declares `NOT NULL`
* *AND* the returned rows MUST equal what Exasol returns for the same join executed locally over the same data
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Join composes with a delegated filter and aggregation

* *GIVEN* a join carrying a `filter` and a `groupBy` in the same `pushdown` request, for each driver this release supports
* *WHEN* the dialect renders the statement
* *THEN* the generated statement MUST carry the join, the `WHERE` clause, and the `GROUP BY` clause together
* *AND* the clauses MUST appear in the target's required order: `FROM`, `WHERE`, `GROUP BY`
* *AND* the returned rows MUST equal what Exasol returns for the same query executed locally over the same data
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Every joined table is aliased and every column is qualified

* *GIVEN* a join whose two tables both declare a column named `id`, for each driver this release supports
* *WHEN* the dialect renders the statement
* *THEN* each of the join's tables in the `FROM` clause MUST carry an alias
* *AND* every column reference MUST be qualified by the alias of the table it belongs to
* *AND* every alias MUST be quoted with the dialect's own quoting character
* *AND* the `EMITS` clause MUST declare two distinct positional column names for the two `id` columns
<!-- /DELTA:CHANGED -->
