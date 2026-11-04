# Feature: postgresql_type_mapping

States which PostgreSQL numeric and temporal column types map to which Exasol types, which are omitted and why, and the source-catalog preconditions the mapping depends on.

## Background

- `vs-adapter/type-mapping` owns the omission contract and the closed type set this mapping obeys.
- This feature states PostgreSQL's mapping table, each departure from the Exasol PostgreSQL JDBC Virtual Schema's mapping, the `UTF8` encoding precondition, and the exposed source namespace.
- A column's read expression, where a scenario names one, renders per `vs-adapter/pushdown-dialect`.
- `vs-adapter/postgresql-collation` owns how a character column compares.
- `vs-adapter/postgresql-extended-type-mapping` states the mapping of character, boolean, bit, `money`, and text-read types.

## Scenarios

### Scenario: Integer types map to DECIMAL with the matching digit count

* *GIVEN* a PostgreSQL column whose `data_type` is `integer`
* *WHEN* the column is mapped
* *THEN* the result MUST be the Exasol type `DECIMAL` with precision 10 and scale 0
* *AND* a `bigint` column MUST map to `DECIMAL` with precision 19 and scale 0
* *AND* a `smallint` column MUST map to `DECIMAL` with precision 5 and scale 0
* *AND* the mapped types MUST equal the types the MySQL mapping produces for `INT`, `BIGINT`, and `SMALLINT`, because the digit counts are properties of the integer widths rather than of the source

### Scenario: numeric preserves the source precision and scale

* *GIVEN* a PostgreSQL column whose `data_type` is `numeric` with precision 12 and scale 4
* *WHEN* the column is mapped
* *THEN* the result MUST be the Exasol type `DECIMAL` with precision 12 and scale 4, and the column MUST render bare
* *AND* a `numeric` column with precision 40 and scale 2 MUST map to `VARCHAR` with size 2000000 and character set `UTF8`, read as `CAST(<column> AS TEXT)`
* *AND* the mapping MUST NOT silently reduce the precision, because that would truncate values

### Scenario: Temporal and floating-point types map to their Exasol counterparts

* *GIVEN* PostgreSQL columns whose `data_type` is `date`, `timestamp without time zone` with `datetime_precision` 6 and 0, `double precision`, and `real`
* *WHEN* each column is mapped
* *THEN* `date` MUST map to Exasol `DATE`
* *AND* the `timestamp without time zone` columns MUST map to `TIMESTAMP(6)` and `TIMESTAMP(0)`, their own fractional-second precision
* *AND* `double precision` and `real` MUST map to Exasol `DOUBLE`, and every listed column MUST render bare
* *AND* a `real` value MUST be emitted as its single-precision value widened to double precision, so a stored `0.1` is emitted as `0.100000001490116119384765625` and a delegated equality with the literal `0.1` selects no row, as Exasol's comparison of the emitted value does

### Scenario: Non-core types are unmapped

* *GIVEN* PostgreSQL columns whose `data_type` is `ARRAY`, such as `text[]` and `integer[]`
* *WHEN* each column is mapped
* *THEN* each result MUST be unmapped, as the JDBC Virtual Schema leaves arrays out
* *AND* each stated reason MUST name `ARRAY` as the PostgreSQL type that was rejected

### Scenario: Database whose encoding is not UTF8 is rejected at creation

* *GIVEN* a reachable PostgreSQL database whose server encoding is not `UTF8`
* *WHEN* `CREATE VIRTUAL SCHEMA` runs against it with `DRIVER = 'postgresql'`
* *THEN* the adapter MUST return an error
* *AND* the error message MUST name the encoding the database reported and `UTF8` as the encoding this release requires
* *AND* the adapter MUST NOT create the virtual schema, because the mapping declares every character column `UTF8`
* *AND* the check MUST run before any table is discovered, and on every request that resolves the source `CONNECTION` — `refresh` and `setProperties` as well as `createVirtualSchema`

### Scenario: Virtual schema exposes the public schema only

* *GIVEN* a PostgreSQL database holding one table in `public` and one table in a second schema
* *WHEN* the virtual schema is created
* *THEN* the returned `schemaMetadata` MUST list the `public` table
* *AND* it MUST NOT list the table of the second schema, because exposing `public` alone is this release's boundary, and a virtual schema per source schema is the shape that needs no new property
* *AND* `adapterNotes` MUST record `public` as the source namespace, which is both PostgreSQL's SQL qualifier and its `information_schema` filter value, so one namespace string serves both
* *AND* the generated pushdown statement MUST qualify a table with that namespace rather than with the database the connection address names

### Scenario: The mapping is a pure function of the PostgreSQL catalog row

* *GIVEN* a row of PostgreSQL's `information_schema.columns`
* *WHEN* the column is mapped
* *THEN* the result MUST depend only on that row
* *AND* the mapping MUST name each source type by the row's `data_type` spelling
* *AND* the mapping MUST perform no I/O

### Scenario: numeric without a declared precision maps to VARCHAR read as its text

* *GIVEN* a PostgreSQL column whose `data_type` is `numeric` and whose `numeric_precision` is null, holding `1.0`, `1.00`, `NaN`, `Infinity`, and `1e400`
* *WHEN* the column is mapped and queried
* *THEN* the result MUST be the Exasol type `VARCHAR` with size 2000000 and character set `UTF8`
* *AND* every reference to the column MUST render as `CAST(<column> AS TEXT)`
* *AND* the scan MUST emit each value as PostgreSQL's text for it, so `1.0` and `1.00` are two distinct values, and `NaN`, `Infinity`, and `1e400` are emitted as text rather than failing the query
* *AND* a delegated equality with the string `'1.0'` MUST select the row holding `1.0` and not the row holding `1.00`, as Exasol's comparison of the emitted text does

### Scenario: timestamp with time zone maps to TIMESTAMP holding the UTC wall-clock value

* *GIVEN* a PostgreSQL column whose `data_type` is `timestamp with time zone` with `datetime_precision` 6
* *WHEN* the column is mapped and queried
* *THEN* the result MUST be the Exasol type `TIMESTAMP(6)` rather than `TIMESTAMP WITH LOCAL TIME ZONE`, which Exasol compares in the Exasol session time zone that no source session follows, and the column MUST render bare
* *AND* the scan MUST emit each value as its UTC wall-clock time, whatever `TimeZone` the source session uses
* *AND* the adapter MUST set no session time zone, so a delegated comparison of the column with a `TIMESTAMP` or `DATE` literal, and a delegated equi-join with a `timestamp without time zone` column, follow the session `TimeZone` that the database or the role sets
* *AND* under a session `TimeZone` of `UTC`, those delegated comparisons and joins, and a delegated `MIN`, `MAX`, `COUNT(DISTINCT)`, and `GROUP BY` over the column, MUST return Exasol's answers over the emitted values
