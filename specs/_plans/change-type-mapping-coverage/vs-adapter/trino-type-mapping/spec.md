# Feature: trino_type_mapping

States which Trino column types map to which Exasol types, which are omitted and why, and how the mapping reads a Trino catalog that reports a column's full type only as a string.

<!-- DELTA:CHANGED -->
## Background

- The omission contract is framework and owned by `vs-adapter/type-mapping`. This feature states only Trino's own mapping table, its omissions, and the catalog shape it reads. The mapping reads `<catalog>.information_schema.columns`, which Trino scopes to one catalog, and exposes the base tables and views of the catalog and the schema the connection address's path names, in that order.
- Trino reports a column's full type only as the `data_type` string, with no length, precision, or scale column at all, so the mapping parses that string in Trino's display form: a bare name, a name with one parameter, or a name with a precision and a scale separated by a comma with no space, all lower case. A `varchar` with no parameter is Trino's unbounded `varchar`, told apart from a sized one by the parentheses alone. A name this release does not map is carried into the omission's reason exactly as the catalog spelled it.
- Seven Trino types are mapped: `integer` and `bigint` to `DECIMAL(10,0)` and `DECIMAL(19,0)`, `decimal(p,s)` to `DECIMAL(p,s)` up to Exasol's precision bound of 36, `varchar(n)` to `VARCHAR(n)` with character set `UTF8`, `date` to `DATE`, `timestamp(p)` to `TIMESTAMP` for every precision Trino admits, and `double` to `DOUBLE`.
- `char(n)` is omitted because Trino blank-pads it, compares two `char` values ignoring trailing spaces, and does not ignore them in `LIKE`. An unbounded `varchar` is omitted because the catalog reports no length. `timestamp(p) with time zone` is omitted because it and Exasol `TIMESTAMP` carry different instants for one wall-clock value. `boolean`, `real`, `smallint`, `tinyint`, both `time` types, `varbinary`, `json`, `uuid`, `ipaddress`, an `array`, a `map`, a `row`, and both `interval` types are omitted as types outside this release's Trino scope.
- Trino declares no collation, no per-column character set, and no per-database encoding, so this technology states no source-server precondition: everything its rendering depends on is fixed in Trino's SQL engine, and the oldest release any rendered construct needs is 335, which introduced `timestamp(p)`. A catalog whose connector pushes a string comparison to a remote database applies that database's collation, which `vs-adapter/trino-collation` owns.
<!-- /DELTA:CHANGED -->

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Non-core types are unmapped

* *GIVEN* Trino columns whose `data_type` is `boolean`, `real`, `smallint`, `tinyint`, `time(3)`, `time(3) with time zone`, `timestamp(3) with time zone`, `varbinary`, `json`, `uuid`, `ipaddress`, `array(integer)`, `map(varchar, integer)`, `row("a" integer)`, `interval day to second`, or `interval year to month`
* *WHEN* each column is mapped
* *THEN* each result MUST be unmapped
* *AND* each stated reason MUST name the Trino type that was rejected
* *AND* `boolean` MUST be unmapped as a type outside this release's Trino scope, although the PostgreSQL mapping produces Exasol `BOOLEAN`
* *AND* `timestamp(3) with time zone` MUST be unmapped rather than mapped to Exasol `TIMESTAMP`, because the two carry different instants for one wall-clock value
<!-- /DELTA:CHANGED -->
