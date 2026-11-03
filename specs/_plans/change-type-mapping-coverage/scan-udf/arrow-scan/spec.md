# Feature: arrow_scan

Executes the pushed-down source statement through the ADBC driver the virtual schema names and streams the driver's Arrow record batches into Exasol without a row-by-row conversion step.

<!-- DELTA:CHANGED -->
## Background

- `ADBC_VS.ADBC_SCAN` is a Rust scalar emitting script whose one string argument, the scan specification, names the driver identity, the driver library's BucketFS path, the `CONNECTION` object, the statement, and the declared output columns.
- This feature owns the scan contract every driver is held to: the entry symbol, credential resolution, batch streaming, the declared-output contract, coercion and widening, and the driver-load error path.
- Each `<technology>-scan` feature states how its driver takes credentials and which Arrow representations it returns.
<!-- /DELTA:CHANGED -->

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Scan connects using credentials resolved at run time

* *GIVEN* a scan specification naming an Exasol `CONNECTION` object, for each driver this release supports
* *WHEN* the scan runs
* *THEN* it MUST read the address, user, and password from that `CONNECTION` object through the UDF SDK at scan time
* *AND* it MUST NOT read any credential from the scan specification, and the resolved credential MUST NOT outlive the connection attempt
* *AND* the credential MUST NOT be written into the scan specification, into `adapterNotes`, into SQL text, into `EXPLAIN VIRTUAL` output, or into any log line
* *AND* the mechanism that passes it to a driver MUST be the one that driver's own `<technology>-scan` feature states, because the drivers accept different connection options, and each such feature also states what its mechanism costs
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: One scan instance streams the whole result as Arrow IPC bytes

* *GIVEN* a delegated query, for each driver this release supports
* *WHEN* the adapter's pushdown SQL runs the scan, which executes the statement without parsing it
* *THEN* the pushdown SQL MUST call the scan once, over one row, with a query-side dynamic `EMITS` clause, so one scan instance streams the whole result as Arrow IPC bytes it serializes inside the UDF with no row-by-row conversion step, because this release distributes no scan across nodes and Arrow types do not cross the UDF shared-object boundary
* *AND* the scan specification it passes MUST be generated fresh for that pushdown and never persisted, so a field added to it needs no migration
<!-- /DELTA:NEW -->

<!-- DELTA:CHANGED -->
### Scenario: Emitted columns match the declared output

* *GIVEN* a scan specification declaring three Exasol output columns
* *WHEN* the scan emits its batches
* *THEN* each emitted batch MUST carry exactly those three columns in the declared order
* *AND* each column's values MUST be coerced to the representation its declared Exasol type requires, keyed on the declared type rather than on the Arrow type the driver returned or on which source produced the batch, because the drivers return one logical value under different Arrow types
* *AND* the declaration MUST be the sole authority for the emitted schema, and the scan MUST NOT infer a column's identity from the statement, because a declared column need not correspond to a source column: the adapter names columns positionally and reads an aggregate's type from `selectListDataTypes`
* *AND* a declared `CHAR` column MUST be emitted as text, which Exasol pads to the declared size
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: A declared TIMESTAMP(p) is emitted at microsecond resolution

* *GIVEN* a PostgreSQL `timestamp(6)` column and a MySQL `DATETIME(6)` column, each declared in Exasol as `TIMESTAMP(6)`
* *WHEN* the scan coerces the batches each driver returns
* *THEN* each declared `TIMESTAMP(p)` MUST be emitted at microsecond resolution, which holds every precision the PostgreSQL and MySQL mappings declare
* *AND* the MySQL column MUST emit the same values whatever Arrow unit the driver returns it at, because the `adbc-drivers/mysql` driver returns `DATETIME` as `timestamp[s]`, `timestamp[ms]`, or `timestamp[us]` depending on how the statement is written
<!-- /DELTA:NEW -->

<!-- DELTA:CHANGED -->
### Scenario: Declared BOOLEAN column emits every representation the driver returns

* *GIVEN* a declared `BOOLEAN` output column for a PostgreSQL `boolean` or `bit` column, a MySQL `BIT` column, or a predicate Exasol delegates as a select-list expression and declares `BOOLEAN`, per `vs-adapter/pushdown-dialect`, and a driver that returns the value as an Arrow boolean, as an integer 0 and 1, or as a null
* *WHEN* the scan coerces that batch
* *THEN* every value MUST be emitted as an Arrow boolean, with a non-zero integer emitted as true and a zero as false
* *AND* a null MUST stay a null rather than becoming false, because both sources evaluate a comparison against a null operand to null and Exasol does the same
* *AND* the scan MUST NOT fail merely because the driver returned the column as a number
* *AND* both representations MUST be reached in practice, because MySQL returns a delegated predicate as a number and PostgreSQL and Trino return it as a boolean
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Provisioned driver loads inside the UDF sandbox

* *GIVEN* each supported driver's shared library provisioned to its BucketFS path
* *WHEN* the scan UDF loads each library by its named entry point inside the Exasol UDF sandbox
* *THEN* each load MUST succeed
* *AND* each library MUST be loaded by the entry-point symbol its driver identity names rather than one derived from its filename, because that derivation is a naming heuristic, the archive's `MANIFEST` can declare no entry point, and one package exports two spellings of its own name
* *AND* the PostgreSQL library, built in C++, MUST load despite linking `libstdc++` and `libgcc_s`, which the Script Language Container supplies rather than BucketFS, while the Go-built MySQL library links neither
* *AND* a library whose own shared-library dependencies cannot be resolved MUST fail with an error naming the library path and the unresolved dependency, distinct from the error a missing file produces, because a missing file is fixed by provisioning and an unresolved dependency only in the container the `.so` runs inside
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: Narrow and unsigned integers coerce into their declared DECIMAL

* *GIVEN* declared `DECIMAL(3,0)`, `DECIMAL(5,0)`, `DECIMAL(8,0)`, `DECIMAL(10,0)`, and `DECIMAL(20,0)` output columns, and a driver that returns them as signed or unsigned 8-, 16-, 32-, and 64-bit integers
* *WHEN* the scan coerces that batch
* *THEN* every value MUST be emitted as its declared `DECIMAL` with its digits intact
* *AND* the unsigned 64-bit value 18446744073709551615 MUST be emitted as `DECIMAL(20,0)` rather than wrap to a negative value or fail
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: A time-zone-aware timestamp is emitted as its UTC wall-clock value

* *GIVEN* a declared `TIMESTAMP(6)` output column and a driver that returns it as an Arrow microsecond timestamp carrying the time zone `UTC`, holding the instant `2024-10-27 00:30:00.123456` UTC
* *WHEN* the scan coerces that batch
* *THEN* the scan MUST emit `2024-10-27 00:30:00.123456`, the instant's UTC wall-clock value
* *AND* the scan MUST emit the same value when the Arrow time zone is spelled `+00:00`, and when it is another zone such as `Europe/Berlin`, because the Arrow value is the UTC instant whatever zone labels it
<!-- /DELTA:NEW -->
