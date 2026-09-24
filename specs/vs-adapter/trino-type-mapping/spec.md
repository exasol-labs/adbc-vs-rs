# Feature: trino_type_mapping

States which Trino column types map to which Exasol types, which are omitted and why, and how the mapping reads a Trino catalog that reports a column's full type only as a string.

## Background

- The omission contract itself is framework and owned by `vs-adapter/type-mapping`: an unmappable column is omitted from the virtual table rather than erroring, and the omission is recorded with the source type under the source-neutral field `sourceType` beside the reason. This feature states only Trino's own mapping table, its omissions, and the catalog shape it reads.
- The mapping reads `<catalog>.information_schema.columns` over the ADBC connection. Trino scopes `information_schema` to one catalog, so the query is qualified by the catalog the virtual schema exposes and every row it returns belongs to that catalog.
- Trino's `information_schema.columns` reports a column's full type only as the string in its `data_type` column. It has no `character_maximum_length`, `numeric_precision`, `numeric_scale`, or `datetime_precision` column at all — those names are absent rather than null, so selecting one fails. The mapping therefore parses `data_type`, which is the only place a declared length, precision, or scale exists. That is what separates this mapping from MySQL's and PostgreSQL's, whose catalogs report those values as columns of their own.
- The `data_type` grammar the parser reads is Trino's own type display form: a bare name, a name carrying one parameter, or a name carrying a precision and a scale separated by a comma with no space. `varchar(100)`, `decimal(12,4)`, `timestamp(6)`, `char(5)`, `integer`, `bigint`, `double`, and `date` are the spellings this release reads, all lower case. A name this release does not map is carried into the omission's reason exactly as the catalog spelled it, whatever its shape, so a parameterized type the parser does not recognize is omitted rather than misread.
- `varchar` carrying no parameter is Trino's unbounded `varchar` rather than a zero-length one, and the two are told apart by the parentheses alone. The distinction decides an omission rather than a size.
- Seven Trino types are mapped: `integer` and `bigint` to `DECIMAL(10,0)` and `DECIMAL(19,0)`, `decimal(p,s)` to `DECIMAL(p,s)`, `varchar(n)` to `VARCHAR(n)` with character set `UTF8`, `date` to `DATE`, `timestamp(p)` to `TIMESTAMP` for every precision Trino admits, and `double` to `DOUBLE`. The set matches the scope the other two mappings cover rather than exhausting Trino's type system, and it produces no Exasol type either of them does not already produce.
- Trino's maximum `DECIMAL` precision is 38 and Exasol's is 36, so a `decimal(p,s)` whose precision exceeds 36 is unmapped for Exasol's bound rather than Trino's. That bound belongs to `vs-adapter/type-mapping` and is read rather than restated.
- `char(n)` is omitted because Trino blank-pads it and compares two `char` values ignoring trailing spaces, so Exasol `VARCHAR` matches neither the value nor the comparison. Trino adds a second reason the other sources do not: its documentation states that `char` comparison ignores trailing spaces while pattern matching does not, so `CAST('abc' AS CHAR(5)) LIKE 'abc'` is expected to be false while `CAST('abc' AS CHAR(5)) = 'abc'` is true, and a delegated `LIKE` over a mapped `char` column would therefore disagree with a delegated equality over the same column. Gate 2.2 measures that disagreement against a live Trino; if the two agree, this reason falls away and the blank padding stands alone. Exasol `CHAR` would match the padded value, but four features state the invariant that every delegated character operand is Exasol `VARCHAR`, so it stays out of scope here and the column is omitted.
- An unbounded `varchar` is omitted because the catalog reports no length and Exasol needs an exact one for a type it compares for equality.
- `timestamp(p) with time zone` is omitted because it and Exasol `TIMESTAMP` carry different instants for one wall-clock value. `boolean` is omitted because a mapped `BOOLEAN` column would widen the set of Exasol types a source column produces, bring a boolean operand into every predicate path, and need `LITERAL_BOOL`, which no dialect advertises. `real`, `smallint`, `tinyint`, `time(p)`, `time(p) with time zone`, `varbinary`, `json`, `uuid`, `ipaddress`, an `array`, a `map`, a `row`, and both `interval` types are omitted as types outside this release's scope.
- Trino declares no collation on a column and offers no collation mechanism at all, so MySQL's collation check and PostgreSQL's per-column determinism check have no Trino counterpart. Trino declares no per-column character set and no per-database encoding either, so PostgreSQL's encoding precondition has none.
- This dialect states no source-server precondition, and the absence is a decision rather than an omission. `vs-adapter/create-virtual-schema` requires a precondition only for what a dialect's rendering depends on, and everything Trino's rendering depends on is fixed in Trino's SQL engine rather than configurable: code-point comparison, the absence of collation, the double-quote identifier form, and the typed-literal requirement are the same on every Trino server. The oldest release any rendered construct needs is 335, which introduced the variable-precision `timestamp(p)` type, and no supported ADBC Trino driver reaches a server older than that.
- One source property this release depends on is real and unenforceable, and `vs-adapter/trino-collation` owns it: a catalog whose connector pushes a string comparison down to a remote database applies that database's collation rather than Trino's. No catalog Trino exposes reports it, so no check here can read it.
- The exposed source namespace is the catalog and the schema the connection address's path names, in that order. A Trino table's fully qualified name is `catalog.schema.table`, so both are needed and neither is derivable from the other; `vs-adapter/create-virtual-schema` owns the recorded shape and `vs-adapter/trino-pushdown` owns the rendering.
- Exasol type bounds constrain the mapping: `DECIMAL` precision at most 36, `VARCHAR` size at most 2000000, and character-type size minimum 1.
- Both base tables and views are exposed. Trino's `information_schema.tables` reports each as `BASE TABLE` or `VIEW`, and a view is a queryable relation with the same catalog shape.

## Scenarios

### Scenario: The column type is parsed out of the catalog's data_type string

* *GIVEN* Trino columns whose `data_type` values are `varchar(100)`, `decimal(12,4)`, `timestamp(6)`, and `integer`
* *WHEN* each column is mapped
* *THEN* the declared length, precision, and scale MUST be taken from the `data_type` string
* *AND* the mapping MUST NOT read a `character_maximum_length`, `numeric_precision`, `numeric_scale`, or `datetime_precision` column, because Trino's `information_schema.columns` has no such column and selecting one fails the catalog query
* *AND* a `data_type` value the parser does not recognize MUST leave the column unmapped with the reason naming that value exactly as the catalog spelled it, rather than being read as the bare name that precedes its parameters
* *AND* the parser MUST read a precision and a scale separated by a comma with no space, which is how Trino spells a `decimal` type

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
* *THEN* the result MUST be unmapped
* *AND* the stated reason MUST name the blank padding as the cause
* *AND* the mapping MUST NOT map it to Exasol `VARCHAR`, because Trino returns the value padded to its declared length and a delegated `LIKE` over it sees the padding while a delegated equality over it does not, so two delegated predicates over one mapped column would disagree
* *AND* the mapping MUST NOT map it to Exasol `CHAR`, because no mapped column produces Exasol `CHAR` in this release

### Scenario: Temporal and floating-point types map to their Exasol counterparts

* *GIVEN* Trino columns whose `data_type` is `date`, `timestamp(6)`, and `double`
* *WHEN* each column is mapped
* *THEN* `date` MUST map to Exasol `DATE`
* *AND* `timestamp(6)` MUST map to Exasol `TIMESTAMP`
* *AND* a `timestamp(p)` of any other precision Trino admits MUST map to Exasol `TIMESTAMP` as well, because Exasol declares one timestamp type whatever fractional precision the source carries
* *AND* `double` MUST map to Exasol `DOUBLE`

### Scenario: Non-core types are unmapped

* *GIVEN* Trino columns whose `data_type` is `boolean`, `real`, `smallint`, `tinyint`, `time(3)`, `time(3) with time zone`, `timestamp(3) with time zone`, `varbinary`, `json`, `uuid`, `ipaddress`, `array(integer)`, `map(varchar, integer)`, `row("a" integer)`, `interval day to second`, or `interval year to month`
* *WHEN* each column is mapped
* *THEN* each result MUST be unmapped
* *AND* each stated reason MUST name the Trino type that was rejected
* *AND* `boolean` MUST be unmapped despite Exasol having a `BOOLEAN` type, because a mapped `BOOLEAN` column would widen the set of Exasol types a source column produces
* *AND* `timestamp(3) with time zone` MUST be unmapped rather than mapped to Exasol `TIMESTAMP`, because the two carry different instants for one wall-clock value

### Scenario: No source-server precondition is checked for Trino

* *GIVEN* a reachable Trino server and a `CREATE VIRTUAL SCHEMA` statement naming `DRIVER = 'trino'`
* *WHEN* the adapter runs the source precondition check
* *THEN* the check MUST succeed without querying the server for a collation, a character set, an encoding, or a version
* *AND* the adapter MUST NOT refuse the statement over any server property, because everything the Trino rendering depends on is fixed in Trino's SQL engine rather than configurable
* *AND* the check MUST still run at the same point every other driver's runs, so the framework contract `vs-adapter/create-virtual-schema` states holds with no driver excepted

### Scenario: Virtual schema exposes one catalog and one schema

* *GIVEN* a Trino server holding a table in the catalog and schema the connection address names and a table in a second schema of the same catalog
* *WHEN* the virtual schema is created
* *THEN* the returned `schemaMetadata` MUST list the table of the named schema
* *AND* it MUST NOT list the table of the second schema
* *AND* `adapterNotes` MUST record the catalog and the schema, in that order, as the source namespace
* *AND* the generated pushdown statement MUST qualify a table with both of them
* *AND* a view in the named schema MUST be listed beside the base tables
