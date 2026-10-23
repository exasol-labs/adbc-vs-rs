# Feature: mysql_type_mapping

States which MySQL column types map to which Exasol types, and which are omitted and why.

## Background

- The omission contract is framework and owned by `vs-adapter/type-mapping`: "unmapped" is not an error, the caller omits the column from the virtual table so one unsupported column never blocks a whole schema, and the omission is recorded with its table name, column name, source type under `sourceType`, and the reason it was omitted. This feature states only MySQL's own mapping table and its omissions.
- The mapping is a pure function of the MySQL catalog row: data type name, character length, numeric precision, numeric scale, the unsigned flag, and the column character set. It performs no I/O.
- The mapping is driven by MySQL catalog metadata rather than by the Arrow type the ADBC driver returns at scan time. The `adbc-drivers/mysql` driver documents that `DATETIME` comes back as `timestamp[s]`, `timestamp[ms]`, or `timestamp[us]` depending on how the query is written, so the Arrow type is not a stable basis for a declared Exasol type.
- This release maps exactly eight MySQL types: `INT`, `BIGINT`, `DECIMAL`, `VARCHAR`, `CHAR`, `DATE`, `DATETIME`, `DOUBLE`. Every other MySQL type is unmapped.
- MySQL `CHAR` maps to Exasol `VARCHAR`, because Exasol `CHAR` is blank-padded and MySQL's comparison of a `CHAR` operand is not. Measured on the test Exasol, a `CHAR(5)` holding `ab` reports `LENGTH` 5, compares equal to `'ab'`, to `'ab '`, and to `'ab   '`, and compares unequal to a `VARCHAR` holding `ab`. Measured on the test MySQL, the same `CHAR(5)` value is returned with its trailing spaces stripped. Mapping to Exasol `CHAR` would therefore declare a blank-padded type for a value MySQL returns unpadded, so the mapped type is the one that matches the value MySQL returns.
- The mapping reads the column's MySQL character set alongside its length, and maps a character column only when that set is `utf8mb4`. The mapping declares every character column `UTF8`, and the MySQL dialect renders every string literal as `utf8mb4`, so a column of another character set would be compared against a literal of a different character set. This release does not measure that conversion, so such a column is unmapped.
- Exasol type bounds constrain the mapping: `DECIMAL` precision at most 36, `VARCHAR` size at most 2000000, and both character types minimum size 1. Exasol's `CHAR` size bound of 2000 constrains nothing, because this release produces no Exasol `CHAR`.
- The five Exasol types a mapped MySQL column produces are `DECIMAL`, `VARCHAR`, `DATE`, `TIMESTAMP`, and `DOUBLE`. That is not the set an `EMITS` column may declare, which `vs-adapter/pushdown-dialect` owns and which adds `BOOLEAN`; MySQL has no boolean column type this release maps, so this mapping produces no `BOOLEAN`.
- The adapter checks no MySQL server collation and no server version. The MySQL dialect renders no collation, so the source check queries neither `information_schema.COLLATIONS` nor the server version, and refuses no server for its version.

## Scenarios

### Scenario: Signed integer types map to DECIMAL with the matching digit count

* *GIVEN* a MySQL column of type `INT`
* *WHEN* the column is mapped
* *THEN* the result MUST be the Exasol type `DECIMAL` with precision 10 and scale 0
* *AND* a `BIGINT` column MUST map to `DECIMAL` with precision 19 and scale 0

### Scenario: DECIMAL preserves the source precision and scale

* *GIVEN* a MySQL column of type `DECIMAL` with precision 12 and scale 4
* *WHEN* the column is mapped
* *THEN* the result MUST be the Exasol type `DECIMAL` with precision 12 and scale 4

### Scenario: Character types preserve the declared length and use UTF8

* *GIVEN* a MySQL column of type `VARCHAR` with a declared character length of 100 and character set `utf8mb4`
* *WHEN* the column is mapped
* *THEN* the result MUST be the Exasol type `VARCHAR` with size 100 and character set `UTF8`
* *AND* a `CHAR` column with declared length 10 and character set `utf8mb4` MUST map to Exasol `VARCHAR` with size 10 and character set `UTF8`
* *AND* the mapping MUST NOT produce Exasol `CHAR` for any MySQL column, because Exasol blank-pads a `CHAR` value and MySQL returns a `CHAR` value with its trailing spaces stripped
* *AND* a declared length of 0 MUST map to size 1, because Exasol's minimum character-type size is 1

### Scenario: Character column of a non-utf8mb4 character set is unmapped

* *GIVEN* MySQL columns of type `VARCHAR` and `CHAR` whose character set is `latin1`
* *WHEN* each column is mapped
* *THEN* each result MUST be unmapped
* *AND* the stated reason MUST name the column's character set and `utf8mb4` as the only character set this release maps
* *AND* the mapping MUST NOT map it to Exasol `VARCHAR` with character set `UTF8`, because the dialect renders every string literal compared against it as `utf8mb4`

### Scenario: Temporal and floating-point types map to their Exasol counterparts

* *GIVEN* MySQL columns of type `DATE`, `DATETIME`, and `DOUBLE`
* *WHEN* each column is mapped
* *THEN* `DATE` MUST map to Exasol `DATE`
* *AND* `DATETIME` MUST map to Exasol `TIMESTAMP`
* *AND* `DOUBLE` MUST map to Exasol `DOUBLE`

### Scenario: Unsigned integer columns are unmapped

* *GIVEN* a MySQL column of type `BIGINT UNSIGNED`
* *WHEN* the column is mapped
* *THEN* the result MUST be unmapped
* *AND* the stated reason MUST identify the unsigned modifier as unsupported in this release
* *AND* the mapping MUST NOT map it to `DECIMAL` with precision 19, because the unsigned range exceeds that precision

### Scenario: Non-core MySQL types are unmapped

* *GIVEN* MySQL columns of type `JSON`, `TEXT`, `BLOB`, `ENUM`, `SET`, `TINYINT`, `SMALLINT`, `MEDIUMINT`, `FLOAT`, `TIME`, `YEAR`, `TIMESTAMP`, and `GEOMETRY`
* *WHEN* each column is mapped
* *THEN* each result MUST be unmapped
* *AND* each stated reason MUST name the MySQL type that was rejected

### Scenario: No server collation or version is checked for MySQL

* *GIVEN* a reachable MySQL server whose `information_schema.COLLATIONS` lists neither `utf8mb4_0900_bin` nor `utf8mb4_0900_as_cs`, as MariaDB and MySQL before 8.0.1 do
* *WHEN* the adapter runs the source check for `DRIVER = 'mysql'`
* *THEN* the check MUST NOT query `information_schema.COLLATIONS` or the server version
* *AND* the check MUST NOT refuse the server for a missing collation or for its version
* *AND* the check MUST still run at the point `vs-adapter/create-virtual-schema` states, on `refresh` and `setProperties` as well as `createVirtualSchema`
