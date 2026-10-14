# Feature: postgresql_type_mapping

States which PostgreSQL column types map to which Exasol types, which are omitted and why, and the source-catalog preconditions the mapping depends on.

<!-- DELTA:CHANGED -->
## Background

- The omission contract itself is framework and owned by `vs-adapter/type-mapping`: an unmappable column is omitted from the virtual table rather than erroring, and the omission is recorded with the source type under the source-neutral field `sourceType` beside the reason. This feature states only PostgreSQL's own mapping table, its omissions, and its source preconditions.
- The mapping reads `information_schema.columns` over the ADBC connection and names each source type by its `data_type` spelling. It is a pure function of the catalog row and performs no I/O.
- Seven PostgreSQL types are mapped: `integer` and `bigint` to `DECIMAL(10,0)` and `DECIMAL(19,0)`, `numeric(p,s)` to `DECIMAL(p,s)`, `character varying(n)` to `VARCHAR(n)` with character set `UTF8`, `date` to `DATE`, `timestamp without time zone` to `TIMESTAMP`, and `double precision` to `DOUBLE`. The set matches MySQL's scope rather than exhausting PostgreSQL's type system, and it produces no Exasol type MySQL's mapping does not already produce.
- The omitted set is wider than MySQL's, and each member has its own reason. `boolean` is omitted because a mapped `BOOLEAN` column would widen the set of Exasol types a source column produces, bring a boolean operand into every predicate path, and need `LITERAL_BOOL`, which neither dialect advertises. `text` and an unconstrained `numeric` are omitted because `information_schema` reports the length or precision as null and Exasol needs an exact one for a type it compares for equality. `timestamptz` — `timestamp with time zone` — is omitted because it and Exasol `TIMESTAMP` carry different instants for one wall-clock value. `uuid`, `jsonb`, and `bytea` are omitted as non-core types outside this release's scope, as are `smallint`, `real`, `time without time zone`, `time with time zone`, `json`, `interval`, `money`, and `ARRAY`.
- `character(n)` is omitted because PostgreSQL blank-pads it. PostgreSQL returns a `character(n)` value padded to n and compares two of them ignoring trailing spaces, so Exasol `VARCHAR` matches neither the value nor the comparison: a delegated predicate would treat `'ab'` and `'ab '` as equal where Exasol comparing two `VARCHAR` values does not. Exasol `CHAR` would match both — measured on Exasol 2025.2.1, a `CHAR(5)` holding `ab` compares equal to `'ab'`, `'ab '`, and `'ab   '` — but four features state the invariant that every delegated character operand is Exasol `VARCHAR`, so Exasol `CHAR` is out of scope here and the column is omitted rather than reopening that invariant. This is where the two mappings' scopes genuinely differ: MySQL `CHAR` maps to Exasol `VARCHAR` because MySQL returns the value with its trailing spaces stripped.
- PostgreSQL declares a collation per column. A delegated comparison over a `character varying` column follows that collation, as `vs-adapter/postgresql-collation` states.
- PostgreSQL has no unsigned integer type and no per-column character set, so MySQL's unsigned-modifier rejection and its per-column character-set rejection have no PostgreSQL counterpart. A database has one encoding, which is the granularity PostgreSQL offers and the granularity the precondition checks.
- The UTF8 encoding precondition exists because the mapping produces Exasol `VARCHAR` with character set `UTF8`. A database of another encoding is refused at creation, before any table is discovered, and on every request that resolves the source `CONNECTION`.
- The exposed source namespace is the `public` schema. The connection address's path component names a PostgreSQL database, which is not a SQL qualifier; `public` is both PostgreSQL's SQL qualifier and its `information_schema` filter value, and it is the one namespace string recorded in `adapterNotes`. Exposing `public` alone is this release's boundary — a virtual schema per source schema is the shape that needs no new property.
- Exasol type bounds constrain the mapping: `DECIMAL` precision at most 36, `VARCHAR` size at most 2000000, and character-type size minimum 1.
<!-- /DELTA:CHANGED -->

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: character varying preserves the declared length and uses UTF8

* *GIVEN* a PostgreSQL column whose `data_type` is `character varying` with a declared length of 100
* *WHEN* the column is mapped
* *THEN* the result MUST be the Exasol type `VARCHAR` with size 100 and character set `UTF8`
* *AND* a declared length of 0 MUST map to size 1, because Exasol's minimum character-type size is 1
* *AND* a `character varying` column whose `character_maximum_length` is null MUST be unmapped, with the reason naming the absent length
* *AND* the mapping MUST NOT read a character set from the column, because PostgreSQL declares an encoding per database rather than per column
<!-- /DELTA:CHANGED -->

<!-- DELTA:REMOVED -->
### Scenario: Character column of a nondeterministic collation is unmapped

* *GIVEN* a PostgreSQL `character varying` column whose declared collation is nondeterministic
* *WHEN* the column is mapped
* *THEN* the result MUST be unmapped, with the stated reason naming the column's collation and identifying it as nondeterministic
* *AND* the mapping MUST NOT map it to Exasol `VARCHAR`, because the delegated equality, `GROUP BY`, and `DISTINCT` this release renders for PostgreSQL carry no collation suffix and would equate values Exasol keeps apart
* *AND* a column whose collation is deterministic, including one declaring no collation of its own, MUST be mapped
* *AND* a collation for which the server reports no determinism flag MUST be treated as deterministic, because a version predating the flag has no nondeterministic collations to distinguish
<!-- /DELTA:REMOVED -->

<!-- DELTA:CHANGED -->
### Scenario: Database whose encoding is not UTF8 is rejected at creation

* *GIVEN* a reachable PostgreSQL database whose server encoding is not `UTF8`
* *WHEN* `CREATE VIRTUAL SCHEMA` runs against it with `DRIVER = 'postgresql'`
* *THEN* the adapter MUST return an error
* *AND* the error message MUST name the encoding the database reported and `UTF8` as the encoding this release requires
* *AND* the adapter MUST NOT create the virtual schema, because the mapping declares every character column `UTF8`
* *AND* the check MUST run before any table is discovered, and on every request that resolves the source `CONNECTION` — `refresh` and `setProperties` as well as `createVirtualSchema`
<!-- /DELTA:CHANGED -->
