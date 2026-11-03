# Feature: postgresql_type_mapping

States which PostgreSQL column types map to which Exasol types, which are omitted and why, and the source-catalog preconditions the mapping depends on.

<!-- DELTA:CHANGED -->
## Background

- The omission contract is framework and owned by `vs-adapter/type-mapping`. The mapping reads `information_schema.columns`, names each source type by its `data_type` spelling, and performs no I/O. It starts from the Exasol PostgreSQL JDBC Virtual Schema's mapping, and this feature states every departure.
- A column the mapping reads through a conversion renders as that read expression in every position, per `vs-adapter/pushdown-dialect`, and every other column renders bare.
- A database must use the `UTF8` encoding, because the mapping declares every character column `UTF8` and PostgreSQL declares an encoding per database rather than per column. A column's collation governs a delegated character comparison, per `vs-adapter/postgresql-collation`.
- The exposed source namespace is the `public` schema, which is both PostgreSQL's SQL qualifier and its `information_schema` filter value and is the one namespace string recorded in `adapterNotes`.
<!-- /DELTA:CHANGED -->

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Integer types map to DECIMAL with the matching digit count

* *GIVEN* a PostgreSQL column whose `data_type` is `integer`
* *WHEN* the column is mapped
* *THEN* the result MUST be the Exasol type `DECIMAL` with precision 10 and scale 0
* *AND* a `bigint` column MUST map to `DECIMAL` with precision 19 and scale 0
* *AND* a `smallint` column MUST map to `DECIMAL` with precision 5 and scale 0
* *AND* the mapped types MUST equal the types the MySQL mapping produces for `INT`, `BIGINT`, and `SMALLINT`, because the digit counts are properties of the integer widths rather than of the source
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: numeric preserves the source precision and scale

* *GIVEN* a PostgreSQL column whose `data_type` is `numeric` with precision 12 and scale 4
* *WHEN* the column is mapped
* *THEN* the result MUST be the Exasol type `DECIMAL` with precision 12 and scale 4, and the column MUST render bare
* *AND* a `numeric` column with precision 40 and scale 2 MUST map to `VARCHAR` with size 2000000 and character set `UTF8`, read as `CAST(<column> AS TEXT)`
* *AND* the mapping MUST NOT silently reduce the precision, because that would truncate values
<!-- /DELTA:CHANGED -->

<!-- DELTA:REMOVED -->
### Scenario: numeric without a declared precision is unmapped

* *GIVEN* a PostgreSQL column whose `data_type` is `numeric` and whose `numeric_precision` is null, as an unconstrained `numeric` reports
* *WHEN* the column is mapped
* *THEN* the result MUST be unmapped
* *AND* the stated reason MUST name the absent precision
* *AND* the mapping MUST NOT substitute Exasol's maximum precision, because Exasol compares the declared type for exact equality and an arbitrary-precision source value need not fit it
<!-- /DELTA:REMOVED -->

<!-- DELTA:NEW -->
### Scenario: numeric without a declared precision maps to VARCHAR read as its text

* *GIVEN* a PostgreSQL column whose `data_type` is `numeric` and whose `numeric_precision` is null, holding `1.0`, `1.00`, `NaN`, `Infinity`, and `1e400`
* *WHEN* the column is mapped and queried
* *THEN* the result MUST be the Exasol type `VARCHAR` with size 2000000 and character set `UTF8`
* *AND* every reference to the column MUST render as `CAST(<column> AS TEXT)`
* *AND* the scan MUST emit each value as PostgreSQL's text for it, so `1.0` and `1.00` are two distinct values, and `NaN`, `Infinity`, and `1e400` are emitted as text rather than failing the query
* *AND* a delegated equality with the string `'1.0'` MUST select the row holding `1.0` and not the row holding `1.00`, as Exasol's comparison of the emitted text does
<!-- /DELTA:NEW -->

<!-- DELTA:CHANGED -->
### Scenario: character varying preserves the declared length and uses UTF8

* *GIVEN* a PostgreSQL column whose `data_type` is `character varying` with a declared length of 100
* *WHEN* the column is mapped
* *THEN* the result MUST be the Exasol type `VARCHAR` with size 100 and character set `UTF8`, read from no column character set, because PostgreSQL declares an encoding per database rather than per column, and the column MUST render bare
* *AND* a declared length of 0 MUST map to size 1, because Exasol's minimum character-type size is 1
* *AND* a `character varying` column whose `character_maximum_length` is null or exceeds 2000000, and a `text` column, MUST map to `VARCHAR` with size 2000000 and character set `UTF8` and render bare
* *AND* a query reading a value longer than 2000000 characters MUST fail rather than return the value truncated
<!-- /DELTA:CHANGED -->

<!-- DELTA:REMOVED -->
### Scenario: character is unmapped because it is blank-padded

* *GIVEN* a PostgreSQL column whose `data_type` is `character` with a declared length of 5
* *WHEN* the column is mapped
* *THEN* the result MUST be unmapped
* *AND* the stated reason MUST name the blank padding as the cause
* *AND* the mapping MUST NOT map it to Exasol `VARCHAR`, because PostgreSQL returns the value padded to its declared length and compares two `character` values ignoring trailing spaces, while Exasol comparing two `VARCHAR` values does neither
* *AND* the mapping MUST NOT map it to Exasol `CHAR`, because no mapped column produces Exasol `CHAR` in this release
<!-- /DELTA:REMOVED -->

<!-- DELTA:NEW -->
### Scenario: character maps to CHAR

* *GIVEN* PostgreSQL columns whose `data_type` is `character` with declared lengths 5 and 3000, each holding `ab`, and a `character varying` column holding `ab`
* *WHEN* each column is mapped and queried
* *THEN* the length-5 column MUST map to Exasol `CHAR` with size 5 and character set `UTF8`, and render bare
* *AND* the length-3000 column MUST map to `VARCHAR` with size 3000 and character set `UTF8`, because Exasol's `CHAR` holds at most 2000 characters, and render as `CAST(<column> AS TEXT)`, which drops the trailing blanks, so the scan emits `ab` without padding and a delegated equality with the literal `'ab'` selects the row, as Exasol's comparison of the emitted `VARCHAR` value does; a declared length above 2000000 MUST map to `VARCHAR` with size 2000000
* *AND* the scan MUST emit the length-5 value as `ab` padded to five characters, and a delegated equality of the length-5 column with the literal `'ab'` MUST select the row, as Exasol's comparison of a `CHAR` value with a string literal does
* *AND* a delegated equality of the length-5 column with the `character varying` column MUST pair the row, because PostgreSQL ignores the trailing blanks of a `character` value, although Exasol's own comparison of `CHAR(5)` `ab` with `VARCHAR` `ab` finds them unequal
<!-- /DELTA:NEW -->

<!-- DELTA:CHANGED -->
### Scenario: Temporal and floating-point types map to their Exasol counterparts

* *GIVEN* PostgreSQL columns whose `data_type` is `date`, `timestamp without time zone` with `datetime_precision` 6 and 0, `double precision`, and `real`
* *WHEN* each column is mapped
* *THEN* `date` MUST map to Exasol `DATE`
* *AND* the `timestamp without time zone` columns MUST map to `TIMESTAMP(6)` and `TIMESTAMP(0)`, their own fractional-second precision
* *AND* `double precision` and `real` MUST map to Exasol `DOUBLE`, and every listed column MUST render bare
* *AND* a `real` value MUST be emitted as its single-precision value widened to double precision, so a stored `0.1` is emitted as `0.100000001490116119384765625` and a delegated equality with the literal `0.1` selects no row, as Exasol's comparison of the emitted value does
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: boolean and bit map to BOOLEAN

* *GIVEN* PostgreSQL columns whose `data_type` is `boolean` and `bit` with a declared length of 1, each holding true, false, and null, and a `bit` column with a declared length of 8 holding `00000101`, `00001001`, and `00000000`
* *WHEN* each column is mapped and queried
* *THEN* each MUST map to Exasol `BOOLEAN`, as the JDBC Virtual Schema maps every `bit` width, the `boolean` column MUST render bare, and each `bit` column MUST render as `(position(B'1' IN <column>) > 0)`, which is true when any bit is set
* *AND* the length-8 column MUST emit true, true, and false, so two different source values read as one `true`
* *AND* a delegated filter on each column alone, on its negation, and on its equality with the other columns MUST select the rows Exasol selects over the emitted values
* *AND* a delegated `MIN`, `MAX`, `COUNT(DISTINCT)`, and `GROUP BY` over each column MUST return Exasol's values, with `false` ordered before `true`, so a `GROUP BY` over the length-8 column puts `00000101` and `00001001` in one group
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: money maps to DOUBLE

* *GIVEN* a PostgreSQL column whose `data_type` is `money`, holding `12.34`
* *WHEN* the column is mapped and queried
* *THEN* the result MUST be the Exasol type `DOUBLE`
* *AND* every reference to the column MUST render as `CAST(CAST(<column> AS NUMERIC) AS DOUBLE PRECISION)`, because PostgreSQL defines no direct conversion from `money` to `double precision`
* *AND* the scan MUST emit `12.34` as a `DOUBLE`, whatever `lc_monetary` the source session uses
* *AND* a delegated comparison with the literal `12.34` MUST select the row, as Exasol's comparison of the emitted value does
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: Types without an Exasol counterpart map to VARCHAR read as their text

* *GIVEN* PostgreSQL columns whose `data_type` is `uuid`, `json`, `jsonb`, `xml`, `bytea`, `interval`, `inet`, `cidr`, `macaddr`, `point`, `tsvector`, `bit varying`, `time without time zone`, `time with time zone`, and `USER-DEFINED` for an enum type
* *WHEN* each column is mapped and queried
* *THEN* `bit varying` with a declared length n MUST map to `VARCHAR` with size n, an unbounded `bit varying` to `VARCHAR` with size 2000000, both `time` types to `VARCHAR` with size 100, and every other listed type to `VARCHAR` with size 2000000, each with character set `UTF8`
* *AND* every reference to each column MUST render as `CAST(<column> AS TEXT)`, so the scan emits PostgreSQL's text for the value
* *AND* the `bytea` column MUST be emitted in the text form PostgreSQL's `bytea_output` setting selects, which is hex by default
* *AND* a query reading a text longer than 2000000 characters MUST fail rather than return the value truncated
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: A value read as text compares as the text the scan emits

* *GIVEN* PostgreSQL `uuid`, `json`, `jsonb`, and `xml` columns, each mapped to `VARCHAR` and read as `CAST(<column> AS TEXT)`
* *WHEN* a query delegates an equality over each column
* *THEN* an equality between the `uuid` column and an upper-case spelling of a stored value MUST select no row, because the scan emits the lower-case text and Exasol compares it exactly
* *AND* an equality over the `json` and `xml` columns MUST compare the stored text rather than fail for a missing operator
* *AND* an equality over the `jsonb` column MUST compare PostgreSQL's text for the value rather than the parsed JSON value
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: timestamp with time zone maps to TIMESTAMP holding the UTC wall-clock value

* *GIVEN* a PostgreSQL column whose `data_type` is `timestamp with time zone` with `datetime_precision` 6
* *WHEN* the column is mapped and queried
* *THEN* the result MUST be the Exasol type `TIMESTAMP(6)` rather than `TIMESTAMP WITH LOCAL TIME ZONE`, which Exasol compares in the Exasol session time zone that no source session follows, and the column MUST render bare
* *AND* the scan MUST emit each value as its UTC wall-clock time, whatever `TimeZone` the source session uses
* *AND* the adapter MUST set no session time zone, so a delegated comparison of the column with a `TIMESTAMP` or `DATE` literal, and a delegated equi-join with a `timestamp without time zone` column, follow the session `TimeZone` that the database or the role sets
* *AND* under a session `TimeZone` of `UTC`, those delegated comparisons and joins, and a delegated `MIN`, `MAX`, `COUNT(DISTINCT)`, and `GROUP BY` over the column, MUST return Exasol's answers over the emitted values
<!-- /DELTA:NEW -->

<!-- DELTA:CHANGED -->
### Scenario: Non-core types are unmapped

* *GIVEN* PostgreSQL columns whose `data_type` is `ARRAY`, such as `text[]` and `integer[]`
* *WHEN* each column is mapped
* *THEN* each result MUST be unmapped, as the JDBC Virtual Schema leaves arrays out
* *AND* each stated reason MUST name `ARRAY` as the PostgreSQL type that was rejected
<!-- /DELTA:CHANGED -->
