# Feature: type_mapping

States the contract every source-to-Exasol column mapping obeys: the closed set of Exasol types a mapped column may declare, how nullability carries, and how a column the mapping cannot represent without loss is omitted.

## Background

- A type mapping is a pure function from one source catalog row to either an Exasol column declaration or an omission carrying a reason. It performs no I/O, and it is total: every catalog row either maps or is omitted.
- The mapping reads source catalog metadata rather than the Arrow type the ADBC driver returns at scan time. The declared type is what Exasol compares for exact equality; the Arrow type is an input the scan coerces. The `adbc-drivers/mysql` driver documents that `DATETIME` comes back as `timestamp[s]`, `timestamp[ms]`, or `timestamp[us]` depending on how the query is written, which is why the Arrow type is not a stable basis, and the reasoning holds for any driver.
- The Exasol types a mapped column may declare are closed at `DECIMAL`, `VARCHAR`, `DATE`, `TIMESTAMP`, and `DOUBLE`. That is one set for the release rather than one per driver, and the pushdown closure argument in `vs-adapter/pushdown-dialect` rests on it: adding a driver adds no Exasol type to the system. It is not the set an `EMITS` column may declare, which adds `BOOLEAN` for a predicate Exasol delegated as a select-list expression — the two sets are modeled as two types, and no mapped column produces `BOOLEAN`.
- Exasol's own bounds constrain every mapping: `DECIMAL` precision at most 36, `VARCHAR` size at most 2000000, and both character types minimum size 1. A source column whose declared precision exceeds what Exasol can declare is unmapped whatever the source names that type, because the bound is Exasol's. Exasol's `CHAR` size bound of 2000 constrains nothing, because this release produces no Exasol `CHAR`.
- "Unmapped" is not an error. The caller omits the column from the virtual table and records the omission, so one unsupported column never blocks a whole schema. The omission names the table, the column, the source type, and the reason, and it records the source type under the source-neutral field `sourceType`. The MySQL-specific spelling `mysqlType` is still accepted on deserialization, because Exasol persists `adapterNotes` and a virtual schema created before this release is read back rather than recreated.
- Nullability is technology-agnostic: the mapped column reports the source column's own nullability unchanged, whatever the catalog spells the flag. That is a declaration rather than a scan-time guarantee — `vs-adapter/pushdown-join` records that an outer join returns nulls for a column the source declares `NOT NULL`.
- There is no shared source-column struct and no lowest-common-denominator catalog row. Each catalog carries fields the other has not: MySQL needs the unsigned flag and the per-column character set, PostgreSQL needs `udt_name` and whether the column's collation is deterministic, and reducing both to one row loses whichever field only one source has. Each driver's mapping therefore lives with that driver's metadata discovery, the module that already knows the catalog's column names, so `vs-adapter/create-virtual-schema` receives columns already mapped and branches on no driver.
- Which source types map, to what, and why the rest are omitted is per technology. `vs-adapter/mysql-type-mapping` and `vs-adapter/postgresql-type-mapping` each state their own table together with the source precondition that table depends on, and neither table derives from the other by addition or removal.

## Scenarios

### Scenario: Signed integer types map to DECIMAL with the matching digit count

* *GIVEN* a MySQL column of type `INT`
* *WHEN* the column is mapped
* *THEN* the result MUST be the Exasol type `DECIMAL` with precision 10 and scale 0
* *AND* a `BIGINT` column MUST map to `DECIMAL` with precision 19 and scale 0

### Scenario: DECIMAL preserves the source precision and scale

* *GIVEN* a MySQL column of type `DECIMAL` with precision 12 and scale 4
* *WHEN* the column is mapped
* *THEN* the result MUST be the Exasol type `DECIMAL` with precision 12 and scale 4

### Scenario: DECIMAL beyond Exasol's maximum precision is unmapped

* *GIVEN* a MySQL column of type `DECIMAL` with precision 40 and scale 2
* *WHEN* the column is mapped
* *THEN* the result MUST be unmapped
* *AND* the stated reason MUST name Exasol's maximum `DECIMAL` precision of 36
* *AND* the mapping MUST NOT silently reduce the precision, because that would truncate values

### Scenario: Character types preserve the declared length and use UTF8

* *GIVEN* a MySQL column of type `VARCHAR` with a declared character length of 100 and character set `utf8mb4`
* *WHEN* the column is mapped
* *THEN* the result MUST be the Exasol type `VARCHAR` with size 100 and character set `UTF8`
* *AND* a `CHAR` column with declared length 10 and character set `utf8mb4` MUST map to Exasol `VARCHAR` with size 10 and character set `UTF8`
* *AND* the mapping MUST NOT produce Exasol `CHAR` for any MySQL column, because Exasol blank-pads a `CHAR` value and MySQL does not pad a `CHAR` operand under the collations pushdown renders
* *AND* a declared length of 0 MUST map to size 1, because Exasol's minimum character-type size is 1

### Scenario: Character column of a non-utf8mb4 character set is unmapped

* *GIVEN* MySQL columns of type `VARCHAR` and `CHAR` whose character set is `latin1`
* *WHEN* each column is mapped
* *THEN* each result MUST be unmapped
* *AND* the stated reason MUST name the column's character set and `utf8mb4` as the only character set this release maps
* *AND* the mapping MUST NOT map it to Exasol `VARCHAR` with character set `UTF8`, because a predicate over that column cannot be delegated with the collation Exasol's comparison semantics require

### Scenario: Temporal and floating-point types map to their Exasol counterparts

* *GIVEN* MySQL columns of type `DATE`, `DATETIME`, and `DOUBLE`
* *WHEN* each column is mapped
* *THEN* `DATE` MUST map to Exasol `DATE`
* *AND* `DATETIME` MUST map to Exasol `TIMESTAMP`
* *AND* `DOUBLE` MUST map to Exasol `DOUBLE`

### Scenario: Unsigned integer columns are unmapped

* *GIVEN* a MySQL column of type `BIGINT UNSIGNED`
* *WHEN* the column is mapped
* *THEN* the result MUST be unmapped
* *AND* the stated reason MUST identify the unsigned modifier as unsupported in this release
* *AND* the mapping MUST NOT map it to `DECIMAL` with precision 19, because the unsigned range exceeds that precision

### Scenario: Non-core MySQL types are unmapped

* *GIVEN* MySQL columns of type `JSON`, `TEXT`, `BLOB`, `ENUM`, `SET`, `TINYINT`, `SMALLINT`, `MEDIUMINT`, `FLOAT`, `TIME`, `YEAR`, `TIMESTAMP`, and `GEOMETRY`
* *WHEN* each column is mapped
* *THEN* each result MUST be unmapped
* *AND* each stated reason MUST name the MySQL type that was rejected

### Scenario: Column nullability carries through unchanged

* *GIVEN* a MySQL column declared `NOT NULL`
* *WHEN* the column is mapped
* *THEN* the mapped column MUST report `isNullable` as false
* *AND* a column without `NOT NULL` MUST report `isNullable` as true
