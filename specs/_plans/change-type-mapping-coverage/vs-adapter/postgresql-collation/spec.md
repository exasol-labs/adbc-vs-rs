# Feature: postgresql_collation

States that the PostgreSQL dialect renders no collation, what PostgreSQL's column collation does to a delegated character comparison, grouping, and `MIN` or `MAX`, and the two ways a user obtains Exasol's code-point order.

<!-- DELTA:CHANGED -->
## Background

- The PostgreSQL dialect renders no collation suffix on any operand, per the framework rule `vs-adapter/pushdown-filter` states. PostgreSQL therefore evaluates every delegated character comparison, `LIKE`, `IN`, grouping key, `DISTINCT` operand, `MIN` or `MAX` operand, and equi-join key under the collation it applies to the column, which is the database default unless the column declares its own. A `character`, `character varying`, or `text` column keeps its own collation, bare or read as `CAST(<column> AS TEXT)`, while a value of another type that the dialect reads as `CAST(<column> AS TEXT)`, such as a `uuid`, `json`, or unconstrained `numeric`, compares under the database default collation.
- PostgreSQL's built-in collations are all deterministic, and a deterministic collation equates two character values only when their bytes are identical, which is Exasol's equality. Over such a column a delegated equality, `LIKE`, `IN`, `GROUP BY`, and `DISTINCT` return the answer Exasol computes. An operator-created nondeterministic collation equates values Exasol keeps apart, and `vs-adapter/postgresql-type-mapping` maps the column like any other.
- Ordering follows the collation's sort order. The official `postgres` image sets `LANG=en_US.utf8`, so `initdb` creates the cluster with libc `LC_COLLATE=en_US.utf8` and text sorts by locale weight: `MIN` over the values `B`, `a`, `C` returns `a` where Exasol returns `B`, and `'a' < 'B'` holds where Exasol finds it false. The same holds for the text of a value the dialect reads as `CAST(<column> AS TEXT)`.
- A user who needs Exasol's code-point order has two remedies. The first declares `COLLATE "C"` at the source, on the column or in a view the virtual schema exposes, because `C` sorts by byte value, which for UTF-8 is code-point order. The second wraps the operand in an expression the dialect does not advertise, such as `"sort_key" || ''`, so Exasol keeps a comparison, `MIN`, or `MAX` over it and evaluates it itself.
- A grouping key renders identically in `GROUP BY` and in its select-list occurrence, bare or as its read expression, so PostgreSQL's full-group-by rule is satisfied without coordination.
<!-- /DELTA:CHANGED -->

## Scenarios

### Scenario: A select-list predicate over the grouped character column is delegated

* *GIVEN* a `pushdown` request whose `aggregationType` is `group_by`, whose `groupBy` names one PostgreSQL `character varying` column, and whose select list carries a predicate reading that same column outside any aggregate
* *WHEN* the PostgreSQL dialect renders the statement
* *THEN* the dialect MUST produce a statement rather than an error
* *AND* the statement MUST execute at the source rather than being rejected by it
* *AND* over a column declaring `COLLATE "C"`, the rows it returns MUST equal what Exasol returns for the same query executed over a local mirror of the same values
* *AND* this scenario MUST be verified against a live PostgreSQL source rather than by rendering alone, because what is asserted is that the source accepts an expression over its bare grouping column
