# Feature: postgresql_pushdown

States the capability set the PostgreSQL dialect advertises and the PostgreSQL SQL it renders for each delegated fragment.

## Background

- The PostgreSQL dialect states its own advertised capability set and derives it from no other dialect's. `vs-adapter/pushdown-dialect` owns the all-or-nothing negotiation rule — Exasol delegates a fragment fully once it receives the capability and never re-applies it — and the invariant that every advertised capability has a rendering path in the dialect that advertises it.

## Scenarios

<!-- DELTA:NEW -->
### Scenario: MIN and MAX over a BOOLEAN column render as bool_and and bool_or

* *GIVEN* a `pushdown` request carrying `MIN` and `MAX` over a PostgreSQL `boolean` column and over a `bit` column, each declared `BOOLEAN`, each with and without `DISTINCT`
* *WHEN* the PostgreSQL dialect renders the request
* *THEN* `MIN` MUST render as `bool_and` and `MAX` as `bool_or` over the column's read expression, the bare column for `boolean` and `(position(B'1' IN <column>) > 0)` for `bit`, keeping `DISTINCT` where the request carries it
* *AND* the dialect MUST NOT render `MIN` or `MAX` over a `BOOLEAN` operand, because PostgreSQL defines neither for `boolean` while Exasol, which orders `false` before `true`, delegates both
* *AND* `MIN` and `MAX` over a column of any other declared type MUST render as `MIN` and `MAX`
<!-- /DELTA:NEW -->
