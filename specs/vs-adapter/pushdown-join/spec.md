# Feature: pushdown_join

Delegates an equi-join between two virtual tables of one virtual schema to the source, so the source joins the rows instead of shipping both tables to Exasol.

## Background

- This feature owns what a join fragment is, for every dialect: the `join_type` discriminator, positional aliasing and column qualification, the unresolvable-column error, outer-join null semantics, and composition with a delegated filter and aggregation. Which join types a dialect advertises is that dialect's own fact and lives in `vs-adapter/mysql-pushdown` and `vs-adapter/postgresql-pushdown`. Whether a character join key carries a collation suffix is also per dialect and lives in `vs-adapter/mysql-collation` and `vs-adapter/postgresql-collation`.
- Exasol pushes a join down only when the adapter advertises `JOIN` together with at least one join type and one join condition capability. It then delegates the whole join and never re-joins the rows itself.
- The join element spells its discriminator `join_type` in snake case, unlike every sibling protocol field, and its value is `inner`, `left_outer`, `right_outer`, or `full_outer`. Its `left` and `right` are each a table element or a nested join element, so a join tree nests to any depth.
- Each of the four values renders one standard keyword — `INNER JOIN`, `LEFT OUTER JOIN`, `RIGHT OUTER JOIN`, `FULL OUTER JOIN` — and the rendering is the same in every target that implements the keyword. A dialect that never advertised a join type fails on a request carrying it rather than substituting another, which is `vs-adapter/pushdown-dialect`'s rule for an unrenderable fragment.
- `JOIN_TYPE_FULL_OUTER` is the one capability the dialects disagree on, and the disagreement is a property of the targets rather than a scope choice: MySQL implements no `FULL OUTER JOIN` and `vs-adapter/mysql-pushdown` therefore withholds it, while PostgreSQL implements it directly and `vs-adapter/postgresql-pushdown` advertises it. Emulating one as a union of two outer joins is what mission.md places out of scope.
- `JOIN_CONDITION_ALL` is advertised by no dialect. mission.md scopes pushdown to equi and outer joins and places general non-equi joins out of scope.
- Both joined tables live in the source namespace `adapterNotes` records, because Exasol pushes a join down only when every table belongs to one virtual schema. What that namespace is per source — the MySQL database, the PostgreSQL `public` schema — is `vs-adapter/create-virtual-schema`'s.
- Every table in a rendered join carries an alias derived from the table's position in the `from` tree, in every dialect. The request's own `alias` is never rendered — it is the namespace a column node's `tableAlias` or `tableName` resolves against, and rendering it would put a name the dialect does not control into the namespace it must keep unambiguous: measured on MySQL 8.4, a request alias colliding with a positional one is rejected with error 1066, and a server with `lower_case_table_names=1` collapses two aliases differing only in case, which Exasol sends for a pair of quoted aliases. Every target rejects a duplicate alias, so the collision half of that argument binds every dialect while the case-collapsing half is MySQL's alone. A single-table statement aliases and qualifies nothing, because one table's column reference cannot be ambiguous.
- Every character join key is Exasol `VARCHAR`, and Exasol pads no `VARCHAR`. `vs-adapter/type-mapping` maps no source column to a blank-padded Exasol type, so an equi-join key is never blank-padded and no padding mismatch can arise between the two sides of a condition.
- An outer join returns nulls for columns of the unmatched side, including columns the source declares `NOT NULL`, so the scan coerces a null into any declared column type rather than treating the source's nullability as a guarantee. A full outer join returns nulls for either side, which is the same coercion rather than a new one.

## Scenarios

### Scenario: Join capabilities cover inner and one-sided outer joins only

* *GIVEN* a virtual schema backed by MySQL
* *WHEN* Exasol sends a `getCapabilities` request
* *THEN* the `capabilities` array MUST contain `JOIN`, `JOIN_TYPE_INNER`, `JOIN_TYPE_LEFT_OUTER`, `JOIN_TYPE_RIGHT_OUTER`, and `JOIN_CONDITION_EQUI`
* *AND* it MUST NOT contain `JOIN_TYPE_FULL_OUTER`, because MySQL implements no `FULL OUTER JOIN`
* *AND* it MUST NOT contain `JOIN_CONDITION_ALL`, because a non-equi join is out of scope

### Scenario: Inner join of two virtual tables renders one MySQL statement

* *GIVEN* virtual tables `orders` and `customers` sharing a key column
* *WHEN* Exasol pushes down an inner join over that key
* *THEN* the generated MySQL statement MUST join both tables in a single `SELECT`
* *AND* both tables MUST be qualified by the database `adapterNotes` records
* *AND* the join condition MUST render as an equality between the two qualified columns
* *AND* the statement MUST NOT read either table in a separate statement

### Scenario: Each join type renders its MySQL counterpart

* *GIVEN* a `pushdown` request whose `from` element carries `join_type`
* *WHEN* the dialect renders the join
* *THEN* `inner` MUST render as `INNER JOIN`
* *AND* `left_outer` MUST render as `LEFT OUTER JOIN`
* *AND* `right_outer` MUST render as `RIGHT OUTER JOIN`
* *AND* the operand order MUST match the request's `left` and `right`, because reversing an outer join changes its result

### Scenario: Unsupported join type fails with a named error

* *GIVEN* a `pushdown` request whose `join_type` is `full_outer`, or a value the dialect does not render
* *WHEN* the adapter builds the response
* *THEN* the adapter MUST return an error
* *AND* the error message MUST name the join type it received
* *AND* the adapter MUST NOT substitute a different join type

### Scenario: Every joined table is aliased and every column is qualified

* *GIVEN* a join whose two tables both declare a column named `id`
* *WHEN* the dialect renders the statement
* *THEN* each of the join's tables in the `FROM` clause MUST carry an alias
* *AND* every column reference MUST be qualified by the alias of the table it belongs to
* *AND* the `EMITS` clause MUST declare two distinct positional column names for the two `id` columns

### Scenario: Column that matches no table in the from tree fails with a named error

* *GIVEN* a join whose select list names a column whose `tableName` and `tableAlias` match no table in the `from` tree
* *WHEN* the adapter builds the response
* *THEN* the adapter MUST return an error
* *AND* the error message MUST name the column and the table reference it could not resolve
* *AND* the adapter MUST NOT emit an unqualified column reference, because MySQL would then resolve it against the wrong table or reject the statement

### Scenario: Outer join emits nulls for the unmatched side

* *GIVEN* a left outer join whose right table has no matching row for one left row
* *WHEN* the query runs through the virtual schema
* *THEN* the unmatched row MUST come back with nulls for every right-table column
* *AND* the scan MUST NOT fail on a null in a column whose MySQL declaration is `NOT NULL`
* *AND* the returned rows MUST equal what Exasol returns for the same join executed locally over the same data

### Scenario: Join composes with a delegated filter and aggregation

* *GIVEN* a join carrying a `filter` and a `groupBy` in the same `pushdown` request
* *WHEN* the dialect renders the statement
* *THEN* the generated statement MUST carry the join, the `WHERE` clause, and the `GROUP BY` clause together
* *AND* the clauses MUST appear in MySQL's required order: `FROM`, `WHERE`, `GROUP BY`
* *AND* the returned rows MUST equal what Exasol returns for the same query executed locally over the same data

### Scenario: Nested join renders as a nested MySQL join

* *GIVEN* a `from` element whose `left` is itself a join element
* *WHEN* the dialect renders the statement
* *THEN* the generated statement MUST render the nested join before joining the outer table
* *AND* every table across both levels MUST carry a distinct alias
