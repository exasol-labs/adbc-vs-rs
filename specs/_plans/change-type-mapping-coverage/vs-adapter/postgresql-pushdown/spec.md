# Feature: postgresql_pushdown

States the capability set the PostgreSQL dialect advertises and the PostgreSQL SQL it renders for each delegated fragment.

## Background

- The PostgreSQL dialect states its own advertised capability set and derives it from no other dialect's. `vs-adapter/pushdown-dialect` owns the all-or-nothing negotiation rule — Exasol delegates a fragment fully once it receives the capability and never re-applies it — and the invariant that every advertised capability has a rendering path in the dialect that advertises it.

## Scenarios

<!-- DELTA:NEW -->
### Scenario: Character and floating-point column references render as their declared type

* *GIVEN* `pushdown` requests referencing a column declared `VARCHAR` and a column declared `DOUBLE` in a `WHERE` comparison, in the select list, as a `GROUP BY` key, as a `MIN` operand, as a `COUNT(DISTINCT)` operand, and as an equi-join key
* *WHEN* the PostgreSQL dialect renders each request
* *THEN* every reference to the `VARCHAR` column MUST render as `CAST("<column>" AS TEXT)`, which keeps the column's own collation as `vs-adapter/postgresql-collation` requires, and every reference to the `DOUBLE` column as `CAST("<column>" AS DOUBLE PRECISION)`
* *AND* a column reference declared `DECIMAL`, `DATE`, `TIMESTAMP`, or `BOOLEAN` MUST render bare, because PostgreSQL compares every source type mapped to those as Exasol compares the declared type
* *AND* a grouped column MUST render identically in `GROUP BY` and in the select list, so PostgreSQL accepts the select-list reference as the grouping key
* *AND* `EXPLAIN VIRTUAL` over a delegated filter on a `uuid` column MUST report the converted reference, and the rows the query returns MUST equal the rows of the same query over the local Exasol mirror table
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: MIN and MAX over a BOOLEAN column render as bool_and and bool_or

* *GIVEN* a `pushdown` request carrying `MIN` and `MAX` over a column declared `BOOLEAN`, each with and without `DISTINCT`
* *WHEN* the PostgreSQL dialect renders the request
* *THEN* `MIN` MUST render as `bool_and` and `MAX` as `bool_or` over the column, keeping `DISTINCT` where the request carries it
* *AND* the dialect MUST NOT render `MIN` or `MAX` over that column, because PostgreSQL defines neither for `boolean` while Exasol, which orders `false` before `true`, delegates both
* *AND* `MIN` and `MAX` over a column of any other declared type MUST render as `MIN` and `MAX`
<!-- /DELTA:NEW -->
