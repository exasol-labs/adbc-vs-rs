# Feature: arrow_coercion

Casts each Arrow record batch the driver returns to the Exasol types the scan declares, normalizing representation differences between drivers.

## Background

- `ADBC_VS.ADBC_SCAN` is a Rust scalar emitting script whose one string argument, the scan specification, names the driver identity, the driver library's BucketFS path, the `CONNECTION` object, the statement, and the declared output columns.
- This feature owns the declared-output contract, coercion, and widening every driver is held to: the emitted columns, timestamp and decimal normalization, null handling, and the representations of `BOOLEAN`, narrow integers, and time-zone-aware timestamps.
- Each `<technology>-scan` feature states how its driver takes credentials and which Arrow representations it returns.
- `scan-udf/arrow-scan` owns the entry symbol, credential resolution, batch streaming, and the driver-load error path.

## Scenarios

### Scenario: Emitted columns match the declared output

* *GIVEN* a scan specification declaring three Exasol output columns
* *WHEN* the scan emits its batches
* *THEN* each emitted batch MUST carry exactly those three columns in the declared order
* *AND* each column's values MUST be coerced to the representation its declared Exasol type requires, keyed on the declared type rather than on the Arrow type the driver returned or on which source produced the batch, because the drivers return one logical value under different Arrow types
* *AND* the declaration MUST be the sole authority for the emitted schema, and the scan MUST NOT infer a column's identity from the statement, because a declared column need not correspond to a source column: the adapter names columns positionally and reads an aggregate's type from `selectListDataTypes`
* *AND* a declared `CHAR` column MUST be emitted as text, which Exasol pads to the declared size

### Scenario: Timestamp unit variation is normalized

* *GIVEN* a source timestamp column declared in Exasol as `TIMESTAMP`
* *WHEN* the driver returns that column as `timestamp[s]`, as `timestamp[ms]`, as `timestamp[us]`, or as `timestamp[ns]`
* *THEN* the scan MUST emit the same Exasol `TIMESTAMP` values in every case
* *AND* the scan MUST NOT fail on a unit it did not expect
* *AND* a PostgreSQL timestamp column MUST emit the same values whatever fractional-seconds precision the column declares, because that driver returns every timestamp at microsecond precision
* *AND* a Trino timestamp column MUST emit the same values whatever precision it declares, because that driver returns the Arrow unit the declared precision implies and reaches `timestamp[ns]` above precision 6
* *AND* the `timestamp[ns]` arm MUST be exercised against a real source column rather than a constructed batch alone, because a unit no driver was measured to return is a unit the coercion is only assumed to handle

### Scenario: Decimal width variation is normalized

* *GIVEN* a source `DECIMAL(12,4)` or `numeric(12,4)` column declared in Exasol as `DECIMAL(12,4)`
* *WHEN* the driver returns it as `decimal64`, as `decimal128`, as a floating-point value, or as a string
* *THEN* the scan MUST emit the declared `DECIMAL(12,4)` values with the source precision and scale intact
* *AND* it MUST fail with a named error rather than truncate when a returned value does not fit the declared precision
* *AND* the string representation MUST be handled as a first-class case rather than a fallback, because the PostgreSQL driver returns every `numeric` as a string

### Scenario: Null values survive coercion

* *GIVEN* a nullable column whose batch contains nulls
* *WHEN* the scan coerces that batch
* *THEN* every null MUST be emitted as a null of the declared type
* *AND* the scan MUST NOT substitute a zero, an empty string, or an epoch value for a null

### Scenario: Declared type wider than the driver's returned type is widened, not rejected

* *GIVEN* a declared `DECIMAL(23,4)` output column for a delegated `MIN` over a `DECIMAL(23,4)` source column, and a driver that returns the aggregate as a narrower decimal, as an integer, or as a string
* *WHEN* the scan coerces that batch
* *THEN* every value MUST be emitted as the declared `DECIMAL(23,4)` with its digits intact
* *AND* the scan MUST NOT fail merely because the returned type is narrower than the declared type
* *AND* a declared scale-0 column of at most 18 digits MUST still emit as the integer representation Exasol requires for that width

### Scenario: Declared BOOLEAN column emits every representation the driver returns

* *GIVEN* a declared `BOOLEAN` output column for a PostgreSQL `boolean` or `bit` column, a MySQL `BIT` column, or a predicate Exasol delegates as a select-list expression and declares `BOOLEAN`, per `vs-adapter/pushdown-dialect`, and a driver that returns the value as an Arrow boolean, as an integer 0 and 1, or as a null
* *WHEN* the scan coerces that batch
* *THEN* every value MUST be emitted as an Arrow boolean, with a non-zero integer emitted as true and a zero as false
* *AND* a null MUST stay a null rather than becoming false, because both sources evaluate a comparison against a null operand to null and Exasol does the same
* *AND* the scan MUST NOT fail merely because the driver returned the column as a number
* *AND* both representations MUST be reached in practice, because MySQL returns a delegated predicate as a number and PostgreSQL and Trino return it as a boolean

### Scenario: A declared TIMESTAMP(p) is emitted at microsecond resolution

* *GIVEN* a PostgreSQL `timestamp(6)` column and a MySQL `DATETIME(6)` column, each declared in Exasol as `TIMESTAMP(6)`
* *WHEN* the scan coerces the batches each driver returns
* *THEN* each declared `TIMESTAMP(p)` MUST be emitted at microsecond resolution, which holds every precision the PostgreSQL and MySQL mappings declare
* *AND* the MySQL column MUST emit the same values whatever Arrow unit the driver returns it at, because the `adbc-drivers/mysql` driver returns `DATETIME` as `timestamp[s]`, `timestamp[ms]`, or `timestamp[us]` depending on how the statement is written

### Scenario: Narrow and unsigned integers coerce into their declared DECIMAL

* *GIVEN* declared `DECIMAL(3,0)`, `DECIMAL(5,0)`, `DECIMAL(8,0)`, `DECIMAL(10,0)`, and `DECIMAL(20,0)` output columns, and a driver that returns them as signed or unsigned 8-, 16-, 32-, and 64-bit integers
* *WHEN* the scan coerces that batch
* *THEN* every value MUST be emitted as its declared `DECIMAL` with its digits intact
* *AND* the unsigned 64-bit value 18446744073709551615 MUST be emitted as `DECIMAL(20,0)` rather than wrap to a negative value or fail

### Scenario: A time-zone-aware timestamp is emitted as its UTC wall-clock value

* *GIVEN* a declared `TIMESTAMP(6)` output column and a driver that returns it as an Arrow microsecond timestamp carrying the time zone `UTC`, holding the instant `2024-10-27 00:30:00.123456` UTC
* *WHEN* the scan coerces that batch
* *THEN* the scan MUST emit `2024-10-27 00:30:00.123456`, the instant's UTC wall-clock value
* *AND* the scan MUST emit the same value when the Arrow time zone is spelled `+00:00`, and when it is another zone such as `Europe/Berlin`, because the Arrow value is the UTC instant whatever zone labels it
