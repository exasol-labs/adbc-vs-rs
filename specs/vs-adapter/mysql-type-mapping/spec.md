# Feature: mysql_type_mapping

States which MySQL numeric, temporal, and floating-point column types map to which Exasol types, which are omitted and why, and that the mapping needs no server check.

## Background

- `vs-adapter/type-mapping` owns the omission contract and the closed type set this mapping obeys.
- This feature states MySQL's mapping table and each departure from the Exasol MySQL JDBC Virtual Schema's mapping.
- A column's read expression, where a scenario names one, renders per `vs-adapter/pushdown-dialect`.
- `vs-adapter/mysql-collation` owns how a character column compares.
- `vs-adapter/mysql-extended-type-mapping` states the mapping of character, text, bit, enumeration, and JSON types and the read expressions they need.

## Scenarios

### Scenario: Signed integer types map to DECIMAL with the matching digit count

* *GIVEN* a MySQL column of type `INT`
* *WHEN* the column is mapped
* *THEN* the result MUST be the Exasol type `DECIMAL` with precision 10 and scale 0
* *AND* a `BIGINT` column MUST map to `DECIMAL` with precision 19 and scale 0
* *AND* `TINYINT`, `SMALLINT`, and `MEDIUMINT` columns MUST map to `DECIMAL` with precision 3, 5, and 7 and scale 0
* *AND* a `TINYINT(1)` column MUST map to `DECIMAL` with precision 3 and emit a stored `5` as `5`, because `BOOL` is an alias for that type rather than a boolean type

### Scenario: DECIMAL preserves the source precision and scale

* *GIVEN* a MySQL column of type `DECIMAL` with precision 12 and scale 4
* *WHEN* the column is mapped
* *THEN* the result MUST be the Exasol type `DECIMAL` with precision 12 and scale 4, and the column MUST render bare
* *AND* a `DECIMAL` column with precision 40 and scale 2 MUST map to `VARCHAR` with size 2000000 and character set `UTF8`, read as `CAST(<column> AS CHAR)`

### Scenario: Temporal and floating-point types map to their Exasol counterparts

* *GIVEN* MySQL columns of type `DATE`, `DATETIME` with datetime precision 0 and 6, `DOUBLE`, and `FLOAT`
* *WHEN* each column is mapped
* *THEN* `DATE` MUST map to Exasol `DATE`
* *AND* the `DATETIME` columns MUST map to `TIMESTAMP(0)` and `TIMESTAMP(6)`, their own fractional-second precision
* *AND* `DOUBLE` and `FLOAT` MUST map to Exasol `DOUBLE`, and every listed column MUST render bare
* *AND* a `FLOAT` value MUST be emitted as its single-precision value widened to double precision, so a stored `0.1` is emitted as `0.100000001490116119384765625` and a delegated equality with the literal `0.1` selects no row, as Exasol's comparison of the emitted value does, because MySQL compares a `FLOAT` operand as a double-precision value

### Scenario: Non-core MySQL types are unmapped

* *GIVEN* MySQL columns of type `BINARY`, `VARBINARY`, `TINYBLOB`, `BLOB`, `MEDIUMBLOB`, `LONGBLOB`, and `GEOMETRY`
* *WHEN* each column is mapped
* *THEN* each result MUST be unmapped, as the JDBC Virtual Schema leaves binary and spatial columns out
* *AND* each stated reason MUST name the MySQL type that was rejected

### Scenario: No server collation or version is checked for MySQL

* *GIVEN* a reachable MySQL server whose `information_schema.COLLATIONS` lists neither `utf8mb4_0900_bin` nor `utf8mb4_0900_as_cs`, as MariaDB and MySQL before 8.0.1 do
* *WHEN* the adapter runs the source check for `DRIVER = 'mysql'`
* *THEN* the check MUST NOT query `information_schema.COLLATIONS` or the server version, because the MySQL dialect renders no collation
* *AND* the check MUST NOT refuse the server for a missing collation or for its version
* *AND* the check MUST still run at the point `vs-adapter/create-virtual-schema` states, on `refresh` and `setProperties` as well as `createVirtualSchema`

### Scenario: The mapping is a pure function of the MySQL catalog row

* *GIVEN* a MySQL column catalog row
* *WHEN* the column is mapped
* *THEN* the result MUST depend only on the row's data type name, character length, numeric precision and scale, datetime precision, and unsigned flag
* *AND* the mapping MUST perform no I/O

### Scenario: Unsigned integer types map to DECIMAL with the digits of their unsigned range

* *GIVEN* MySQL columns of type `TINYINT UNSIGNED`, `SMALLINT UNSIGNED`, `MEDIUMINT UNSIGNED`, `INT UNSIGNED`, and `BIGINT UNSIGNED`
* *WHEN* each column is mapped
* *THEN* each MUST map to `DECIMAL` with scale 0 and precision 3, 5, 8, 10, and 20 respectively
* *AND* a `BIGINT UNSIGNED` value of 18446744073709551615 MUST be emitted with its digits intact
* *AND* the mapping MUST NOT map `BIGINT UNSIGNED` to `DECIMAL` with precision 19, because the unsigned range exceeds that precision

### Scenario: BIT of every width maps to BOOLEAN

* *GIVEN* a MySQL `BIT(1)` column holding `b'1'`, `b'0'`, and null, and a `BIT(8)` column holding `b'00000101'`, `b'00001001'`, and `b'00000000'`
* *WHEN* each column is mapped and queried
* *THEN* each column MUST map to Exasol `BOOLEAN`, as the JDBC Virtual Schema maps every `BIT` width, and render as `(<column> <> 0)`, which is true when any bit is set
* *AND* the `BIT(1)` column MUST emit true, false, and null, and the `BIT(8)` column true, true, and false, so two different source values read as one `true`
* *AND* a `BIT(64)` column MUST map to `BOOLEAN`
* *AND* a delegated filter on each column alone and on its negation, and a delegated `MIN`, `MAX`, and `GROUP BY` over each, MUST return Exasol's answers over the emitted values, so a `GROUP BY` over the `BIT(8)` column puts `b'00000101'` and `b'00001001'` in one group

### Scenario: TIME maps to a TIMESTAMP on 1970-01-01 and YEAR to a DATE

* *GIVEN* a MySQL `TIME(3)` column holding `16:59:59.123` and `25:00:00`, and a `YEAR` column holding `2024`
* *WHEN* each column is mapped and queried
* *THEN* the `TIME` column MUST map to `TIMESTAMP(3)`, read as `TIMESTAMP(DATE '1970-01-01', <column>)`, and emit `1970-01-01 16:59:59.123` and `1970-01-02 01:00:00.000`
* *AND* the `YEAR` column MUST map to `DATE`, read as `MAKEDATE(<column>, 1)`, and emit `2024-01-01`
* *AND* a delegated equality of the `TIME` column with `TIMESTAMP '1970-01-01 16:59:59.123'`, and of the `YEAR` column with `DATE '2024-01-01'`, MUST select the row, as Exasol's comparison of the emitted values does

### Scenario: TIMESTAMP maps to TIMESTAMP in the session time zone

* *GIVEN* a MySQL column of type `TIMESTAMP` with datetime precision 6
* *WHEN* the column is mapped and queried
* *THEN* the result MUST be the Exasol type `TIMESTAMP(6)`, and the column MUST render bare
* *AND* the adapter MUST set no session time zone, so the scan emits each value as its wall-clock time in the session `time_zone`, which the server default sets
* *AND* under a session `time_zone` of `+00:00`, a delegated comparison with a `TIMESTAMP` or `DATE` literal, a delegated `MIN`, `MAX`, `COUNT(DISTINCT)`, and `GROUP BY`, and a delegated equi-join with a `DATETIME(6)` column MUST return Exasol's answers over the emitted values
* *AND* the mapping MUST NOT map it to Exasol `TIMESTAMP WITH LOCAL TIME ZONE`, because Exasol compares such a value in the Exasol session time zone, which no source session follows
