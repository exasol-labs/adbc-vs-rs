# Mission: adbc-vs — Exasol Virtual Schema for ADBC Sources

> A Rust Exasol Virtual Schema that queries any ADBC-fronted database (PostgreSQL, Snowflake, Databricks, DuckDB) through Arrow record batches end to end, self-installing the matching driver into BucketFS instead of requiring a manual packaging step.

## Problem Statement

Exasol's existing Virtual Schema dialects (`virtual-schemas`, `postgresql-virtual-schema`) run on the JVM and move data through JDBC's row-by-row `ResultSet` cursor. That cursor, plus JVM startup and GC overhead, caps throughput and inflates the SLC container. Each new backend also means writing and maintaining a whole new Java dialect — one codebase per database.

ADBC gives every backend a uniform C API and an Arrow-native result format. One ADBC-based adapter can reach any database with an ADBC driver, and Arrow record batches move from driver to UDF to Exasol with no row-by-row conversion step. adbc-vs is that adapter: a Rust Virtual Schema that speaks ADBC instead of JDBC, and fetches its own drivers instead of asking the operator to package them.

## Target Users

| Persona | Goal | Key Workflow |
|---------|------|--------------|
| Data engineer | Query an external database (Postgres, Snowflake, Databricks, DuckDB) from Exasol SQL without standing up a separate ETL job | `CREATE VIRTUAL SCHEMA ... USING ADBC_VS.ADAPTER WITH DRIVER = 'postgresql' CONNECTION = '...'`, then `SELECT` against the virtual tables |
| Exasol operator (air-gapped) | Run adbc-vs where the cluster has no outbound internet access | Pre-fetch the driver tarball, upload it into BucketFS with `exapump` before creating the Virtual Schema |

## Core Capabilities

1. **ADBC-backed Virtual Schema** — one adapter script maps any ADBC-fronted database's tables into Exasol virtual tables, replacing the one-dialect-per-database model of the JVM Virtual Schemas.
2. **Self-installing drivers** — on first use, the adapter checks BucketFS for the requested driver; if absent, it fetches the matching platform tarball from the `dbc` ADBC driver registry (`dbc-cdn.columnar.tech/index.yaml`) and writes it into the BucketFS mount path itself, with no external CLI step. When outbound HTTPS is blocked (air-gapped clusters), the operator uploads the driver tarball with `exapump` ahead of time, and the adapter uses it in place.
3. **SQL pushdown** — projection, filter predicates, aggregation (single-group and `GROUP BY`), and equi/outer joins are rewritten into the target dialect's SQL behind the adapter's own `SqlDialect` trait and executed by the ADBC driver at the source. `ORDER BY` and `LIMIT`/`OFFSET` are not pushed down; see Constraints.
4. **Arrow-native scan path** — the scan UDF opens an ADBC connection, executes the pushed-down statement, and streams the driver's Arrow record batches back to Exasol without a row-by-row marshaling step.
5. **Multi-backend driver support** — MySQL and PostgreSQL are shipped drivers; Snowflake, Databricks, and DuckDB/SQLite remain the aspirational v1 driver set, added as the project matures, without adapter code changes.

## Out of Scope

- Writes: `INSERT`/`UPDATE`/`DELETE` pass-through to the source. adbc-vs is read-only.
- Caching, result reuse, or materialization. Every query re-executes against the source live.
- General multi-way or non-equi joins. Only equi and outer joins in Core Capability 3 are pushed down; complex query rewriting stays out of scope.
- A driver marketplace, browsing UI, or any user-facing driver discovery surface. adbc-vs consumes the `dbc` registry programmatically only.
- Bundling ADBC driver binaries in adbc-vs's own release artifact. Drivers are always fetched from the registry or supplied by the operator, never vendored.

## Domain Glossary

| Term | Definition |
|------|------------|
| Virtual Schema (VS) | Exasol adapter that maps an external data source into Exasol as a queryable schema |
| ADBC | Arrow Database Connectivity — a uniform C API that lets one client library talk to many databases and returns results as Arrow record batches |
| Adapter Script | The Rust component that handles `CREATE VIRTUAL SCHEMA`, metadata discovery, and pushdown decisions; it does not fetch data itself |
| Scan UDF | The Rust UDF invoked via `IMPORT FROM SCRIPT` that opens the ADBC connection, executes the pushed-down statement, and emits Arrow batches back to Exasol |
| `dbc` registry | The public YAML driver index at `dbc-cdn.columnar.tech/index.yaml`, listing ADBC driver versions and per-platform download URLs; adbc-vs fetches it directly over HTTPS, with no dependency on the `dbc` Go CLI |
| Self-install | The adapter/UDF's own check-then-fetch of a missing driver into BucketFS, done in-process instead of through an external upload step |
| Pushdown | Exasol delegating a query fragment (projection, filter, aggregation, join) to the Virtual Schema so it executes at the source instead of in Exasol |

---

## Tech Stack

| Layer | Technology | Purpose |
|-------|------------|---------|
| Language | Rust | Adapter script + scan UDF implementation |
| Data access | ADBC (Rust ADBC bindings over `arrow-adbc`) | Uniform driver interface to Postgres, Snowflake, Databricks, DuckDB |
| SQL transpilation | Adapter's own `SqlDialect` trait (`MySqlDialect`, `PostgresDialect`) | Rewrites pushed-down Exasol query fragments into the target dialect's SQL |
| Driver distribution | `dbc` ADBC driver registry (`dbc-cdn.columnar.tech/index.yaml`), fetched directly via HTTPS | Source of driver tarballs for self-install; no dependency on the `dbc` Go binary |
| UDF runtime | `exasol-udf-sdk`, `exasol-udf-macros`, `language-container-rs` Rust SLC | UDF ABI, connect-back SQL session, same conventions as `lakehouse-engine-rs` |
| Build | `rust:1.94-trixie` (glibc 2.41) in Docker | Builds the `.so` matching the SLC; never built on host |
| Testing | `cargo test`; E2E against a local Exasol Docker container | Unit + Virtual Schema behavior validation |
| Air-gapped fallback | `exapump` | Manual BucketFS upload of a pre-fetched driver tarball when outbound HTTPS is blocked |

## Commands

```bash
# Build (UDF .so — inside the rust:1.94-trixie container, never host `cargo build --release`)
make cross-udf-build

# Test (host unit tests)
cargo test

# Test (E2E against local Exasol Docker container)
make test-e2e

# Lint & Format
cargo clippy --all-targets && cargo fmt
```

## Project Structure

```
adbc-vs/
├── crates/
│   ├── adbc-vs-adapter/   # Adapter script: CREATE VIRTUAL SCHEMA, metadata, pushdown decisions,
│   │                      # SqlDialect transpile, driver self-install/registry logic
│   └── adbc-vs-scan/      # Scan UDF: ADBC connect, execute, stream Arrow batches back to Exasol
├── specs/                 # speq mission, plans, decision logs
├── docs/                  # User + developer guides
├── deploy/                # BucketFS/SLC deployment assets
└── Makefile               # cross-udf-build, test-e2e, lint targets
```

## Architecture

Two Rust components build into one Script Language Container `.so`, mirroring the classic Virtual Schema split used by `virtual-schemas`/`postgresql-virtual-schema`:

1. **Adapter Script** handles `CREATE VIRTUAL SCHEMA`, table/column metadata discovery over the ADBC connection, and pushdown planning — deciding which projection/filter/aggregation/join fragments the target dialect can execute. It rewrites those fragments into the target SQL dialect with its own `SqlDialect` trait implementation (e.g. `MySqlDialect`). Before the first query against a given driver, it checks BucketFS for that driver's shared library; if missing, it fetches the matching platform tarball from the `dbc` registry over HTTPS and writes it into the BucketFS mount path itself.
2. **Scan UDF**, invoked via `IMPORT FROM SCRIPT`, opens an ADBC connection using the self-installed (or operator-uploaded) driver, executes the rewritten statement, and emits the driver's Arrow record batches back to Exasol — no row-by-row conversion between the ADBC driver and the Exasol result set.

Data flows one direction per query: Exasol → Adapter Script (plan + rewrite) → Scan UDF → ADBC driver → source database, and Arrow batches flow back the same path in reverse. No state persists between queries.

## Constraints

- **Technical**: Exasol UDFs MUST have outbound HTTPS egress for self-install to reach `dbc-cdn.columnar.tech`. Air-gapped clusters without that egress MUST pre-load the driver into BucketFS with `exapump` before the adapter can use it.
- **Technical**: the UDF `.so` MUST be built inside the `rust:1.94-trixie` container matching `language-container-rs`'s SLC glibc fingerprint; a host-built `.so` is rejected at load time.
- **Business**: adbc-vs MUST NOT bundle any ADBC driver binary in its own release artifact — drivers come only from the registry fetch or an operator upload.
- **Performance**: data MUST move as Arrow record batches from the ADBC driver through the UDF to Exasol with no row-by-row marshaling step. No numeric throughput target is set yet; a benchmark against `postgresql-virtual-schema` is future work.
- **Technical**: a scan UDF's `EMITS` clause is spliced into the pushed-down query as a script-emitted derived table, which carries no ordering guarantee. `ORDER BY` and `LIMIT`/`OFFSET` are therefore never pushed down; Exasol applies both itself over the emitted rows.
- **Technical**: each driver's dialect states its own source-server precondition, checked before any table is discovered and on every request that resolves the source `CONNECTION`. MySQL's precondition: the source server MUST be 8.0.17 or newer, because the renderer's character predicates and `MIN`/`MAX` depend on the `utf8mb4_0900_as_cs` and `utf8mb4_0900_bin` collations, which exist only from that version; MariaDB implements neither and is not supported. PostgreSQL's precondition: the source database MUST use the `UTF8` encoding, because the mapping declares every character column `UTF8` and the ordering collation renders `COLLATE "C"`, which reproduces Exasol's code-point order only over UTF-8 bytes.
- **Security**: credentials do not reach the driver the same way for every technology. MySQL's ADBC driver accepts `username` and `password` as options distinct from `uri`, so no credential ever passes through a connection string. PostgreSQL's ADBC driver accepts only a `uri` option, so its credential travels instead in an escaped libpq keyword/value connection string built in memory at connect time; the string is never persisted, never logged, and stripped from surfaced driver errors, but the separate-option protection MySQL gets does not extend to it.

## External Dependencies

| Service | Purpose | Failure Impact |
|---------|---------|-----------------|
| `dbc-cdn.columnar.tech` registry | Source of driver version/platform metadata and tarballs for self-install | Unreachable → self-install fails; operator falls back to a manual `exapump` upload of a pre-fetched driver tarball |
| Target ADBC source (PostgreSQL, Snowflake, Databricks, DuckDB) | The database being queried through the Virtual Schema | Unreachable → the ADBC driver returns a connection error, surfaced through the adapter script to the querying session |
| `language-container-rs` SLC | Build-time base image/toolchain the UDF `.so` must match | Not a runtime dependency; a mismatched build environment produces a `.so` the SLC rejects at load time |
