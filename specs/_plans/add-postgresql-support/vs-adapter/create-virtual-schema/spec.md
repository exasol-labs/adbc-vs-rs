# Feature: create_virtual_schema

Turns a `CREATE VIRTUAL SCHEMA` statement into a browsable set of Exasol virtual tables backed by an ADBC-fronted source database, provisioning that source's ADBC driver on the way.

## Background

<!-- DELTA:CHANGED -->
- The statement takes three properties: `DRIVER`, `CONNECTION_NAME`, and `BUCKETFS_CONNECTION`. This release accepts `mysql` and `postgresql` for `DRIVER`; `vs-adapter/driver-selection` owns the accepted set, the case-insensitive match, and the error that names every accepted value. The property is `CONNECTION_NAME` rather than `CONNECTION` because Exasol's parser reserves `CONNECTION` inside a `WITH` clause and rejects a quoted property name, so a virtual schema carrying that property name can never be created.
- No fourth property is added. The source namespace, the identifier case, and the driver version all stay derived or pinned rather than configurable, because a property is a decision the adapter declined to make and each one becomes a supported input the next release cannot narrow.
- `CONNECTION_NAME` names an Exasol `CONNECTION` object whose address is the source's own driver URI — `mysql://host:port/database` or `postgresql://host:port/database` — with the source user and password in the object's user and password fields. The adapter reads it through the UDF SDK's connection accessor, so no credential is ever written into SQL text, into `adapterNotes`, or into a log line. The address's scheme MUST match the named driver, per `vs-adapter/driver-selection`.
- `BUCKETFS_CONNECTION` names an Exasol `CONNECTION` object holding the BucketFS write endpoint and its write credentials, which the driver self-install needs.
- The source namespace the virtual schema exposes is derived, not configured, and what it is derived from differs by driver because the two sources nest their objects differently. For MySQL it is the database in the address's path component, which is also MySQL's SQL qualifier and its `information_schema.tables.table_schema` value. For PostgreSQL the address's path component names a database, which is not a SQL qualifier; the exposed namespace is the `public` schema, which is both PostgreSQL's SQL qualifier and its `information_schema` filter value. Both drivers therefore need the address to carry a database path component, and both record one namespace string in `adapterNotes` under the field an earlier release already wrote, so notes written before this release parse unchanged.
- Exposing only PostgreSQL's `public` schema is this release's boundary. A PostgreSQL database holds many schemas, and a virtual schema per source schema is the shape that needs no new property; naming another schema is future work rather than an omission.
- Source identifiers keep their original case, and quoting is what preserves it. Exasol folds unquoted identifiers to upper case and PostgreSQL folds them to lower case, so a source name reaches the generated statement quoted for either driver and is returned to Exasol character for character. No case-conversion property is offered in this release.
- Metadata discovery reads the source's `information_schema` over the ADBC connection, for both drivers, because it is the only source that reports exact character lengths and exact `DECIMAL` precision and scale — ADBC's own `GetTableSchema` reports a character column as Arrow `Utf8` with no length. The two drivers read different column names from it, so one metadata source exists per driver and each maps its own catalog rows, per `vs-adapter/type-mapping`. Both expose `BASE TABLE` and `VIEW` entries.
- Exasol 2025.2.1 discards a `schemaMetadata.properties` object returned by an adapter. State the adapter needs back at pushdown time therefore travels in `schemaMetadata.adapterNotes` as a stringified JSON object, which Exasol persists and returns. It carries the driver identity every later request resolves its dialect from.
- Every source has preconditions its dialect's rendering depends on, and the check is per driver because the requirement is. The requirement is stated by the dialect that renders it rather than by a second list, and it is verified by the metadata source, which is the module that already speaks the source's SQL. Both drivers run their check before any table is discovered, and on every request that resolves the source `CONNECTION` — `refresh` and `setProperties` as well as `createVirtualSchema` — because `setProperties` can point an existing virtual schema at a different server and `refresh` re-discovers against whatever the connection resolves to now.
- MySQL's precondition is two named collations, `utf8mb4_0900_as_cs` on every character comparison operand and `utf8mb4_0900_bin` on a character operand of `MIN` or `MAX`. MySQL added the UCA 9.0.0 `utf8mb4_0900_*` collations in 8.0.1 and `utf8mb4_0900_bin` in 8.0.17, so 8.0.17 is the source-server floor MySQL requires. MariaDB implements none of the `utf8mb4_0900_*` collations and rejects one with error 1273, `Unknown collation`. `DRIVER = 'mysql'` reaches whatever endpoint the pinned `adbc-drivers/mysql` driver connects to, and that driver's own published matrix names MariaDB, so a server missing a required collation is reachable and every delegated character predicate against it would fail at query time rather than at creation time.
- PostgreSQL's precondition is the database encoding, and it is one check rather than two because PostgreSQL declares an encoding per database where MySQL declares a character set per column. The mapping produces Exasol `VARCHAR` with character set `UTF8`, and the ordering collation the PostgreSQL dialect renders for `MIN` and `MAX` reproduces Exasol's code-point order only while the server compares UTF-8 bytes, so a database of another encoding is refused at creation.
- PostgreSQL needs no server-version floor from the rendering, unlike MySQL. The collation the PostgreSQL dialect renders for `MIN` and `MAX` is a built-in collation present in every PostgreSQL cluster rather than one a release added, so no version check can be justified by what the dialect renders. A floor the ADBC driver itself imposes surfaces as the driver's own connection error, and the adapter does not restate it.
<!-- /DELTA:CHANGED -->

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Creating a virtual schema exposes the source tables

* *GIVEN* a reachable source database holding two tables whose columns are all core types, for each driver this release supports
* *AND* an Exasol `CONNECTION` addressing that database with the driver's own URI scheme
* *WHEN* `CREATE VIRTUAL SCHEMA` runs with `DRIVER`, `CONNECTION_NAME`, and `BUCKETFS_CONNECTION` set
* *THEN* the adapter MUST return a `schemaMetadata` object listing both tables
* *AND* each table MUST carry its columns in the source's ordinal position order
* *AND* each column MUST carry the Exasol data type produced by that driver's type mapping
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Source identifiers keep their original case

* *GIVEN* a source database holding a lower-case table name and lower-case column names, for each driver this release supports
* *WHEN* the virtual schema is created
* *THEN* the returned table and column names MUST match the source names character for character, including case
* *AND* the adapter MUST NOT fold any identifier to upper case
* *AND* a mixed-case PostgreSQL table name MUST be returned with its case intact, because PostgreSQL preserves the case of a quoted identifier and the adapter quotes every one
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Views are exposed alongside base tables

* *GIVEN* a source database holding one base table and one view over it, for each driver this release supports
* *WHEN* the virtual schema is created
* *THEN* the returned `schemaMetadata` MUST list both the base table and the view
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Unsupported driver value fails with a named error

* *GIVEN* a `CREATE VIRTUAL SCHEMA` statement setting `DRIVER` to `snowflake`
* *WHEN* the adapter handles the request
* *THEN* the adapter MUST return an error
* *AND* the error message MUST name `snowflake` as the value it received
* *AND* the error message MUST list both `mysql` and `postgresql` as the drivers this release supports
* *AND* the error message MUST NOT name any single driver as the only one supported
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: PostgreSQL driver value is accepted

* *GIVEN* a `CREATE VIRTUAL SCHEMA` statement setting `DRIVER` to `postgresql` against a reachable PostgreSQL database
* *WHEN* the adapter handles the request
* *THEN* the statement MUST succeed
* *AND* the returned `schemaMetadata` MUST list the tables of the source's `public` schema
* *AND* `adapterNotes` MUST record the driver as `postgresql`
* *AND* the value `PostgreSQL` in any letter case MUST be accepted identically
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: PostgreSQL virtual schema exposes the public schema only

* *GIVEN* a PostgreSQL database holding one table in `public` and one table in a second schema
* *WHEN* the virtual schema is created
* *THEN* the returned `schemaMetadata` MUST list the `public` table
* *AND* it MUST NOT list the table of the second schema
* *AND* `adapterNotes` MUST record `public` as the source namespace
* *AND* the generated pushdown statement MUST qualify a table with that namespace rather than with the database the connection address names
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: PostgreSQL database whose encoding is not UTF8 is rejected at creation

* *GIVEN* a reachable PostgreSQL database whose server encoding is not `UTF8`
* *WHEN* `CREATE VIRTUAL SCHEMA` runs against it with `DRIVER = 'postgresql'`
* *THEN* the adapter MUST return an error
* *AND* the error message MUST name the encoding the database reported and `UTF8` as the encoding this release requires
* *AND* the adapter MUST NOT create the virtual schema, because the mapping declares every character column `UTF8` and the ordering collation the dialect renders reproduces Exasol's code-point order only over UTF-8 bytes
* *AND* the check MUST run before any table is discovered, and on every request that resolves the source `CONNECTION` — `refresh` and `setProperties` as well as `createVirtualSchema`
<!-- /DELTA:NEW -->

<!-- DELTA:CHANGED -->
### Scenario: Connection address without a database fails with a named error

* *GIVEN* an Exasol `CONNECTION` whose address is `mysql://host:3306` with no database path
* *WHEN* the virtual schema is created
* *THEN* the adapter MUST return an error
* *AND* the error message MUST state that the connection address needs a database path component
* *AND* the same MUST hold for a `postgresql://host:5432` address, because libpq selects the database from that component and the adapter passes the address through unchanged
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Adapter notes record the state pushdown needs and no credential

* *GIVEN* a successfully created virtual schema, for each driver this release supports
* *WHEN* `schemaMetadata.adapterNotes` is read back from `SYS.EXA_ALL_VIRTUAL_SCHEMAS`
* *THEN* it MUST be a stringified JSON object
* *AND* it MUST record the driver name in its canonical lower-case spelling, the pinned driver version, the BucketFS path of the installed driver shared library, the `CONNECTION` object's name, and the source namespace
* *AND* the recorded driver name MUST be the field `getCapabilities` and `pushdown` resolve their dialect from
* *AND* it MUST NOT contain the source password, the source user, or any BucketFS credential
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Columns of unmappable type are omitted and recorded

* *GIVEN* a source table holding two core-type columns and one column of a type the driver's mapping does not map
* *WHEN* the virtual schema is created
* *THEN* the returned table MUST list only the two core-type columns
* *AND* `adapterNotes` MUST record the omitted column's table name, column name, source type, and the reason it was omitted
* *AND* the adapter MUST NOT fail the request
* *AND* the same MUST hold for a PostgreSQL `jsonb` column and for a MySQL `JSON` column, so one omission path serves both drivers
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Table with no mappable column is omitted

* *GIVEN* a source table whose every column is of an unmapped type, for each driver this release supports
* *WHEN* the virtual schema is created
* *THEN* the returned `schemaMetadata` MUST NOT list that table
* *AND* `adapterNotes` MUST record the table as omitted
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Empty source database yields an empty table list

* *GIVEN* a reachable source namespace holding no tables, for each driver this release supports
* *WHEN* the virtual schema is created
* *THEN* the adapter MUST return a `schemaMetadata` object whose `tables` array is empty
* *AND* the adapter MUST NOT return an error
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Source server without the required collations is rejected at creation

* *GIVEN* a reachable MySQL server whose `information_schema.COLLATIONS` omits `utf8mb4_0900_bin`, or omits `utf8mb4_0900_as_cs`, as MariaDB and every MySQL older than 8.0.17 do
* *WHEN* `CREATE VIRTUAL SCHEMA` runs against it with `DRIVER = 'mysql'`
* *THEN* the adapter MUST return an error
* *AND* the error message MUST name whichever of the two collations it did not find, the server version string the source reported, and `8.0.17` as the minimum MySQL version this release supports
* *AND* the adapter MUST NOT create the virtual schema, because its advertised capabilities would make Exasol delegate a character fragment the source cannot execute
* *AND* a server listing both collations MUST be accepted, and MUST NOT be rejected on its version string alone, so that a fork reporting an unexpected version but supplying the collations still works
* *AND* the check MUST refuse the source before any table is discovered, MUST run on `refresh` and `setProperties` as well as `createVirtualSchema`, and MUST require the collation names the MySQL dialect renders rather than a second list of its own
* *AND* the check MUST NOT run for a PostgreSQL source, because the collations it names are MySQL's and the requirement is stated by the dialect that renders them
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Unreachable source fails creation with a diagnosable message

* *GIVEN* an Exasol `CONNECTION` whose address names a host and port nothing listens on, for each driver this release supports
* *WHEN* `CREATE VIRTUAL SCHEMA` runs
* *THEN* the statement MUST fail
* *AND* the error surfaced to the querying session MUST name the address it could not reach
* *AND* the adapter MUST NOT create the virtual schema
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: PostgreSQL source error surfaces no credential

* *GIVEN* an Exasol `CONNECTION` addressing a PostgreSQL database, whose password field holds a value
* *WHEN* `createVirtualSchema`, `refresh`, or `setProperties` fails against that source, once with a wrong password and once against a host and port nothing listens on
* *THEN* every error surfaced to the querying session MUST NOT contain the source password, in any encoding or escaping
* *AND* it MUST NOT contain the libpq connection string the adapter built, because that string carries the password verbatim
* *AND* it MUST still name the host and the database it addressed, so the failure stays diagnosable
* *AND* the redaction MUST apply to the subject the adapter names as well as to text the driver or libpq produced, and MUST hold for every caller of the shared connection-open path rather than for the scan alone, because the adapter opens the source connection through that same path
<!-- /DELTA:NEW -->
