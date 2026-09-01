# Feature: postgresql_type_mapping

States which PostgreSQL column types map to which Exasol types, which are omitted and why, and the source-catalog preconditions the mapping depends on.

## Background

- The omission contract itself is framework and owned by `vs-adapter/type-mapping`: an unmappable column is omitted from the virtual table rather than erroring, and the omission is recorded with the source type under the source-neutral field `sourceType` beside the reason. This feature states only PostgreSQL's own mapping table, its omissions, and its source preconditions.
- The mapping reads `information_schema.columns` over the ADBC connection and names each source type by its `data_type` spelling. It is a pure function of the catalog row and performs no I/O.
- Seven PostgreSQL types are mapped: `integer` and `bigint` to `DECIMAL(10,0)` and `DECIMAL(19,0)`, `numeric(p,s)` to `DECIMAL(p,s)`, `character varying(n)` to `VARCHAR(n)` with character set `UTF8`, `date` to `DATE`, `timestamp without time zone` to `TIMESTAMP`, and `double precision` to `DOUBLE`. The set matches MySQL's scope rather than exhausting PostgreSQL's type system, and it produces no Exasol type MySQL's mapping does not already produce.
- The omitted set is wider than MySQL's, and each member has its own reason. `boolean` is omitted because a mapped `BOOLEAN` column would widen the set of Exasol types a source column produces, bring a boolean operand into every predicate path, and need `LITERAL_BOOL`, which neither dialect advertises. `text` and an unconstrained `numeric` are omitted because `information_schema` reports the length or precision as null and Exasol needs an exact one for a type it compares for equality. `timestamptz` — `timestamp with time zone` — is omitted because it and Exasol `TIMESTAMP` carry different instants for one wall-clock value. `uuid`, `jsonb`, and `bytea` are omitted as non-core types outside this release's scope, as are `smallint`, `real`, `time without time zone`, `time with time zone`, `json`, `interval`, `money`, and `ARRAY`.
- `character(n)` is omitted because PostgreSQL blank-pads it. PostgreSQL returns a `character(n)` value padded to n and compares two of them ignoring trailing spaces, so Exasol `VARCHAR` matches neither the value nor the comparison: a delegated predicate would treat `'ab'` and `'ab '` as equal where Exasol comparing two `VARCHAR` values does not. Exasol `CHAR` would match both — measured on Exasol 2025.2.1, a `CHAR(5)` holding `ab` compares equal to `'ab'`, `'ab '`, and `'ab   '` — but four features state the invariant that every delegated character operand is Exasol `VARCHAR`, so Exasol `CHAR` is out of scope here and the column is omitted rather than reopening that invariant. This is where the two mappings' scopes genuinely differ: MySQL `CHAR` maps to Exasol `VARCHAR` because MySQL returns the value with its trailing spaces stripped.
- PostgreSQL declares a collation per column, and a nondeterministic one is unmappable. `vs-adapter/postgresql-collation` renders equality, `LIKE`, `GROUP BY`, and `DISTINCT` with no collation suffix, which is correct only while a deterministic collation equates two character values just when their bytes are identical. A nondeterministic collation equates values Exasol keeps apart and makes `LIKE` fail outright, so the column is refused rather than exposed. Every collation PostgreSQL ships is deterministic, so this reaches only a collation an operator created.
- PostgreSQL has no unsigned integer type and no per-column character set, so MySQL's unsigned-modifier rejection and its per-column character-set rejection have no PostgreSQL counterpart. A database has one encoding, which is the granularity PostgreSQL offers and the granularity the precondition checks.
- The UTF8 encoding precondition exists because the mapping produces Exasol `VARCHAR` with character set `UTF8`, and because the ordering collation `vs-adapter/postgresql-collation` renders reproduces Exasol's code-point order only while the server compares UTF-8 bytes. A database of another encoding is refused at creation, before any table is discovered, and on every request that resolves the source `CONNECTION`.
- The exposed source namespace is the `public` schema. The connection address's path component names a PostgreSQL database, which is not a SQL qualifier; `public` is both PostgreSQL's SQL qualifier and its `information_schema` filter value, and it is the one namespace string recorded in `adapterNotes`. Exposing `public` alone is this release's boundary — a virtual schema per source schema is the shape that needs no new property.
- Exasol type bounds constrain the mapping: `DECIMAL` precision at most 36, `VARCHAR` size at most 2000000, and character-type size minimum 1.

## Scenarios

### Scenario: Integer types map to DECIMAL with the matching digit count

* *GIVEN* a PostgreSQL column whose `data_type` is `integer`
* *WHEN* the column is mapped
* *THEN* the result MUST be the Exasol type `DECIMAL` with precision 10 and scale 0
* *AND* a `bigint` column MUST map to `DECIMAL` with precision 19 and scale 0
* *AND* the mapped types MUST equal the types the MySQL mapping produces for `INT` and `BIGINT`, because the digit counts are properties of the integer widths rather than of the source

### Scenario: numeric preserves the source precision and scale

* *GIVEN* a PostgreSQL column whose `data_type` is `numeric` with precision 12 and scale 4
* *WHEN* the column is mapped
* *THEN* the result MUST be the Exasol type `DECIMAL` with precision 12 and scale 4
* *AND* a `numeric` column with precision 40 and scale 2 MUST be unmapped, with the reason naming Exasol's maximum `DECIMAL` precision of 36
* *AND* the mapping MUST NOT silently reduce the precision, because that would truncate values

### Scenario: numeric without a declared precision is unmapped

* *GIVEN* a PostgreSQL column whose `data_type` is `numeric` and whose `numeric_precision` is null, as an unconstrained `numeric` reports
* *WHEN* the column is mapped
* *THEN* the result MUST be unmapped
* *AND* the stated reason MUST name the absent precision
* *AND* the mapping MUST NOT substitute Exasol's maximum precision, because Exasol compares the declared type for exact equality and an arbitrary-precision source value need not fit it

### Scenario: character varying preserves the declared length and uses UTF8

* *GIVEN* a PostgreSQL column whose `data_type` is `character varying` with a declared length of 100
* *WHEN* the column is mapped
* *THEN* the result MUST be the Exasol type `VARCHAR` with size 100 and character set `UTF8`
* *AND* a declared length of 0 MUST map to size 1, because Exasol's minimum character-type size is 1
* *AND* a `character varying` column whose `character_maximum_length` is null MUST be unmapped, with the reason naming the absent length
* *AND* the mapping MUST NOT read a character set from the column, because PostgreSQL declares an encoding per database rather than per column

### Scenario: Character column of a nondeterministic collation is unmapped

* *GIVEN* a PostgreSQL `character varying` column whose declared collation is nondeterministic
* *WHEN* the column is mapped
* *THEN* the result MUST be unmapped, with the stated reason naming the column's collation and identifying it as nondeterministic
* *AND* the mapping MUST NOT map it to Exasol `VARCHAR`, because the delegated equality, `GROUP BY`, and `DISTINCT` this release renders for PostgreSQL carry no collation suffix and would equate values Exasol keeps apart
* *AND* a column whose collation is deterministic, including one declaring no collation of its own, MUST be mapped
* *AND* a collation for which the server reports no determinism flag MUST be treated as deterministic, because a version predating the flag has no nondeterministic collations to distinguish

### Scenario: character is unmapped because it is blank-padded

* *GIVEN* a PostgreSQL column whose `data_type` is `character` with a declared length of 5
* *WHEN* the column is mapped
* *THEN* the result MUST be unmapped
* *AND* the stated reason MUST name the blank padding as the cause
* *AND* the mapping MUST NOT map it to Exasol `VARCHAR`, because PostgreSQL returns the value padded to its declared length and compares two `character` values ignoring trailing spaces, while Exasol comparing two `VARCHAR` values does neither
* *AND* the mapping MUST NOT map it to Exasol `CHAR`, because no mapped column produces Exasol `CHAR` in this release

### Scenario: Temporal and floating-point types map to their Exasol counterparts

* *GIVEN* PostgreSQL columns whose `data_type` is `date`, `timestamp without time zone`, and `double precision`
* *WHEN* each column is mapped
* *THEN* `date` MUST map to Exasol `DATE`
* *AND* `timestamp without time zone` MUST map to Exasol `TIMESTAMP`
* *AND* `double precision` MUST map to Exasol `DOUBLE`

### Scenario: Non-core types are unmapped

* *GIVEN* PostgreSQL columns whose `data_type` is `boolean`, `text`, `smallint`, `real`, `time without time zone`, `time with time zone`, `timestamp with time zone`, `uuid`, `json`, `jsonb`, `bytea`, `interval`, `money`, or `ARRAY`
* *WHEN* each column is mapped
* *THEN* each result MUST be unmapped
* *AND* each stated reason MUST name the PostgreSQL type that was rejected
* *AND* `boolean` MUST be unmapped despite Exasol having a `BOOLEAN` type, because a mapped `BOOLEAN` column would widen the set of Exasol types a source column produces
* *AND* `timestamp with time zone` MUST be unmapped rather than mapped to Exasol `TIMESTAMP`, because the two carry different instants for one wall-clock value

### Scenario: Database whose encoding is not UTF8 is rejected at creation

* *GIVEN* a reachable PostgreSQL database whose server encoding is not `UTF8`
* *WHEN* `CREATE VIRTUAL SCHEMA` runs against it with `DRIVER = 'postgresql'`
* *THEN* the adapter MUST return an error
* *AND* the error message MUST name the encoding the database reported and `UTF8` as the encoding this release requires
* *AND* the adapter MUST NOT create the virtual schema, because the mapping declares every character column `UTF8` and the ordering collation the dialect renders reproduces Exasol's code-point order only over UTF-8 bytes
* *AND* the check MUST run before any table is discovered, and on every request that resolves the source `CONNECTION` — `refresh` and `setProperties` as well as `createVirtualSchema`

### Scenario: Virtual schema exposes the public schema only

* *GIVEN* a PostgreSQL database holding one table in `public` and one table in a second schema
* *WHEN* the virtual schema is created
* *THEN* the returned `schemaMetadata` MUST list the `public` table
* *AND* it MUST NOT list the table of the second schema
* *AND* `adapterNotes` MUST record `public` as the source namespace
* *AND* the generated pushdown statement MUST qualify a table with that namespace rather than with the database the connection address names
