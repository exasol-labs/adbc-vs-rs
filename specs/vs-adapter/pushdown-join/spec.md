# Feature: pushdown_join

Delegates an equi-join between two virtual tables of one virtual schema to the source, so the source joins the rows instead of shipping both tables to Exasol.

## Background

- This feature owns the join fragment for every dialect: the `join_type` discriminator, positional aliasing and column qualification, the unresolvable-column error, outer-join null semantics, and composition with a delegated filter and aggregation.
- Each dialect's advertised join types, and what its target requires of a full join's condition, live in its `<technology>-pushdown` feature, and `vs-adapter/pushdown-dialect` owns the rule for a fragment a dialect cannot render.
- Each `<technology>-collation` feature states what its source's default collation does to a character join key.
- A join request carries a `from` element whose `join_type` is `inner`, `left_outer`, `right_outer`, or `full_outer`, and whose `left` and `right` are each a table element or a nested join element, so a join tree nests to any depth.
- `vs-adapter/create-virtual-schema` owns the shape of the source namespace `adapterNotes` records, and each technology's own feature states its value.
- `vs-adapter/pushdown-join-aliasing` owns positional aliasing, column qualification, and the unresolvable-column error.

## Scenarios

### Scenario: Unsupported join type fails with a named error

* *GIVEN* a `pushdown` request whose `join_type` is a value the resolved dialect does not render
* *WHEN* the adapter builds the response
* *THEN* the adapter MUST return an error
* *AND* the error message MUST name the join type it received
* *AND* the adapter MUST NOT substitute a different join type
* *AND* a `join_type` of `full_outer` MUST fail this way in a dialect that never advertised it and MUST render in one that did

### Scenario: Outer join emits nulls for the unmatched side

* *GIVEN* a left outer join whose right table has no matching row for one left row, for each driver this release supports
* *WHEN* the query runs through the virtual schema
* *THEN* the unmatched row MUST come back with nulls for every right-table column
* *AND* the scan MUST NOT fail on a null in a column the source declares `NOT NULL`, because the scan coerces a null into any declared column type rather than treating the source's nullability as a guarantee
* *AND* a full outer join MUST return nulls for the unmatched rows of either side through the same coercion, in every dialect that advertises `JOIN_TYPE_FULL_OUTER`
* *AND* the returned rows MUST equal what Exasol returns for the same join executed locally over the same data, where every character operand the source evaluates either declares a code-point collation at the source or holds no values that the source's collation equates or orders differently from Exasol

### Scenario: Join composes with a delegated filter and aggregation

* *GIVEN* a join carrying a `filter` and a `groupBy` in the same `pushdown` request, for each driver this release supports
* *WHEN* the dialect renders the statement
* *THEN* the generated statement MUST carry the join, the `WHERE` clause, and the `GROUP BY` clause together
* *AND* the clauses MUST appear in the target's required order: `FROM`, `WHERE`, `GROUP BY`
* *AND* the returned rows MUST equal what Exasol returns for the same query executed locally over the same data, where every character operand the source evaluates either declares a code-point collation at the source or holds no values that the source's collation equates or orders differently from Exasol

### Scenario: Each join type renders its counterpart

* *GIVEN* a `pushdown` request whose `from` element carries `join_type`, which Exasol spells in snake case unlike every sibling protocol field
* *WHEN* each dialect renders the join
* *THEN* `inner` MUST render as `INNER JOIN`, and each keyword MUST render identically in every dialect that advertises its join type, because every target that implements the keyword spells it the same way
* *AND* `left_outer` MUST render as `LEFT OUTER JOIN`, and `right_outer` as `RIGHT OUTER JOIN`
* *AND* `full_outer` MUST render as `FULL OUTER JOIN` in every dialect that advertises `JOIN_TYPE_FULL_OUTER`
* *AND* the operand order MUST match the request's `left` and `right`, because reversing an outer join changes its result

### Scenario: Inner join of two virtual tables renders one statement

* *GIVEN* virtual tables `orders` and `customers` sharing a key column, for each driver this release supports
* *WHEN* Exasol pushes down an inner join over that key
* *THEN* the generated statement MUST join both tables in a single `SELECT`, reading neither in a separate statement
* *AND* both tables MUST be qualified by the source namespace `adapterNotes` records, because Exasol pushes a join down only when every table belongs to one virtual schema
* *AND* the join condition MUST render as an equality between the two qualified columns
* *AND* a character join key MUST carry no collation suffix, so the source's collation decides which keys match

### Scenario: Nested join renders as a nested join

* *GIVEN* a `from` element whose `left` is itself a join element, for each driver this release supports
* *WHEN* the dialect renders the statement
* *THEN* the generated statement MUST render the nested join before joining the outer table
* *AND* every table across both levels MUST carry a distinct alias

### Scenario: Every dialect advertises JOIN with an equality condition only

* *GIVEN* the capability set of each dialect this release supports
* *WHEN* the set is read
* *THEN* it MUST advertise `JOIN` together with at least one join type and `JOIN_CONDITION_EQUI`, because Exasol pushes a join down only when the adapter advertises `JOIN` with at least one join type and one join condition capability, and it then delegates the whole join and never re-joins the rows itself
* *AND* it MUST NOT advertise `JOIN_CONDITION_ALL`, because mission.md scopes pushdown to equi and outer joins and places general non-equi joins out of scope, so the withholding is this release's scope rather than any target's limit, and a target that would execute such a condition is withheld from it for the same reason as one that would not
* *AND* a delegated full outer join MUST therefore carry an equality condition alone, which no target's own full-join restriction refuses: PostgreSQL refuses a full join only when its condition is neither merge-joinable nor hash-joinable, and Trino refuses none

### Scenario: Only a dialect whose target implements FULL OUTER JOIN advertises it

* *GIVEN* the capability sets of the MySQL, PostgreSQL, and Trino dialects
* *WHEN* each set is read
* *THEN* the MySQL dialect MUST withhold `JOIN_TYPE_FULL_OUTER`, because MySQL implements no `FULL OUTER JOIN`
* *AND* the PostgreSQL and Trino dialects MUST advertise it, because both targets implement it directly, so `JOIN_TYPE_FULL_OUTER` is the one join capability the dialects disagree on and the disagreement is a property of the targets rather than a scope choice
* *AND* the MySQL dialect MUST NOT emulate a full outer join as a union of two outer joins, which mission.md places out of scope

### Scenario: A CHAR join key pairs rows as the source pairs them

* *GIVEN* a PostgreSQL virtual table with a `character(5)` column and a `character varying(5)` column, each holding `ab`, so the first is Exasol `CHAR(5)` and the second Exasol `VARCHAR(5)`
* *WHEN* self-joins of the table on the `CHAR` column equal to the `VARCHAR` column, and on the `CHAR` column equal to itself, are delegated
* *THEN* the join of the `CHAR` key with the `VARCHAR` key MUST pair the rows, because PostgreSQL ignores the trailing blanks of a `character` value, although Exasol's own join of `CHAR(5)` `ab` with `VARCHAR(5)` `ab` leaves them unpaired
* *AND* the join of the two `CHAR` keys MUST pair the rows Exasol's own join pairs, because Exasol ignores the padding between two `CHAR` values too
