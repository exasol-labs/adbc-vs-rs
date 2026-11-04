# Feature: postgresql_extended_type_mapping

States how PostgreSQL character, boolean, bit, `money`, and text-read column types map to Exasol types.

## Background

- `vs-adapter/type-mapping` owns the omission contract and the closed type set this mapping obeys.
- `vs-adapter/postgresql-type-mapping` owns PostgreSQL's core mapping table, which these types extend, the `UTF8` encoding precondition, and the exposed source namespace.
- A column's read expression, where a scenario names one, renders per `vs-adapter/pushdown-dialect`.
- `vs-adapter/postgresql-collation` owns how a character column compares.

## Scenarios

### Scenario: character varying preserves the declared length and uses UTF8

* *GIVEN* a PostgreSQL column whose `data_type` is `character varying` with a declared length of 100
* *WHEN* the column is mapped
* *THEN* the result MUST be the Exasol type `VARCHAR` with size 100 and character set `UTF8`, read from no column character set, because PostgreSQL declares an encoding per database rather than per column, and the column MUST render bare
* *AND* a declared length of 0 MUST map to size 1, because Exasol's minimum character-type size is 1
* *AND* a `character varying` column whose `character_maximum_length` is null or exceeds 2000000, and a `text` column, MUST map to `VARCHAR` with size 2000000 and character set `UTF8` and render bare
* *AND* a query reading a value longer than 2000000 characters MUST fail rather than return the value truncated

### Scenario: character maps to CHAR

* *GIVEN* PostgreSQL columns whose `data_type` is `character` with declared lengths 5 and 3000, each holding `ab`, and a `character varying` column holding `ab`
* *WHEN* each column is mapped and queried
* *THEN* the length-5 column MUST map to Exasol `CHAR` with size 5 and character set `UTF8` and render bare, and a declared length above 2000000 MUST map to `VARCHAR` with size 2000000
* *AND* the length-3000 column MUST map to `VARCHAR` with size 3000 and character set `UTF8` and render as `CAST(<column> AS TEXT)`, because Exasol's `CHAR` holds at most 2000 characters, so the scan emits `ab` without the trailing blanks and a delegated equality with the literal `'ab'` selects the row, as Exasol's comparison of the emitted `VARCHAR` value does
* *AND* the scan MUST emit the length-5 value as `ab` padded to five characters, and a delegated equality of the length-5 column with the literal `'ab'` MUST select the row, as Exasol's comparison of a `CHAR` value with a string literal does
* *AND* a delegated equality of the length-5 column with the `character varying` column MUST pair the row, because PostgreSQL ignores the trailing blanks of a `character` value, although Exasol's own comparison of `CHAR(5)` `ab` with `VARCHAR` `ab` finds them unequal

### Scenario: boolean and bit map to BOOLEAN

* *GIVEN* PostgreSQL columns whose `data_type` is `boolean` and `bit` with a declared length of 1, each holding true, false, and null, and a `bit` column with a declared length of 8 holding `00000101`, `00001001`, and `00000000`
* *WHEN* each column is mapped and queried
* *THEN* each MUST map to Exasol `BOOLEAN`, as the JDBC Virtual Schema maps every `bit` width, with the `boolean` column rendered bare and each `bit` column rendered as `(position(B'1' IN <column>) > 0)`, which is true when any bit is set
* *AND* the length-8 column MUST emit true, true, and false, so two different source values read as one `true`
* *AND* a delegated filter on each column alone, on its negation, and on its equality with the other columns MUST select the rows Exasol selects over the emitted values
* *AND* a delegated `MIN`, `MAX`, `COUNT(DISTINCT)`, and `GROUP BY` over each column MUST return Exasol's values, with `false` ordered before `true`, so a `GROUP BY` over the length-8 column puts `00000101` and `00001001` in one group

### Scenario: money maps to DOUBLE

* *GIVEN* a PostgreSQL column whose `data_type` is `money`, holding `12.34`
* *WHEN* the column is mapped and queried
* *THEN* the result MUST be the Exasol type `DOUBLE`
* *AND* every reference to the column MUST render as `CAST(CAST(<column> AS NUMERIC) AS DOUBLE PRECISION)`, because PostgreSQL defines no direct conversion from `money` to `double precision`
* *AND* the scan MUST emit `12.34` as a `DOUBLE` under a session `lc_monetary` with two fractional digits, because PostgreSQL scales the stored amount by that setting
* *AND* a delegated comparison with the literal `12.34` MUST select the row, as Exasol's comparison of the emitted value does

### Scenario: Types without an Exasol counterpart map to VARCHAR read as their text

* *GIVEN* PostgreSQL columns whose `data_type` is `uuid`, `json`, `jsonb`, `xml`, `bytea`, `interval`, `inet`, `cidr`, `macaddr`, `point`, `tsvector`, `bit varying`, `time without time zone`, `time with time zone`, and `USER-DEFINED` for an enum type
* *WHEN* each column is mapped and queried
* *THEN* `bit varying` with a declared length n MUST map to `VARCHAR` with size n, an unbounded `bit varying` to `VARCHAR` with size 2000000, both `time` types to `VARCHAR` with size 100, and every other listed type to `VARCHAR` with size 2000000, each with character set `UTF8`
* *AND* every reference to each column MUST render as `CAST(<column> AS TEXT)`, so the scan emits PostgreSQL's text for the value
* *AND* the `bytea` column MUST be emitted in the text form PostgreSQL's `bytea_output` setting selects, which is hex by default
* *AND* a query reading a text longer than 2000000 characters MUST fail rather than return the value truncated

### Scenario: A value read as text compares as the text the scan emits

* *GIVEN* PostgreSQL `uuid`, `json`, `jsonb`, and `xml` columns, each mapped to `VARCHAR` and read as `CAST(<column> AS TEXT)`
* *WHEN* a query delegates an equality over each column
* *THEN* an equality between the `uuid` column and an upper-case spelling of a stored value MUST select no row, because the scan emits the lower-case text and Exasol compares it exactly
* *AND* an equality over the `json` and `xml` columns MUST compare the stored text rather than fail for a missing operator
* *AND* an equality over the `jsonb` column MUST compare PostgreSQL's text for the value rather than the parsed JSON value
