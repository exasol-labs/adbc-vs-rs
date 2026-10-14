# Feature: type_mapping

States the contract every source-to-Exasol column mapping obeys: the closed set of Exasol types a mapped column may declare, how nullability carries, and how a column the mapping cannot represent without loss is omitted.

<!-- DELTA:CHANGED -->
## Background

- A type mapping is a pure function from one source catalog row to either an Exasol column declaration or an omission carrying a reason. It performs no I/O, and it is total: every catalog row either maps or is omitted.
- The mapping reads source catalog metadata rather than the Arrow type the ADBC driver returns at scan time. The declared type is what Exasol compares for exact equality; the Arrow type is an input the scan coerces. The `adbc-drivers/mysql` driver documents that `DATETIME` comes back as `timestamp[s]`, `timestamp[ms]`, or `timestamp[us]` depending on how the query is written, which is why the Arrow type is not a stable basis, and the reasoning holds for any driver.
- The Exasol types a mapped column may declare are closed at `DECIMAL`, `VARCHAR`, `DATE`, `TIMESTAMP`, and `DOUBLE`. That is one set for the release rather than one per driver, and the pushdown closure argument in `vs-adapter/pushdown-dialect` rests on it: adding a driver adds no Exasol type to the system. It is not the set an `EMITS` column may declare, which adds `BOOLEAN` for a predicate Exasol delegated as a select-list expression — the two sets are modeled as two types, and no mapped column produces `BOOLEAN`.
- Exasol's own bounds constrain every mapping: `DECIMAL` precision at most 36, `VARCHAR` size at most 2000000, and both character types minimum size 1. A source column whose declared precision exceeds what Exasol can declare is unmapped whatever the source names that type, and whatever that source's own maximum is. Exasol's `CHAR` size bound of 2000 constrains nothing, because this release produces no Exasol `CHAR`.
- "Unmapped" is not an error. The caller omits the column from the virtual table and records the omission, so one unsupported column never blocks a whole schema. The omission names the table, the column, the source type, and the reason, and it records the source type under the source-neutral field `sourceType`. The MySQL-specific spelling `mysqlType` is still accepted on deserialization, because Exasol persists `adapterNotes` and a virtual schema created before this release is read back rather than recreated.
- Nullability is technology-agnostic: the mapped column reports the source column's own nullability unchanged, whatever the catalog spells the flag. That is a declaration rather than a scan-time guarantee — `vs-adapter/pushdown-join` records that an outer join returns nulls for a column the source declares `NOT NULL`.
- There is no shared source-column struct and no lowest-common-denominator catalog row, and the catalogs disagree about more than which fields they carry. MySQL reports an unsigned flag and a per-column character set; PostgreSQL reports its type name in `data_type` and leaves the length or precision null for a `text` or unconstrained `numeric` column; Trino reports no length, precision, or scale column at all and carries a column's full type only as a string its mapping parses. Reducing them to one row would lose whichever field only one source has and would have nowhere to put a source whose detail is not a field. Each driver's mapping therefore lives with that driver's metadata discovery, the module that already knows the catalog's column names, so `vs-adapter/create-virtual-schema` receives columns already mapped and branches on no driver.
- Which source types map, to what, and why the rest are omitted is per technology. Each `<technology>-type-mapping` feature states its own table together with the source precondition that table depends on, and no table derives from another by addition or removal. A technology whose rendering and type mapping depend on no configurable source property states no precondition, which is a stated answer rather than a gap.
<!-- /DELTA:CHANGED -->

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Each technology states its own mapping table

* *GIVEN* drivers whose catalogs report a column's type differently — MySQL's unsigned flag and per-column character set, PostgreSQL's `data_type` with a null length or precision for a `text` or unconstrained `numeric` column, and Trino's single type string carrying the length, precision, and scale no column of its catalog reports
* *WHEN* a virtual schema is created over any of them
* *THEN* each driver's metadata source MUST return columns already mapped, so `create_schema` MUST branch on no driver
* *AND* there MUST be no shared source-column struct and no lowest-common-denominator catalog row, because reducing them to one loses whichever detail only one source carries
* *AND* which source types map, and why the rest are omitted, MUST be stated by each `<technology>-type-mapping` feature rather than here
<!-- /DELTA:CHANGED -->
