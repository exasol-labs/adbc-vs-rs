# Feature: exasol_type_mapping

States which Exasol column types map to which Exasol types when the source is itself Exasol, which are omitted and why, and the source-catalog precondition the mapping depends on.

## Background

- The omission contract is framework and owned by `vs-adapter/type-mapping`: an unmappable column is omitted from the virtual table rather than erroring, and the omission is recorded with the source type under the source-neutral field `sourceType` beside the reason. This feature states only Exasol's own mapping table, its omissions, and its source precondition.
- The mapping reads `SYS.EXA_ALL_COLUMNS` over the ADBC connection and names each source type by its `COLUMN_TYPE` spelling. It reads the declared numbers from `COLUMN_NUM_PREC`, `COLUMN_NUM_SCALE`, and `COLUMN_MAXSIZE` rather than parsing them back out of that spelling, its nullability from `COLUMN_IS_NULLABLE`, and its position from `COLUMN_ORDINAL_POSITION`. It is a pure function of the catalog row and performs no I/O.
- `COLUMN_IS_NULLABLE` is null for a view's column and never null for a table's. Discovery reads table names from `SYS.EXA_ALL_TABLES`, which lists no view, so this release exposes tables only and the flag is always present. A view is not omitted, it is never discovered.
- This is the self-referential case: source and target are the same product, so a mapped column's Exasol type is the source column's own type rather than a translation of a foreign one. Every rule below that omits a column omits it because of a bound this adapter carries, not because Exasol and the source disagree about a value.
- Exasol normalizes its own type aliases in the catalog, so the mapping's input domain is the canonical spellings alone and no alias table is needed. Measured against Exasol 2025.2.1: `INTEGER` reports `DECIMAL(18,0)`, `BIGINT` reports `DECIMAL(36,0)`, `SMALLINT` reports `DECIMAL(9,0)`, and `DOUBLE PRECISION` and `FLOAT` both report `DOUBLE`. The mapping nevertheless treats an unrecognized spelling as an omission naming it, so a source version that normalizes differently loses a column rather than mapping it wrongly.
- Six spellings are mapped, and each maps to the type the source column already declares: `DECIMAL(p,s)` to `DECIMAL(p,s)`, `DOUBLE` to `DOUBLE`, `VARCHAR(n) UTF8` and `VARCHAR(n) ASCII` to `VARCHAR(n)` with character set `UTF8`, `DATE` to `DATE`, and `TIMESTAMP` and `TIMESTAMP(p)` with p at most 3 to `TIMESTAMP`.
- `DECIMAL` is an identity mapping, which makes the framework's precision rejection unreachable from this source. Exasol's own maximum `DECIMAL` precision is 36 and `vs-adapter/type-mapping`'s bound is the same 36, so no Exasol source column can declare a precision Exasol cannot declare. The rule stays stated by the framework and is not restated as a reachable case here.
- A `VARCHAR(n) ASCII` column is declared `VARCHAR(n) UTF8` rather than omitted. Exasol's `ASCII` character set is a subset of its `UTF8` one, `n` counts characters in both, and the ordering of the ASCII subset is the same under either, so the declaration widens the character set without changing a value or a comparison. `crates/adbc-vs-adapter/src/exa_type.rs` renders `UTF8` for every `VARCHAR` it declares, so no other declaration is reachable.
- `TIMESTAMP(p)` with p above 3 is omitted rather than truncated. The declarable set carries no fractional-seconds precision — `ExasolType::Timestamp` renders as a bare `TIMESTAMP`, which is Exasol's default precision of 3 — so a source value with more fractional digits would lose them silently. That is the outcome `vs-adapter/type-mapping`'s decimal rule refuses for a numeric, applied to a temporal. A source column declaring `TIMESTAMP` with no precision, as an Exasol version predating the precision syntax reports, is mapped.
- `TIMESTAMP WITH LOCAL TIME ZONE` is omitted at every precision, because it and Exasol `TIMESTAMP` carry different instants for one wall-clock value once the reading session's time zone differs from the writing session's. This is the same reason `vs-adapter/postgresql-type-mapping` omits `timestamp with time zone`.
- `CHAR(n)` is omitted at every character set, and the reason is this adapter's invariant rather than a semantic gap. Measured on Exasol 2025.2.1: two `CHAR(10)` values holding `ab` and `ab ` compare equal, two `VARCHAR(10)` values holding the same two do not, and a `CHAR(10)` compared against a `VARCHAR(10)` does not either. Four features state the invariant that every delegated character operand is Exasol `VARCHAR`, and `vs-adapter/type-mapping` closes the mapped set against Exasol `CHAR`, so a `CHAR` source column cannot be declared as itself. Unlike PostgreSQL's `character`, mapping it would round-trip exactly, so what the omission costs is a real column an operator can see in their source; lifting the invariant is a later release's decision rather than a local one.
- `BOOLEAN` is omitted for the framework's reason rather than an Exasol one: `vs-adapter/type-mapping` closes the mapped set at `DECIMAL`, `VARCHAR`, `DATE`, `TIMESTAMP`, and `DOUBLE`, and states that no mapped column produces `BOOLEAN`, which only an `EMITS` column declares for a predicate Exasol delegated into the select list.
- `GEOMETRY`, `HASHTYPE`, `INTERVAL DAY TO SECOND`, and `INTERVAL YEAR TO MONTH` are omitted because no member of that closed set represents them. Declaring one as `VARCHAR` would change the type Exasol compares for exact equality and would make every delegated predicate over it a string comparison rather than the source's own.
- The exposed source namespace is the schema the connection address's path component names. An Exasol schema is Exasol's own SQL qualifier, so the address's path component is both the namespace and the qualifier the dialect renders — MySQL's answer rather than PostgreSQL's, which has to substitute `public` because a PostgreSQL database is not a qualifier.
- The schema name is taken from the address verbatim and compared against `SCHEMA_NAME` exactly. Exasol folds an unquoted identifier to upper case, so a schema created without quotes is held in the catalog in upper case and has to be spelled that way in the address; folding the address's value instead would make a schema created under a quoted lower-case name unreachable.
- The source precondition is that the named schema exists, is visible to the connecting user, and is not a virtual schema. `SYS.EXA_ALL_SCHEMAS` answers all three through `SCHEMA_NAME` and `SCHEMA_IS_VIRTUAL`. Refusing a virtual schema is what keeps this technology from chaining one virtual schema through another: every table of a virtual schema is virtual, an Exasol source can be the same Exasol instance the adapter runs in, and a virtual schema served by this adapter named as its own source would re-enter the adapter for every row. No MySQL or PostgreSQL catalog carries the concept, so neither of those technologies states this check.
- Exasol-as-source states no collation floor and no encoding precondition, and the absence is a fact about this technology rather than an omission. MySQL's precondition exists because its rendering needs two named collations, and PostgreSQL's because `COLLATE "C"` reproduces Exasol's order only over UTF-8 bytes. `vs-adapter/exasol-collation` renders no collation at all, because the source's comparison semantics are the target's own.

## Scenarios

### Scenario: DECIMAL and DOUBLE map to the types the source column declares

* *GIVEN* Exasol source columns whose `COLUMN_TYPE` is `DECIMAL(18,0)`, `DECIMAL(36,9)`, and `DOUBLE`
* *WHEN* each column is mapped
* *THEN* `DECIMAL(18,0)` MUST map to the Exasol type `DECIMAL` with precision 18 and scale 0
* *AND* `DECIMAL(36,9)` MUST map to `DECIMAL` with precision 36 and scale 9
* *AND* `DOUBLE` MUST map to the Exasol type `DOUBLE`
* *AND* the mapping MUST read the precision and scale from `COLUMN_NUM_PREC` and `COLUMN_NUM_SCALE` rather than parse them out of the `COLUMN_TYPE` spelling
* *AND* no source column MUST be unmapped over `vs-adapter/type-mapping`'s maximum precision of 36, because Exasol's own maximum is the same 36 and the mapping is an identity

### Scenario: VARCHAR keeps its declared length and is declared UTF8 whichever character set the source uses

* *GIVEN* Exasol source columns whose `COLUMN_TYPE` is `VARCHAR(100) UTF8` and `VARCHAR(11) ASCII`
* *WHEN* each column is mapped
* *THEN* the `UTF8` column MUST map to the Exasol type `VARCHAR` with size 100 and character set `UTF8`
* *AND* the `ASCII` column MUST map to `VARCHAR` with size 11 and character set `UTF8`, because `ASCII` is a subset of `UTF8`, the size counts characters under both, and the ordering of that subset is the same under both
* *AND* the mapping MUST NOT omit the `ASCII` column, and MUST NOT declare a character set other than `UTF8`
* *AND* the size MUST be read from `COLUMN_MAXSIZE`, and a `VARCHAR(2000000) UTF8` column MUST map to size 2000000, which is Exasol's own maximum

### Scenario: DATE and millisecond TIMESTAMP map to their counterparts

* *GIVEN* Exasol source columns whose `COLUMN_TYPE` is `DATE`, `TIMESTAMP(3)`, `TIMESTAMP(0)`, and the bare spelling `TIMESTAMP`
* *WHEN* each column is mapped
* *THEN* `DATE` MUST map to the Exasol type `DATE`
* *AND* `TIMESTAMP(3)`, `TIMESTAMP(0)`, and the bare `TIMESTAMP` MUST each map to the Exasol type `TIMESTAMP`
* *AND* the bare spelling MUST be mapped rather than unmapped, because an Exasol version predating the precision syntax reports its timestamp columns that way

### Scenario: TIMESTAMP the adapter cannot declare is unmapped

* *GIVEN* Exasol source columns whose `COLUMN_TYPE` is `TIMESTAMP(6)`, `TIMESTAMP(9)`, `TIMESTAMP(3) WITH LOCAL TIME ZONE`, and `TIMESTAMP(9) WITH LOCAL TIME ZONE`
* *WHEN* each column is mapped
* *THEN* each result MUST be unmapped
* *AND* the reason for `TIMESTAMP(6)` and `TIMESTAMP(9)` MUST name the precision the source declared and the precision 3 the adapter declares
* *AND* the mapping MUST NOT map either to a bare `TIMESTAMP`, because the fractional digits beyond the third would be lost with no error
* *AND* the reason for either `WITH LOCAL TIME ZONE` column MUST name that the source type and Exasol `TIMESTAMP` carry different instants for one wall-clock value
* *AND* a `WITH LOCAL TIME ZONE` column MUST be unmapped at every precision, including 3, so the rejection follows from the time-zone semantics rather than from the precision

### Scenario: CHAR is unmapped because Exasol compares it blank-padded

* *GIVEN* Exasol source columns whose `COLUMN_TYPE` is `CHAR(32) ASCII` and `CHAR(5) UTF8`
* *WHEN* each column is mapped
* *THEN* each result MUST be unmapped
* *AND* the stated reason MUST name the blank-padded comparison as the cause and MUST name that no mapped column declares Exasol `CHAR`
* *AND* the mapping MUST NOT map either to Exasol `VARCHAR`, because two `CHAR` values differing only in trailing spaces compare equal at the source while the two `VARCHAR` values the adapter would declare do not
* *AND* the mapping MUST NOT map either to Exasol `CHAR`, because `vs-adapter/type-mapping` closes the mapped set against it

### Scenario: BOOLEAN, GEOMETRY, HASHTYPE, and INTERVAL are unmapped

* *GIVEN* Exasol source columns whose `COLUMN_TYPE` is `BOOLEAN`, `GEOMETRY`, `HASHTYPE(16 BYTE)`, `INTERVAL DAY TO SECOND`, and `INTERVAL YEAR TO MONTH`
* *WHEN* each column is mapped
* *THEN* each result MUST be unmapped
* *AND* each stated reason MUST name the `COLUMN_TYPE` spelling that was rejected
* *AND* `BOOLEAN` MUST be unmapped despite the source and the target both having the type, because `vs-adapter/type-mapping` closes the mapped set against it and no mapped column produces it
* *AND* none of them MUST be mapped to Exasol `VARCHAR`, because that would change the type Exasol compares for exact equality and turn every delegated predicate over the column into a string comparison

### Scenario: An unrecognized type spelling is unmapped rather than guessed

* *GIVEN* an Exasol source column whose `COLUMN_TYPE` is a spelling this release's mapping does not recognize
* *WHEN* the column is mapped
* *THEN* the result MUST be unmapped
* *AND* the stated reason MUST name the spelling verbatim, so an operator reads back the text their own catalog holds
* *AND* the mapping MUST NOT infer a type from `COLUMN_TYPE_ID`, from the declared numbers, or from the spelling's leading word alone
* *AND* the mapping MUST remain total, so a source version spelling a type differently loses that one column rather than failing the whole discovery

### Scenario: Virtual schema exposes the schema the connection address names

* *GIVEN* an Exasol source holding one table in the schema the connection address's path component names and one table in a second schema
* *WHEN* the virtual schema is created
* *THEN* the returned `schemaMetadata` MUST list the named schema's table
* *AND* it MUST NOT list the second schema's table
* *AND* `adapterNotes` MUST record the named schema as the source namespace
* *AND* the generated pushdown statement MUST qualify a table with that schema
* *AND* the schema name MUST be compared against `SCHEMA_NAME` exactly as the address spells it, so a schema created under a quoted lower-case name stays reachable

### Scenario: A source schema that is missing or virtual is rejected at creation

* *GIVEN* an Exasol source, and connection addresses naming a schema that does not exist, a schema the connecting user cannot see, and a virtual schema
* *WHEN* `CREATE VIRTUAL SCHEMA` runs against each with `DRIVER = 'exasol'`
* *THEN* the adapter MUST return an error for each
* *AND* the error for a missing or invisible schema MUST name the schema it looked for
* *AND* the error for a virtual schema MUST name the schema and state that a virtual schema cannot be a source
* *AND* the adapter MUST NOT create the virtual schema in any of the three cases, because a virtual schema named as its own source would re-enter this adapter for every row when the source is the same Exasol instance
* *AND* the check MUST run before any table is discovered, and on every request that resolves the source `CONNECTION` — `refresh` and `setProperties` as well as `createVirtualSchema`
