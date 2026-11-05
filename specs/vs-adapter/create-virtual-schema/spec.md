# Feature: create_virtual_schema

Turns a `CREATE VIRTUAL SCHEMA` statement into a browsable set of Exasol virtual tables backed by an ADBC-fronted source database.

## Background

- `vs-adapter/virtual-schema-properties` owns the statement's properties and the driver provisioning and BucketFS credential rules they select, and `vs-adapter/driver-selection` owns the accepted `DRIVER` set.
- The source qualifier a virtual schema exposes is an ordered list of name segments, derived rather than configured. It is what `adapterNotes` records and what every rendered statement puts in front of a table name. How many segments a technology needs, and which source fact each is derived from, is that technology's own feature — `vs-adapter/mysql-type-mapping`, `vs-adapter/postgresql-type-mapping`, `vs-adapter/trino-type-mapping` — because the sources nest their objects to different depths. What is framework is the shape: the address carries the segments in its path component, one path element per segment, and the adapter records the list under the notes field an earlier release already wrote.
- The recorded qualifier keeps one field, not two. A single-segment qualifier is recorded exactly as an earlier release recorded it, so notes Exasol persisted before this release parse unchanged and a MySQL or PostgreSQL virtual schema created before it needs no recreation. A multi-segment qualifier is recorded as the ordered list. One field rather than a second one, because a second field would let two records of one fact disagree.
- Source identifiers keep their original case, and quoting is what carries them. Exasol folds an unquoted identifier to upper case and a source may fold one to its own case, so a source name reaches the generated statement quoted for every driver and is returned to Exasol character for character. What quoting then buys at the source is per target: two targets resolve a quoted identifier case-sensitively, and Trino resolves every identifier case-insensitively whether it is quoted or not. That is a widening rather than a loss, because a name that reaches Trino spelled as the catalog reported it resolves to the object the catalog reported it from. No case-conversion property is offered in this release.
- Metadata discovery reads the source's `information_schema` over the ADBC connection, for every driver, because it is the only source that reports the column detail Exasol's declared types need — ADBC's own `GetTableSchema` reports a character column as Arrow `Utf8` with no length. One metadata source exists per driver, each reading its own catalog's column names and mapping its own rows, per `vs-adapter/type-mapping` and that driver's own mapping feature. Every driver exposes base tables and views.
- Exasol discards a `schemaMetadata.properties` object returned by an adapter. State the adapter needs back at pushdown time therefore travels in `schemaMetadata.adapterNotes` as a stringified JSON object, which Exasol persists and returns. It carries the driver identity every later request resolves its dialect from.
- A source can have preconditions its rendering or its type mapping depends on, and which precondition a technology has is stated by that technology's own feature: `vs-adapter/postgresql-type-mapping` states PostgreSQL's database encoding, and `vs-adapter/mysql-type-mapping` and `vs-adapter/trino-type-mapping` each state that their source has no server-side precondition. What is framework is when the check runs and what it may not do. It runs before any table is discovered, and on every request that resolves the source `CONNECTION` — `refresh` and `setProperties` as well as `createVirtualSchema` — because `setProperties` can point an existing virtual schema at a different server and `refresh` re-discovers against whatever the connection resolves to now. Each requirement is stated and executed by that technology's metadata source, which is the module that already speaks the source's SQL. No check runs for a driver whose rendering and type mapping depend on no server property.

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

### Scenario: Adapter notes record the state pushdown needs and no credential

* *GIVEN* a successfully created virtual schema, for each driver this release supports
* *WHEN* `schemaMetadata.adapterNotes` is read back from `SYS.EXA_ALL_VIRTUAL_SCHEMAS`
* *THEN* it MUST be a stringified JSON object
* *AND* it MUST record the driver name in its canonical lower-case spelling, the resolved driver version, the BucketFS path of the driver shared library in use, the `CONNECTION` object's name, and the source qualifier
* *AND* a single-segment qualifier MUST be recorded exactly as an earlier release recorded it, and a multi-segment qualifier MUST be recorded as the ordered list of its segments, under the one field rather than under a second one
* *AND* the recorded version MUST be the version the driver's range resolved to rather than a version compiled into adbc-vs, because under a range it is the only record of which driver a virtual schema is running
* *AND* notes written by an earlier release, which recorded a pinned version, an unversioned driver path, and a single-segment qualifier, MUST still parse
* *AND* the recorded driver name MUST be the field `getCapabilities` and `pushdown` resolve their dialect from
* *AND* it MUST NOT contain the source password, the source user, or any BucketFS credential

### Scenario: Columns of unmappable type are omitted and recorded

* *GIVEN* a source table holding two core-type columns and one column of a type the driver's mapping does not map
* *WHEN* the virtual schema is created
* *THEN* the returned table MUST list only the two core-type columns
* *AND* `adapterNotes` MUST record the omitted column's table name, column name, source type, and the reason it was omitted
* *AND* the adapter MUST NOT fail the request
* *AND* the same MUST hold for a PostgreSQL `ARRAY` column, a MySQL `BLOB` column, and a Trino `json` column, so one omission path serves every driver

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
