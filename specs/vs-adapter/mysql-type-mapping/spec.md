# Feature: mysql_type_mapping

States which MySQL column types map to which Exasol types, which are omitted and why, and the source-catalog precondition the mapping depends on.

## Background

- The omission contract is framework and owned by `vs-adapter/type-mapping`: "unmapped" is not an error, the caller omits the column from the virtual table so one unsupported column never blocks a whole schema, and the omission is recorded with its table name, column name, source type under `sourceType`, and the reason it was omitted. This feature states only MySQL's own mapping table, its omissions, and its source precondition.
- The mapping is a pure function of the MySQL catalog row: data type name, character length, numeric precision, numeric scale, the unsigned flag, and the column character set. It performs no I/O.
- The mapping is driven by MySQL catalog metadata rather than by the Arrow type the ADBC driver returns at scan time. The `adbc-drivers/mysql` driver documents that `DATETIME` comes back as `timestamp[s]`, `timestamp[ms]`, or `timestamp[us]` depending on how the query is written, so the Arrow type is not a stable basis for a declared Exasol type.
- This release maps exactly eight MySQL types: `INT`, `BIGINT`, `DECIMAL`, `VARCHAR`, `CHAR`, `DATE`, `DATETIME`, `DOUBLE`. Every other MySQL type is unmapped.
- MySQL `CHAR` maps to Exasol `VARCHAR`, because Exasol `CHAR` is blank-padded and MySQL's comparison of a `CHAR` operand is not. Measured on Exasol 2025.2.1, a `CHAR(5)` holding `ab` reports `LENGTH` 5, compares equal to `'ab'`, to `'ab '`, and to `'ab   '`, and compares unequal to a `VARCHAR` holding `ab`. Measured on MySQL 8.4, the same `CHAR(5)` value is returned with its trailing spaces stripped and, under the `NO PAD` collations pushdown renders, compares equal to `'ab'` and to the `VARCHAR` column and unequal to `'ab '`. Mapping to Exasol `CHAR` would therefore return wrong rows in both directions for a delegated predicate or equi-join, so the mapped type is the one that matches the value MySQL returns.
- The mapping reads the column's MySQL character set alongside its length. `vs-adapter/mysql-collation` renders `COLLATE utf8mb4_0900_as_cs` on every character equality operand, and MySQL rejects that collation for any character set other than `utf8mb4` with error 1253. A character column of another character set is therefore unmapped rather than mapped into a virtual table whose predicates cannot be delegated.
- Exasol type bounds constrain the mapping: `DECIMAL` precision at most 36, `VARCHAR` size at most 2000000, and both character types minimum size 1. Exasol's `CHAR` size bound of 2000 constrains nothing, because this release produces no Exasol `CHAR`.
- The five Exasol types a mapped MySQL column produces are `DECIMAL`, `VARCHAR`, `DATE`, `TIMESTAMP`, and `DOUBLE`. That is not the set an `EMITS` column may declare, which `vs-adapter/pushdown-dialect` owns and which adds `BOOLEAN`; MySQL has no boolean column type this release maps, so this mapping produces no `BOOLEAN`.
- MySQL's source precondition is two named collations existing on the server: `utf8mb4_0900_as_cs` on every character equality operand and `utf8mb4_0900_bin` on a character operand of `MIN` or `MAX`. The requirement is stated by the dialect that renders it — `vs-adapter/mysql-collation` — rather than by a second list of its own, and it is executed by the MySQL metadata source, which is the module that already speaks MySQL's SQL and reads `information_schema.COLLATIONS`. `vs-adapter/create-virtual-schema` owns the framework rule that each driver's precondition runs before any table is discovered and on every request that resolves the source `CONNECTION`.
- MySQL added the UCA 9.0.0 `utf8mb4_0900_*` collations in 8.0.1 and `utf8mb4_0900_bin` in 8.0.17, so 8.0.17 is the source-server floor.
- `DRIVER = 'mysql'` reaches whatever server the resolved `adbc-drivers/mysql` driver connects to, and that driver's documentation names MariaDB as a supported server. A server missing a required collation is therefore reachable, and refusing it at creation is what keeps a delegated character fragment from failing at query time instead, where the advertised capabilities have already committed Exasol to the delegation.
- Whether the check refuses a MariaDB server depends on its version. Measured on 2026-10-02 with `adbc-drivers/mysql` 0.6.1: MariaDB 10.11.19 lists neither collation and rejects `COLLATE utf8mb4_0900_as_cs` with error 1273, `Unknown collation`, so the check refuses it. MariaDB 11.4.13, 11.8.9, and 12.2.2 list both collations, so the check accepts them. On MariaDB 12.2.2, `utf8mb4_0900_as_cs` is an alias of `utf8mb4_uca1400_nopad_as_cs` and `utf8mb4_0900_bin` is an alias of `utf8mb4_nopad_bin`, and both are `NO PAD`.
- MariaDB is not a supported source. No test tier verifies delegated results against it, and MariaDB's `utf8mb4_0900_as_cs` alias uses UCA 14.0.0 weights where MySQL's collation uses UCA 9.0.0. Measured on MariaDB 12.2.2 and MySQL 8.4.11, the two servers agree on case, accent, and trailing-space equality, `LIKE`, `GROUP BY`, and `COUNT(DISTINCT)` under `utf8mb4_0900_as_cs`, and on `MIN` and `MAX` under `utf8mb4_0900_bin`. They disagree on `U+1FAE0 < U+1F600` under `utf8mb4_0900_as_cs`, a comparison between a character Unicode 9.0 does not assign and one it does: MariaDB returns true and MySQL returns false.
- The check is not a version-string comparison: a fork reporting an 8.x version without the collations is rejected on the collation, not on the number.

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

### Scenario: Source server without the required collations is rejected at creation

* *GIVEN* a reachable MySQL server whose `information_schema.COLLATIONS` omits `utf8mb4_0900_bin`, or omits `utf8mb4_0900_as_cs`, as MariaDB 10.11 and every MySQL older than 8.0.17 do
* *WHEN* `CREATE VIRTUAL SCHEMA` runs against it with `DRIVER = 'mysql'`
* *THEN* the adapter MUST return an error naming whichever of the two collations it did not find, the server version string the source reported, and `8.0.17` as the minimum MySQL version this release supports, and MUST NOT create the virtual schema
* *AND* a server listing both collations MUST be accepted, and MUST NOT be rejected on its version string alone, so that a fork reporting an unexpected version but supplying the collations still works
* *AND* the check MUST refuse the source before any table is discovered, MUST run on `refresh` and `setProperties` as well as `createVirtualSchema`, and MUST require the collation names the MySQL dialect renders rather than a second list of its own
* *AND* the check MUST NOT run for a PostgreSQL source, because the collations it names are MySQL's and the requirement is stated by the dialect that renders them
