# Feature: create_virtual_schema

Turns a `CREATE VIRTUAL SCHEMA` statement into a browsable set of Exasol virtual tables backed by an ADBC-fronted source database, provisioning that source's ADBC driver on the way.

## Background

- The statement takes two required properties — `DRIVER` and `CONNECTION_NAME` — and two optional ones, `BUCKETFS_CONNECTION` and `DRIVER_PATH`. `vs-adapter/driver-selection` owns the accepted `DRIVER` set, the case-insensitive match, and the error that names every accepted value. The property is `CONNECTION_NAME` rather than `CONNECTION` because Exasol's parser reserves `CONNECTION` inside a `WITH` clause and rejects a quoted property name, so a virtual schema carrying that property name can never be created.

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Columns of unmappable type are omitted and recorded

* *GIVEN* a source table holding two core-type columns and one column of a type the driver's mapping does not map
* *WHEN* the virtual schema is created
* *THEN* the returned table MUST list only the two core-type columns
* *AND* `adapterNotes` MUST record the omitted column's table name, column name, source type, and the reason it was omitted
* *AND* the adapter MUST NOT fail the request
* *AND* the same MUST hold for a PostgreSQL `bytea` column, a MySQL `JSON` column, and a Trino `json` column, so one omission path serves every driver
<!-- /DELTA:CHANGED -->
