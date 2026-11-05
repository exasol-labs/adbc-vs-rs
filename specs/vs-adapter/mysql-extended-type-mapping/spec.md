# Feature: mysql_extended_type_mapping

States how MySQL character, text, `ENUM`, `SET`, and `JSON` columns map to Exasol types, and how a grouped converted column renders.

## Background

- `vs-adapter/type-mapping` owns the omission contract and the closed type set this mapping obeys.
- `vs-adapter/mysql-type-mapping` owns MySQL's core mapping table, which these types extend, and each departure from the Exasol MySQL JDBC Virtual Schema's mapping.
- A column's read expression, where a scenario names one, renders per `vs-adapter/pushdown-dialect`.
- `vs-adapter/mysql-collation` owns how a character column compares.

## Scenarios

### Scenario: Character types preserve the declared length and use UTF8

* *GIVEN* MySQL columns of type `VARCHAR` with declared character lengths 100 and 0, and of type `CHAR` with declared lengths 10 and 0
* *WHEN* each column is mapped
* *THEN* the length-100 `VARCHAR` column MUST map to Exasol `VARCHAR` with size 100 and character set `UTF8`, and render bare
* *AND* the length-10 `CHAR` column MUST map to Exasol `CHAR` with size 10 and character set `UTF8`, as the JDBC Virtual Schema maps it, and render bare
* *AND* the length-0 `VARCHAR` column MUST map to `VARCHAR` with size 1, and the length-0 `CHAR` column to `CHAR` with size 1, because Exasol's minimum character-type size is 1 and Exasol rejects a `CHAR(0)` declaration

### Scenario: A CHAR value emits padded and compares as MySQL compares it

* *GIVEN* a MySQL `CHAR(10)` column holding `ab`, mapped to Exasol `CHAR(10)`
* *WHEN* the column is scanned and filtered by delegated predicates
* *THEN* the scan MUST emit `ab` padded to ten characters, because, measured on the test MySQL, MySQL returns a `CHAR` value without trailing spaces, and Exasol pads a value it reads into a `CHAR` column to the declared size
* *AND* a delegated equality of the column with the literal `'ab'` MUST select the row, as Exasol's comparison of a `CHAR` value with a string literal does
* *AND* a delegated `LIKE 'ab'` over the column MUST select the row, because MySQL matches the unpadded value, although Exasol's own `LIKE 'ab'` over `CHAR(10)` `ab` matches no row

### Scenario: Character columns of every character set map to UTF8

* *GIVEN* MySQL `VARCHAR`, `CHAR`, and `TEXT` columns whose character set is `latin1`, each holding `café`
* *WHEN* each column is mapped and queried
* *THEN* each MUST map as a `utf8mb4` column of the same type and length maps, with character set `UTF8`, as the JDBC Virtual Schema declares every character column whatever its MySQL character set
* *AND* the scan MUST emit `café` with its accented character intact
* *AND* a delegated equality of each column with the literal `'café'` MUST select the row
* *AND* a delegated equality of each column with a literal holding a character `latin1` cannot hold, such as `'ł'`, MUST fail with MySQL's illegal-mix-of-collations error rather than select or drop rows, because the column's collation takes precedence over the `utf8mb4` literal and MySQL cannot convert the literal to `latin1`

### Scenario: Text types map to VARCHAR bounded by their storage limit

* *GIVEN* MySQL columns of type `TINYTEXT`, `TEXT`, `MEDIUMTEXT`, and `LONGTEXT`, whose catalog lengths are 255, 65535, 16777215, and 4294967295
* *WHEN* each column is mapped
* *THEN* `TINYTEXT` MUST map to `VARCHAR` with size 255, and `TEXT` to `VARCHAR` with size 65535, each with character set `UTF8`
* *AND* `MEDIUMTEXT` and `LONGTEXT` MUST map to `VARCHAR` with size 2000000 and character set `UTF8`, because their limits exceed Exasol's maximum `VARCHAR` size
* *AND* a query reading a value longer than 2000000 characters MUST fail rather than return the value truncated

### Scenario: A select-list predicate over a grouped converted column is delegated

* *GIVEN* `pushdown` requests whose `groupBy` names a MySQL `YEAR` column or a `BIT(8)` column, each carrying in its select list the grouped column and a predicate reading it outside any aggregate, such as `Y = DATE '2024-01-01'` and `NOT BITS`
* *WHEN* the MySQL dialect renders each statement and MySQL executes it
* *THEN* `GROUP BY` MUST carry the column's read expression, and each select-list reference to the grouped column outside an aggregate MUST render as `ANY_VALUE(<read expression>)`
* *AND* the statement MUST execute at the source rather than being rejected with MySQL error 1055
* *AND* the rows MUST equal what Exasol returns for the same query over a local copy of the emitted values
* *AND* a grouped column that renders bare MUST keep its bare select-list reference, with no `ANY_VALUE`

### Scenario: ENUM and SET map to CHAR

* *GIVEN* a MySQL `ENUM('ab','abcde')` column holding `ab` and a `SET('x','yz')` column, whose catalog lengths are 5 and 4
* *WHEN* each column is mapped and queried
* *THEN* the `ENUM` column MUST map to Exasol `CHAR` with size 5 and the `SET` column to `CHAR` with size 4, each with character set `UTF8`, and each MUST render bare
* *AND* a column whose catalog length exceeds 2000 MUST map to `VARCHAR` of that length, because Exasol's `CHAR` holds at most 2000 characters
* *AND* the scan MUST emit the `ENUM` value `ab` padded to five characters, because MySQL returns an `ENUM` or `SET` value without trailing spaces and Exasol pads a value it reads into a `CHAR` column to the declared size, and a delegated equality of the `ENUM` column with the literal `'ab'` MUST select the row, as Exasol's comparison of a `CHAR` value with a string literal does
* *AND* a delegated `LIKE 'ab'` over the `ENUM` column MUST select the row, because MySQL matches the unpadded member, although Exasol's own `LIKE 'ab'` over `CHAR(5)` `ab` matches no row

### Scenario: JSON maps to VARCHAR read as its text

* *GIVEN* a MySQL `JSON` column holding `{"a": 1}`
* *WHEN* the column is mapped and queried
* *THEN* the result MUST be `VARCHAR` with size 2000000 and character set `UTF8`, read as `CAST(<column> AS CHAR)`
* *AND* the scan MUST emit MySQL's text for the value
* *AND* a delegated equality with the literal `'{"a": 1}'` MUST compare that text rather than a parsed JSON value
