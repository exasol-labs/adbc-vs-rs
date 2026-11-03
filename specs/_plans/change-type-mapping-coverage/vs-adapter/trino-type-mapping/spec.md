# Feature: trino_type_mapping

States which Trino column types map to which Exasol types, which are omitted and why, and how the mapping reads a Trino catalog that reports a column's full type only as a string.

<!-- DELTA:CHANGED -->
## Background

- The omission contract is framework and owned by `vs-adapter/type-mapping`. The mapping reads `<catalog>.information_schema.columns`, which Trino scopes to one catalog, and exposes the base tables and views of the catalog and the schema the connection address's path names, in that order.
- Trino reports a column's full type only as the `data_type` string, with no length, precision, or scale column, so the mapping parses Trino's display form: a bare name, a name with one parameter, or a precision and a scale separated by a comma with no space, all lower case. A `varchar` with no parameter is Trino's unbounded `varchar`, and a name this release does not map is carried into the omission's reason as the catalog spelled it.
- Seven Trino types are mapped: `integer` and `bigint` to `DECIMAL(10,0)` and `DECIMAL(19,0)`, `decimal(p,s)` up to precision 36 to `DECIMAL(p,s)`, `varchar(n)` to `VARCHAR(n)` with character set `UTF8`, `date` to `DATE`, `timestamp(p)` to `TIMESTAMP(3)`, and `double` to `DOUBLE`. This release carries a source's fractional-second precision for PostgreSQL and MySQL only, so a delegated comparison over a Trino timestamp acts at Trino's precision while Exasol holds milliseconds.
- An unbounded `varchar` is omitted because the catalog reports no length. `timestamp(p) with time zone` is omitted because it and Exasol `TIMESTAMP` carry different instants for one wall-clock value. `boolean`, `real`, `smallint`, `tinyint`, both `time` types, `varbinary`, `json`, `uuid`, `ipaddress`, an `array`, a `map`, a `row`, and both `interval` types are omitted as types outside this release's Trino scope.
- Trino declares no collation, no per-column character set, and no per-database encoding, so this technology states no source-server precondition, and the oldest release any rendered construct needs is 335, which introduced `timestamp(p)`. A catalog whose connector pushes a string comparison to a remote database applies that database's collation, which `vs-adapter/trino-collation` owns.
<!-- /DELTA:CHANGED -->

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: char is unmapped because it is blank-padded

* *GIVEN* a Trino column whose `data_type` is `char(5)`
* *WHEN* the column is mapped
* *THEN* the result MUST be unmapped
* *AND* the stated reason MUST name the blank padding as the cause
* *AND* the mapping MUST NOT map it to Exasol `VARCHAR`, because Trino returns the value padded to its declared length and a delegated `LIKE` over it sees the padding while a delegated equality over it does not, so two delegated predicates over one mapped column would disagree
* *AND* the mapping MUST NOT map it to Exasol `CHAR`, because this release maps a source column to Exasol `CHAR` for PostgreSQL and MySQL only
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Temporal and floating-point types map to their Exasol counterparts

* *GIVEN* Trino columns whose `data_type` is `date`, `timestamp(6)`, and `double`
* *WHEN* each column is mapped
* *THEN* `date` MUST map to Exasol `DATE`
* *AND* `timestamp(6)` MUST map to Exasol `TIMESTAMP(3)`
* *AND* a `timestamp(p)` of any other precision Trino admits MUST map to Exasol `TIMESTAMP(3)` as well, so a Trino virtual table declares what it declared before this release
* *AND* `double` MUST map to Exasol `DOUBLE`
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Non-core types are unmapped

* *GIVEN* Trino columns whose `data_type` is `boolean`, `real`, `smallint`, `tinyint`, `time(3)`, `time(3) with time zone`, `timestamp(3) with time zone`, `varbinary`, `json`, `uuid`, `ipaddress`, `array(integer)`, `map(varchar, integer)`, `row("a" integer)`, `interval day to second`, or `interval year to month`
* *WHEN* each column is mapped
* *THEN* each result MUST be unmapped
* *AND* each stated reason MUST name the Trino type that was rejected
* *AND* `boolean` MUST be unmapped as a type outside this release's Trino scope, although the PostgreSQL and MySQL mappings produce Exasol `BOOLEAN`
* *AND* `timestamp(3) with time zone` MUST be unmapped rather than mapped to Exasol `TIMESTAMP`, because the two carry different instants for one wall-clock value
<!-- /DELTA:CHANGED -->
