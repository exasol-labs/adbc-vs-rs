<!-- DELTA:CHANGED -->
# Feature: mysql_scan

States how the scan reaches a MySQL source, passes its credentials, and pins its session time zone.
<!-- /DELTA:CHANGED -->

## Background

- This is the mechanism `scan-udf/arrow-scan`'s framework invariant is satisfied by for MySQL. That feature requires the credential to be resolved from the named Exasol `CONNECTION` object at scan time and never persisted; this feature states only where it goes from there.
- The `adbc-drivers/mysql` driver accepts `username` and `password` as ADBC database options distinct from `uri`, so the scan sets three options and no credential passes through a connection string.
- The `uri` option's value is the full driver URI the `CONNECTION` object's address carries — `mysql://host:port/database` — never a bare `host:port` and never a URI a credential was concatenated into. The scan names that value as the subject of a connection error, so a value holding no credential is what keeps the error safe to surface.
- Because no option value is parsed as a connection string, no password byte needs escaping and no password character can change how the driver reads the address. The guarantee is structural rather than a property of the escaping.
- The scan still strips the resolved password from a driver message before surfacing it, as defense in depth. Nothing upstream enforces that the driver's own diagnostics omit a value it was handed as an option.
- This shape is not one PostgreSQL can use: that driver rejects `username` and `password` as unknown options and accepts only `uri`. `scan-udf/postgresql-scan` records the divergence, why the driver's API forces it, and which invariants are asserted in its place.

## Scenarios

<!-- DELTA:NEW -->
### Scenario: Connection pins the session time zone to UTC

* *GIVEN* a MySQL source whose server default `time_zone` is not UTC
* *WHEN* the scan or the adapter opens the MySQL connection
* *THEN* the connection MUST set the session `time_zone` to `'+00:00'` before any other statement runs on it
* *AND* a later statement on the same connection MUST read `+00:00` as `@@session.time_zone`
* *AND* the `uri` option value MUST stay the address the `CONNECTION` object carries, so the pin adds no parameter to it
<!-- /DELTA:NEW -->
