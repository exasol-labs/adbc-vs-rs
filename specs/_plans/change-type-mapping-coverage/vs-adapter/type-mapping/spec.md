<!-- DELTA:CHANGED -->
# Feature: type_mapping

States the contract every source-to-Exasol column mapping obeys: the closed set of Exasol types a mapped column may declare, how a mapped column records its source type so each dialect reads the value the scan emits, how a timestamp keeps its fractional-second precision, how nullability carries, and how a column with no Exasol representation is omitted.
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
## Background

- A type mapping turns one source catalog row into an Exasol column declaration or an omission with a reason.
- This feature owns the contract every driver's mapping obeys: the closed type set, the column `adapterNotes`, the timestamp precision, nullability, and the omission record.
- A read expression is the SQL a dialect renders for a column so that the source yields the value the scan emits, and `vs-adapter/pushdown-dialect` owns its rendering.
- Each `<technology>-type-mapping` feature states its mapping table, read expressions, known differences from Exasol's comparison, and source precondition.
- `specs/exasol-types.md` records Exasol's bounds on each type.
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
* *THEN* the result MUST NOT be a `DECIMAL`, so no value is truncated or rounded to fit Exasol's maximum precision of 36, whatever the source names the type and whatever the source's own maximum precision is, because the bound is Exasol's own
* *AND* a PostgreSQL `numeric` or a MySQL `DECIMAL` column MUST map to `VARCHAR` with size 2000000 and character set `UTF8`, holding the source's text for each value, as the Exasol JDBC Virtual Schemas map it
* *AND* a Trino `decimal` column MUST be unmapped, with the reason naming Exasol's maximum `DECIMAL` precision of 36
<!-- /DELTA:NEW -->

<!-- DELTA:CHANGED -->
### Scenario: Each technology states its own mapping table

* *GIVEN* drivers whose catalogs report a column's type differently: MySQL's unsigned flag and per-column character set, PostgreSQL's `data_type` with a null length or precision for a `text` or unconstrained `numeric` column, and Trino's single type string carrying the length, precision, and scale no column of its catalog reports
* *WHEN* a virtual schema is created over any of them
* *THEN* each driver's metadata source MUST return columns already mapped, so `create_schema` branches on no driver and no shared source-column struct exists, because one lowest-common-denominator catalog row loses whichever detail only one source carries and has nowhere to put a detail that is not a field
* *AND* a source type with no Exasol counterpart MUST map to a `VARCHAR` holding the source's text for each value, unless the driver's `<technology>-type-mapping` feature names the type unmapped and states why
* *AND* which source types map, and why the rest are omitted, MUST be stated by each `<technology>-type-mapping` feature rather than here or by addition to or removal from another feature's table
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: A mapping reads one catalog row and performs no I/O

* *GIVEN* a catalog row any driver's metadata source reads
* *WHEN* the driver's mapping maps the row
* *THEN* the mapping MUST be a pure, total function of that row that performs no I/O, so the row either maps to an Exasol column declaration or is omitted with a reason
* *AND* the mapping MUST read catalog metadata rather than the Arrow type the driver returns at scan time, because Exasol compares the declared type for exact equality, while the Arrow type varies with the statement: the `adbc-drivers/mysql` driver returns `DATETIME` as `timestamp[s]`, `timestamp[ms]`, or `timestamp[us]` depending on how the query is written
<!-- /DELTA:NEW -->

<!-- DELTA:CHANGED -->
### Scenario: A mapped column declares only an Exasol type this release produces

* *GIVEN* every column a driver's mapping returns as mapped
* *WHEN* the declared Exasol type of each is read
* *THEN* each MUST be one of `DECIMAL`, `VARCHAR`, `CHAR`, `DATE`, `TIMESTAMP(p)`, `DOUBLE`, and `BOOLEAN`, which is also the set of types an `EMITS` column can declare, so a mapped column and a select-list entry draw their declared type from one set
* *AND* none MUST be Exasol `TIMESTAMP WITH LOCAL TIME ZONE`, `INTERVAL`, `GEOMETRY`, or `HASHTYPE`
* *AND* every driver's mapped types MUST fall within that one set, so adding a driver adds no Exasol type to the system
* *AND* a mapping that would widen the set MUST be treated as a change to the pushdown closure argument `vs-adapter/pushdown-dialect` states rather than as a local addition
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Omission records the source type under a source-neutral field

* *GIVEN* a virtual schema whose source holds one column of an unmapped type
* *WHEN* the recorded omission is read back from `adapterNotes`
* *THEN* the omission MUST name the table, the column, the source type, and the reason
* *AND* the field carrying the source type MUST be `sourceType`, spelled for any source rather than for MySQL alone
* *AND* an `adapterNotes` value written by an earlier release under the MySQL-specific spelling `mysqlType` MUST still parse, because Exasol persists those notes and a virtual schema created before this release is read back rather than recreated
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: A mapped column records its source type in its adapterNotes

* *GIVEN* a source table whose columns map, for any driver
* *WHEN* `createVirtualSchema`, `refresh`, or `setProperties` returns the table's metadata
* *THEN* each mapped column MUST carry its own `adapterNotes`, a JSON object whose `sourceType` field holds the source type as the catalog spelled it, such as `uuid` or `time`
* *AND* every `pushdown` request MUST read that field from `involvedTables[].columns[].adapterNotes` for each column reference, and the recorded source type MUST select the read expression each dialect renders for the column, per `vs-adapter/pushdown-dialect`
* *AND* a column without `adapterNotes`, as a virtual schema created before this release declares it, MUST render as the bare quoted column
* *AND* the column `adapterNotes` MUST hold no credential
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: A TIMESTAMP column declares the source's fractional-second precision

* *GIVEN* a PostgreSQL `timestamp(6)` column and a MySQL `DATETIME(3)` column
* *WHEN* each column is mapped and the virtual schema is created
* *THEN* the virtual columns MUST declare `TIMESTAMP(6)` and `TIMESTAMP(3)`, carried in the `dataType` field `fractionalSecondsPrecision`
* *AND* a `dataType` entry without `fractionalSecondsPrecision`, as a virtual schema created before this release declares it, MUST read as `TIMESTAMP(3)`
* *AND* a Trino `timestamp(p)` column MUST map to `TIMESTAMP(3)` for every precision Trino admits
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: A microsecond timestamp keeps every fractional digit through the scan and a delegated comparison

* *GIVEN* a PostgreSQL `timestamp(6)` column, mapped to `TIMESTAMP(6)`, storing `2024-01-01 12:00:00.123456`
* *WHEN* the column is scanned and filtered by delegated equalities
* *THEN* the scan MUST emit the value with all six fractional digits
* *AND* a delegated equality with `TIMESTAMP '2024-01-01 12:00:00.123'` MUST select no row, as Exasol's comparison of the emitted `TIMESTAMP(6)` value does
* *AND* a delegated equality with `TIMESTAMP '2024-01-01 12:00:00.123456'` MUST select the row through a generated statement that carries all six fractional digits
<!-- /DELTA:NEW -->
