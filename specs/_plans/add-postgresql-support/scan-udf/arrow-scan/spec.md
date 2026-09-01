# Feature: arrow_scan

Executes the pushed-down source statement through the ADBC driver the virtual schema names and streams the driver's Arrow record batches into Exasol without a row-by-row conversion step.

## Background

<!-- DELTA:CHANGED -->
- The scan UDF is registered as `ADBC_VS.ADBC_SCAN`, a Rust scalar emitting script taking one string argument: the scan specification the adapter produced.
- The adapter's pushdown SQL calls it once with a query-side dynamic `EMITS` clause, so a single UDF instance streams the whole result. Distributing a scan across nodes is not part of this release.
- This feature owns the scan contract every driver is held to: the entry symbol the shared object exports, credential resolution at run time and the non-persistence that follows from it, batch streaming, the declared-output contract, coercion and widening, and the driver-load error path. What is per technology is how a driver takes its credentials, stated by `scan-udf/mysql-scan` and `scan-udf/postgresql-scan`, and PostgreSQL's own coercion specifics, stated by `scan-udf/postgresql-scan`.
- The scan specification carries the driver identity, the BucketFS path of the driver shared library, the `CONNECTION` object's name, the statement to run, and the declared Exasol output columns. It carries no credential. The specification is generated fresh on every pushdown and never persisted, so adding the driver identity to it needs no migration.
- The scan is otherwise driver-agnostic: it executes a statement it does not parse and coerces batches by their declared Exasol types, so nothing in the streaming or coercion path branches on which source produced a batch. What is per driver is which library to load, which symbol to load it by, and how that library takes its credentials.
- The driver is loaded with the ADBC entry-point symbol the driver identity names — `AdbcDriverMysqlInit` or `AdbcDriverPostgresqlInit` — rather than one derived from the filename, because the derivation is a naming heuristic and the archive's `MANIFEST` may declare no entry point.
- A provisioned library is not necessarily a loadable one. The PostgreSQL driver is built in C++ and links `libstdc++` and `libgcc_s` at run time, where the Go-built MySQL driver links neither, and the UDF sandbox resolves a shared library's own dependencies from the Script Language Container rather than from BucketFS. A load failure over an unresolved dependency therefore reports a missing symbol or object rather than a missing file, and the two are diagnosed differently: one is fixed by provisioning, the other only in the container the `.so` runs inside.
- Credentials are read at scan time from the named Exasol `CONNECTION` object through the UDF SDK's connection accessor, for every driver. The resolved credential never reaches the scan specification, `adapterNotes`, SQL text, `EXPLAIN VIRTUAL` output, or any log line, and it does not outlive the connection attempt. Where it goes from there differs by driver, because the two drivers accept different connection options, and each driver's feature states its own mechanism and what that mechanism costs.
- A declared output column need not correspond to a source column. The adapter names every declared column positionally and reads the declared type of an aggregate expression from the `pushdown` request's `selectListDataTypes`, so the scan treats the declaration as the sole authority for the emitted schema and never infers a column's identity from the statement it was given.
- A declared column may be `BOOLEAN`, which no mapped source column produces: Exasol delegates a predicate as a select-list expression and declares it `BOOLEAN`, per `vs-adapter/pushdown-dialect`. MySQL returns such an expression as a number and PostgreSQL returns it as a boolean, so the declared type is again the authority and the driver's Arrow type is an input to convert.
- The drivers return the same logical value under different Arrow types, which is why the coercion is keyed on the declared type rather than on the returned one. The `adbc-drivers/mysql` driver returns `DATETIME` as `timestamp[s]`, `timestamp[ms]`, or `timestamp[us]` depending on how the statement is written; the PostgreSQL driver returns every timestamp at microsecond precision and every `numeric` as a UTF-8 string. A textual representation is therefore a first-class input to the decimal coercion rather than a fallback; `scan-udf/postgresql-scan` states what that driver's text form costs and where it fails.
- Arrow types do not cross the UDF shared-object boundary. Batches are serialized to Arrow IPC bytes inside the UDF.
<!-- /DELTA:CHANGED -->

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Scan connects using credentials resolved at run time

* *GIVEN* a scan specification naming an Exasol `CONNECTION` object, for each driver this release supports
* *WHEN* the scan runs
* *THEN* it MUST read the address, user, and password from that `CONNECTION` object at scan time
* *AND* it MUST NOT read any credential from the scan specification, and the resolved credential MUST NOT outlive the connection attempt
* *AND* the credential MUST NOT be written into the scan specification, into `adapterNotes`, into SQL text, into `EXPLAIN VIRTUAL` output, or into any log line
* *AND* the mechanism that passes it to a driver MUST be the one that driver's own feature states — `scan-udf/mysql-scan` or `scan-udf/postgresql-scan`
<!-- /DELTA:CHANGED -->

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
