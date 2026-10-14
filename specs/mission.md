# Mission: adbc-vs

> A Rust Exasol Virtual Schema that queries MySQL, PostgreSQL, and Trino through ADBC, moving data as Arrow record batches end to end and installing the matching driver into BucketFS itself.

## Problem Statement

Exasol's existing Virtual Schema dialects run on the JVM and move data through the JDBC `ResultSet` cursor, one row at a time. That cursor, plus JVM startup and garbage collection, limits throughput and enlarges the Script Language Container. Each new backend also needs its own Java dialect and a manual driver packaging step.

ADBC gives every backend a uniform C API and an Arrow-native result format. adbc-vs is a Rust Virtual Schema that speaks ADBC instead of JDBC. Arrow record batches move from the driver to Exasol with no row-by-row conversion. The adapter fetches the driver it needs, so the operator does not package it.

## Target Users

| Persona | Goal | Key Workflow |
|---------|------|--------------|
| Data engineer | Query an external database from Exasol SQL without a separate ETL job | `CREATE VIRTUAL SCHEMA ... USING ADBC_VS.ADBC_ADAPTER WITH DRIVER = 'postgresql' CONNECTION_NAME = '...'`, then `SELECT` against the virtual tables |
| Exasol operator (air-gapped) | Run adbc-vs on a cluster without outbound internet access | Upload the driver tarball into BucketFS with `exapump`, then create the Virtual Schema with `DRIVER_PATH` or the default `supplied/` location |

## Core Capabilities

1. **ADBC-backed Virtual Schema**: one adapter exposes the tables of a source database as Exasol virtual tables. Column types map to a closed set of Exasol types, and columns or tables without a mapping are omitted and recorded.
2. **Self-installing drivers**: when a schema is created or refreshed, the adapter resolves the driver version from the `dbc` registry and installs the platform tarball into BucketFS. Without registry access, the operator supplies the driver.
3. **SQL pushdown**: projection, filters, aggregation (single group and `GROUP BY`), and equi and outer joins, including nested joins, are rewritten into the source dialect and executed at the source. `ORDER BY` and `LIMIT`/`OFFSET` stay in Exasol. No dialect renders a collation, so string comparison follows the source's collation.
4. **Arrow-native scan path**: the scan UDF runs the pushed-down statement through the ADBC driver and returns the Arrow record batches to Exasol without row-by-row marshaling.
5. **Supported sources**: MySQL, PostgreSQL, and Trino. Adding a source technology requires a dialect and a metadata source in the adapter code.

## Out of Scope

- Writes: `INSERT`, `UPDATE`, and `DELETE` pass-through. adbc-vs is read-only.
- Caching, result reuse, or materialization. Every query runs against the source.
- Non-equi joins and general query rewriting.
- A driver marketplace or any user-facing driver discovery. adbc-vs reads the `dbc` registry programmatically only.
- Bundling ADBC driver binaries in the adbc-vs release artifact. Drivers come from the registry or the operator.

Architecture: see specs/architecture.md.

## Domain Glossary

| Term | Definition |
|------|------------|
| Virtual Schema | Exasol adapter that maps an external data source into Exasol as a queryable schema |
| ADBC | Arrow Database Connectivity: a uniform C API that returns query results as Arrow record batches |
| Adapter Script | The Rust component that handles Virtual Schema requests, metadata discovery, pushdown planning, and driver installation. It does not fetch table data |
| Scan UDF | The Rust UDF called through `IMPORT FROM SCRIPT` that executes the pushed-down statement and emits Arrow batches to Exasol |
| Pushdown | Exasol delegating a query fragment to the Virtual Schema so it runs at the source |
| `dbc` registry | The public driver index at `dbc-cdn.columnar.tech/index.yaml` with driver versions and per-platform download URLs |
| Self-install | The adapter's own check and fetch of a missing driver into BucketFS during a schema request |
| Driver identity | The `DRIVER` value. It selects one dialect, one driver package, and one metadata source |
| `DRIVER_PATH` | Schema property that points at an operator-supplied driver library and skips self-install |

---

## Tech Stack

| Layer | Technology | Purpose |
|-------|------------|---------|
| Language | Rust | Adapter script and scan UDF |
| Data access | `arrow-adbc` | Uniform driver interface |
| UDF runtime | `exasol-udf-sdk`, `exasol-udf-macros`, `language-container-rs` | UDF ABI and Script Language Container |
| Build | `rust:1.94-trixie` in Docker | Builds the `.so` that matches the container |
| Testing | `cargo test`, E2E against a local Exasol Docker container | Unit and Virtual Schema behavior tests |
| Air-gapped upload | `exapump` | BucketFS upload of a driver tarball |

## Commands

```bash
# Build (UDF .so inside the container, never a host release build)
make cross-udf-build

# Test (host unit tests)
cargo test

# Test (E2E against a local Exasol Docker container)
make test-e2e

# Lint & Format
cargo clippy --all-targets && cargo fmt
```

## Project Structure

```
adbc-vs/
├── crates/
│   ├── adbc-vs-adapter/   # Adapter script, scan UDF entry point, dialects, driver installation
│   └── adbc-vs-scan/      # Library: driver identity, ADBC connection, batch streaming
├── specs/                 # speq mission, architecture, specs, decisions, plans
├── docker/                # Local Exasol and source database stacks
└── Makefile               # cross-udf-build, test-e2e, lint targets
```

## Constraints

- **Business**: adbc-vs MUST NOT bundle any ADBC driver binary in its release artifact.
