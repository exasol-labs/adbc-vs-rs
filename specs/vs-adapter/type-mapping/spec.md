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

### Scenario: DECIMAL beyond Exasol's maximum precision is unmapped

* *GIVEN* a source column of a numeric type whose declared precision exceeds what Exasol can declare, such as precision 40 and scale 2
* *WHEN* the column is mapped
* *THEN* the result MUST be unmapped
* *AND* the stated reason MUST name Exasol's maximum `DECIMAL` precision of 36
* *AND* the mapping MUST NOT silently reduce the precision, because that would truncate values
* *AND* the rule MUST hold whatever the source names that type, because the bound is Exasol's own rather than the source's

### Scenario: Column nullability carries through unchanged

* *GIVEN* a source column the catalog reports as not nullable, for each driver this release supports
* *WHEN* the column is mapped
* *THEN* the mapped column MUST report `isNullable` as false
* *AND* a column the catalog reports as nullable MUST report `isNullable` as true
* *AND* the mapping MUST read the catalog's own flag rather than infer it, so one rule covers every driver whatever that catalog spells the flag

### Scenario: Each technology states its own mapping table

* *GIVEN* two drivers whose catalogs carry different fields — MySQL's unsigned flag and per-column character set, PostgreSQL's `udt_name` and collation determinism
* *WHEN* a virtual schema is created over either
* *THEN* each driver's metadata source MUST return columns already mapped, so `create_schema` MUST branch on no driver
* *AND* there MUST be no shared source-column struct and no lowest-common-denominator catalog row, because reducing both to one loses whichever field only one source has
* *AND* which source types map, and why the rest are omitted, MUST be stated by `vs-adapter/mysql-type-mapping` and `vs-adapter/postgresql-type-mapping` rather than here

### Scenario: A mapped column declares only an Exasol type this release produces

* *GIVEN* every column a driver's mapping returns as mapped
* *WHEN* the declared Exasol type of each is read
* *THEN* each MUST be one of `DECIMAL`, `VARCHAR`, `DATE`, `TIMESTAMP`, and `DOUBLE`, and none MUST be `BOOLEAN`, a type an `EMITS` clause declares for a delegated predicate but no source column produces
* *AND* the set MUST be identical across drivers, so adding a driver adds no Exasol type to the system
* *AND* a mapping that would widen the set MUST be treated as a change to the pushdown closure argument `vs-adapter/pushdown-dialect` states rather than as a local addition

### Scenario: Omission records the source type under a source-neutral field

* *GIVEN* a virtual schema whose source holds one column of an unmapped type
* *WHEN* the recorded omission is read back from `adapterNotes`
* *THEN* the omission MUST name the table, the column, the source type, and the reason
* *AND* the field carrying the source type MUST be spelled for any source rather than for MySQL alone
* *AND* an `adapterNotes` value written by an earlier release under the MySQL-specific spelling MUST still parse, because Exasol persists those notes and a virtual schema created before this release is read back rather than recreated
