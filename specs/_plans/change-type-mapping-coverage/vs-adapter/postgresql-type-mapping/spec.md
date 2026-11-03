# Feature: postgresql_type_mapping

States which PostgreSQL column types map to which Exasol types, which are omitted and why, and the source-catalog preconditions the mapping depends on.

<!-- DELTA:CHANGED -->
## Background

- The omission contract is framework and owned by `vs-adapter/type-mapping`. This feature states only PostgreSQL's own mapping table, its omissions, and its source preconditions. The mapping reads `information_schema.columns` over the ADBC connection, names each source type by its `data_type` spelling, and performs no I/O.
- Fifteen PostgreSQL `data_type` spellings are mapped: `smallint`, `integer`, and `bigint` to `DECIMAL(5,0)`, `DECIMAL(10,0)`, and `DECIMAL(19,0)`; `numeric(p,s)` to `DECIMAL(p,s)`; an unconstrained `numeric`, `real`, and `double precision` to `DOUBLE`; `character varying(n)` to `VARCHAR(n)`, and an unbounded `character varying`, `text`, `json`, and `jsonb` to `VARCHAR(2000000)`, and `uuid` to `VARCHAR(36)`, each with character set `UTF8`; `date` to `DATE`; `timestamp without time zone`, and `timestamp with time zone` as its UTC wall-clock value, to `TIMESTAMP`; and `boolean` to `BOOLEAN`.
- Several of those types compare differently from their declared Exasol type when PostgreSQL compares them as themselves: an unconstrained `numeric` compares exactly at arbitrary precision, a `uuid` literal is read case-insensitively, `json` has no equality operator, and `jsonb` compares parsed values. They map because the PostgreSQL dialect renders every `VARCHAR` column reference as `text` and every `DOUBLE` column reference as `double precision`, per `vs-adapter/postgresql-pushdown`, so PostgreSQL compares the value the scan emits. A query reading a value longer than 2000000 characters fails rather than returning it truncated.
- `character(n)` is omitted because PostgreSQL blank-pads it and compares two `character` values ignoring trailing spaces, while Exasol `VARCHAR` does neither and no mapped column produces Exasol `CHAR`. `time` types, `interval`, `money`, `bytea`, and `ARRAY` are omitted as types outside this release's scope.
- A database must use the `UTF8` encoding, because the mapping declares every character column `UTF8`, and PostgreSQL declares an encoding per database rather than per column. A column's collation governs a delegated character comparison, per `vs-adapter/postgresql-collation`. The exposed source namespace is the `public` schema, which is both PostgreSQL's SQL qualifier and its `information_schema` filter value and is the one namespace string recorded in `adapterNotes`.
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

<!-- DELTA:REMOVED -->
### Scenario: numeric without a declared precision is unmapped

* *GIVEN* a PostgreSQL column whose `data_type` is `numeric` and whose `numeric_precision` is null, as an unconstrained `numeric` reports
* *WHEN* the column is mapped
* *THEN* the result MUST be unmapped
* *AND* the stated reason MUST name the absent precision
* *AND* the mapping MUST NOT substitute Exasol's maximum precision, because Exasol compares the declared type for exact equality and an arbitrary-precision source value need not fit it
<!-- /DELTA:REMOVED -->

<!-- DELTA:NEW -->
### Scenario: numeric without a declared precision maps to DOUBLE

* *GIVEN* a PostgreSQL column whose `data_type` is `numeric` and whose `numeric_precision` is null, as an unconstrained `numeric` reports
* *WHEN* the column is mapped
* *THEN* the result MUST be the Exasol type `DOUBLE`
* *AND* the mapping MUST NOT declare a `DECIMAL` of Exasol's maximum precision, because an arbitrary-precision source value need not fit it
* *AND* a delegated comparison, grouping key, and `COUNT(DISTINCT)` over the column MUST select the rows Exasol selects over the emitted `DOUBLE` values, so two source values that round to one `DOUBLE` compare equal at the source too
* *AND* a delegated filter or `COUNT` over rows holding `NaN` or an infinity, in a query that does not project the column, MUST succeed and follow PostgreSQL's ordering, which places `NaN` above every number and treats two `NaN` values as equal, because the scan never reads those values
* *AND* a query whose delegated statement converts a value outside the double-precision range MUST fail with PostgreSQL's out-of-range error rather than emit a value
<!-- /DELTA:NEW -->

<!-- DELTA:CHANGED -->
### Scenario: character varying preserves the declared length and uses UTF8

* *GIVEN* a PostgreSQL column whose `data_type` is `character varying` with a declared length of 100
* *WHEN* the column is mapped
* *THEN* the result MUST be the Exasol type `VARCHAR` with size 100 and character set `UTF8`
* *AND* a declared length of 0 MUST map to size 1, because Exasol's minimum character-type size is 1
* *AND* a `character varying` column whose `character_maximum_length` is null MUST map to `VARCHAR` with size 2000000 and character set `UTF8`, as `text` does, because PostgreSQL stores an unbounded `character varying` and a `text` value alike
* *AND* the mapping MUST NOT read a character set from the column, because PostgreSQL declares an encoding per database rather than per column
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: text, uuid, json, and jsonb map to VARCHAR and compare as the text the scan emits

* *GIVEN* PostgreSQL columns whose `data_type` is `text`, `uuid`, `json`, and `jsonb`
* *WHEN* each column is mapped
* *THEN* `text`, `json`, and `jsonb` MUST map to Exasol `VARCHAR` with size 2000000, and `uuid` to Exasol `VARCHAR` with size 36, each with character set `UTF8`
* *AND* a delegated equality between the `uuid` column and an upper-case spelling of a stored value MUST select no row, because the scan emits the lower-case text and Exasol compares it exactly
* *AND* a delegated equality over the `json` column MUST compare the text PostgreSQL stored, and one over the `jsonb` column the text PostgreSQL returns for it, rather than fail for a missing operator or compare parsed JSON values
* *AND* a query reading a `text` value longer than 2000000 characters MUST fail rather than return the value truncated
<!-- /DELTA:NEW -->

<!-- DELTA:CHANGED -->
### Scenario: Temporal and floating-point types map to their Exasol counterparts

* *GIVEN* PostgreSQL columns whose `data_type` is `date`, `timestamp without time zone`, `double precision`, and `real`
* *WHEN* each column is mapped
* *THEN* `date` MUST map to Exasol `DATE`
* *AND* `timestamp without time zone` MUST map to Exasol `TIMESTAMP`
* *AND* `double precision` and `real` MUST map to Exasol `DOUBLE`
* *AND* a `real` value MUST be emitted as its single-precision value widened to double precision, so a stored `0.1` is emitted as `0.100000001490116119384765625` and a delegated equality with the literal `0.1` selects no row, as Exasol's comparison of the emitted value does
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: boolean maps to BOOLEAN

* *GIVEN* a PostgreSQL column whose `data_type` is `boolean`
* *WHEN* the column is mapped
* *THEN* the result MUST be the Exasol type `BOOLEAN`
* *AND* a delegated filter on the column alone, on its negation, and on its equality with a second `boolean` column MUST select the rows Exasol selects over the emitted values
* *AND* a delegated `MIN`, `MAX`, `COUNT(DISTINCT)`, and `GROUP BY` over the column MUST return Exasol's values, with `false` ordered before `true`
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: timestamp with time zone maps to TIMESTAMP holding the UTC wall-clock value

* *GIVEN* a PostgreSQL column whose `data_type` is `timestamp with time zone`, in a database of any default `TimeZone`, including one with daylight saving time, read through a connection whose session `TimeZone` is pinned to UTC, per `scan-udf/postgresql-scan`
* *WHEN* the column is mapped and queried
* *THEN* the result MUST be the Exasol type `TIMESTAMP`
* *AND* the scan MUST emit each value as its UTC wall-clock time, so two instants that share one local wall-clock time across a daylight-saving transition are emitted as two distinct values
* *AND* the mapping MUST NOT map it to Exasol `TIMESTAMP WITH LOCAL TIME ZONE`, because Exasol compares such a value against a `TIMESTAMP` or a `DATE` in the Exasol session time zone, which no source session follows
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: Delegated operations over a timestamp with time zone column follow the UTC session

* *GIVEN* a PostgreSQL `timestamp with time zone` column mapped to Exasol `TIMESTAMP`, read through a connection whose session `TimeZone` is pinned to UTC
* *WHEN* a query delegates a comparison, an aggregate, a grouping, or an equi-join over the column
* *THEN* a delegated comparison of the column with a `TIMESTAMP` literal and with a `DATE` literal MUST select the rows Exasol selects over the emitted values, because PostgreSQL reads each literal in the pinned UTC session
* *AND* a delegated `MIN`, `MAX`, `COUNT(DISTINCT)`, and `GROUP BY` over the column MUST return Exasol's values
* *AND* a delegated equi-join of the column with a `timestamp without time zone` column MUST pair the rows Exasol pairs over the emitted values
<!-- /DELTA:NEW -->

<!-- DELTA:CHANGED -->
### Scenario: Non-core types are unmapped

* *GIVEN* PostgreSQL columns whose `data_type` is `time without time zone`, `time with time zone`, `bytea`, `interval`, `money`, or `ARRAY`
* *WHEN* each column is mapped
* *THEN* each result MUST be unmapped
* *AND* each stated reason MUST name the PostgreSQL type that was rejected
<!-- /DELTA:CHANGED -->
