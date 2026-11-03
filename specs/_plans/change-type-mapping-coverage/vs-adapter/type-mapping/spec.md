<!-- DELTA:CHANGED -->
# Feature: type_mapping

States the contract every source-to-Exasol column mapping obeys: the closed set of Exasol types a mapped column may declare, how a mapped column records its source type so each dialect reads the value the scan emits, how a timestamp keeps its fractional-second precision, how nullability carries, and how a column with no Exasol representation is omitted.
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
## Background

- A type mapping is a pure, total function from one source catalog row to an Exasol column declaration or an omission with a reason. It reads catalog metadata, not the Arrow type the driver returns, because that Arrow type varies with the statement.
- The mapped set is `DECIMAL`, `VARCHAR`, `CHAR`, `DATE`, `TIMESTAMP(p)`, `DOUBLE`, and `BOOLEAN`, and it equals the set an `EMITS` column may declare. `specs/exasol-types.md` records Exasol's bounds on each type.
- The PostgreSQL and MySQL mappings start from the Exasol JDBC Virtual Schema's mapping for the same source, and each `<technology>-type-mapping` feature states every departure. A source type with no Exasol counterpart maps to a `VARCHAR` holding the source's text for each value, except the types each `<technology>-type-mapping` feature leaves unmapped.
- A mapped column records its source type in its own `adapterNotes`, and the source type selects the read expression each dialect renders for the column, per `vs-adapter/pushdown-dialect`.
- The catalogs report different fields, so each driver's mapping table, read expressions, known differences from Exasol's comparison, and source precondition live in its own `<technology>-type-mapping` feature.
<!-- /DELTA:CHANGED -->

## Scenarios

<!-- DELTA:REMOVED -->
### Scenario: DECIMAL beyond Exasol's maximum precision is unmapped

* *GIVEN* a source column of a numeric type whose declared precision exceeds what Exasol can declare, such as precision 40 and scale 2
* *WHEN* the column is mapped
* *THEN* the result MUST be unmapped
* *AND* the stated reason MUST name Exasol's maximum `DECIMAL` precision of 36
* *AND* the mapping MUST NOT silently reduce the precision, because that would truncate values
* *AND* the rule MUST hold whatever the source names that type and whatever that source's own maximum precision is, because the bound is Exasol's own rather than the source's
<!-- /DELTA:REMOVED -->

<!-- DELTA:NEW -->
### Scenario: An exact numeric beyond Exasol's maximum precision is never declared DECIMAL

* *GIVEN* a source column of an exact numeric type whose declared precision exceeds 36, such as precision 40 and scale 2, or whose precision the catalog leaves unset
* *WHEN* the column is mapped
* *THEN* the result MUST NOT be a `DECIMAL`, so no value is truncated or rounded to fit Exasol's maximum precision of 36
* *AND* a PostgreSQL `numeric` or a MySQL `DECIMAL` column MUST map to `VARCHAR` with size 2000000 and character set `UTF8`, holding the source's text for each value, as the Exasol JDBC Virtual Schemas map it
* *AND* a Trino `decimal` column MUST be unmapped, with the reason naming Exasol's maximum `DECIMAL` precision of 36
<!-- /DELTA:NEW -->

<!-- DELTA:CHANGED -->
### Scenario: A mapped column declares only an Exasol type this release produces

* *GIVEN* every column a driver's mapping returns as mapped
* *WHEN* the declared Exasol type of each is read
* *THEN* each MUST be one of `DECIMAL`, `VARCHAR`, `CHAR`, `DATE`, `TIMESTAMP(p)`, `DOUBLE`, and `BOOLEAN`
* *AND* none MUST be Exasol `TIMESTAMP WITH LOCAL TIME ZONE`, `INTERVAL`, `GEOMETRY`, or `HASHTYPE`
* *AND* every driver's mapped types MUST fall within that one set, so adding a driver adds no Exasol type to the system
* *AND* a mapping that would widen the set MUST be treated as a change to the pushdown closure argument `vs-adapter/pushdown-dialect` states rather than as a local addition
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: A mapped column records its source type in its adapterNotes

* *GIVEN* a source table whose columns map, for any driver
* *WHEN* `createVirtualSchema`, `refresh`, or `setProperties` returns the table's metadata
* *THEN* each mapped column MUST carry its own `adapterNotes`, a JSON object whose `sourceType` field holds the source type as the catalog spelled it, such as `uuid` or `time`
* *AND* every `pushdown` request MUST read that field from `involvedTables[].columns[].adapterNotes` for each column reference
* *AND* a column without `adapterNotes`, as a virtual schema created before this release declares it, MUST render as the bare quoted column
* *AND* the column `adapterNotes` MUST hold no credential
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: A TIMESTAMP column declares the source's fractional-second precision

* *GIVEN* a PostgreSQL `timestamp(6)` column and a MySQL `DATETIME(3)` column
* *WHEN* each column is mapped and the virtual schema is created
* *THEN* the virtual columns MUST declare `TIMESTAMP(6)` and `TIMESTAMP(3)`, carried in the `dataType` field `fractionalSecondsPrecision`
* *AND* a stored PostgreSQL value `2024-01-01 12:00:00.123456` MUST be emitted with all six fractional digits, a delegated equality of that column with `TIMESTAMP '2024-01-01 12:00:00.123'` MUST select no row, as Exasol's comparison of the emitted `TIMESTAMP(6)` value does, and a delegated equality with `TIMESTAMP '2024-01-01 12:00:00.123456'` MUST select the row through a generated statement that carries all six fractional digits
* *AND* a `dataType` entry without `fractionalSecondsPrecision`, as a virtual schema created before this release declares it, MUST read as `TIMESTAMP(3)`
* *AND* a Trino `timestamp(p)` column MUST map to `TIMESTAMP(3)` for every precision Trino admits
<!-- /DELTA:NEW -->
