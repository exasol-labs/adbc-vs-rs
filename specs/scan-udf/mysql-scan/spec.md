# Feature: mysql_scan

States how the scan reaches a MySQL source and passes its credentials.

## Background

- This is the mechanism `scan-udf/arrow-scan`'s framework invariant is satisfied by for MySQL. That feature requires the credential to be resolved from the named Exasol `CONNECTION` object at scan time and never persisted; this feature states only where it goes from there.
- The `adbc-drivers/mysql` driver accepts `username` and `password` as ADBC database options distinct from `uri`, so the scan sets three options and no credential passes through a connection string.
- The `uri` option's value is the full driver URI the `CONNECTION` object's address carries — `mysql://host:port/database` — never a bare `host:port` and never a URI a credential was concatenated into. The scan names that value as the subject of a connection error, so a value holding no credential is what keeps the error safe to surface.
- Because no option value is parsed as a connection string, no password byte needs escaping and no password character can change how the driver reads the address. The guarantee is structural rather than a property of the escaping.
- The scan still strips the resolved password from a driver message before surfacing it, as defense in depth. Nothing upstream enforces that the driver's own diagnostics omit a value it was handed as an option.
- This shape is not one PostgreSQL can use: that driver rejects `username` and `password` as unknown options and accepts only `uri`. `scan-udf/postgresql-scan` records the divergence, why the driver's API forces it, and which invariants are asserted in its place.

## Scenarios

### Scenario: Credentials pass as separate driver options

* *GIVEN* a scan specification naming an Exasol `CONNECTION` object whose address is `mysql://host:3306/appdb` and whose user and password fields hold values
* *WHEN* the scan opens the MySQL ADBC connection
* *THEN* the scan MUST pass the address as the `uri` database option and the user and password as the separate `username` and `password` database options
* *AND* it MUST NOT concatenate either credential into the `uri` value
* *AND* the connection MUST succeed against a source whose password contains a single quote, a backslash, a space, or an ampersand, because no option value is parsed as a connection string

### Scenario: Connection string carries no credential

* *GIVEN* a resolved MySQL `CONNECTION` object whose password field holds a value
* *WHEN* the scan builds the `uri` option value
* *THEN* the value MUST carry the host, the port, and the database and nothing else
* *AND* no text derived from that value MUST contain the user or the password, including an error naming the URI it could not reach
* *AND* the scan MUST still strip the resolved password from a driver message it surfaces, as defense in depth rather than as the mechanism this guarantee rests on
