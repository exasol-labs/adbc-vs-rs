# Feature: create_virtual_schema

Turns a `CREATE VIRTUAL SCHEMA` statement into a browsable set of Exasol virtual tables backed by an ADBC-fronted source database, provisioning that source's ADBC driver on the way.

## Background

- The statement takes two required properties — `DRIVER` and `CONNECTION_NAME` — and two optional ones, `BUCKETFS_CONNECTION` and `DRIVER_PATH`. `vs-adapter/driver-selection` owns the accepted `DRIVER` set, the case-insensitive match, and the error that names every accepted value. The property is `CONNECTION_NAME` rather than `CONNECTION` because Exasol's parser reserves `CONNECTION` inside a `WITH` clause and rejects a quoted property name, so a virtual schema carrying that property name can never be created.
- `DRIVER_PATH` names a BucketFS path holding the ADBC driver shared library for the named driver. When it is set, the adapter uses exactly that file and skips driver self-install entirely, per `driver-provisioning/driver-self-install`. It applies to every driver technology rather than to one. It exists because the adapter cannot derive the answer: only the operator of an air-gapped cluster knows where the driver was placed. That is the test a property has to pass here — the source namespace and the identifier case stay derived precisely because the adapter *can* derive them, and no property selects a driver version because the version range supersedes the question.
- `BUCKETFS_CONNECTION` is optional because the adapter can derive it. The property names the `CONNECTION` object holding the BucketFS write endpoint and its write credentials, which driver self-install needs; a statement that does not name one is answered by the reserved connection `ADBC_VS_BUCKETFS`, the name this project's own documentation and deployment instructions already give that object. Most clusters run one BucketFS bucket behind every virtual schema, so naming it in every statement is boilerplate rather than a decision.
- The BucketFS write credential is resolved by precedence, and each level is reached only by the *absence* of the one above it. `DRIVER_PATH` set means no BucketFS connection is resolved at all, because that route writes nothing. `BUCKETFS_CONNECTION` set means exactly the connection it names, and neither a connection that does not resolve nor a registry that cannot be reached through it falls through — an explicit property is an instruction, not a preference. Neither property set means the reserved connection `ADBC_VS_BUCKETFS`, and that route alone has a last resort: the driver location `driver-provisioning/driver-path-override` owns, where an operator may have placed the library by hand.
- Two failures reach that last resort, and both describe the cluster rather than the statement. The reserved connection does not resolve, so no write credential exists. Or it resolves and the request for the registry index reaches no host at all, so no version can be resolved to install. `driver-provisioning/driver-path-override` owns what happens then, including the rule that a registry which answered anything at all fails the statement instead, and the rule that a body which stopped arriving after its status did fails it too, on either the index leg or the package leg.
- Falling through on the derived route is safe rather than lax. Exasol enforces access to a `CONNECTION` object at resolution, so a session that cannot resolve the reserved connection was never entitled to its credentials; and the source `CONNECTION` is resolved first on every request, so a connect-back transport that is broken has already failed the statement before the reserved connection is reached. Reachability of a public CDN is likewise a fact about the cluster, not about the request. Neither fall-through is silent: the notes record the pre-placed path and no version, so an operator reads back which route answered.
- The UDF SDK reports every connection-resolution failure as one opaque error, so "no such connection" cannot be told from "no access". The cause is therefore carried into the last-resort error rather than discarded, and an unreachable registry is carried there in its place when that is what blocked self-install, so the error names the failure that actually happened rather than the one the route began with.
- `DRIVER_PATH` is supplied once, at `CREATE`. The adapter records it in `adapterNotes`, so `refresh` reuses the recorded value without the property being supplied again, and `setProperties` may change it.
- `CONNECTION_NAME` names an Exasol `CONNECTION` object whose address is the source's own driver URI, `<scheme>://host:port/database`, with the source user and password in the object's user and password fields. The adapter reads it through the UDF SDK's connection accessor, so no credential is ever written into SQL text, into `adapterNotes`, or into a log line. The address's scheme MUST match the named driver, per `vs-adapter/driver-selection`.
- The source namespace the virtual schema exposes is derived, not configured, and what it is derived from is a per-technology fact stated by that technology's own feature — `vs-adapter/mysql-type-mapping` for MySQL, `vs-adapter/postgresql-type-mapping` for PostgreSQL — because the two sources nest their objects differently. What is framework is the shape: every driver needs the address to carry a database path component, and every driver records one namespace string in `adapterNotes` under the field an earlier release already wrote, so notes written before this release parse unchanged.
- Source identifiers keep their original case, and quoting is what preserves it. Exasol folds unquoted identifiers to upper case and a source may fold them to its own case, so a source name reaches the generated statement quoted for every driver and is returned to Exasol character for character. No case-conversion property is offered in this release.
- Metadata discovery reads the source's `information_schema` over the ADBC connection, for every driver, because it is the only source that reports exact character lengths and exact `DECIMAL` precision and scale — ADBC's own `GetTableSchema` reports a character column as Arrow `Utf8` with no length. One metadata source exists per driver, each reading its own catalog's column names and mapping its own rows, per `vs-adapter/type-mapping` and that driver's own mapping feature. Every driver exposes `BASE TABLE` and `VIEW` entries.
- Exasol 2025.2.1 discards a `schemaMetadata.properties` object returned by an adapter. State the adapter needs back at pushdown time therefore travels in `schemaMetadata.adapterNotes` as a stringified JSON object, which Exasol persists and returns. It carries the driver identity every later request resolves its dialect from.
- Every source has preconditions its dialect's rendering depends on, and which precondition a technology has is stated by that technology's own feature: `vs-adapter/mysql-type-mapping` states MySQL's required collations, `vs-adapter/postgresql-type-mapping` states PostgreSQL's database encoding. What is framework is when the check runs and what it may not do. It runs before any table is discovered, and on every request that resolves the source `CONNECTION` — `refresh` and `setProperties` as well as `createVirtualSchema` — because `setProperties` can point an existing virtual schema at a different server and `refresh` re-discovers against whatever the connection resolves to now. Each requirement is stated by the dialect that renders it rather than by a second list, and executed by the metadata source, which is the module that already speaks the source's SQL. No check runs for a driver whose dialect does not render the thing it requires.

## Scenarios

### Scenario: Creating a virtual schema exposes the source tables

* *GIVEN* a reachable source database holding two tables whose columns are all core types, for each driver this release supports
* *AND* an Exasol `CONNECTION` addressing that database with the driver's own URI scheme
* *WHEN* `CREATE VIRTUAL SCHEMA` runs with `DRIVER`, `CONNECTION_NAME`, and `BUCKETFS_CONNECTION` set
* *THEN* the adapter MUST return a `schemaMetadata` object listing both tables
* *AND* each table MUST carry its columns in the source's ordinal position order
* *AND* each column MUST carry the Exasol data type produced by that driver's type mapping

### Scenario: Source identifiers keep their original case

* *GIVEN* a source database holding a lower-case table name and lower-case column names, for each driver this release supports
* *WHEN* the virtual schema is created
* *THEN* the returned table and column names MUST match the source names character for character, including case
* *AND* the adapter MUST NOT fold any identifier to upper case
* *AND* a mixed-case source table name MUST be returned with its case intact, because the adapter quotes every identifier it renders

### Scenario: Views are exposed alongside base tables

* *GIVEN* a source database holding one base table and one view over it, for each driver this release supports
* *WHEN* the virtual schema is created
* *THEN* the returned `schemaMetadata` MUST list both the base table and the view

### Scenario: Missing required property fails with a named error

* *GIVEN* a `CREATE VIRTUAL SCHEMA` statement omitting `CONNECTION_NAME`
* *WHEN* the adapter handles the request
* *THEN* the adapter MUST return an error
* *AND* the error message MUST name the missing property
* *AND* the same MUST hold when `DRIVER` is omitted
* *AND* omitting `BUCKETFS_CONNECTION` MUST NOT produce a missing-property error, whether `DRIVER_PATH` is set or absent, because the adapter derives the write credential it needs rather than requiring the statement to name it
* *AND* omitting `DRIVER_PATH` MUST NOT produce a missing-property error

### Scenario: Unsupported driver value fails with a named error

* *GIVEN* a `CREATE VIRTUAL SCHEMA` statement setting `DRIVER` to `snowflake`
* *WHEN* the adapter handles the request
* *THEN* the adapter MUST return an error
* *AND* the error message MUST name `snowflake` as the value it received
* *AND* the error message MUST list both `mysql` and `postgresql` as the drivers this release supports
* *AND* the error message MUST NOT name any single driver as the only one supported

### Scenario: Connection address without a database fails with a named error

* *GIVEN* an Exasol `CONNECTION` whose address is `mysql://host:3306` with no database path
* *WHEN* the virtual schema is created
* *THEN* the adapter MUST return an error
* *AND* the error message MUST state that the connection address needs a database path component
* *AND* the same MUST hold for a `postgresql://host:5432` address, because libpq selects the database from that component and the adapter passes the address through unchanged

### Scenario: Adapter notes record the state pushdown needs and no credential

* *GIVEN* a successfully created virtual schema, for each driver this release supports
* *WHEN* `schemaMetadata.adapterNotes` is read back from `SYS.EXA_ALL_VIRTUAL_SCHEMAS`
* *THEN* it MUST be a stringified JSON object
* *AND* it MUST record the driver name in its canonical lower-case spelling, the resolved driver version, the BucketFS path of the driver shared library in use, the `CONNECTION` object's name, and the source namespace
* *AND* the recorded version MUST be the version the driver's range resolved to rather than a version compiled into adbc-vs, because under a range it is the only record of which driver a virtual schema is running
* *AND* notes written by an earlier release, which recorded a pinned version and an unversioned driver path, MUST still parse
* *AND* the recorded driver name MUST be the field `getCapabilities` and `pushdown` resolve their dialect from
* *AND* it MUST NOT contain the source password, the source user, or any BucketFS credential

### Scenario: Columns of unmappable type are omitted and recorded

* *GIVEN* a source table holding two core-type columns and one column of a type the driver's mapping does not map
* *WHEN* the virtual schema is created
* *THEN* the returned table MUST list only the two core-type columns
* *AND* `adapterNotes` MUST record the omitted column's table name, column name, source type, and the reason it was omitted
* *AND* the adapter MUST NOT fail the request
* *AND* the same MUST hold for a PostgreSQL `jsonb` column and for a MySQL `JSON` column, so one omission path serves both drivers

### Scenario: Table with no mappable column is omitted

* *GIVEN* a source table whose every column is of an unmapped type, for each driver this release supports
* *WHEN* the virtual schema is created
* *THEN* the returned `schemaMetadata` MUST NOT list that table
* *AND* `adapterNotes` MUST record the table as omitted

### Scenario: Empty source database yields an empty table list

* *GIVEN* a reachable source namespace holding no tables, for each driver this release supports
* *WHEN* the virtual schema is created
* *THEN* the adapter MUST return a `schemaMetadata` object whose `tables` array is empty
* *AND* the adapter MUST NOT return an error

### Scenario: Unreachable source fails creation with a diagnosable message

* *GIVEN* an Exasol `CONNECTION` whose address names a host and port nothing listens on, for each driver this release supports
* *WHEN* `CREATE VIRTUAL SCHEMA` runs
* *THEN* the statement MUST fail
* *AND* the error surfaced to the querying session MUST name the address it could not reach
* *AND* the adapter MUST NOT create the virtual schema

### Scenario: Every supported driver value creates a virtual schema

* *GIVEN* a reachable source database for each driver value this release supports, each addressed by an Exasol `CONNECTION` carrying that driver's own URI scheme
* *WHEN* `CREATE VIRTUAL SCHEMA` runs once per supported driver value
* *THEN* every statement MUST succeed, and each returned `schemaMetadata` MUST list the tables of that source's namespace
* *AND* each virtual schema's `adapterNotes` MUST record its driver in the canonical lower-case spelling, whatever letter case the statement used
* *AND* the driver values MUST be enumerated from the supported set rather than listed one by one, so a release adding a driver extends this coverage without editing it

### Scenario: DRIVER_PATH is accepted and bypasses driver provisioning

* *GIVEN* a BucketFS path holding a readable driver shared library for the named driver
* *WHEN* `CREATE VIRTUAL SCHEMA` runs with `DRIVER`, `CONNECTION_NAME`, and `DRIVER_PATH` set
* *THEN* the statement MUST succeed
* *AND* the adapter MUST use exactly the path given, without modifying or re-rooting it
* *AND* `adapterNotes` MUST record that path as the driver path
* *AND* the statement MUST succeed with `BUCKETFS_CONNECTION` absent, because no BucketFS write is required
* *AND* the same MUST hold for each driver this release supports

### Scenario: DRIVER_PATH naming no readable file fails with a named error

* *GIVEN* a `CREATE VIRTUAL SCHEMA` statement whose `DRIVER_PATH` names a path holding no readable file
* *WHEN* the adapter handles the request
* *THEN* the adapter MUST return an error
* *AND* the error message MUST name the `DRIVER_PATH` property, the path it received, and the driver
* *AND* the adapter MUST NOT provision the driver from the registry instead
* *AND* the adapter MUST NOT create the virtual schema

### Scenario: The BucketFS write credential is resolved by precedence

* *GIVEN* a cluster holding an Exasol `CONNECTION` named `ADBC_VS_BUCKETFS` that addresses its BucketFS write endpoint
* *WHEN* a `CREATE VIRTUAL SCHEMA` statement setting neither `BUCKETFS_CONNECTION` nor `DRIVER_PATH` is handled
* *THEN* the adapter MUST resolve `ADBC_VS_BUCKETFS` and self-install the driver through it
* *AND* the created schema's `adapterNotes` MUST record the versioned install path and the resolved driver version, exactly as a statement naming the connection explicitly produces
* *AND* a statement setting `BUCKETFS_CONNECTION` to a non-empty value MUST use exactly the connection that property names, and MUST NOT resolve `ADBC_VS_BUCKETFS`
* *AND* a statement setting `BUCKETFS_CONNECTION` to the empty string MUST be answered exactly as one omitting it, because an empty property reads as absent throughout the adapter
* *AND* a statement setting `DRIVER_PATH` MUST resolve no BucketFS connection at all, neither a named one nor `ADBC_VS_BUCKETFS`, because that route performs no write
* *AND* the same precedence MUST apply to `refresh` and `setProperties`, which resolve the credential from the properties they are answered under rather than from the route creation took

### Scenario: An explicitly named BucketFS connection never falls back

* *GIVEN* a `CREATE VIRTUAL SCHEMA` statement setting `BUCKETFS_CONNECTION` and no `DRIVER_PATH`
* *WHEN* the connection that property names does not resolve
* *THEN* the adapter MUST fail the statement with an error naming that connection, and MUST NOT fall through to `ADBC_VS_BUCKETFS` or to any driver location
* *AND* the same MUST hold when that connection resolves and the driver registry answers nothing at all: the statement MUST fail with the registry's own error and MUST NOT fall through to the default driver location
* *AND* the last resort MUST be reachable from the reserved connection alone, because an explicit property is an instruction rather than a preference
