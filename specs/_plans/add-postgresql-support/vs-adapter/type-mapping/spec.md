# Feature: type_mapping

Converts a source column definition into the Exasol data type the virtual table declares, and refuses any type it cannot represent without loss or delegate a predicate over.

## Background

<!-- DELTA:CHANGED -->
- The mapping is a pure function of the source catalog row: data type name, character length, numeric precision, numeric scale, and — where the source has them — the unsigned flag and the column character set. It performs no I/O.
- One mapping exists per driver, because the catalog row it reads is the source's own shape and no common row survives reduction to a shared struct without losing the fields one source has and the other does not. Each driver's mapping lives with that driver's metadata discovery, which is the module that already knows the catalog's column names, so `vs-adapter/create-virtual-schema` receives mapped columns and branches on no driver itself.
- Both mappings produce the same five Exasol types — `DECIMAL`, `VARCHAR`, `DATE`, `TIMESTAMP`, and `DOUBLE`. The produced set is one set for the release rather than one per driver, so the closure argument `vs-adapter/pushdown-dialect` makes over the declarable `EMITS` types holds unchanged with a second driver: no new Exasol type enters the system.
- The mapping is driven by source catalog metadata rather than by the Arrow type the ADBC driver returns at scan time. The `adbc-drivers/mysql` driver documents that `DATETIME` comes back as `timestamp[s]`, `timestamp[ms]`, or `timestamp[us]` depending on how the query is written, so the Arrow type is not a stable basis for a declared Exasol type. The same reasoning applies to any driver: the declared type is what Exasol checks for exact equality, and the returned Arrow type is an input the scan coerces.
- Release 0.1 maps exactly eight MySQL types: `INT`, `BIGINT`, `DECIMAL`, `VARCHAR`, `CHAR`, `DATE`, `DATETIME`, `DOUBLE`. Every other MySQL type is unmapped.
- This release maps seven PostgreSQL types, named by their `information_schema.columns.data_type` spelling: `integer`, `bigint`, `numeric`, `character varying`, `date`, `timestamp without time zone`, `double precision`. Every other PostgreSQL type is unmapped. The set is chosen to match MySQL's scope rather than to exhaust PostgreSQL's type system.
- PostgreSQL `character` is unmapped, which is where the two mappings' scopes genuinely differ rather than merely renaming a type. MySQL `CHAR` maps to Exasol `VARCHAR` because MySQL returns a `CHAR` value with its trailing spaces stripped, so `VARCHAR` is the type that matches the value the source returns. PostgreSQL returns a `character(n)` value blank-padded to n and compares two of them ignoring trailing spaces, so `VARCHAR` matches neither the value nor the comparison: a delegated predicate would treat `'ab'` and `'ab '` as equal where Exasol comparing two `VARCHAR` values does not. Exasol `CHAR` would match both — measured on Exasol 2025.2.1, a `CHAR(5)` holding `ab` compares equal to `'ab'`, `'ab '`, and `'ab   '`, exactly as PostgreSQL's `character(n)` does — but no mapped column produces Exasol `CHAR` in this release, and four features state the invariant that every delegated character operand is Exasol `VARCHAR`. Mapping `character(n)` therefore stays out of scope rather than reopening that invariant, and the column is omitted with a reason naming the padding.
- PostgreSQL has no unsigned integer type, so the unsigned-modifier rejection has no PostgreSQL counterpart, and `smallint` is unmapped for the same reason MySQL's `SMALLINT` is: the mapped set is deliberately narrow, not exhaustive.
- PostgreSQL has no per-column character set. A database has one encoding, so the per-column character-set rejection MySQL needs has no PostgreSQL counterpart; `vs-adapter/create-virtual-schema` checks the database encoding once at creation instead, which is the granularity PostgreSQL offers.
- PostgreSQL does declare a collation per column, and one kind of collation is unmappable. `vs-adapter/pushdown-filter` renders a PostgreSQL equality comparison, a `GROUP BY` key, and a `DISTINCT` operand with no collation suffix, which is correct only because a deterministic collation equates two character values just when their bytes are identical — Exasol's own equality. A nondeterministic collation breaks that: it equates values Exasol keeps apart, which is the silent wrong-rows failure MySQL's default collation causes, and it makes `LIKE` fail outright. Every collation PostgreSQL ships is deterministic, so this reaches only a collation an operator created, but the consequence is severe enough to refuse the column rather than to document it. The rejection is the structural counterpart of MySQL's character-set rejection: in both, a character column whose comparison cannot be delegated correctly is omitted rather than exposed.
- PostgreSQL `boolean` is unmapped in this release even though Exasol has `BOOLEAN` and the `EMITS` clause can already declare it. Mapping it would make `BOOLEAN` a type a source column produces, which widens the mapped set that `vs-adapter/pushdown-dialect`'s closure argument is built on, brings a boolean operand into every predicate path, and would need `LITERAL_BOOL` — a capability neither dialect advertises. The two type sets stay modeled as two types, and `BOOLEAN` stays declarable-but-unmappable.
- A PostgreSQL `numeric` carrying no declared precision, and a `character varying` carrying no declared length, are unmapped: `information_schema` reports the field as null, and Exasol needs an exact precision or size for a declared type it compares for equality. `text` is unmapped for the same reason it has no length to declare.
- "Unmapped" is not an error. The caller omits the column from the virtual table and records the omission, so one unsupported column never blocks a whole schema. The recorded omission names the source type in a field spelled for any source rather than for MySQL alone; a note written by an earlier release under the MySQL-specific spelling is still read back.
- Exasol type bounds constrain both mappings: `DECIMAL` precision at most 36, `VARCHAR` size at most 2000000, and both character types minimum size 1. Exasol's `CHAR` size bound of 2000 constrains nothing, because this release produces no Exasol `CHAR`.
- The MySQL mapping reads the column's MySQL character set alongside its length. Filter, join, and aggregation pushdown render a character operand with an explicit `COLLATE utf8mb4_0900_as_cs` suffix to reproduce Exasol's case-sensitive, accent-sensitive comparison, and MySQL rejects that collation for any character set other than `utf8mb4` with error 1253. A character column of another character set is therefore unmapped rather than mapped into a virtual table whose predicates cannot be delegated.
<!-- /DELTA:CHANGED -->

## Scenarios

<!-- DELTA:NEW -->
### Scenario: PostgreSQL integer types map to DECIMAL with the matching digit count

* *GIVEN* a PostgreSQL column whose `data_type` is `integer`
* *WHEN* the column is mapped
* *THEN* the result MUST be the Exasol type `DECIMAL` with precision 10 and scale 0
* *AND* a `bigint` column MUST map to `DECIMAL` with precision 19 and scale 0
* *AND* the mapped types MUST equal the types the MySQL mapping produces for `INT` and `BIGINT`, because the digit counts are properties of the integer widths rather than of the source
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: PostgreSQL numeric preserves the source precision and scale

* *GIVEN* a PostgreSQL column whose `data_type` is `numeric` with precision 12 and scale 4
* *WHEN* the column is mapped
* *THEN* the result MUST be the Exasol type `DECIMAL` with precision 12 and scale 4
* *AND* a `numeric` column with precision 40 and scale 2 MUST be unmapped, with the reason naming Exasol's maximum `DECIMAL` precision of 36
* *AND* the mapping MUST NOT silently reduce the precision, because that would truncate values
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: PostgreSQL numeric without a declared precision is unmapped

* *GIVEN* a PostgreSQL column whose `data_type` is `numeric` and whose `numeric_precision` is null, as an unconstrained `numeric` reports
* *WHEN* the column is mapped
* *THEN* the result MUST be unmapped
* *AND* the stated reason MUST name the absent precision
* *AND* the mapping MUST NOT substitute Exasol's maximum precision, because Exasol compares the declared type for exact equality and an arbitrary-precision source value need not fit it
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: PostgreSQL character varying preserves the declared length and uses UTF8

* *GIVEN* a PostgreSQL column whose `data_type` is `character varying` with a declared length of 100
* *WHEN* the column is mapped
* *THEN* the result MUST be the Exasol type `VARCHAR` with size 100 and character set `UTF8`
* *AND* a declared length of 0 MUST map to size 1, because Exasol's minimum character-type size is 1
* *AND* a `character varying` column whose `character_maximum_length` is null MUST be unmapped, with the reason naming the absent length
* *AND* the mapping MUST NOT read a character set from the column, because PostgreSQL declares an encoding per database rather than per column
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: PostgreSQL character column of a nondeterministic collation is unmapped

* *GIVEN* a PostgreSQL `character varying` column whose declared collation is nondeterministic
* *WHEN* the column is mapped
* *THEN* the result MUST be unmapped, with the stated reason naming the column's collation and identifying it as nondeterministic
* *AND* the mapping MUST NOT map it to Exasol `VARCHAR`, because the delegated equality, `GROUP BY`, and `DISTINCT` this release renders for PostgreSQL carry no collation suffix and would equate values Exasol keeps apart
* *AND* a column whose collation is deterministic, including one declaring no collation of its own, MUST be mapped
* *AND* a collation for which the server reports no determinism flag MUST be treated as deterministic, because a version predating the flag has no nondeterministic collations to distinguish
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: PostgreSQL character is unmapped because it is blank-padded

* *GIVEN* a PostgreSQL column whose `data_type` is `character` with a declared length of 5
* *WHEN* the column is mapped
* *THEN* the result MUST be unmapped
* *AND* the stated reason MUST name the blank padding as the cause
* *AND* the mapping MUST NOT map it to Exasol `VARCHAR`, because PostgreSQL returns the value padded to its declared length and compares two `character` values ignoring trailing spaces, while Exasol comparing two `VARCHAR` values does neither
* *AND* the mapping MUST NOT map it to Exasol `CHAR`, because no mapped column produces Exasol `CHAR` in this release
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: PostgreSQL temporal and floating-point types map to their Exasol counterparts

* *GIVEN* PostgreSQL columns whose `data_type` is `date`, `timestamp without time zone`, and `double precision`
* *WHEN* each column is mapped
* *THEN* `date` MUST map to Exasol `DATE`
* *AND* `timestamp without time zone` MUST map to Exasol `TIMESTAMP`
* *AND* `double precision` MUST map to Exasol `DOUBLE`
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: Non-core PostgreSQL types are unmapped

* *GIVEN* PostgreSQL columns whose `data_type` is `boolean`, `text`, `smallint`, `real`, `time without time zone`, `time with time zone`, `timestamp with time zone`, `uuid`, `json`, `jsonb`, `bytea`, `interval`, `money`, or `ARRAY`
* *WHEN* each column is mapped
* *THEN* each result MUST be unmapped
* *AND* each stated reason MUST name the PostgreSQL type that was rejected
* *AND* `boolean` MUST be unmapped despite Exasol having a `BOOLEAN` type, because a mapped `BOOLEAN` column would widen the set of Exasol types a source column produces
* *AND* `timestamp with time zone` MUST be unmapped rather than mapped to Exasol `TIMESTAMP`, because the two carry different instants for one wall-clock value
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: PostgreSQL column nullability carries through unchanged

* *GIVEN* a PostgreSQL column whose `is_nullable` is `NO`
* *WHEN* the column is mapped
* *THEN* the mapped column MUST report `isNullable` as false
* *AND* a column whose `is_nullable` is `YES` MUST report `isNullable` as true
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: Omission records the source type under a source-neutral field

* *GIVEN* a virtual schema whose source holds one column of an unmapped type
* *WHEN* the recorded omission is read back from `adapterNotes`
* *THEN* the omission MUST name the table, the column, the source type, and the reason
* *AND* the field carrying the source type MUST be spelled for any source rather than for MySQL alone
* *AND* an `adapterNotes` value written by an earlier release under the MySQL-specific spelling MUST still parse, because Exasol persists those notes and a virtual schema created before this release is read back rather than recreated
<!-- /DELTA:NEW -->
