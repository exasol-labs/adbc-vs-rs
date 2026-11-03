<!-- DELTA:CHANGED -->
# Feature: type_mapping

States the contract every source-to-Exasol column mapping obeys: the closed set of Exasol types a mapped column may declare, the rule that a source type maps only when the source answers every delegated operation over it as Exasol answers it over the declared type, apart from two stated exceptions, how nullability carries, and how a column the mapping cannot represent without loss is omitted.
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
## Background

- A type mapping is a pure, total function from one source catalog row to either an Exasol column declaration or an omission carrying a reason. It reads catalog metadata rather than the Arrow type the ADBC driver returns at scan time, because Exasol compares the declared type for exact equality and the Arrow type varies with the statement: the `adbc-drivers/mysql` driver returns `DATETIME` as `timestamp[s]`, `timestamp[ms]`, or `timestamp[us]` depending on how the query is written.
- The Exasol types a mapped column may declare are closed at `DECIMAL`, `VARCHAR`, `DATE`, `TIMESTAMP`, `DOUBLE`, and `BOOLEAN`. That is one set for the release, each driver produces a subset of it, and the pushdown closure argument in `vs-adapter/pushdown-dialect` rests on it. Exasol's bounds constrain every mapping: `DECIMAL` precision at most 36, `VARCHAR` size at most 2000000, and character-type size at least 1.
- Exasol never re-applies a delegated fragment, so a delegated comparison, `LIKE`, `IN`, `BETWEEN`, grouping key, `DISTINCT` operand, `MIN` or `MAX` operand, and join key over a mapped column return the source's answer. Capability negotiation is per capability rather than per column, so no mechanism keeps one column's predicates in Exasol. A source type therefore maps only when the source answers each of those operations as Exasol answers it over the declared type and the value the scan emits. Two exceptions are stated: a character comparison follows the source's collation, as `vs-adapter/pushdown-filter` states, and a comparison, grouping key, `DISTINCT` operand, or join key over a source timestamp acts at the source's fractional-second precision, while Exasol holds the emitted value at millisecond precision.
- "Unmapped" is not an error. The caller omits the column, so one unsupported column never blocks a whole schema, and records the table, the column, the source type under the source-neutral field `sourceType`, and the reason. The MySQL-specific spelling `mysqlType` is still accepted on deserialization, because Exasol persists `adapterNotes`. Nullability carries unchanged from the catalog's own flag, as a declaration rather than a scan-time guarantee, because an outer join returns nulls for a column the source declares `NOT NULL`.
- There is no shared source-column struct, because the catalogs disagree about more than which fields they carry: MySQL reports an unsigned flag and a per-column character set, PostgreSQL leaves the length or precision null for a `text` or unconstrained `numeric` column, and Trino reports a column's full type only as a string. Each driver's mapping lives with that driver's metadata discovery, and each `<technology>-type-mapping` feature states its own table and source precondition, derived from no other table.
<!-- /DELTA:CHANGED -->

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: A mapped column declares only an Exasol type this release produces

* *GIVEN* every column a driver's mapping returns as mapped
* *WHEN* the declared Exasol type of each is read
* *THEN* each MUST be one of `DECIMAL`, `VARCHAR`, `DATE`, `TIMESTAMP`, `DOUBLE`, and `BOOLEAN`
* *AND* none MUST be Exasol `CHAR` or `TIMESTAMP WITH LOCAL TIME ZONE`, which no source column produces in this release
* *AND* every driver's mapped types MUST fall within that one set, so adding a driver adds no Exasol type to the system
* *AND* a mapping that would widen the set MUST be treated as a change to the pushdown closure argument `vs-adapter/pushdown-dialect` states rather than as a local addition
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: A source type whose delegated comparison differs from Exasol's is unmapped

* *GIVEN* a source type whose delegated comparison answers differently from Exasol's comparison of the emitted value under every declared type in the set, such as MySQL `YEAR`, which MySQL compares against the integer constant `10` as the year 2010, and MySQL `JSON`, which MySQL compares by its JSON comparison rules rather than as text
* *WHEN* a column of that type is mapped
* *THEN* the result MUST be unmapped
* *AND* the stated reason MUST name how the source's comparison differs from Exasol's
* *AND* the mapping MUST NOT map the type and leave its predicates to Exasol, because no per-column mechanism withholds a delegated fragment
* *AND* a difference that is one of the two stated exceptions MUST NOT by itself unmap a type: a character comparison under the source's collation, and a timestamp comparison, grouping key, `DISTINCT` operand, or join key at the source's fractional-second precision, so a delegated equality with `TIMESTAMP '2024-01-01 12:00:00.123'` selects no row the source stores as `12:00:00.123456`
<!-- /DELTA:NEW -->
