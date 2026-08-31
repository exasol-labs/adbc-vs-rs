# Feature: arrow_scan

Executes the pushed-down source statement through the ADBC driver the virtual schema names and streams the driver's Arrow record batches into Exasol without a row-by-row conversion step.

## Background

<!-- DELTA:CHANGED -->
- The scan UDF is registered as `ADBC_VS.ADBC_SCAN`, a Rust scalar emitting script taking one string argument: the scan specification the adapter produced.
- The adapter's pushdown SQL calls it once with a query-side dynamic `EMITS` clause, so a single UDF instance streams the whole result. Distributing a scan across nodes is not part of this release.
- The scan specification carries the driver identity, the BucketFS path of the driver shared library, the `CONNECTION` object's name, the statement to run, and the declared Exasol output columns. It carries no credential. The specification is generated fresh on every pushdown and never persisted, so adding the driver identity to it needs no migration.
- The scan is otherwise driver-agnostic: it executes a statement it does not parse and coerces batches by their declared Exasol types, so nothing in the streaming or coercion path branches on which source produced a batch. What is per driver is which library to load, which symbol to load it by, and how that library takes its credentials.
- The driver is loaded with the ADBC entry-point symbol the driver identity names — `AdbcDriverMysqlInit` or `AdbcDriverPostgresqlInit` — rather than one derived from the filename, because the derivation is a naming heuristic and the archive's `MANIFEST` may declare no entry point.
- A provisioned library is not necessarily a loadable one. The PostgreSQL driver is built in C++ and links `libstdc++` and `libgcc_s` at run time, where the Go-built MySQL driver links neither, and the UDF sandbox resolves a shared library's own dependencies from the Script Language Container rather than from BucketFS. A load failure over an unresolved dependency therefore reports a missing symbol or object rather than a missing file, and the two are diagnosed differently: one is fixed by provisioning, the other only in the container the `.so` runs inside.
- Credentials are read at scan time from the named Exasol `CONNECTION` object through the UDF SDK's connection accessor, for every driver. What differs is where they go from there, because the two drivers accept different connection options.
- The MySQL driver takes the address as the ADBC `uri` option and the user and password as separate `username` and `password` options, so no credential passes through a connection string.
- The PostgreSQL driver accepts exactly one database option, `uri`, and hands its value verbatim to libpq's connect call; `username` and `password` are rejected as unknown options. For PostgreSQL the credential therefore has to travel inside that one option's value, and the invariant that no credential passes through a connection string cannot be preserved for it. What is preserved is every invariant about persistence and disclosure: the value is built in memory at the moment of connecting from the freshly resolved `CONNECTION` object, it is never written into the scan specification, into `adapterNotes`, into SQL text, into `EXPLAIN VIRTUAL` output, or into any log line or error message, and it is discarded with the connection attempt.
- The PostgreSQL connection value is built as a libpq keyword/value connection string rather than by injecting the credential into the `postgresql://` URI. libpq's connect call accepts either form, and the keyword form escapes a value by wrapping it in single quotes and backslash-escaping an embedded quote or backslash, which is a rule that holds for every byte a password may contain. URI form would instead require percent-encoding, where a password containing `&`, `#`, `@`, or `/` changes the parse rather than the value. Every parameter the operator put in the address's query string carries across as a keyword of the same name, so a `sslmode` or `connect_timeout` the address sets is not silently dropped.
- Redaction is asserted rather than assumed, because a driver's own diagnostic text is outside the adapter's control. The PostgreSQL driver's error messages quote libpq, which can echo connection parameters, so the scan's error path strips the resolved password from any message it surfaces rather than trusting the driver to omit it.
- A declared output column need not correspond to a source column. The adapter names every declared column positionally and reads the declared type of an aggregate expression from the `pushdown` request's `selectListDataTypes`, so the scan treats the declaration as the sole authority for the emitted schema and never infers a column's identity from the statement it was given.
- A declared column may be `BOOLEAN`, which no mapped source column produces: Exasol delegates a predicate as a select-list expression and declares it `BOOLEAN`, per `vs-adapter/pushdown-dialect`. MySQL returns such an expression as a number and PostgreSQL returns it as a boolean, so the declared type is again the authority and the driver's Arrow type is an input to convert.
- The two drivers return the same logical value under different Arrow types, which is why the coercion is keyed on the declared type rather than on the returned one. The `adbc-drivers/mysql` driver returns `DATETIME` as `timestamp[s]`, `timestamp[ms]`, or `timestamp[us]` depending on how the statement is written; the PostgreSQL driver returns every timestamp at microsecond precision regardless of the column's declared precision. The PostgreSQL driver returns every `numeric` — constrained or not — as a UTF-8 string, documented as the only lossless representation, so a column declared `DECIMAL(p,s)` arrives as text and is parsed rather than cast from a decimal.
- A PostgreSQL `numeric` legitimately holds `NaN`, `Infinity`, and `-Infinity`, none of which any Exasol `DECIMAL` can represent. Those values reach the scan as the strings the driver produces, so they are a failure the coercion names rather than a cast that silently yields a zero or a null.
- Arrow types do not cross the UDF shared-object boundary. Batches are serialized to Arrow IPC bytes inside the UDF.
<!-- /DELTA:CHANGED -->

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Scan connects using credentials resolved at run time

* *GIVEN* a scan specification naming an Exasol `CONNECTION` object, for each driver this release supports
* *WHEN* the scan runs
* *THEN* it MUST read the address, user, and password from that `CONNECTION` object
* *AND* it MUST NOT read any credential from the scan specification, and the resolved credential MUST NOT outlive the connection attempt
* *AND* the MySQL driver MUST receive the user and password as separate ADBC options rather than inside the connection string
* *AND* the PostgreSQL driver MUST receive them inside the single `uri` option it accepts, because it rejects a `username` or `password` option as unknown
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: PostgreSQL connection string is built from the address and escapes every credential byte

* *GIVEN* an Exasol `CONNECTION` whose address is `postgresql://host:5432/appdb?sslmode=require` and whose password contains a single quote, a backslash, a space, and an ampersand
* *WHEN* the scan builds the PostgreSQL connection value
* *THEN* the value MUST be a libpq keyword/value connection string rather than a URI carrying the credential
* *AND* the host, port, and database MUST be taken from the address, and every parameter its query string carries MUST appear as a keyword of the same name, so `sslmode` is not dropped
* *AND* each value MUST be enclosed in single quotes with an embedded quote or backslash escaped, so the password reaches libpq as the exact bytes the `CONNECTION` object holds
* *AND* the connection MUST succeed against a source whose password contains each of those characters
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: PostgreSQL credential never appears in any surfaced text

* *GIVEN* a scan against a PostgreSQL source, and a second scan whose `CONNECTION` object holds a wrong password
* *WHEN* the first scan's generated SQL, scan specification, and `adapterNotes` are read back, and the second scan's error is surfaced
* *THEN* none of the generated SQL, the scan specification, or `adapterNotes` MUST contain the password or the user
* *AND* the error the failing scan surfaces MUST NOT contain the password, including within any text the driver or libpq produced
* *AND* the error MUST still name the host and the database it tried to reach, so a wrong credential stays diagnosable
* *AND* the redaction MUST be applied by the scan rather than relied on from the driver
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: Provisioned driver loads inside the UDF sandbox

* *GIVEN* each supported driver's shared library provisioned to its BucketFS path
* *WHEN* the scan UDF loads each library by its named entry point inside the Exasol UDF sandbox
* *THEN* each load MUST succeed
* *AND* the PostgreSQL library MUST load despite linking `libstdc++` and `libgcc_s`, which the Script Language Container supplies rather than BucketFS
* *AND* a library whose own shared-library dependencies cannot be resolved MUST fail with an error naming the library path and the unresolved dependency, distinct from the error a missing file produces
<!-- /DELTA:NEW -->

<!-- DELTA:CHANGED -->
### Scenario: Decimal width variation is normalized

* *GIVEN* a source `DECIMAL(12,4)` or `numeric(12,4)` column declared in Exasol as `DECIMAL(12,4)`
* *WHEN* the driver returns it as `decimal64`, as `decimal128`, as a floating-point value, or as a string
* *THEN* the scan MUST emit the declared `DECIMAL(12,4)` values with the source precision and scale intact
* *AND* it MUST fail with a named error rather than truncate when a returned value does not fit the declared precision
* *AND* the string representation MUST be handled as a first-class case rather than a fallback, because the PostgreSQL driver returns every `numeric` as a string
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: Non-finite PostgreSQL numeric fails with a named error

* *GIVEN* a PostgreSQL `numeric(12,4)` column declared in Exasol as `DECIMAL(12,4)`, holding the values `NaN`, `Infinity`, and `-Infinity`
* *WHEN* the scan coerces that batch
* *THEN* each value MUST fail with an error naming the value it received and the declared Exasol type
* *AND* the scan MUST NOT emit a zero, a null, or a saturated value in its place
* *AND* a finite value in the same column MUST still coerce, so the failure is per value rather than per column
<!-- /DELTA:NEW -->

<!-- DELTA:CHANGED -->
### Scenario: Timestamp unit variation is normalized

* *GIVEN* a source timestamp column declared in Exasol as `TIMESTAMP`
* *WHEN* the driver returns that column as `timestamp[s]`, as `timestamp[ms]`, or as `timestamp[us]`
* *THEN* the scan MUST emit the same Exasol `TIMESTAMP` values in every case
* *AND* the scan MUST NOT fail on a unit it did not expect
* *AND* a PostgreSQL timestamp column MUST emit the same values whatever fractional-seconds precision the column declares, because that driver returns every timestamp at microsecond precision
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Source failure surfaces the driver's own diagnosis

* *GIVEN* a source server that refuses the connection or rejects the statement, for each driver this release supports
* *WHEN* the scan runs
* *THEN* it MUST return an error
* *AND* the error message MUST include the ADBC status and the driver's message
* *AND* the error message MUST NOT contain the source password
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Missing driver library fails with a named error

* *GIVEN* a scan specification whose driver path does not exist on the BucketFS mount
* *WHEN* the scan runs
* *THEN* it MUST return an error
* *AND* the error message MUST name the path it tried to load
* *AND* it MUST name the driver identity the specification carried, so an operator can tell which driver failed to install
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Scan streams every result batch to Exasol

* *GIVEN* a source table holding more rows than fit in one Arrow record batch, for each driver this release supports
* *WHEN* the scan executes the pushed-down statement
* *THEN* it MUST emit every batch the driver produces
* *AND* the total emitted row count MUST equal the source row count
* *AND* it MUST release each batch before requesting the next, so peak memory does not grow with the result size
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Declared BOOLEAN column emits every representation the driver returns

* *GIVEN* a declared `BOOLEAN` output column for a delegated predicate, and a driver that returns the projected comparison as an Arrow boolean, as an integer 0 and 1, or as a null
* *WHEN* the scan coerces that batch
* *THEN* every value MUST be emitted as an Arrow boolean, with a non-zero integer emitted as true and a zero as false
* *AND* a null MUST stay a null rather than becoming false, because both sources evaluate a comparison against a null operand to null and Exasol does the same
* *AND* the scan MUST NOT fail merely because the driver returned the column as a number
* *AND* both representations MUST be reached in practice, because MySQL returns such a comparison as a number and PostgreSQL returns it as a boolean
<!-- /DELTA:CHANGED -->
