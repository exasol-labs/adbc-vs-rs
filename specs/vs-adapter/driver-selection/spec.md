# Feature: driver_selection

Resolves the `DRIVER` property of a virtual schema to the one dialect, driver package, and metadata source that serve it, so every per-driver fact in the adapter is keyed on a single identity rather than restated per call site.

## Background

- The identity is a closed enum, one variant per supported driver, not a string passed around. Every per-driver table is keyed on that enum, so a Rust `match` over it is exhaustive and adding a variant fails the build at each table that needs a new row. That is the mechanism that makes "adding a driver adds an implementation rather than editing a renderer" true rather than aspirational. Release 0.2 supported one driver and named `MySqlDialect` at both of its dialect call sites, compared `DRIVER` against one string constant, and described the only driver there was with one set of pinned package constants; each of those was a place a later driver could be forgotten.
- Three tables are keyed on the identity, and they live in two crates because their consumers do. The driver package — version range, shared-library filename, registry driver path, and ADBC entry-point symbol — lives in `adbc-vs-scan`, because the scan UDF loads the library and the adapter provisions it, and both already read that module. The dialect and the metadata source live in `adbc-vs-adapter`, which the scan crate does not depend on. The enum itself lives with the package table, so the adapter's tables key on the lower crate's type and the dependency still points one way.
- Two of the three tables are *values* and one is *behavior*, and the difference decides where each is written. A driver package row, a dialect's advertised capability list, and the number of name segments that technology's source qualifier carries are values: one checked-in TOML file per technology holds all three, and the build generates the tables from them, so a technology's data is a data edit. A dialect's SQL rendering and a metadata source's catalog queries and type mapping are behavior, which no data format expresses, so they stay code. `SqlDialect::capabilities` remains a trait method that answers per dialect — only the values it returns move — so `vs-adapter/pushdown-dialect`'s per-dialect ownership is unchanged.
- The TOML files are compiled in, never read at run time. Nothing reads them from BucketFS or from a filesystem path while a query runs, so a malformed table fails the build rather than a query, and the generated tables keep the `'static` lifetime and exhaustive-`match` properties the enum exists for.
- The build validates the `[package]` table's fields and nothing else. A capability name is protocol vocabulary rather than a type this repo defines — `SqlDialect::capabilities` returns bare strings — so the build has nothing to check a name against, and an allowlist inside the build script would be the second capability list `vs-adapter/pushdown-dialect` forbids. An unknown capability name therefore reaches the generated table and is caught by that feature's invariant test, which renders every advertised capability and has no rendering path for it.
- The supported set is therefore fixed at build time, and the TOML files do not make it extensible. A technology needs a dialect and a metadata source as well as a package row, so a TOML file naming a technology with no code behind it could only fail. Adding a technology requires a new adbc-vs release carrying all three; what varies per virtual schema is which of the supported technologies it uses, not which technologies exist.
- The `DRIVER` property value is matched case-insensitively. The canonical lower-case spelling is what `adapterNotes` records and what an error message names.
- An unsupported value names every supported value rather than one. The message is generated from the enum, so a release adding a technology cannot leave it naming fewer drivers than the release supports.
- `getCapabilities` and `pushdown` MUST resolve the same identity from the same field, or the adapter can advertise one dialect's capabilities and render another's — the wrong-rows failure `vs-adapter/pushdown-dialect` exists to prevent, reintroduced one layer up. Both read `schemaMetadataInfo.adapterNotes`, whose `driver` field `createVirtualSchema` records.
- That a `getCapabilities` request carries `schemaMetadataInfo.adapterNotes` is what makes a per-dialect capability set legal at all. It is held in place through what the adapter does rather than through what it receives: virtual schemas over different drivers are served different capability sets, which stays observable only while the identity reaches the request. Nothing asserts the raw request body, because an adapter that echoed what it received would be a disclosure surface added for a test.
- The connection address's URI scheme is checked against the resolved driver. `vs-adapter/create-virtual-schema` takes the source qualifier from the address's path component and passes the address to the driver, so a `postgresql://` address under `DRIVER = 'mysql'` reaches the MySQL driver and fails inside it, with a message about the source rather than about the mismatch the operator made. Each driver's scheme is its own canonical name, so the check reads the identity rather than a table of schemes.
- Driver identity does not reach the source SQL. The dialect renders identifiers, literals, qualifiers, and collations; the metadata source queries the source's catalog. Nothing else in the adapter branches on which driver is in use, and a module that would need to is a module that should be behind one of the three tables instead.

## Scenarios

### Scenario: Each supported DRIVER value resolves to its own dialect and package

* *GIVEN* the `DRIVER` property values `mysql`, `postgresql`, and `trino`
* *WHEN* each value is resolved to a driver identity
* *THEN* each MUST resolve to a distinct identity
* *AND* each identity MUST yield the dialect, the driver package, and the metadata source belonging to that driver
* *AND* no two identities MUST yield the same shared-library filename or the same registry driver path, because a shared value would make each driver's presence check answer for the other
* *AND* the values MUST be enumerated from the driver identities rather than listed in the test, so a release adding a technology extends this coverage without editing it

### Scenario: DRIVER value is matched case-insensitively and recorded in canonical form

* *GIVEN* the `DRIVER` property values `postgresql`, `PostgreSQL`, and `POSTGRESQL`
* *WHEN* each is resolved
* *THEN* each MUST resolve to the same driver identity
* *AND* the identity's canonical spelling MUST be the lower-case form
* *AND* `adapterNotes` MUST record that canonical spelling rather than the spelling the statement used, so a value read back is independent of how it was typed

### Scenario: Unsupported driver value names every supported value

* *GIVEN* a `DRIVER` property value of `snowflake`
* *WHEN* the value is resolved
* *THEN* the resolution MUST fail with an error
* *AND* the error message MUST name the value it received
* *AND* the error message MUST list every driver value this release supports
* *AND* that list MUST be generated from the driver identities rather than written into the message, so a release adding a driver cannot leave the message naming fewer than it supports

### Scenario: getCapabilities resolves the driver from the request

* *GIVEN* a `getCapabilities` request whose `schemaMetadataInfo.adapterNotes` records the driver `postgresql`
* *WHEN* the adapter builds the capability response
* *THEN* the advertised set MUST be the PostgreSQL dialect's own set
* *AND* a request recording `mysql` MUST advertise the MySQL dialect's own set
* *AND* the adapter MUST NOT return one set for both, because Exasol delegates against the set it was given and the dialect that renders is chosen from the same field

### Scenario: Each virtual schema is served the capability set its own driver advertises

* *GIVEN* one virtual schema created through `CREATE VIRTUAL SCHEMA` over the shipped adapter for a driver whose dialect advertises `JOIN_TYPE_FULL_OUTER` and one for a driver that withholds it
* *WHEN* Exasol plans a `FULL OUTER JOIN` over each schema
* *THEN* the statement generated for each schema whose dialect advertises `JOIN_TYPE_FULL_OUTER` MUST carry the delegated `FULL OUTER JOIN`
* *AND* no statement generated for a schema whose dialect withholds that capability MUST carry a join, because Exasol joins the separately delegated table scans itself
* *AND* at least one schema MUST fall on each side of that difference, or the test passes without observing anything
* *AND* the observed difference MUST come from the shipped adapter alone, with no probe adapter, no request-echo instrumentation, and no test-only code path through it, because a capability set observed anywhere but the production path says nothing about the production path
* *AND* a departure from this shape MUST fail the test rather than be absorbed, because a per-dialect capability set is only sound while `getCapabilities` resolves the driver identity each schema recorded

### Scenario: getCapabilities without adapter notes fails rather than guessing a driver

* *GIVEN* a `getCapabilities` request carrying no `schemaMetadataInfo.adapterNotes`
* *WHEN* the adapter builds the capability response
* *THEN* the adapter MUST return an error naming the missing field
* *AND* the adapter MUST NOT fall back to a default driver, because advertising one dialect's set for another's source returns wrong rows for every capability the two do not share

### Scenario: Pushdown and capabilities resolve the driver from one field

* *GIVEN* a `getCapabilities` request and a `pushdown` request carrying the same `adapterNotes`
* *WHEN* the adapter handles each
* *THEN* both MUST resolve the driver identity from `schemaMetadataInfo.adapterNotes`
* *AND* the dialect the pushdown renders through MUST be the dialect whose set the capability response advertised
* *AND* neither handler MUST name a concrete dialect, so no call site can pin one driver's renderer to another's advertised set

### Scenario: Connection address scheme must match the named driver

* *GIVEN* a `CREATE VIRTUAL SCHEMA` statement naming `DRIVER = 'mysql'` and a `CONNECTION` object whose address begins `postgresql://`
* *WHEN* the adapter handles the request
* *THEN* the adapter MUST return an error
* *AND* the error message MUST name both the driver the statement asked for and the scheme the address carries
* *AND* the adapter MUST NOT open the connection, because the driver would report a source-level failure for what is a property mismatch
* *AND* an address whose scheme matches the named driver MUST be accepted unchanged

### Scenario: Scan specification carries the driver identity

* *GIVEN* a `pushdown` response whose scan specification is read back
* *WHEN* the scan UDF loads the driver named in it
* *THEN* the specification MUST carry the driver identity alongside the driver path
* *AND* the scan MUST load the library with the ADBC entry-point symbol that identity names
* *AND* the scan MUST NOT derive the entry-point symbol from the library filename, which is the naming heuristic `scan-udf/arrow-scan` already rejects

### Scenario: Each technology's package row and capability list come from its own TOML file

* *GIVEN* one checked-in TOML file per supported technology, each holding a `[package]` table, a `[capabilities]` list, and the number of name segments that technology's source qualifier carries
* *WHEN* the crate is built
* *THEN* each driver identity's package row, advertised capability set, and qualifier segment count MUST be generated from that technology's own file
* *AND* the qualifier segment count MUST have exactly one owner, read both by the check that validates a connection address's path and by the assertion that a metadata source produced the right number of segments, because two places deciding how deeply a technology nests its tables can disagree
* *AND* the generated values MUST be compiled into the build output, with no TOML file read at query time from BucketFS or from any filesystem path
* *AND* `SqlDialect::capabilities` MUST remain the per-dialect trait method that answers the question, so the values moving to data does not move the ownership

### Scenario: A malformed or incomplete driver TOML file fails the build

* *GIVEN* a driver TOML file missing a required `[package]` field, or holding one of the wrong type
* *WHEN* the crate is built
* *THEN* the build MUST fail
* *AND* the failure MUST name the file and the field at fault
* *AND* the failure MUST occur at build time rather than when a query resolves that driver, because a driver table that cannot be trusted is not a query-time condition
* *AND* the build MUST NOT check a capability name against a list of its own, because capability names are protocol vocabulary and a list in the build script is the second capability list `vs-adapter/pushdown-dialect` forbids

### Scenario: A capability name no dialect renders fails the invariant test rather than the build

* *GIVEN* a driver TOML file whose `[capabilities]` list names a capability that technology's dialect cannot render
* *WHEN* the capability invariant test runs over every driver identity
* *THEN* the test MUST fail, naming the capability that has no rendering path
* *AND* the build MUST succeed, because no capability type exists for the build script to match the name against
* *AND* the guard MUST stay the invariant test rather than becoming a build check, so the adapter keeps exactly one list of capability names

### Scenario: Adding a technology requires code, not only a TOML file

* *GIVEN* the set of supported driver identities
* *WHEN* the adapter resolves a `DRIVER` value
* *THEN* the accepted set MUST be fixed at build time, enumerated from the closed identity type
* *AND* the adapter MUST NOT discover a technology by scanning a directory, reading a configuration file at run time, or loading a plugin
* *AND* a technology MUST NOT be resolvable without a dialect and a metadata source compiled in for it, because a package row alone cannot render SQL or read a catalog

### Scenario: Every driver identity has a complete set of tables

* *GIVEN* every driver identity this release supports
* *WHEN* each identity is resolved through each of the three per-driver tables
* *THEN* every identity MUST yield a dialect, a driver package, and a metadata source
* *AND* no identity MUST reach a fallback, a placeholder, or a panic
* *AND* the test MUST enumerate the identities from the type rather than from a list of its own, so a release adding a driver extends the test's coverage without editing it
