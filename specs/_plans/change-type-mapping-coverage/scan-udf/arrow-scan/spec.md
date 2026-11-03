# Feature: arrow_scan

Executes the pushed-down source statement through the ADBC driver the virtual schema names and streams the driver's Arrow record batches into Exasol without a row-by-row conversion step.

<!-- DELTA:CHANGED -->
## Background

- The scan UDF is `ADBC_VS.ADBC_SCAN`, a Rust scalar emitting script taking one string argument, the scan specification, which carries the driver identity, the BucketFS path of the driver library, the `CONNECTION` object's name, the statement, and the declared output columns, and never a credential. The adapter's pushdown SQL calls it once with a query-side dynamic `EMITS` clause, so one UDF instance streams the whole result, and batches cross the shared-object boundary as Arrow IPC bytes.
- This feature owns the scan contract every driver is held to: the entry symbol, credential resolution at run time, batch streaming, the declared-output contract, coercion and widening, and the driver-load error path. Each `<technology>-scan` feature states how its driver takes credentials and which representations it returns. The driver is loaded by the entry-point symbol its driver identity names, and a load failure over a library dependency the Script Language Container lacks reports a missing symbol or object rather than a missing file.
- Credentials are read at scan time from the named `CONNECTION` object through the UDF SDK. A resolved credential never reaches the scan specification, `adapterNotes`, SQL text, `EXPLAIN VIRTUAL` output, or any log line, and it does not outlive the connection attempt.
- The declaration is the sole authority for the emitted schema. A declared column need not correspond to a source column, since the adapter names columns positionally and reads an aggregate's type from `selectListDataTypes`. A declared `BOOLEAN` column comes from a mapped PostgreSQL `boolean` column or from a delegated predicate, which MySQL returns as a number and PostgreSQL and Trino as a boolean.
- The coercion is keyed on the declared type rather than on the returned Arrow type, because the drivers return one logical value under different Arrow types: the MySQL driver returns `DATETIME` at a unit that depends on the statement, the PostgreSQL driver returns every `numeric` as a UTF-8 string, and the Trino driver reaches `timestamp[ns]` above precision 6. Exasol's `DOUBLE` holds no `NaN` and no infinity, so a declared `DOUBLE` admits finite values only.
<!-- /DELTA:CHANGED -->

## Scenarios

<!-- DELTA:NEW -->
### Scenario: Non-finite DOUBLE value fails with a named error

* *GIVEN* a declared `DOUBLE` output column and a batch whose values include `NaN`, positive infinity, and negative infinity beside a finite value, as a PostgreSQL `double precision`, `real`, or unconstrained `numeric` column can hold
* *WHEN* the scan coerces that batch
* *THEN* the scan MUST fail with an error naming the non-finite value and the declared Exasol type
* *AND* the scan MUST NOT emit a null, a zero, or a saturated value in its place, because Exasol reads a `NaN` as a null and a delegated filter has already compared the original value
* *AND* a batch holding only finite values MUST still coerce
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: Narrow and unsigned integers coerce into their declared DECIMAL

* *GIVEN* declared `DECIMAL(3,0)`, `DECIMAL(5,0)`, `DECIMAL(8,0)`, `DECIMAL(10,0)`, and `DECIMAL(20,0)` output columns, and a driver that returns them as signed or unsigned 8-, 16-, 32-, and 64-bit integers
* *WHEN* the scan coerces that batch
* *THEN* every value MUST be emitted as its declared `DECIMAL` with its digits intact
* *AND* the unsigned 64-bit value 18446744073709551615 MUST be emitted as `DECIMAL(20,0)` rather than wrap to a negative value or fail
<!-- /DELTA:NEW -->
