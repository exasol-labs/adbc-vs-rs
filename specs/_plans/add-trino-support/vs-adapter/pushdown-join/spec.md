# Feature: pushdown_join

Delegates an equi-join between two virtual tables of one virtual schema to the source, so the source joins the rows instead of shipping both tables to Exasol.

<!-- DELTA:CHANGED -->
## Background

- This feature owns what a join fragment is, for every dialect: the `join_type` discriminator, positional aliasing and column qualification, the unresolvable-column error, outer-join null semantics, and composition with a delegated filter and aggregation. Which join types a dialect advertises is that dialect's own fact and lives in its `<technology>-pushdown` feature. Whether a character join key carries a collation suffix is also per dialect and lives in its `<technology>-collation` feature.
- Exasol pushes a join down only when the adapter advertises `JOIN` together with at least one join type and one join condition capability. It then delegates the whole join and never re-joins the rows itself.
- The join element spells its discriminator `join_type` in snake case, unlike every sibling protocol field, and its value is `inner`, `left_outer`, `right_outer`, or `full_outer`. Its `left` and `right` are each a table element or a nested join element, so a join tree nests to any depth.
- Each of the four values renders one standard keyword — `INNER JOIN`, `LEFT OUTER JOIN`, `RIGHT OUTER JOIN`, `FULL OUTER JOIN` — and the rendering is the same in every target that implements the keyword. A dialect that never advertised a join type fails on a request carrying it rather than substituting another, which is `vs-adapter/pushdown-dialect`'s rule for an unrenderable fragment.
- `JOIN_TYPE_FULL_OUTER` is the one join capability the dialects disagree on, and the disagreement is a property of the targets rather than a scope choice: MySQL implements no `FULL OUTER JOIN` and `vs-adapter/mysql-pushdown` therefore withholds it, while PostgreSQL and Trino implement it directly and their own features advertise it. Emulating one as a union of two outer joins is what mission.md places out of scope.
- What a target additionally requires of a full join's condition is that target's own fact and belongs to its own feature. PostgreSQL refuses a full join whose condition is neither merge-joinable nor hash-joinable, and Trino refuses none. Neither restriction binds this release, because the only condition capability any dialect advertises admits an equality alone.
- `JOIN_CONDITION_ALL` is advertised by no dialect. mission.md scopes pushdown to equi and outer joins and places general non-equi joins out of scope, so the withholding is this release's scope rather than any target's limit, and a target that would execute such a condition is withheld from it for the same reason as one that would not.
- Both joined tables live in the source namespace `adapterNotes` records, because Exasol pushes a join down only when every table belongs to one virtual schema. What that namespace is per source, and how many name segments it carries, is `vs-adapter/create-virtual-schema`'s framework shape and that technology's own feature's value.
- Every table in a rendered join carries an alias derived from the table's position in the `from` tree, in every dialect. The request's own `alias` is never rendered — it is the namespace a column node's `tableAlias` or `tableName` resolves against, and rendering it would put a name the dialect does not control into the namespace it must keep unambiguous: measured on MySQL 8.4, a request alias colliding with a positional one is rejected with error 1066, and a server with `lower_case_table_names=1` collapses two aliases differing only in case, which Exasol sends for a pair of quoted aliases. Every target rejects a duplicate alias, so the collision half of that argument binds every dialect, while the case-collapsing half binds MySQL and Trino, which resolves every identifier case-insensitively. A single-table statement aliases and qualifies nothing, because one table's column reference cannot be ambiguous.
- Every character join key is Exasol `VARCHAR`, and Exasol pads no `VARCHAR`. `vs-adapter/type-mapping` maps no source column to a blank-padded Exasol type, so an equi-join key is never blank-padded and no padding mismatch can arise between the two sides of a condition.
- An outer join returns nulls for columns of the unmatched side, including columns the source declares `NOT NULL`, so the scan coerces a null into any declared column type rather than treating the source's nullability as a guarantee. A full outer join returns nulls for either side, which is the same coercion rather than a new one.
<!-- /DELTA:CHANGED -->

## Scenarios

### Scenario: Each join type renders its counterpart

* *GIVEN* a `pushdown` request whose `from` element carries `join_type`
* *WHEN* each dialect renders the join
* *THEN* `inner` MUST render as `INNER JOIN`
* *AND* `left_outer` MUST render as `LEFT OUTER JOIN`, and `right_outer` as `RIGHT OUTER JOIN`
* *AND* `full_outer` MUST render as `FULL OUTER JOIN` in every dialect that advertises `JOIN_TYPE_FULL_OUTER`
* *AND* the operand order MUST match the request's `left` and `right`, because reversing an outer join changes its result
