# Feature: mysql_type_mapping

States which MySQL column types map to which Exasol types, and which are omitted and why.

<!-- DELTA:CHANGED -->
## Background

- The omission contract is framework and owned by `vs-adapter/type-mapping`. This feature states only MySQL's own mapping table and its omissions. The mapping is a pure function of the MySQL catalog row: data type name, character length, numeric precision, numeric scale, the unsigned flag, and the column character set.
- Seventeen MySQL types are mapped: `TINYINT`, `SMALLINT`, `MEDIUMINT`, `INT`, and `BIGINT`, signed or unsigned, to `DECIMAL` with the digit count of their range; `DECIMAL` to `DECIMAL`; `VARCHAR`, `CHAR`, `TINYTEXT`, `TEXT`, `MEDIUMTEXT`, and `LONGTEXT` to `VARCHAR` with character set `UTF8`; `DATE` to `DATE`; `DATETIME`, and `TIMESTAMP` as its UTC wall-clock value, to `TIMESTAMP`; and `FLOAT` and `DOUBLE` to `DOUBLE`. MySQL has no boolean column type, since `BOOL` is `TINYINT(1)`, so this mapping produces no `BOOLEAN`.
- MySQL `CHAR` maps to Exasol `VARCHAR`, because MySQL returns a `CHAR` value with its trailing spaces stripped while Exasol `CHAR` is blank-padded. A character column maps only when its character set is `utf8mb4`, because the mapping declares it `UTF8` and the MySQL dialect renders every string literal as `utf8mb4`. A text type declares a storage limit in bytes rather than a length, and a character never takes fewer than one byte, so that limit bounds the length too.
- MySQL compares a `FLOAT` operand as a double-precision value, so a `FLOAT` value compares as Exasol compares its widening to `DOUBLE`. A character column follows its collation, per `vs-adapter/mysql-collation`. Under a `PAD SPACE` collation MySQL also groups and deduplicates by the first `max_sort_length` bytes of a value, which binds a long `VARCHAR` and a text value alike, so it is a property of that collation rather than a reason to omit the text types.
- `JSON`, `TIME`, and `YEAR` are omitted because MySQL compares each differently from any Exasol type in the set, and `BLOB`, `ENUM`, `SET`, and `GEOMETRY` are omitted as types outside this release's scope. The adapter checks no MySQL server collation and no server version.
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

* *GIVEN* MySQL columns of type `TINYTEXT`, `TEXT`, `MEDIUMTEXT`, and `LONGTEXT` with character set `utf8mb4`, whose catalog lengths are 255, 65535, 16777215, and 4294967295
* *WHEN* each column is mapped
* *THEN* `TINYTEXT` MUST map to `VARCHAR` with size 255, and `TEXT` to `VARCHAR` with size 65535, each with character set `UTF8`
* *AND* `MEDIUMTEXT` and `LONGTEXT` MUST map to `VARCHAR` with size 2000000 and character set `UTF8`, because their limits exceed Exasol's maximum `VARCHAR` size
* *AND* a query reading a value longer than 2000000 characters MUST fail rather than return the value truncated
<!-- /DELTA:NEW -->

<!-- DELTA:CHANGED -->
### Scenario: Character column of a non-utf8mb4 character set is unmapped

* *GIVEN* MySQL columns of type `VARCHAR`, `CHAR`, and `TEXT` whose character set is `latin1`
* *WHEN* each column is mapped
* *THEN* each result MUST be unmapped
* *AND* the stated reason MUST name the column's character set and `utf8mb4` as the only character set this release maps
* *AND* the mapping MUST NOT map it to Exasol `VARCHAR` with character set `UTF8`, because the dialect renders every string literal compared against it as `utf8mb4`
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Temporal and floating-point types map to their Exasol counterparts

* *GIVEN* MySQL columns of type `DATE`, `DATETIME`, `DOUBLE`, and `FLOAT`
* *WHEN* each column is mapped
* *THEN* `DATE` MUST map to Exasol `DATE`
* *AND* `DATETIME` MUST map to Exasol `TIMESTAMP`
* *AND* `DOUBLE` and `FLOAT` MUST map to Exasol `DOUBLE`
* *AND* a `FLOAT` value MUST be emitted as its single-precision value widened to double precision, so a stored `0.1` is emitted as `0.100000001490116119384765625` and a delegated equality with the literal `0.1` selects no row, as Exasol's comparison of the emitted value does
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: TIMESTAMP maps to TIMESTAMP holding the UTC wall-clock value

* *GIVEN* a MySQL column of type `TIMESTAMP`, on a server of any default `time_zone`, including one with daylight saving time, read through a connection whose session `time_zone` is pinned to `+00:00`, per `scan-udf/mysql-scan`
* *WHEN* the column is mapped and queried
* *THEN* the result MUST be the Exasol type `TIMESTAMP`
* *AND* the scan MUST emit each value as its UTC wall-clock time, so two stored instants that share one local wall-clock time across a daylight-saving transition are emitted as two distinct values
* *AND* the mapping MUST NOT map it to Exasol `TIMESTAMP WITH LOCAL TIME ZONE`, because Exasol compares such a value against a `TIMESTAMP` or a `DATE` in the Exasol session time zone, which no source session follows
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: Delegated operations over a TIMESTAMP column follow the UTC session

* *GIVEN* a MySQL `TIMESTAMP` column mapped to Exasol `TIMESTAMP`, read through a connection whose session `time_zone` is pinned to `+00:00`
* *WHEN* a query delegates a comparison, an aggregate, a grouping, or an equi-join over the column
* *THEN* a delegated comparison of the column with a `TIMESTAMP` literal and with a `DATE` literal MUST select the rows Exasol selects over the emitted values, because MySQL converts the stored value to the pinned UTC session zone, which is one-to-one
* *AND* a delegated `MIN`, `MAX`, `COUNT(DISTINCT)`, and `GROUP BY` over the column MUST return Exasol's values
* *AND* a delegated equi-join of the column with a `DATETIME` column MUST pair the rows Exasol pairs over the emitted values
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: JSON, TIME, and YEAR are unmapped for how MySQL compares them

* *GIVEN* MySQL columns of type `JSON`, `TIME`, and `YEAR`
* *WHEN* each column is mapped
* *THEN* each result MUST be unmapped, with a reason naming how MySQL's comparison of that type differs from Exasol's
* *AND* the `JSON` and `YEAR` reasons MUST name the conversion of the compared constant, because MySQL converts a compared string into a JSON value rather than comparing text, and a compared integer constant into a year, so `10` compares as 2010 where Exasol's `DECIMAL` compares 10
* *AND* the `TIME` reason MUST name the duration comparison, because MySQL compares a `TIME` value as a signed duration of up to 838 hours and reads a compared string such as `'1112'` as `00:11:12`, while Exasol has no time type and compares `VARCHAR` text
<!-- /DELTA:NEW -->

<!-- DELTA:CHANGED -->
### Scenario: Non-core MySQL types are unmapped

* *GIVEN* MySQL columns of type `BLOB`, `ENUM`, `SET`, and `GEOMETRY`
* *WHEN* each column is mapped
* *THEN* each result MUST be unmapped
* *AND* each stated reason MUST name the MySQL type that was rejected
<!-- /DELTA:CHANGED -->
