# Feature: arrow_scan

Executes the pushed-down MySQL statement through the ADBC driver and streams the driver's Arrow record batches into Exasol without a row-by-row conversion step.

## Background

- The scan UDF is registered as `ADBC_VS.ADBC_SCAN`, a Rust scalar emitting script taking one string argument: the scan specification the adapter produced.
- The adapter's pushdown SQL calls it once with a query-side dynamic `EMITS` clause, so a single UDF instance streams the whole result. Distributing a scan across nodes is not part of this release.
- The scan specification carries the BucketFS path of the driver shared library, the `CONNECTION` object's name, the MySQL statement to run, and the declared Exasol output columns. It carries no credential.
- A declared output column need not correspond to a source column. The adapter names every declared column positionally and reads the declared type of an aggregate expression from the `pushdown` request's `selectListDataTypes`, so the scan treats the declaration as the sole authority for the emitted schema and never infers a column's identity from the statement it was given.
- A declared column may be `BOOLEAN`, which no mapped MySQL column produces: Exasol delegates a predicate as a select-list expression and declares it `BOOLEAN`, per `vs-adapter/pushdown-dialect`. MySQL returns such an expression as a number, so the declared type is again the authority and the driver's Arrow type is an input to convert.
- Credentials are read at scan time from the named Exasol `CONNECTION` object through the UDF SDK's connection accessor. The address supplies the ADBC `uri` option; the user and password supply the `username` and `password` options, so no credential is ever embedded in the connection URI.
- The driver is loaded with an explicitly named ADBC entry point rather than one derived from the filename, because the derivation is a naming heuristic and the archive's `MANIFEST` may declare no entry point.
- The `adbc-drivers/mysql` driver returns `DATETIME` as `timestamp[s]`, `timestamp[ms]`, or `timestamp[us]` depending on how the statement is written, so the emitted batch is coerced to the declared Exasol type rather than trusted to match it.
- Arrow types do not cross the UDF shared-object boundary. Batches are serialized to Arrow IPC bytes inside the UDF.

## Scenarios

### Scenario: Shared object exports the scan entry symbol

* *GIVEN* a `.so` produced by `make cross-udf-build`
* *WHEN* its dynamic symbol table is inspected
* *THEN* it MUST export the scan entry symbol `__exa_udf_entry_ADBC_SCAN`

### Scenario: Scan connects using credentials resolved at run time

* *GIVEN* a scan specification naming an Exasol `CONNECTION` object
* *WHEN* the scan runs
* *THEN* it MUST read the address, user, and password from that `CONNECTION` object
* *AND* it MUST pass the user and password as separate ADBC options rather than embedding them in the URI
* *AND* it MUST NOT read any credential from the scan specification

### Scenario: Scan streams every result batch to Exasol

* *GIVEN* a MySQL table holding more rows than fit in one Arrow record batch
* *WHEN* the scan executes the pushed-down statement
* *THEN* it MUST emit every batch the driver produces
* *AND* the total emitted row count MUST equal the source row count
* *AND* it MUST release each batch before requesting the next, so peak memory does not grow with the result size

### Scenario: Emitted columns match the declared output

* *GIVEN* a scan specification declaring three Exasol output columns
* *WHEN* the scan emits its batches
* *THEN* each emitted batch MUST carry exactly those three columns in the declared order
* *AND* each column's values MUST be coerced to the representation its declared Exasol type requires

### Scenario: Timestamp unit variation is normalized

* *GIVEN* a MySQL `DATETIME` column declared in Exasol as `TIMESTAMP`
* *WHEN* the driver returns that column as `timestamp[s]`, as `timestamp[ms]`, or as `timestamp[us]`
* *THEN* the scan MUST emit the same Exasol `TIMESTAMP` values in every case
* *AND* the scan MUST NOT fail on a unit it did not expect

### Scenario: Decimal width variation is normalized

* *GIVEN* a MySQL `DECIMAL(12,4)` column declared in Exasol as `DECIMAL(12,4)`
* *WHEN* the driver returns it as `decimal64`, as `decimal128`, or as a floating-point or string representation
* *THEN* the scan MUST emit the declared `DECIMAL(12,4)` values with the source precision and scale intact
* *AND* it MUST fail with a named error rather than truncate when a returned value does not fit the declared precision

### Scenario: Null values survive coercion

* *GIVEN* a nullable column whose batch contains nulls
* *WHEN* the scan coerces that batch
* *THEN* every null MUST be emitted as a null of the declared type
* *AND* the scan MUST NOT substitute a zero, an empty string, or an epoch value for a null

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

### Scenario: Source failure surfaces the driver's own diagnosis

* *GIVEN* a MySQL server that refuses the connection or rejects the statement
* *WHEN* the scan runs
* *THEN* it MUST return an error
* *AND* the error message MUST include the ADBC status and the driver's message
* *AND* the error message MUST NOT contain the MySQL password

### Scenario: Declared type wider than the driver's returned type is widened, not rejected

* *GIVEN* a declared `DECIMAL(23,4)` output column for a delegated `MIN` over a `DECIMAL(23,4)` source column, and a driver that returns the aggregate as a narrower decimal, as an integer, or as a string
* *WHEN* the scan coerces that batch
* *THEN* every value MUST be emitted as the declared `DECIMAL(23,4)` with its digits intact
* *AND* the scan MUST NOT fail merely because the returned type is narrower than the declared type
* *AND* a declared scale-0 column of at most 18 digits MUST still emit as the integer representation Exasol requires for that width

### Scenario: Declared BOOLEAN column emits every representation the driver returns

* *GIVEN* a declared `BOOLEAN` output column for a delegated predicate, and a driver that returns the projected comparison as an Arrow boolean, as an integer 0 and 1, or as a null
* *WHEN* the scan coerces that batch
* *THEN* every value MUST be emitted as an Arrow boolean, with a non-zero integer emitted as true and a zero as false
* *AND* a null MUST stay a null rather than becoming false, because MySQL evaluates a comparison against a null operand to null and Exasol does the same
* *AND* the scan MUST NOT fail merely because the driver returned the column as a number
