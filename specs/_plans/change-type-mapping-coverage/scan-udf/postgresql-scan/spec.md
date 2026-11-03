# Feature: postgresql_scan

States how the scan reaches a PostgreSQL source, passes its credentials, and coerces the types its driver returns as text.

<!-- DELTA:CHANGED -->
## Background

- The ADBC PostgreSQL driver's `SetOption` accepts exactly `uri`, rejects `username` and `password` as unknown options, and hands `uri`'s value verbatim to libpq. The credential therefore travels inside that one option, as a libpq keyword/value connection string built in memory at connect time from the freshly resolved `CONNECTION` object. Conninfo form is used rather than URI form because its escaping is total: a single-quoted value with an escaped quote or backslash holds any byte a password can hold, and every parameter of the address's query string crosses over as a keyword of the same name.
- The connection string is built when connecting and discarded with the attempt, and it reaches no scan specification, `adapterNotes`, SQL text, `EXPLAIN VIRTUAL` output, log line, or error message. The adapter strips the resolved password from every surfaced message, including the subject it names, and the redaction binds `createVirtualSchema`, `refresh`, and `setProperties` too, because they share the connection-open path. This is the one deliberate narrowing of a security property, forced by the driver's API.
- The driver returns every `numeric` as a UTF-8 string, documented as its only lossless representation, so a column declared `DECIMAL(p,s)` arrives as text and is parsed as the ordinary case rather than a fallback.
- PostgreSQL `numeric` holds `NaN`, `Infinity`, and `-Infinity`, which the driver spells `nan`, `inf`, and `-inf`. A constrained `numeric(p,s)` can hold `nan` but not an infinity, and a `nan` reaching a declared `DECIMAL` is a failure the coercion names rather than a zero, a null, or a saturated value. An unconstrained `numeric` maps to `VARCHAR` and is read as `CAST(<column> AS TEXT)`, per `vs-adapter/postgresql-type-mapping`, so its non-finite values reach the scan as PostgreSQL's text and are emitted as text.
<!-- /DELTA:CHANGED -->

## Scenarios

### Scenario: Connection string is built from the address and escapes every credential byte

* *GIVEN* an Exasol `CONNECTION` whose address is `postgresql://host:5432/appdb?sslmode=require` and whose password contains a single quote, a backslash, a space, and an ampersand
* *WHEN* the scan builds the PostgreSQL connection value
* *THEN* the value MUST be a libpq keyword/value connection string rather than a URI carrying the credential
* *AND* the host, port, and database MUST be taken from the address, and every parameter its query string carries MUST appear as a keyword of the same name, so `sslmode` is not dropped
* *AND* each value MUST be enclosed in single quotes with an embedded quote or backslash escaped, so the password reaches libpq as the exact bytes the `CONNECTION` object holds
* *AND* the connection MUST succeed against a source whose password contains each of those characters
