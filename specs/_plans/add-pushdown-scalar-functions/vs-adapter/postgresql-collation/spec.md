# Feature: postgresql_collation

States that the PostgreSQL dialect renders no collation, what PostgreSQL's column collation does to a delegated character comparison, grouping, and `MIN` or `MAX`, and the two ways a user obtains Exasol's code-point order.

## Background

- The PostgreSQL dialect renders no collation suffix on any operand, per the framework rule `vs-adapter/pushdown-filter` states, so PostgreSQL applies the column's collation to every delegated character operand.
- A column's collation is the database default unless the column declares its own.
- A user obtains Exasol's code-point order by declaring `COLLATE "C"` at the source or by wrapping the operand in an expression the PostgreSQL dialect does not advertise.
- `vs-adapter/pushdown-dialect` owns how a column reference renders, and `vs-adapter/postgresql-type-mapping` owns which columns read as `CAST(<column> AS TEXT)`.


## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: An ordering comparison over a wrapped operand is evaluated by Exasol

* *GIVEN* a virtual table over a PostgreSQL `character varying` column under the database default collation `en_US.utf8`, holding the values `B`, `a`, and `C`, and a local Exasol table holding the same values
* *WHEN* the same comparison selecting the values ordered before `B`, and the same `MIN` and `MAX`, each with the column wrapped as `"<column>" || ''`, are run over each
* *THEN* `EXPLAIN VIRTUAL` MUST report a generated statement that carries neither the comparison nor either aggregate, because the PostgreSQL dialect does not advertise `FN_CONCAT`
* *AND* each virtual query MUST return exactly what the local query returns: no row holding `a` for the comparison, `B` for `MIN`, and `a` for `MAX`, because Exasol evaluates the wrapped operand itself over the rows PostgreSQL returns
<!-- /DELTA:CHANGED -->
