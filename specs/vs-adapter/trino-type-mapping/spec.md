# Feature: trino_type_mapping

States which Trino column types map to which Exasol types, which are omitted and why, and how the mapping reads a Trino catalog that reports a column's full type only as a string.

## Background

- `vs-adapter/type-mapping` owns the omission contract and the closed type set this mapping obeys.
- Trino reports a column's full type only as the `data_type` string of `<catalog>.information_schema.columns`.
- This feature states which Trino types map, which are omitted and why, and which source namespace a Trino virtual schema exposes.
- `vs-adapter/trino-collation` owns string comparison, including the remote database's collation that applies when a catalog's connector pushes a comparison to that database.

## Scenarios

### Scenario: The column type is parsed out of the catalog's data_type string

* *GIVEN* Trino columns whose `data_type` values are `varchar(100)`, `decimal(12,4)`, `timestamp(6)`, and `integer`
* *WHEN* each column is mapped
* *THEN* the declared length, precision, and scale MUST be taken from the `data_type` string
* *AND* the mapping MUST NOT read a `character_maximum_length`, `numeric_precision`, `numeric_scale`, or `datetime_precision` column, because Trino's `information_schema.columns` has no such column and selecting one fails the catalog query
* *AND* the parser MUST read Trino's display form, which spells a type in lower case as a bare name, a name with one parameter, or a name with a precision and a scale separated by a comma with no space, as Trino spells a `decimal` type
* *AND* a `data_type` value the parser does not recognize MUST leave the column unmapped with the reason naming that value exactly as the catalog spelled it, rather than being read as the bare name that precedes its parameters

### Scenario: Integer types map to DECIMAL with the matching digit count

* *GIVEN* a Trino column whose `data_type` is `integer`
* *WHEN* the column is mapped
* *THEN* the result MUST be the Exasol type `DECIMAL` with precision 10 and scale 0
* *AND* a `bigint` column MUST map to `DECIMAL` with precision 19 and scale 0
* *AND* the mapped types MUST equal the types the other mappings produce for a 32-bit and a 64-bit integer, because the digit counts are properties of the integer widths rather than of any source

### Scenario: decimal preserves the source precision and scale

* *GIVEN* a Trino column whose `data_type` is `decimal(12,4)`
* *WHEN* the column is mapped
* *THEN* the result MUST be the Exasol type `DECIMAL` with precision 12 and scale 4
* *AND* a `decimal(38,2)` column MUST be unmapped, with the reason naming Exasol's maximum `DECIMAL` precision of 36 rather than Trino's own maximum of 38
* *AND* the mapping MUST NOT silently reduce the precision, because that would truncate values

### Scenario: varchar preserves the declared length and uses UTF8

* *GIVEN* a Trino column whose `data_type` is `varchar(100)`
* *WHEN* the column is mapped
* *THEN* the result MUST be the Exasol type `VARCHAR` with size 100 and character set `UTF8`
* *AND* a declared length of 0 MUST map to size 1, because Exasol's minimum character-type size is 1
* *AND* a column whose `data_type` is `varchar` carrying no parentheses MUST be unmapped, with the reason naming the absent length, because that spelling is Trino's unbounded `varchar` rather than a zero-length one
* *AND* a declared length above Exasol's maximum `VARCHAR` size MUST be unmapped, with the reason naming that bound

### Scenario: char is unmapped because it is blank-padded

* *GIVEN* a Trino column whose `data_type` is `char(5)`
* *WHEN* the column is mapped
* *THEN* the result MUST be unmapped, with a stated reason naming the blank padding as the cause, because a delegated `LIKE` over the column sees the padding while a delegated equality does not: Trino's documentation states that `CAST('abc' AS CHAR(5))` equals `'abc'` but does not match `LIKE 'abc'`, so two delegated predicates over one mapped column would disagree
* *AND* the mapping MUST NOT map it to Exasol `VARCHAR`, because Trino returns the value padded to its declared length and compares two `char` values ignoring trailing spaces, so Exasol `VARCHAR` matches neither the value nor the comparison
* *AND* the mapping MUST NOT map it to Exasol `CHAR`, although Exasol `CHAR` would match the padded value, because this release maps a source column to Exasol `CHAR` for PostgreSQL and MySQL only

### Scenario: Temporal and floating-point types map to their Exasol counterparts

* *GIVEN* Trino columns whose `data_type` is `date`, `timestamp(6)`, and `double`
* *WHEN* each column is mapped
* *THEN* `date` MUST map to Exasol `DATE`
* *AND* `timestamp(6)` MUST map to Exasol `TIMESTAMP(3)`
* *AND* a `timestamp(p)` of any other precision Trino admits MUST map to Exasol `TIMESTAMP(3)` as well, because this release carries a source's fractional-second precision for PostgreSQL and MySQL only, so a Trino virtual table declares what it declared before this release, and Trino evaluates a delegated comparison over the column at the column's own precision while the emitted value holds milliseconds
* *AND* `double` MUST map to Exasol `DOUBLE`

### Scenario: Non-core types are unmapped

* *GIVEN* Trino columns whose `data_type` is `boolean`, `real`, `smallint`, `tinyint`, `time(3)`, `time(3) with time zone`, `timestamp(3) with time zone`, `varbinary`, `json`, `uuid`, `ipaddress`, `array(integer)`, `map(varchar, integer)`, `row("a" integer)`, `interval day to second`, or `interval year to month`
* *WHEN* each column is mapped
* *THEN* each result MUST be unmapped
* *AND* each stated reason MUST name the Trino type that was rejected
* *AND* every listed type other than `timestamp(3) with time zone` MUST be unmapped as a type outside this release's Trino scope, `boolean` included, although the PostgreSQL and MySQL mappings produce Exasol `BOOLEAN`
* *AND* `timestamp(3) with time zone` MUST be unmapped rather than mapped to Exasol `TIMESTAMP`, because the two carry different instants for one wall-clock value

### Scenario: No source-server precondition is checked for Trino

* *GIVEN* a reachable Trino server and a `CREATE VIRTUAL SCHEMA` statement naming `DRIVER = 'trino'`
* *WHEN* the adapter runs the source precondition check
* *THEN* the check MUST succeed without querying the server for a collation, a character set, an encoding, or a version, which is a decision rather than an omission, because `vs-adapter/create-virtual-schema` calls for a precondition only for what a technology's rendering or type mapping depends on
* *AND* the adapter MUST NOT refuse the statement over any server property, because everything the Trino rendering depends on is fixed in Trino's SQL engine: code-point comparison, no collation mechanism, the double-quote identifier form, typed literals, and no per-column character set or per-database encoding, while no Trino catalog reports the collation a remote database applies when a connector pushes a comparison to it
* *AND* the adapter MUST NOT refuse a server for its version, because no construct the Trino dialect renders needs a release older than Trino 335, which introduced the variable-precision `timestamp(p)` type, and no supported ADBC Trino driver reaches a server older than that
* *AND* the check MUST still run at the same point every other driver's runs, so the framework contract `vs-adapter/create-virtual-schema` states holds with no driver excepted

### Scenario: Virtual schema exposes one catalog and one schema

* *GIVEN* a Trino server holding a table in the catalog and schema the connection address names and a table in a second schema of the same catalog
* *WHEN* the virtual schema is created
* *THEN* the column catalog MUST be read from `<catalog>.information_schema.columns`, because Trino scopes `information_schema` to one catalog, and the returned `schemaMetadata` MUST list the table of the named schema
* *AND* it MUST NOT list the table of the second schema
* *AND* `adapterNotes` MUST record the catalog and the schema, in that order, as the source namespace, and the generated pushdown statement MUST qualify a table with both of them, because a Trino table's fully qualified name is `catalog.schema.table`, so both are needed and neither is derivable from the other
* *AND* a view in the named schema MUST be listed beside the base tables, because Trino's `information_schema.tables` reports each relation as `BASE TABLE` or `VIEW` and a view is a queryable relation with the same catalog shape
