# Feature: mysql_type_mapping

States which MySQL column types map to which Exasol types, and which are omitted and why.

<!-- DELTA:CHANGED -->
## Background

- The omission contract is framework and owned by `vs-adapter/type-mapping`. The mapping is a pure function of the MySQL catalog row: data type name, character length, numeric precision and scale, datetime precision, and the unsigned flag. It performs no I/O. It starts from the Exasol MySQL JDBC Virtual Schema's mapping, and this feature states every departure.
- A column the mapping reads through a conversion renders as that read expression in every position, per `vs-adapter/pushdown-dialect`, and every other column renders bare.
- The mapping declares every character column `UTF8` whatever its MySQL character set, as the JDBC Virtual Schema does. A character column follows its collation, per `vs-adapter/mysql-collation`. MySQL returns a `CHAR`, `ENUM`, or `SET` value without trailing spaces, and Exasol pads a value it reads into a `CHAR` column to the declared size.
- MySQL compares a `FLOAT` operand as a double-precision value, so a `FLOAT` value compares as Exasol compares its widening to `DOUBLE`.
- The adapter checks no MySQL server collation and no server version.
<!-- /DELTA:CHANGED -->

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Signed integer types map to DECIMAL with the matching digit count

* *GIVEN* a MySQL column of type `INT`
* *WHEN* the column is mapped
* *THEN* the result MUST be the Exasol type `DECIMAL` with precision 10 and scale 0
* *AND* a `BIGINT` column MUST map to `DECIMAL` with precision 19 and scale 0
* *AND* `TINYINT`, `SMALLINT`, and `MEDIUMINT` columns MUST map to `DECIMAL` with precision 3, 5, and 7 and scale 0
* *AND* a `TINYINT(1)` column MUST map to `DECIMAL` with precision 3 and emit a stored `5` as `5`, because `BOOL` is an alias for that type rather than a boolean type
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: DECIMAL preserves the source precision and scale

* *GIVEN* a MySQL column of type `DECIMAL` with precision 12 and scale 4
* *WHEN* the column is mapped
* *THEN* the result MUST be the Exasol type `DECIMAL` with precision 12 and scale 4, and the column MUST render bare
* *AND* a `DECIMAL` column with precision 40 and scale 2 MUST map to `VARCHAR` with size 2000000 and character set `UTF8`, read as `CAST(<column> AS CHAR)`
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Character types preserve the declared length and use UTF8

* *GIVEN* a MySQL column of type `VARCHAR` with a declared character length of 100
* *WHEN* the column is mapped
* *THEN* the result MUST be the Exasol type `VARCHAR` with size 100 and character set `UTF8`, the column MUST render bare, and a declared length of 0 MUST map to size 1, because Exasol's minimum character-type size is 1
* *AND* a `CHAR` column with declared length 10 MUST map to Exasol `CHAR` with size 10 and character set `UTF8`, as the JDBC Virtual Schema maps it, and render bare
* *AND* the scan MUST emit a `CHAR(10)` value `ab` padded to ten characters, and a delegated equality of the column with the literal `'ab'` MUST select the row, as Exasol's comparison of a `CHAR` value with a string literal does
* *AND* a delegated `LIKE 'ab'` over the `CHAR` column MUST select the row, because MySQL matches the unpadded value, although Exasol's own `LIKE 'ab'` over `CHAR(10)` `ab` matches no row
<!-- /DELTA:CHANGED -->

<!-- DELTA:REMOVED -->
### Scenario: Character column of a non-utf8mb4 character set is unmapped

* *GIVEN* MySQL columns of type `VARCHAR` and `CHAR` whose character set is `latin1`
* *WHEN* each column is mapped
* *THEN* each result MUST be unmapped
* *AND* the stated reason MUST name the column's character set and `utf8mb4` as the only character set this release maps
* *AND* the mapping MUST NOT map it to Exasol `VARCHAR` with character set `UTF8`, because the dialect renders every string literal compared against it as `utf8mb4`
<!-- /DELTA:REMOVED -->

<!-- DELTA:NEW -->
### Scenario: Character columns of every character set map to UTF8

* *GIVEN* MySQL `VARCHAR`, `CHAR`, and `TEXT` columns whose character set is `latin1`, each holding `café`
* *WHEN* each column is mapped and queried
* *THEN* each MUST map as a `utf8mb4` column of the same type and length maps, with character set `UTF8`
* *AND* the scan MUST emit `café` with its accented character intact
* *AND* a delegated equality of each column with the literal `'café'` MUST select the row
* *AND* a delegated equality of each column with a literal holding a character `latin1` cannot hold, such as `'ł'`, MUST fail with MySQL's illegal-mix-of-collations error rather than select or drop rows, because the column's collation takes precedence over the `utf8mb4` literal and MySQL cannot convert the literal to `latin1`
<!-- /DELTA:NEW -->

<!-- DELTA:CHANGED -->
### Scenario: Temporal and floating-point types map to their Exasol counterparts

* *GIVEN* MySQL columns of type `DATE`, `DATETIME` with datetime precision 0 and 6, `DOUBLE`, and `FLOAT`
* *WHEN* each column is mapped
* *THEN* `DATE` MUST map to Exasol `DATE`
* *AND* the `DATETIME` columns MUST map to `TIMESTAMP(0)` and `TIMESTAMP(6)`, their own fractional-second precision
* *AND* `DOUBLE` and `FLOAT` MUST map to Exasol `DOUBLE`, and every listed column MUST render bare
* *AND* a `FLOAT` value MUST be emitted as its single-precision value widened to double precision, so a stored `0.1` is emitted as `0.100000001490116119384765625` and a delegated equality with the literal `0.1` selects no row, as Exasol's comparison of the emitted value does
<!-- /DELTA:CHANGED -->

<!-- DELTA:REMOVED -->
### Scenario: Unsigned integer columns are unmapped

* *GIVEN* a MySQL column of type `BIGINT UNSIGNED`
* *WHEN* the column is mapped
* *THEN* the result MUST be unmapped
* *AND* the stated reason MUST identify the unsigned modifier as unsupported in this release
* *AND* the mapping MUST NOT map it to `DECIMAL` with precision 19, because the unsigned range exceeds that precision
<!-- /DELTA:REMOVED -->

<!-- DELTA:NEW -->
### Scenario: Unsigned integer types map to DECIMAL with the digits of their unsigned range

* *GIVEN* MySQL columns of type `TINYINT UNSIGNED`, `SMALLINT UNSIGNED`, `MEDIUMINT UNSIGNED`, `INT UNSIGNED`, and `BIGINT UNSIGNED`
* *WHEN* each column is mapped
* *THEN* each MUST map to `DECIMAL` with scale 0 and precision 3, 5, 8, 10, and 20 respectively
* *AND* a `BIGINT UNSIGNED` value of 18446744073709551615 MUST be emitted with its digits intact
* *AND* the mapping MUST NOT map `BIGINT UNSIGNED` to `DECIMAL` with precision 19, because the unsigned range exceeds that precision
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: Text types map to VARCHAR bounded by their storage limit

* *GIVEN* MySQL columns of type `TINYTEXT`, `TEXT`, `MEDIUMTEXT`, and `LONGTEXT`, whose catalog lengths are 255, 65535, 16777215, and 4294967295
* *WHEN* each column is mapped
* *THEN* `TINYTEXT` MUST map to `VARCHAR` with size 255, and `TEXT` to `VARCHAR` with size 65535, each with character set `UTF8`
* *AND* `MEDIUMTEXT` and `LONGTEXT` MUST map to `VARCHAR` with size 2000000 and character set `UTF8`, because their limits exceed Exasol's maximum `VARCHAR` size
* *AND* a query reading a value longer than 2000000 characters MUST fail rather than return the value truncated
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: BIT of every width maps to BOOLEAN

* *GIVEN* a MySQL `BIT(1)` column holding `b'1'`, `b'0'`, and null, and a `BIT(8)` column holding `b'00000101'`, `b'00001001'`, and `b'00000000'`
* *WHEN* each column is mapped and queried
* *THEN* each column MUST map to Exasol `BOOLEAN`, as the JDBC Virtual Schema maps every `BIT` width, and render as `(<column> <> 0)`, which is true when any bit is set
* *AND* the `BIT(1)` column MUST emit true, false, and null, and the `BIT(8)` column true, true, and false, so two different source values read as one `true`
* *AND* a `BIT(64)` column MUST map to `BOOLEAN`
* *AND* a delegated filter on each column alone and on its negation, and a delegated `MIN`, `MAX`, and `GROUP BY` over each, MUST return Exasol's answers over the emitted values, so a `GROUP BY` over the `BIT(8)` column puts `b'00000101'` and `b'00001001'` in one group
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: A select-list predicate over a grouped converted column is delegated

* *GIVEN* `pushdown` requests whose `groupBy` names a MySQL `YEAR` column or a `BIT(8)` column, each carrying in its select list the grouped column and a predicate reading it outside any aggregate, such as `Y = DATE '2024-01-01'` and `NOT BITS`
* *WHEN* the MySQL dialect renders each statement and MySQL executes it
* *THEN* `GROUP BY` MUST carry the column's read expression, and each select-list reference to the grouped column outside an aggregate MUST render as `ANY_VALUE(<read expression>)`
* *AND* the statement MUST execute at the source rather than being rejected with MySQL error 1055
* *AND* the rows MUST equal what Exasol returns for the same query over a local copy of the emitted values
* *AND* a grouped column that renders bare MUST keep its bare select-list reference, with no `ANY_VALUE`
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: ENUM and SET map to CHAR

* *GIVEN* a MySQL `ENUM('ab','abcde')` column holding `ab` and a `SET('x','yz')` column, whose catalog lengths are 5 and 4
* *WHEN* each column is mapped and queried
* *THEN* the `ENUM` column MUST map to Exasol `CHAR` with size 5 and the `SET` column to `CHAR` with size 4, each with character set `UTF8`, and each MUST render bare
* *AND* a column whose catalog length exceeds 2000 MUST map to `VARCHAR` of that length, because Exasol's `CHAR` holds at most 2000 characters
* *AND* the scan MUST emit the `ENUM` value `ab` padded to five characters, and a delegated equality of the `ENUM` column with the literal `'ab'` MUST select the row, as Exasol's comparison of a `CHAR` value with a string literal does
* *AND* a delegated `LIKE 'ab'` over the `ENUM` column MUST select the row, because MySQL matches the unpadded member, although Exasol's own `LIKE 'ab'` over `CHAR(5)` `ab` matches no row
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: TIME maps to a TIMESTAMP on 1970-01-01 and YEAR to a DATE

* *GIVEN* a MySQL `TIME(3)` column holding `16:59:59.123` and `25:00:00`, and a `YEAR` column holding `2024`
* *WHEN* each column is mapped and queried
* *THEN* the `TIME` column MUST map to `TIMESTAMP(3)`, read as `TIMESTAMP(DATE '1970-01-01', <column>)`, and emit `1970-01-01 16:59:59.123` and `1970-01-02 01:00:00.000`
* *AND* the `YEAR` column MUST map to `DATE`, read as `MAKEDATE(<column>, 1)`, and emit `2024-01-01`
* *AND* a delegated equality of the `TIME` column with `TIMESTAMP '1970-01-01 16:59:59.123'`, and of the `YEAR` column with `DATE '2024-01-01'`, MUST select the row, as Exasol's comparison of the emitted values does
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: JSON maps to VARCHAR read as its text

* *GIVEN* a MySQL `JSON` column holding `{"a": 1}`
* *WHEN* the column is mapped and queried
* *THEN* the result MUST be `VARCHAR` with size 2000000 and character set `UTF8`, read as `CAST(<column> AS CHAR)`
* *AND* the scan MUST emit MySQL's text for the value
* *AND* a delegated equality with the literal `'{"a": 1}'` MUST compare that text rather than a parsed JSON value
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: TIMESTAMP maps to TIMESTAMP in the session time zone

* *GIVEN* a MySQL column of type `TIMESTAMP` with datetime precision 6
* *WHEN* the column is mapped and queried
* *THEN* the result MUST be the Exasol type `TIMESTAMP(6)`, and the column MUST render bare
* *AND* the adapter MUST set no session time zone, so the scan emits each value as its wall-clock time in the session `time_zone`, which the server default sets
* *AND* under a session `time_zone` of `+00:00`, a delegated comparison with a `TIMESTAMP` or `DATE` literal, a delegated `MIN`, `MAX`, `COUNT(DISTINCT)`, and `GROUP BY`, and a delegated equi-join with a `DATETIME(6)` column MUST return Exasol's answers over the emitted values
* *AND* the mapping MUST NOT map it to Exasol `TIMESTAMP WITH LOCAL TIME ZONE`, because Exasol compares such a value in the Exasol session time zone, which no source session follows
<!-- /DELTA:NEW -->

<!-- DELTA:CHANGED -->
### Scenario: Non-core MySQL types are unmapped

* *GIVEN* MySQL columns of type `BINARY`, `VARBINARY`, `TINYBLOB`, `BLOB`, `MEDIUMBLOB`, `LONGBLOB`, and `GEOMETRY`
* *WHEN* each column is mapped
* *THEN* each result MUST be unmapped, as the JDBC Virtual Schema leaves binary and spatial columns out
* *AND* each stated reason MUST name the MySQL type that was rejected
<!-- /DELTA:CHANGED -->
