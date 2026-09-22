# Feature: create_virtual_schema

Turns a `CREATE VIRTUAL SCHEMA` statement into a browsable set of Exasol virtual tables backed by an ADBC-fronted source database, provisioning that source's ADBC driver on the way.

<!-- DELTA:CHANGED -->
## Background

- The statement takes two required properties — `DRIVER` and `CONNECTION_NAME` — and two optional ones, `BUCKETFS_CONNECTION` and `DRIVER_PATH`. `vs-adapter/driver-selection` owns the accepted `DRIVER` set, the case-insensitive match, and the error that names every accepted value. The property is `CONNECTION_NAME` rather than `CONNECTION` because Exasol's parser reserves `CONNECTION` inside a `WITH` clause and rejects a quoted property name, so a virtual schema carrying that property name can never be created.
- `DRIVER_PATH` names a BucketFS path holding the ADBC driver shared library for the named driver. When it is set, the adapter uses exactly that file and skips driver self-install entirely, per `driver-provisioning/driver-self-install`. It applies to every driver technology rather than to one. It exists because the adapter cannot derive the answer: only the operator of an air-gapped cluster knows where the driver was placed. That is the test a property has to pass here — the source qualifier and the identifier case stay derived precisely because the adapter *can* derive them, and no property selects a driver version because the version range supersedes the question.
- `BUCKETFS_CONNECTION` is optional because the adapter can derive it. The property names the `CONNECTION` object holding the BucketFS write endpoint and its write credentials, which driver self-install needs; a statement that does not name one is answered by the reserved connection `ADBC_VS_BUCKETFS`, the name this project's own documentation and deployment instructions already give that object. Most clusters run one BucketFS bucket behind every virtual schema, so naming it in every statement is boilerplate rather than a decision.
- The BucketFS write credential is resolved by precedence, and each level is reached only by the *absence* of the one above it. `DRIVER_PATH` set means no BucketFS connection is resolved at all, because that route writes nothing. `BUCKETFS_CONNECTION` set means exactly the connection it names, and neither a connection that does not resolve nor a registry that cannot be reached through it falls through — an explicit property is an instruction, not a preference. Neither property set means the reserved connection `ADBC_VS_BUCKETFS`, and that route alone has a last resort: the driver location `driver-provisioning/driver-path-override` owns, where an operator may have placed the library by hand.
- Two failures reach that last resort, and both describe the cluster rather than the statement. The reserved connection does not resolve, so no write credential exists. Or it resolves and the request for the registry index reaches no host at all, so no version can be resolved to install. `driver-provisioning/driver-path-override` owns what happens then, including the rule that a registry which answered anything at all fails the statement instead, and the rule that a body which stopped arriving after its status did fails it too, on either the index leg or the package leg.
- Falling through on the derived route is safe rather than lax. Exasol enforces access to a `CONNECTION` object at resolution, so a session that cannot resolve the reserved connection was never entitled to its credentials; and the source `CONNECTION` is resolved first on every request, so a connect-back transport that is broken has already failed the statement before the reserved connection is reached. Reachability of a public CDN is likewise a fact about the cluster, not about the request. Neither fall-through is silent: the notes record the pre-placed path and no version, so an operator reads back which route answered.
- The UDF SDK reports every connection-resolution failure as one opaque error, so "no such connection" cannot be told from "no access". The cause is therefore carried into the last-resort error rather than discarded, and an unreachable registry is carried there in its place when that is what blocked self-install, so the error names the failure that actually happened rather than the one the route began with.
- `DRIVER_PATH` is supplied once, at `CREATE`. The adapter records it in `adapterNotes`, so `refresh` reuses the recorded value without the property being supplied again, and `setProperties` may change it.
- `CONNECTION_NAME` names an Exasol `CONNECTION` object whose address is the source's own driver URI, `<scheme>://host[:port]/<qualifier path>`, with the source user and password in the object's user and password fields. The adapter reads it through the UDF SDK's connection accessor, so no credential is ever written into SQL text, into `adapterNotes`, or into a log line. The address's scheme MUST match the named driver, per `vs-adapter/driver-selection`.
- The source qualifier a virtual schema exposes is an ordered list of name segments, derived rather than configured. It is what `adapterNotes` records and what every rendered statement puts in front of a table name. How many segments a technology needs, and which source fact each is derived from, is that technology's own feature — `vs-adapter/mysql-type-mapping`, `vs-adapter/postgresql-type-mapping`, `vs-adapter/trino-type-mapping` — because the sources nest their objects to different depths. What is framework is the shape: the address carries the segments in its path component, one path element per segment, and the adapter records the list under the notes field an earlier release already wrote.
- The recorded qualifier keeps one field, not two. A single-segment qualifier is recorded exactly as an earlier release recorded it, so notes Exasol persisted before this release parse unchanged and a MySQL or PostgreSQL virtual schema created before it needs no recreation. A multi-segment qualifier is recorded as the ordered list. One field rather than a second one, because a second field would let two records of one fact disagree.
- Source identifiers keep their original case, and quoting is what carries them. Exasol folds an unquoted identifier to upper case and a source may fold one to its own case, so a source name reaches the generated statement quoted for every driver and is returned to Exasol character for character. What quoting then buys at the source is per target: two targets resolve a quoted identifier case-sensitively, and Trino resolves every identifier case-insensitively whether it is quoted or not. That is a widening rather than a loss, because a name that reaches Trino spelled as the catalog reported it resolves to the object the catalog reported it from. No case-conversion property is offered in this release.
- Metadata discovery reads the source's `information_schema` over the ADBC connection, for every driver, because it is the only source that reports the column detail Exasol's declared types need — ADBC's own `GetTableSchema` reports a character column as Arrow `Utf8` with no length. One metadata source exists per driver, each reading its own catalog's column names and mapping its own rows, per `vs-adapter/type-mapping` and that driver's own mapping feature. Every driver exposes base tables and views.
- Exasol 2025.2.1 discards a `schemaMetadata.properties` object returned by an adapter. State the adapter needs back at pushdown time therefore travels in `schemaMetadata.adapterNotes` as a stringified JSON object, which Exasol persists and returns. It carries the driver identity every later request resolves its dialect from.
- Every source has preconditions its dialect's rendering depends on, and which precondition a technology has is stated by that technology's own feature: `vs-adapter/mysql-type-mapping` states MySQL's required collations, `vs-adapter/postgresql-type-mapping` states PostgreSQL's database encoding, `vs-adapter/trino-type-mapping` states Trino's. What is framework is when the check runs and what it may not do. It runs before any table is discovered, and on every request that resolves the source `CONNECTION` — `refresh` and `setProperties` as well as `createVirtualSchema` — because `setProperties` can point an existing virtual schema at a different server and `refresh` re-discovers against whatever the connection resolves to now. Each requirement is stated by the dialect that renders it rather than by a second list, and executed by the metadata source, which is the module that already speaks the source's SQL. No check runs for a driver whose dialect does not render the thing it requires.
<!-- /DELTA:CHANGED -->

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Unsupported driver value fails with a named error

* *GIVEN* a `CREATE VIRTUAL SCHEMA` statement setting `DRIVER` to `snowflake`
* *WHEN* the adapter handles the request
* *THEN* the adapter MUST return an error
* *AND* the error message MUST name `snowflake` as the value it received
* *AND* the error message MUST list every driver value this release supports, enumerated from the driver identities rather than written into the message
* *AND* the error message MUST NOT name any single driver as the only one supported
<!-- /DELTA:CHANGED -->

<!-- DELTA:REMOVED -->
### Scenario: Connection address without a database fails with a named error

* *GIVEN* an Exasol `CONNECTION` whose address is `mysql://host:3306` with no database path
* *WHEN* the virtual schema is created
* *THEN* the adapter MUST return an error
* *AND* the error message MUST state that the connection address needs a database path component
* *AND* the same MUST hold for a `postgresql://host:5432` address, because libpq selects the database from that component and the adapter passes the address through unchanged
<!-- /DELTA:REMOVED -->

<!-- DELTA:NEW -->
### Scenario: Connection address without the qualifier its driver needs fails with a named error

* *GIVEN* an Exasol `CONNECTION` whose address is `mysql://host:3306` with no path component
* *WHEN* the virtual schema is created
* *THEN* the adapter MUST return an error
* *AND* the error message MUST state how many qualifier segments the named driver's address needs and how many it carried
* *AND* the same MUST hold for a `postgresql://host:5432` address, because libpq selects the database from that component and the adapter passes the address through unchanged
* *AND* a `trino://host:8443/memory` address MUST fail the same way, because a Trino table is qualified by a catalog and a schema and one path element names only the catalog
* *AND* an address carrying more path elements than the named driver's qualifier needs MUST fail the same way rather than silently ignoring the extra element
<!-- /DELTA:NEW -->

<!-- DELTA:CHANGED -->
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
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Columns of unmappable type are omitted and recorded

* *GIVEN* a source table holding two core-type columns and one column of a type the driver's mapping does not map
* *WHEN* the virtual schema is created
* *THEN* the returned table MUST list only the two core-type columns
* *AND* `adapterNotes` MUST record the omitted column's table name, column name, source type, and the reason it was omitted
* *AND* the adapter MUST NOT fail the request
* *AND* the same MUST hold for a PostgreSQL `jsonb` column, a MySQL `JSON` column, and a Trino `json` column, so one omission path serves every driver
<!-- /DELTA:CHANGED -->
