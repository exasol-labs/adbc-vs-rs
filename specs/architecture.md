# Architecture

## Overview

```
Adapter script (adbc-vs-adapter)

  Exasol VS request ──▶ dispatch ──┬──▶ create_schema ──┬──▶ provisioning ──HTTPS GET──▶ dbc registry
                                   │                    │         └──────HTTPS PUT──▶ BucketFS endpoint
                                   │                    └──▶ metadata ──▶ connection ──ADBC──▶ source DB
                                   ├──▶ capabilities ◀── sql_dialect
                                   └──▶ pushdown ──▶ pushdown_query ──▶ sql_dialect ──▶ scan_spec
                                            │
                                            ▼
  SELECT ADBC_VS.ADBC_SCAN('<scan spec>') EMITS (...) FROM DUAL

Scan UDF (scan_entry + adbc-vs-scan)

  Exasol ──▶ scan_entry ──▶ connection ──ADBC──▶ source DB
  Exasol ◀── scan_entry ◀── scan (coerce) ◀── Arrow record batches
```

- Classic Exasol Virtual Schema split: a stateless adapter script plans and rewrites each query, and a scan UDF executes the rewritten statement over ADBC and emits Arrow record batches.
- Two workspace crates link into one shared object. `adbc-vs-adapter` holds both UDF entry points, and `adbc-vs-scan` is a library for driver identity, ADBC connection, and batch streaming.
- One framework path serves every technology. Per-technology behavior lives behind the `SqlDialect` trait, the `MetadataSource` interface, and the compiled-in driver declarations.

## Components

- entry points (crates/adbc-vs-adapter/src/lib.rs): exports the adapter script entry `ADBC_ADAPTER` and the scan UDF entry `ADBC_SCAN` | owns: none | depends on: dispatch, scan_entry
- dispatch (crates/adbc-vs-adapter/src/dispatch.rs): routes each Virtual Schema request type to its handler and turns handler failures into a `UdfError` | owns: none, holds no state between calls | depends on: create_schema, capabilities, pushdown, sql_dialect, adapter_notes, driver identity
- create_schema (crates/adbc-vs-adapter/src/create_schema.rs): handles `createVirtualSchema`, `refresh`, and `setProperties`, validates properties, selects the driver route, checks the source precondition, and assembles the schema metadata response | owns: property names, the BucketFS mount path, the reserved connection name `ADBC_VS_BUCKETFS` | depends on: provisioning, metadata, adapter_notes, exa_type, type_mapping, connection, driver identity
- adapter_notes (crates/adbc-vs-adapter/src/adapter_notes.rs): defines the state `createVirtualSchema` records for later requests | owns: the `adapterNotes` JSON shape (driver, driver version, driver path, CONNECTION name, source qualifier, omitted columns) | depends on: none
- provisioning (crates/adbc-vs-adapter/src/provisioning.rs): makes a driver library readable on BucketFS through self-install or an operator-supplied path | owns: the BucketFS driver layout, download and unpack size limits, timeouts, the registry and BucketFS HTTP clients | depends on: registry_index, driver identity
- registry_index (crates/adbc-vs-adapter/src/registry_index.rs): parses the registry `index.yaml` and resolves a driver's version range to one package | owns: none | depends on: none
- metadata (crates/adbc-vs-adapter/src/metadata.rs, mysql_source.rs, postgres_source.rs, trino_source.rs): checks the source precondition and discovers tables and columns from the source `information_schema` | owns: none | depends on: type_mapping, adapter_notes, driver identity
- type_mapping (crates/adbc-vs-adapter/src/type_mapping.rs, mysql_type_mapping.rs, postgres_type_mapping.rs, trino_type_mapping.rs): maps each source column type to an Exasol type or an omission | owns: the Exasol type bounds every mapping uses | depends on: exa_type
- exa_type (crates/adbc-vs-adapter/src/exa_type.rs, emit_type.rs): defines the declarable Exasol types, their DDL and JSON encodings, and the `EMITS` type of each select-list entry | owns: none | depends on: none
- pushdown_query (crates/adbc-vs-adapter/src/pushdown_query.rs): parses a `pushdown` request into a dialect-neutral query model | owns: none | depends on: exa_type, adapter_notes
- sql_dialect (crates/adbc-vs-adapter/src/sql_dialect.rs, mysql_dialect.rs, postgres_dialect.rs, trino_dialect.rs): renders the query model into one technology's SQL, with no collation on any operand | owns: none | depends on: pushdown_query, exa_type, driver identity
- capabilities (crates/adbc-vs-adapter/src/capabilities.rs): shapes the dialect's advertised capability set into the `getCapabilities` response | owns: none | depends on: sql_dialect
- pushdown (crates/adbc-vs-adapter/src/pushdown.rs): builds the `pushdown` response that calls the scan UDF with a dynamic `EMITS` clause over the rendered statement | owns: the scan script name `ADBC_VS.ADBC_SCAN` | depends on: pushdown_query, sql_dialect, scan_spec, adapter_notes, exa_type
- scan_spec (crates/adbc-vs-adapter/src/scan_spec.rs): defines the JSON contract between the pushdown response and the scan UDF | owns: the scan specification shape, which carries no credential | depends on: exa_type, driver identity
- scan_entry (crates/adbc-vs-adapter/src/scan_entry.rs): runs the scan UDF body, resolves the source CONNECTION, opens the driver, and emits each batch to Exasol | owns: none | depends on: scan_spec, connection, scan
- driver identity (crates/adbc-vs-scan/src/driver.rs, build.rs, drivers/*.toml): defines the supported technologies and generates each one's package row, capability list, and qualifier segment count at build time | owns: the compiled-in driver tables, the registry base URL, the registry platform of the compile target (`linux_amd64` for x86_64, `linux_arm64` for aarch64) | depends on: none
- connection (crates/adbc-vs-scan/src/connection.rs): loads a driver library by its entry point, opens the ADBC database and connection, and redacts credentials from errors | owns: none, credentials live only for one open | depends on: driver identity
- scan (crates/adbc-vs-scan/src/scan.rs, coerce.rs, error.rs): executes a statement and streams one coerced Arrow batch at a time | owns: none | depends on: connection

## Data Flow

- `createVirtualSchema`, `refresh`, or `setProperties` JSON -> dispatch -> create_schema: properties are validated before any CONNECTION is resolved
- create_schema -> provisioning -> dbc registry: an HTTPS GET reads `index.yaml`, registry_index resolves the version, and a second GET downloads the package tarball when the resolved version is absent from the mount
- provisioning -> BucketFS write endpoint -> BucketFS mount: an HTTPS PUT with the user and password of the resolved BucketFS CONNECTION uploads the library, and provisioning polls the read-only mount until the file is readable
- create_schema -> connection -> source database -> metadata -> type_mapping: catalog rows arrive as Arrow batches and become mapped columns and omissions
- create_schema -> dispatch -> Exasol: a schema metadata JSON response carries the virtual tables and the `adapterNotes`
- `getCapabilities` JSON -> dispatch -> sql_dialect -> capabilities -> Exasol: the dialect is selected from the driver recorded in `adapterNotes`
- `pushdown` JSON -> pushdown_query -> sql_dialect -> pushdown -> Exasol: the response is `SELECT ADBC_VS.ADBC_SCAN('<scan spec JSON>') EMITS (...) FROM DUAL`
- Exasol -> scan_entry -> connection -> source database: the scan UDF resolves the CONNECTION through the UDF SDK and loads the driver from the path in the scan specification
- source database -> scan -> scan_entry -> Exasol: Arrow record batches are cast to the declared Exasol types and emitted batch by batch, with no row-by-row step

## Interfaces

- Adapter script: Exasol calls the `__exa_udf_entry_ADBC_ADAPTER` symbol with Virtual Schema JSON requests `createVirtualSchema`, `refresh`, `setProperties`, `dropVirtualSchema`, `getCapabilities`, and `pushdown`
- Scan UDF: Exasol calls `ADBC_VS.ADBC_SCAN` (`__exa_udf_entry_ADBC_SCAN`), a `SCALAR ... EMITS` script with one string argument holding the scan specification JSON and a query-side dynamic `EMITS` clause
- Virtual schema properties: `DRIVER` (`mysql`, `postgresql`, `trino`) and `CONNECTION_NAME` are required, and `BUCKETFS_CONNECTION` and `DRIVER_PATH` are optional
- Reserved CONNECTION `ADBC_VS_BUCKETFS`: the BucketFS write credential self-install uses when no `BUCKETFS_CONNECTION` is set
- `adapterNotes`: JSON that Exasol persists in schema metadata and resends on every later request, with no credential
- Scan specification: JSON with `driver`, `driver_path`, `connection`, `sql`, and `columns`, with no credential
- BucketFS layout: the self-installed library is at `<mount>/adbc-drivers/<driver>/<version>/<library>`, the operator-supplied default is at `<mount>/adbc-drivers/<driver>/supplied/<library>`, and the mount is `/buckets/bfsdefault/default`
- Driver declaration: `crates/adbc-vs-scan/drivers/<technology>.toml` with a `[package]` table, a `[qualifier]` segment count, and a capability list, read at build time only
- Release artifacts: one per architecture, `dist/adbc-vs.tar.gz` for x86_64 and `dist/adbc-vs-linux-arm64.tar.gz` for aarch64, each holding `udf/libadbc_vs_adapter.so` built for that architecture

## Constraints

- The UDF `.so` is built only inside the Rust build image that the `Makefile` names by `make cross-udf-build`, on a host of the `.so`'s own architecture, so its glibc fingerprint matches the `language-container-rs` SLC and `cargo exasol-udf validate` can load it
- The Rust toolchain version lives in `rust-toolchain.toml`, and `SLC_VERSION` in the `Makefile` tracks the `exasol-udf-sdk` version
- The minimum supported Exasol version is 2025.1
- The supported cluster architectures are Linux x86_64 and Linux aarch64, and each runs the `language-container-rs` SLC of its own architecture
- The supported source technologies are MySQL, PostgreSQL, and Trino
- Drivers are fetched only for the registry platform of the architecture the `.so` was compiled for, which is the cluster's architecture because the `.so` loads nowhere else
- Outbound HTTPS egress from the UDF enables registry self-install only, and `DRIVER_PATH` or the `supplied` default location works without egress
- No numeric throughput or latency target exists
- The adapter holds no state between calls, and all persisted state is in `adapterNotes`
- Driver self-install runs only in the adapter script during `createVirtualSchema`, `refresh`, and `setProperties`, never in the scan UDF or during `pushdown`
- The UDF reads BucketFS through a read-only mount, so self-install writes through the BucketFS HTTPS endpoint with credentials from a resolved CONNECTION
- `DRIVER_PATH` skips the registry read, the download, and the BucketFS write
- An explicit `BUCKETFS_CONNECTION` never falls back on failure, and only the reserved-connection route falls back to the `supplied` default location, when the connection does not resolve or the registry index request reaches no host
- Self-install verifies no checksum and no signature, and registry TLS certificate verification against the bundled webpki roots is the only integrity check
- The BucketFS client accepts the cluster's self-signed certificate, and the registry client is a separate client that always verifies certificates
- Download limits are 4 MiB for the index and 256 MiB for a package, and an unpacked package is limited to 512 MiB
- Registry and BucketFS requests use a 15 s connect timeout and a 600 s transfer timeout, and a written driver must become readable on the mount within 120 s
- No dialect advertises `ORDER BY`, `LIMIT`, or `LIMIT ... OFFSET`, because the emitting UDF carries no ordering guarantee, so Exasol applies them over the emitted rows
- Each advertised capability must be renderable by its dialect, because Exasol does not re-apply a delegated fragment
- No dialect renders a collation, so a delegated character comparison, grouping key, `DISTINCT` operand, and `MIN` or `MAX` operand follow the collation the source applies to the column, and a user who needs Exasol's code-point comparison either declares that collation at the source or wraps the operand in an expression no dialect advertises, so that Exasol evaluates it
- Each driver's source precondition is checked on every request that resolves the source CONNECTION, before any table is discovered
- Data moves as Arrow record batches from the driver to Exasol, and the scan holds one batch at a time
- Adding a technology is a reviewed code change: a TOML declaration and dialect code compiled into a release, never a file loaded at run time
- No release artifact bundles an ADBC driver binary
- The dependency tree accepts only the permissive licenses listed in `deny.toml`

## External Dependencies

- dbc driver registry (`https://dbc-cdn.columnar.tech/index.yaml`): source of driver versions and platform tarballs for self-install | failure impact: self-install is unavailable, and an operator supplies the driver through `DRIVER_PATH` or the `supplied` default location, which need no egress; an unreachable index host makes the reserved-connection route fall back to the `supplied` library, and every other registry failure fails the statement
- BucketFS HTTPS write endpoint (address of the BucketFS CONNECTION): destination of the self-installed driver, absent on Exasol Personal local deployments | failure impact: self-install fails and the statement fails, except that an unresolved reserved connection falls back to the `supplied` default library
- Source database over ADBC (MySQL, PostgreSQL, Trino): the database a virtual schema queries for metadata and data | failure impact: the driver returns a connection error, which the adapter or the scan UDF surfaces to the querying session with credentials redacted
- ADBC driver libraries (`libadbc_driver_mysql.so`, `libadbc_driver_postgresql.so`, `libadbc_driver_trino.so`): the driver each technology loads through `adbc_driver_manager` by its declared entry point | failure impact: no virtual schema can be created or queried for that technology
- Exasol UDF runtime (`exasol-udf-sdk`, `exasol-udf-macros`, `language-container-rs` SLC, published as one asset per architecture): the UDF ABI, CONNECTION resolution, and Arrow batch emission | failure impact: a `.so` that does not match the SLC's version or architecture is rejected at load time
- Docker Rust build image that the `Makefile` names (amd64 and arm64 variants): build environment for each release `.so` | failure impact: no release `.so` can be built for that architecture, with no effect at run time
