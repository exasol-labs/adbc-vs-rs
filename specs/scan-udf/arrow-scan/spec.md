# Feature: arrow_scan

Executes the pushed-down source statement through the ADBC driver the virtual schema names and streams the driver's Arrow record batches into Exasol without a row-by-row conversion step.

## Background

- `ADBC_VS.ADBC_SCAN` is a Rust scalar emitting script whose one string argument, the scan specification, names the driver identity, the driver library's BucketFS path, the `CONNECTION` object, the statement, and the declared output columns.
- This feature owns the scan contract every driver is held to: the entry symbol, credential resolution, batch streaming, and the driver-load and source error paths.
- Each `<technology>-scan` feature states how its driver takes credentials and which Arrow representations it returns.
- `scan-udf/arrow-coercion` owns the declared-output contract, coercion, and widening.

## Scenarios

### Scenario: Shared object exports the scan entry symbol

* *GIVEN* a `.so` produced by `make cross-udf-build`
* *WHEN* its dynamic symbol table is inspected
* *THEN* it MUST export the scan entry symbol `__exa_udf_entry_ADBC_SCAN`

### Scenario: Scan connects using credentials resolved at run time

* *GIVEN* a scan specification naming an Exasol `CONNECTION` object, for each driver this release supports
* *WHEN* the scan runs
* *THEN* it MUST read the address, user, and password from that `CONNECTION` object through the UDF SDK at scan time
* *AND* it MUST NOT read any credential from the scan specification, and the resolved credential MUST NOT outlive the connection attempt
* *AND* the credential MUST NOT be written into the scan specification, into `adapterNotes`, into SQL text, into `EXPLAIN VIRTUAL` output, or into any log line
* *AND* the mechanism that passes it to a driver MUST be the one that driver's own `<technology>-scan` feature states, because the drivers accept different connection options, and each such feature also states what its mechanism costs

### Scenario: Scan streams every result batch to Exasol

* *GIVEN* a source table holding more rows than fit in one Arrow record batch, for each driver this release supports
* *WHEN* the scan executes the pushed-down statement
* *THEN* it MUST emit every batch the driver produces
* *AND* the total emitted row count MUST equal the source row count
* *AND* it MUST release each batch before requesting the next, so peak memory does not grow with the result size

### Scenario: Empty result set succeeds without emitting a row

* *GIVEN* a pushed-down statement matching no rows
* *WHEN* the scan runs
* *THEN* it MUST complete without error
* *AND* it MUST emit no row

### Scenario: Missing driver library fails with a named error

* *GIVEN* a scan specification whose driver path does not exist on the BucketFS mount
* *WHEN* the scan runs
* *THEN* it MUST return an error
* *AND* the error message MUST name the path it tried to load
* *AND* it MUST name the driver identity the specification carried, so an operator can tell which driver failed to install

### Scenario: Source failure surfaces the driver's own diagnosis

* *GIVEN* a source server that refuses the connection or rejects the statement, for each driver this release supports
* *WHEN* the scan runs
* *THEN* it MUST return an error
* *AND* the error message MUST include the ADBC status and the driver's message
* *AND* the error message MUST NOT contain the source password

### Scenario: Provisioned driver loads inside the UDF sandbox

* *GIVEN* each supported driver's shared library provisioned to its BucketFS path
* *WHEN* the scan UDF loads each library by its named entry point inside the Exasol UDF sandbox
* *THEN* each load MUST succeed
* *AND* each library MUST be loaded by the entry-point symbol its driver identity names rather than one derived from its filename, because that derivation is a naming heuristic, the archive's `MANIFEST` can declare no entry point, and one package exports two spellings of its own name
* *AND* the PostgreSQL library, built in C++, MUST load despite linking `libstdc++` and `libgcc_s`, which the Script Language Container supplies rather than BucketFS, while the Go-built MySQL library links neither
* *AND* a library whose own shared-library dependencies cannot be resolved MUST fail with an error naming the library path and the unresolved dependency, distinct from the error a missing file produces, because a missing file is fixed by provisioning and an unresolved dependency only in the container the `.so` runs inside

### Scenario: One scan instance streams the whole result as Arrow IPC bytes

* *GIVEN* a delegated query, for each driver this release supports
* *WHEN* the adapter's pushdown SQL runs the scan, which executes the statement without parsing it
* *THEN* the pushdown SQL MUST call the scan once, over one row, with a query-side dynamic `EMITS` clause, so one scan instance streams the whole result as Arrow IPC bytes it serializes inside the UDF with no row-by-row conversion step, because this release distributes no scan across nodes and Arrow types do not cross the UDF shared-object boundary
* *AND* the scan specification it passes MUST be generated fresh for that pushdown and never persisted, so a field added to it needs no migration
