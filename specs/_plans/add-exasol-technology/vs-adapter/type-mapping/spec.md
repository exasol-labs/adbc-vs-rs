# Feature: type_mapping

States the contract every source-to-Exasol column mapping obeys: the closed set of Exasol types a mapped column may declare, how nullability carries, and how a column the mapping cannot represent without loss is omitted.

## Background

This file is a scenario delta. The permanent feature's own Background governs, and the bullets below are what this plan adds to it; task 6.1 applies them to `specs/vs-adapter/type-mapping/spec.md`, because `/speq:spec-merge` merges an existing feature by scenario name and reads nothing outside `## Scenarios`.

- Three technologies state their own mapping table: `vs-adapter/mysql-type-mapping`, `vs-adapter/postgresql-type-mapping`, and `vs-adapter/exasol-type-mapping`. None derives from another by addition or removal.
- A source that is Exasol itself is held to the same contract rather than exempted as an identity mapping. The closed five-type set still binds it, so it omits `BOOLEAN`, `GEOMETRY`, `HASHTYPE`, and both `INTERVAL` types even though the target has every one of them, and it omits Exasol `CHAR` even though the values would round-trip.
- Exasol's `DECIMAL` precision bound of 36 is this feature's bound, and an Exasol source enforces the same one, so the beyond-precision rule is unreachable from that source. It stays stated here rather than per technology, because the bound is the target's.

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: DECIMAL beyond Exasol's maximum precision is unmapped

* *GIVEN* a source column of a numeric type whose declared precision exceeds what Exasol can declare, such as precision 40 and scale 2
* *WHEN* the column is mapped
* *THEN* the result MUST be unmapped
* *AND* the stated reason MUST name Exasol's maximum `DECIMAL` precision of 36
* *AND* the mapping MUST NOT silently reduce the precision, because that would truncate values
* *AND* the rule MUST hold whatever the source names that type, because the bound is Exasol's own rather than the source's
* *AND* the rule MAY be unreachable for a technology whose source enforces the same bound, as an Exasol source does, and MUST still be stated here rather than per technology, because it is Exasol's bound that makes it a rule
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Each technology states its own mapping table

* *GIVEN* three drivers whose catalogs carry different fields — MySQL's unsigned flag and per-column character set, PostgreSQL's `udt_name` and collation determinism, Exasol's rendered `COLUMN_TYPE` spelling and its virtual-object flags
* *WHEN* a virtual schema is created over any of them
* *THEN* each driver's metadata source MUST return columns already mapped, so `create_schema` MUST branch on no driver
* *AND* there MUST be no shared source-column struct and no lowest-common-denominator catalog row, because reducing them to one loses whichever field only one source has
* *AND* which source types map, and why the rest are omitted, MUST be stated by `vs-adapter/mysql-type-mapping`, `vs-adapter/postgresql-type-mapping`, and `vs-adapter/exasol-type-mapping` rather than here
* *AND* a technology whose source is Exasol itself MUST state its own table on the same terms rather than be exempted as an identity mapping, because which types it declines to map is a decision this adapter makes rather than one the source forces
<!-- /DELTA:CHANGED -->
