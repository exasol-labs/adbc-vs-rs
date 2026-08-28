# Feature: type_mapping

Converts a MySQL column definition into the Exasol data type the virtual table declares, and refuses any type it cannot represent without loss or delegate a predicate over.

## Background

- The mapping is a pure function of the MySQL catalog row: data type name, character length, numeric precision, numeric scale, and the unsigned flag. It performs no I/O.
- The mapping is driven by MySQL catalog metadata rather than by the Arrow type the ADBC driver returns at scan time. The `adbc-drivers/mysql` driver documents that `DATETIME` comes back as `timestamp[s]`, `timestamp[ms]`, or `timestamp[us]` depending on how the query is written, so the Arrow type is not a stable basis for a declared Exasol type.
- Release 0.1 maps exactly eight MySQL types: `INT`, `BIGINT`, `DECIMAL`, `VARCHAR`, `CHAR`, `DATE`, `DATETIME`, `DOUBLE`. Every other MySQL type is unmapped.
- "Unmapped" is not an error. The caller omits the column from the virtual table and records the omission, so one unsupported column never blocks a whole schema.
- Exasol type bounds constrain the mapping: `DECIMAL` precision at most 36, `VARCHAR` size at most 2000000, and both character types minimum size 1. Exasol's `CHAR` size bound of 2000 constrains nothing, because this release produces no Exasol `CHAR`.
- MySQL `CHAR` maps to Exasol `VARCHAR`, because Exasol `CHAR` is blank-padded and MySQL's comparison of a `CHAR` operand is not. Measured on Exasol 2025.2.1, a `CHAR(5)` holding `ab` reports `LENGTH` 5, compares equal to `'ab'`, to `'ab '`, and to `'ab   '`, and compares unequal to a `VARCHAR` holding `ab`. Measured on MySQL 8.4, the same `CHAR(5)` value is returned with its trailing spaces stripped and, under the `NO PAD` collations pushdown renders, compares equal to `'ab'` and to the `VARCHAR` column and unequal to `'ab '`. Mapping to Exasol `CHAR` would therefore return wrong rows in both directions for a delegated predicate or equi-join, so the mapped type is the one that matches the value MySQL returns.
- The five Exasol types a mapped MySQL column produces are therefore `DECIMAL`, `VARCHAR`, `DATE`, `TIMESTAMP`, and `DOUBLE`. That set is not the set an `EMITS` column may declare, which `vs-adapter/pushdown-dialect` owns and which adds `BOOLEAN` for a predicate Exasol delegated as a select-list expression. MySQL has no boolean column type this release maps, so the two sets are modeled as two types and this mapping cannot produce `BOOLEAN`.
- The mapping reads the column's MySQL character set alongside its length. Filter, join, and aggregation pushdown render a character operand with an explicit `COLLATE utf8mb4_0900_as_cs` suffix to reproduce Exasol's case-sensitive, accent-sensitive comparison, and MySQL rejects that collation for any character set other than `utf8mb4` with error 1253. A character column of another character set is therefore unmapped rather than mapped into a virtual table whose predicates cannot be delegated.

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

### Scenario: DECIMAL beyond Exasol's maximum precision is unmapped

* *GIVEN* a MySQL column of type `DECIMAL` with precision 40 and scale 2
* *WHEN* the column is mapped
* *THEN* the result MUST be unmapped
* *AND* the stated reason MUST name Exasol's maximum `DECIMAL` precision of 36
* *AND* the mapping MUST NOT silently reduce the precision, because that would truncate values

### Scenario: Character types preserve the declared length and use UTF8

* *GIVEN* a MySQL column of type `VARCHAR` with a declared character length of 100 and character set `utf8mb4`
* *WHEN* the column is mapped
* *THEN* the result MUST be the Exasol type `VARCHAR` with size 100 and character set `UTF8`
* *AND* a `CHAR` column with declared length 10 and character set `utf8mb4` MUST map to Exasol `VARCHAR` with size 10 and character set `UTF8`
* *AND* the mapping MUST NOT produce Exasol `CHAR` for any MySQL column, because Exasol blank-pads a `CHAR` value and MySQL does not pad a `CHAR` operand under the collations pushdown renders
* *AND* a declared length of 0 MUST map to size 1, because Exasol's minimum character-type size is 1

### Scenario: Character column of a non-utf8mb4 character set is unmapped

* *GIVEN* MySQL columns of type `VARCHAR` and `CHAR` whose character set is `latin1`
* *WHEN* each column is mapped
* *THEN* each result MUST be unmapped
* *AND* the stated reason MUST name the column's character set and `utf8mb4` as the only character set this release maps
* *AND* the mapping MUST NOT map it to Exasol `VARCHAR` with character set `UTF8`, because a predicate over that column cannot be delegated with the collation Exasol's comparison semantics require

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

### Scenario: Column nullability carries through unchanged

* *GIVEN* a MySQL column declared `NOT NULL`
* *WHEN* the column is mapped
* *THEN* the mapped column MUST report `isNullable` as false
* *AND* a column without `NOT NULL` MUST report `isNullable` as true
