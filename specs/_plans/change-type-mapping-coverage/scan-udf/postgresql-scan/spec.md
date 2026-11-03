# Feature: postgresql_scan

States how the scan reaches a PostgreSQL source, passes its credentials, and coerces the types its driver returns as text.

<!-- DELTA:CHANGED -->
## Background

- This feature states how the scan reaches a PostgreSQL source through the ADBC PostgreSQL driver, how it passes the credential, and how it coerces the values that driver returns as text.
- A conninfo string is libpq's keyword/value connection string, a list of `keyword='value'` pairs.
- `scan-udf/arrow-scan` owns the coercion contract every driver is held to, and `vs-adapter/postgresql-type-mapping` owns which `numeric` columns map to `DECIMAL` and which are read as text.
<!-- /DELTA:CHANGED -->

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Connection string is built from the address and escapes every credential byte

* *GIVEN* an Exasol `CONNECTION` whose address is `postgresql://host:5432/appdb?sslmode=require` and whose password contains a single quote, a backslash, a space, and an ampersand
* *WHEN* the scan builds the PostgreSQL connection value
* *THEN* the value MUST be a libpq keyword/value connection string rather than a URI carrying the credential, because conninfo escaping is total: a single-quoted value with an escaped quote or backslash holds any byte a password can hold, where percent-encoding would let a password containing `&`, `#`, `@`, or `/` change the parse rather than the value
* *AND* the host, port, and database MUST be taken from the address, and every parameter its query string carries MUST appear as a keyword of the same name, so an `sslmode` or `connect_timeout` the operator set is not silently dropped
* *AND* each value MUST be enclosed in single quotes with an embedded quote or backslash escaped, so the password reaches libpq as the exact bytes the `CONNECTION` object holds
* *AND* the connection MUST succeed against a source whose password contains each of those characters
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: The credential travels only inside the driver's uri option for one connection attempt

* *GIVEN* an Exasol `CONNECTION` addressing a PostgreSQL database
* *WHEN* the scan opens the source connection
* *THEN* the scan MUST pass the connection string as the driver's one `uri` option and pass no `username` or `password` option, because the ADBC PostgreSQL driver's `SetOption` accepts exactly `uri`, rejects `username` and `password` as unknown options, and hands `uri`'s value verbatim to libpq, so the separate credential options `scan-udf/mysql-scan` uses are impossible for this driver
* *AND* the string MUST be built in memory at connect time from the freshly resolved `CONNECTION` object and discarded with the connection attempt
* *AND* the string MUST NOT reach `EXPLAIN VIRTUAL` output or any log line, because passing the credential inside a connection string is the one deliberate narrowing of a security property, which the driver's API forces rather than a choice between designs, and the credential is still never persisted and never disclosed
<!-- /DELTA:NEW -->

<!-- DELTA:CHANGED -->
### Scenario: A numeric arriving as a string is coerced into its declared DECIMAL

* *GIVEN* a PostgreSQL `numeric(12,4)` column declared in Exasol as `DECIMAL(12,4)`, which the driver returns as a UTF-8 string
* *WHEN* the scan coerces that batch
* *THEN* every value MUST be emitted as the declared `DECIMAL(12,4)` with the source precision and scale intact
* *AND* the string form MUST be parsed as a first-class case rather than reached through a fallback, because this driver returns every `numeric` that way, documented as its only lossless representation
* *AND* a value whose digits do not fit the declared precision MUST fail with a named error rather than be truncated or rounded
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Non-finite numeric fails with a named error

* *GIVEN* a PostgreSQL `numeric(12,4)` column declared in Exasol as `DECIMAL(12,4)`, and a batch arriving with the driver's own spellings `nan`, `inf`, and `-inf` of PostgreSQL's `NaN`, `Infinity`, and `-Infinity`: `nan` reachable from a value the column actually stores, `inf` and `-inf` only from strings built for the test, since PostgreSQL itself rejects an infinite value into a constrained `numeric` with a numeric field overflow error
* *WHEN* the scan coerces that batch
* *THEN* each value MUST fail with an error naming the value it received and the declared Exasol type, because no Exasol `DECIMAL` can represent it, and the coercion MUST recognize the driver's spellings rather than PostgreSQL's own
* *AND* the scan MUST NOT emit a zero, a null, or a saturated value in its place
* *AND* a finite value in the same column MUST still coerce, so the failure is per value rather than per column
* *AND* a non-finite value of an unconstrained `numeric` column MUST be emitted as PostgreSQL's text for it, because that column maps to `VARCHAR` and is read as `CAST(<column> AS TEXT)`, per `vs-adapter/postgresql-type-mapping`
<!-- /DELTA:CHANGED -->
