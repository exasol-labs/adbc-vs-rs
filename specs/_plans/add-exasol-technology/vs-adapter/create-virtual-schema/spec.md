# Feature: create_virtual_schema

Turns a `CREATE VIRTUAL SCHEMA` statement into a browsable set of Exasol virtual tables backed by an ADBC-fronted source database, provisioning that source's ADBC driver on the way.

## Background

This file is a scenario delta. The permanent feature's own Background governs, and the bullets below are what this plan adds to it; task 6.1 applies them to `specs/vs-adapter/create-virtual-schema/spec.md`, because `/speq:spec-merge` merges an existing feature by scenario name and reads nothing outside `## Scenarios`.

- Every driver needs the connection address to carry a path component, and what that component names is the technology's own answer: a MySQL database, a PostgreSQL database whose namespace is then `public`, or an Exasol schema. `vs-adapter/exasol-type-mapping` states the third, as the two sibling features state the first two.
- Every source has preconditions its dialect's rendering depends on, and `vs-adapter/exasol-type-mapping` states Exasol's: the addressed schema exists, is visible, and is not itself virtual. It is the only precondition of the three that guards against the source being the adapter's own database.

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Unsupported driver value fails with a named error

* *GIVEN* a `CREATE VIRTUAL SCHEMA` statement setting `DRIVER` to `snowflake`
* *WHEN* the adapter handles the request
* *THEN* the adapter MUST return an error
* *AND* the error message MUST name `snowflake` as the value it received
* *AND* the error message MUST list `mysql`, `postgresql`, and `exasol` as the drivers this release supports
* *AND* the error message MUST NOT name any single driver as the only one supported
* *AND* the list MUST be generated from the driver identities rather than written into the message, so a release adding a driver cannot leave the message naming fewer than it supports
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Connection address without a database fails with a named error

* *GIVEN* an Exasol `CONNECTION` whose address is `mysql://host:3306` with no path component
* *WHEN* the virtual schema is created
* *THEN* the adapter MUST return an error
* *AND* the error message MUST state that the connection address needs a database path component
* *AND* the same MUST hold for a `postgresql://host:5432` address, because libpq selects the database from that component and the adapter passes the address through unchanged
* *AND* the same MUST hold for an `exasol://host:8563` address, and its message MUST name the missing component as a schema rather than a database, because an Exasol connection reaches one database and the path component selects the schema inside it
* *AND* requiring a path component MUST stay the framework's rule while what that component names stays the technology's, because every driver needs one and no two need it to name the same kind of object
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Columns of unmappable type are omitted and recorded

* *GIVEN* a source table holding two core-type columns and one column of a type the driver's mapping does not map
* *WHEN* the virtual schema is created
* *THEN* the returned table MUST list only the two core-type columns
* *AND* `adapterNotes` MUST record the omitted column's table name, column name, source type, and the reason it was omitted
* *AND* the adapter MUST NOT fail the request
* *AND* the same MUST hold for a PostgreSQL `jsonb` column, a MySQL `JSON` column, and an Exasol `HASHTYPE` column, so one omission path serves every driver
<!-- /DELTA:CHANGED -->
