<div align="center">

[![Rust](https://img.shields.io/badge/rust-stable-brightgreen.svg)](https://www.rust-lang.org/)
[![CI](https://github.com/exasol-labs/adbc-vs-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/exasol-labs/adbc-vs-rs/actions/workflows/ci.yml)
[![spec|driven](https://img.shields.io/badge/spec-driven-blueviolet.svg)](specs/)
[![Exasol|database](https://img.shields.io/badge/Exasol-database-blue.svg)](https://www.exasol.com)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](./LICENSE)

**A Rust Exasol Virtual Schema that queries any ADBC-fronted database through Arrow record batches end to end, self-installing the matching driver into BucketFS instead of requiring a manual packaging step.**

</div>

---

## Quick start

Create a virtual schema pointed at a MySQL database, then query it like any other Exasol schema:

```sql
CREATE VIRTUAL SCHEMA MYSQLVS USING ADBC_VS.ADBC_ADAPTER WITH
  DRIVER = 'mysql'
  CONNECTION_NAME = 'ADBC_VS_MYSQL';

SELECT * FROM MYSQLVS.customers;
```

`CONNECTION_NAME` names an Exasol `CONNECTION` object whose address is a `mysql://host:port/database` URI, carrying the source user and password. A Trino source uses the address form `trino://host[:port]/<catalog>/<schema>[?parameters]` instead, with the source user and password in the `CONNECTION` object's own fields rather than in the address — no credential, encoded or otherwise, belongs in a Trino `CONNECTION`'s URI. Self-installing the driver needs a BucketFS write credential, resolved through one precedence chain. The adapter first tries the reserved `CONNECTION` object `ADBC_VS_BUCKETFS` — no property required. Setting `BUCKETFS_CONNECTION` overrides that reserved connection and never falls back to it, or to anything else, on failure. Below both routes sits a pre-placed library at `adbc-drivers/<driver>/supplied/<library>` on the BucketFS mount, reached on either of two conditions: the reserved connection does not resolve at all, or it resolves but the driver registry's index is unreachable through it. An operator who kept `ADBC_VS_BUCKETFS` but has no HTTPS egress meets the second condition, so pre-placing the library there needs no property either.

---

## What this is

Exasol's existing Virtual Schema dialects run on the JVM and move data through JDBC's row-by-row `ResultSet` cursor. adbc-vs replaces that with [ADBC](https://arrow.apache.org/adbc/) — a uniform C API returning Arrow record batches natively.

- **ADBC-backed Virtual Schema.** One adapter maps any ADBC-fronted database's tables into Exasol virtual tables, replacing the one-dialect-per-database model of the JVM Virtual Schemas.
- **Self-installing drivers.** On first use, the adapter checks BucketFS for the requested driver; if absent, it fetches the matching platform tarball from the `dbc` ADBC driver registry and installs it itself. Air-gapped clusters instead pre-load the driver by uploading it to `adbc-drivers/<driver>/supplied/<library>` on the BucketFS mount — no property, no download.
- **SQL pushdown.** Projection, filter predicates, aggregation, and equi/outer joins are rewritten into the target dialect's SQL and executed by the ADBC driver at the source.
- **Arrow-native scan path.** The scan UDF streams the driver's Arrow record batches straight back to Exasol, with no row-by-row marshaling step.

See [`specs/mission.md`](specs/mission.md) for the full problem statement, architecture, and constraints.

## Limits

MySQL support is tested against MySQL 8.4.

**String comparison follows the source's collation.** A delegated character comparison, `LIKE`, `IN`, grouping key, `DISTINCT` operand, `MIN` or `MAX` operand, and join key runs under the collation the source applies to the column. The adapter renders no collation, and Exasol accepts no `COLLATE` in a query. `ORDER BY` stays in Exasol and orders by code point.

A source collation that folds case or accents, such as MySQL's default `utf8mb4_0900_ai_ci`, can therefore give a different answer than Exasol. A folded group, or a `MIN` or `MAX` tie between values of equal collation weight, reports one of the folded spellings, and the source chooses which.

Two remedies restore Exasol's code-point semantics:

- **(a) Declare a binary collation at the source.** Use `utf8mb4_0900_bin` on MySQL or `"C"` on PostgreSQL, on the column or in a view the virtual schema exposes. The fragment stays delegated. `utf8mb4_0900_bin` needs MySQL 8.0.17 or newer, and MariaDB has no equivalent `NO PAD` binary collation.
- **(b) Wrap the operand in an expression no dialect advertises**, such as `"name" || ''`. Exasol then keeps the predicate, grouping, or `MIN`/`MAX` and evaluates it itself. This is slower, because Exasol fetches the rows the source would otherwise have filtered. A later release that advertises scalar functions can change which wrappers work.

Two engine details also apply:

- On PostgreSQL before 18, a delegated `LIKE` over a column of a nondeterministic collation fails at the source.
- On Trino, a backslash in a `LIKE` pattern is an ordinary character unless the query writes an explicit `ESCAPE` clause, which Exasol then evaluates.

## Roadmap

ADBC-backed databases supported today, and the v1 driver set this project is working toward:

- [x] MySQL
- [x] PostgreSQL
- [x] Trino
- [ ] Snowflake
- [ ] Databricks
- [ ] DuckDB

## License

Community-supported. Licensed under [MIT](LICENSE).

---

<div align="center">

Built with Rust 🦀 and made with ❤️. Maintained by [Exasol Labs 🧪](https://github.com/exasol-labs/).

</div>
