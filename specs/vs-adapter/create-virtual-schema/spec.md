# Feature: create_virtual_schema

Turns a `CREATE VIRTUAL SCHEMA` statement into a browsable set of Exasol virtual tables backed by a MySQL database, provisioning the ADBC driver on the way.

## Background

- The statement takes three properties: `DRIVER`, `CONNECTION_NAME`, and `BUCKETFS_CONNECTION`. Release 0.1 accepts only `mysql` for `DRIVER`. The property is `CONNECTION_NAME` rather than `CONNECTION` because Exasol's parser reserves `CONNECTION` inside a `WITH` clause and rejects a quoted property name, so a virtual schema carrying that property name can never be created.
- `CONNECTION_NAME` names an Exasol `CONNECTION` object whose address is a MySQL URI of the form `mysql://host:port/database`, with the MySQL user and password in the object's user and password fields. The adapter reads it through the UDF SDK's connection accessor, so no credential is ever written into SQL text, into `adapterNotes`, or into a log line.
- The MySQL database the virtual schema exposes is taken from the path component of that URI. It is not a separate property.
- `BUCKETFS_CONNECTION` names an Exasol `CONNECTION` object holding the BucketFS write endpoint and its write credentials, which the driver self-install needs.
- MySQL identifiers keep their original case. Exasol folds unquoted identifiers to upper case, so a lower-case MySQL table is addressed from Exasol as a quoted identifier. No case-conversion property is offered in this release.
- Metadata discovery reads MySQL's `information_schema` over the ADBC connection, because it is the only source that reports exact character lengths and exact `DECIMAL` precision and scale. Both `BASE TABLE` and `VIEW` entries are exposed.
- Exasol 2025.2.1 discards a `schemaMetadata.properties` object returned by an adapter. State the adapter needs back at pushdown time therefore travels in `schemaMetadata.adapterNotes` as a stringified JSON object, which Exasol persists and returns.
- Pushdown rendering depends on two named collations existing on the source server: `utf8mb4_0900_as_cs` on every character comparison operand and `utf8mb4_0900_bin` on a character operand of `MIN` or `MAX`. MySQL added the UCA 9.0.0 `utf8mb4_0900_*` collations in 8.0.1 and `utf8mb4_0900_bin` in 8.0.17, so 8.0.17 is the source-server floor this release requires. MariaDB implements none of the `utf8mb4_0900_*` collations and rejects one with error 1273, `Unknown collation`.
- `DRIVER = 'mysql'` reaches whatever endpoint the pinned `adbc-drivers/mysql` driver connects to, and that driver's own published matrix names MariaDB. A server missing a required collation is therefore reachable, and every delegated character predicate against it fails at query time with a MySQL error rather than at creation time. The collations are checked once, at `CREATE VIRTUAL SCHEMA`, where the operator can act on the answer.
- The check reads `information_schema.COLLATIONS`, which is the same source `vs-adapter/type-mapping` already reads column metadata from, and reads the server's own version string. It is not a version-string comparison alone: a fork reporting an 8.x version without the collations is rejected on the collation, not on the number.

## Scenarios

### Scenario: Creating a virtual schema exposes the source tables

* *GIVEN* a reachable MySQL database holding two tables whose columns are all core types
* *AND* an Exasol `CONNECTION` addressing that database
* *WHEN* `CREATE VIRTUAL SCHEMA` runs with `DRIVER`, `CONNECTION_NAME`, and `BUCKETFS_CONNECTION` set
* *THEN* the adapter MUST return a `schemaMetadata` object listing both tables
* *AND* each table MUST carry its columns in the source's ordinal position order
* *AND* each column MUST carry the Exasol data type produced by the type mapping

### Scenario: Source identifiers keep their original case

* *GIVEN* a MySQL database holding a lower-case table name and lower-case column names
* *WHEN* the virtual schema is created
* *THEN* the returned table and column names MUST match the MySQL names character for character, including case
* *AND* the adapter MUST NOT fold any identifier to upper case

### Scenario: Views are exposed alongside base tables

* *GIVEN* a MySQL database holding one base table and one view over it
* *WHEN* the virtual schema is created
* *THEN* the returned `schemaMetadata` MUST list both the base table and the view

### Scenario: Missing required property fails with a named error

* *GIVEN* a `CREATE VIRTUAL SCHEMA` statement omitting `CONNECTION_NAME`
* *WHEN* the adapter handles the request
* *THEN* the adapter MUST return an error
* *AND* the error message MUST name the missing property
* *AND* the same MUST hold when `DRIVER` or `BUCKETFS_CONNECTION` is omitted

### Scenario: Unsupported driver value fails with a named error

* *GIVEN* a `CREATE VIRTUAL SCHEMA` statement setting `DRIVER` to `postgresql`
* *WHEN* the adapter handles the request
* *THEN* the adapter MUST return an error
* *AND* the error message MUST name `mysql` as the only driver this release supports

### Scenario: Connection address without a database fails with a named error

* *GIVEN* an Exasol `CONNECTION` whose address is `mysql://host:3306` with no database path
* *WHEN* the virtual schema is created
* *THEN* the adapter MUST return an error
* *AND* the error message MUST state that the connection address needs a database path component

### Scenario: Adapter notes record the state pushdown needs and no credential

* *GIVEN* a successfully created virtual schema
* *WHEN* `schemaMetadata.adapterNotes` is read back from `SYS.EXA_ALL_VIRTUAL_SCHEMAS`
* *THEN* it MUST be a stringified JSON object
* *AND* it MUST record the driver name, the pinned driver version, the BucketFS path of the installed driver shared library, the `CONNECTION` object's name, and the MySQL database name
* *AND* it MUST NOT contain the MySQL password, the MySQL user, or any BucketFS credential

### Scenario: Columns of unmappable type are omitted and recorded

* *GIVEN* a MySQL table holding two core-type columns and one `JSON` column
* *WHEN* the virtual schema is created
* *THEN* the returned table MUST list only the two core-type columns
* *AND* `adapterNotes` MUST record the omitted column's table name, column name, MySQL type, and the reason it was omitted
* *AND* the adapter MUST NOT fail the request

### Scenario: Table with no mappable column is omitted

* *GIVEN* a MySQL table whose every column is of an unmapped type
* *WHEN* the virtual schema is created
* *THEN* the returned `schemaMetadata` MUST NOT list that table
* *AND* `adapterNotes` MUST record the table as omitted

### Scenario: Empty source database yields an empty table list

* *GIVEN* a reachable MySQL database holding no tables
* *WHEN* the virtual schema is created
* *THEN* the adapter MUST return a `schemaMetadata` object whose `tables` array is empty
* *AND* the adapter MUST NOT return an error

### Scenario: Source server without the required collations is rejected at creation

* *GIVEN* a reachable source server whose `information_schema.COLLATIONS` omits `utf8mb4_0900_bin`, or omits `utf8mb4_0900_as_cs`, as MariaDB and every MySQL older than 8.0.17 do
* *WHEN* `CREATE VIRTUAL SCHEMA` runs against it
* *THEN* the adapter MUST return an error
* *AND* the error message MUST name whichever of the two collations it did not find, the server version string the source reported, and `8.0.17` as the minimum MySQL version this release supports
* *AND* the adapter MUST NOT create the virtual schema, because its advertised capabilities would make Exasol delegate a character fragment the source cannot execute
* *AND* a server listing both collations MUST be accepted, and MUST NOT be rejected on its version string alone, so that a fork reporting an unexpected version but supplying the collations still works
* *AND* every request that resolves the source `CONNECTION` MUST perform the check — `refresh` and `setProperties` as well as `createVirtualSchema` — because `setProperties` can point an existing virtual schema at a different server and `refresh` re-discovers against whatever the connection resolves to now
* *AND* the check MUST refuse the source before any table is discovered from it, and the collation names it requires MUST be the ones the dialect renders rather than a second list of its own

### Scenario: Unreachable source fails creation with a diagnosable message

* *GIVEN* an Exasol `CONNECTION` whose address names a MySQL host and port nothing listens on
* *WHEN* `CREATE VIRTUAL SCHEMA` runs
* *THEN* the statement MUST fail
* *AND* the error surfaced to the querying session MUST name the address it could not reach
* *AND* the adapter MUST NOT create the virtual schema
