# Plan: add-exasol-technology

> **Status:** blocked — see open-questions.md

## Summary

Add `exasol` as a third ADBC driver technology, reaching an Exasol source through the registry-published `libadbc_driver_exasol.so` packaging of `exarrow-rs`. It ships four new features and no new architecture: the technology fits the existing three-table driver identity exactly.

## Design

### Context

adbc-vs supports MySQL and PostgreSQL through a closed `Driver` identity keyed to three tables — a package row, a dialect, and a metadata source. The requested feature adds Exasol as a source, so the first design question is whether an Exasol-to-Exasol virtual schema fits that shape or needs new architecture. The second is what performance claim the feature can honestly carry.

Research answered both, and the second answer contradicts the requested goal.

**The technology fits with no architectural change.** `exarrow-rs` is published to the `dbc` registry as the driver path `exasol`, up to `v0.13.0`. The `linux_amd64` package installs `libadbc_driver_exasol.so`, its `MANIFEST` declares ADBC `v1.1.0`, and it exports exactly one init symbol, the unqualified `AdbcDriverInit`. It links `libgcc_s`, `librt`, `libpthread`, `libm`, `libdl`, and libc, requires at most `GLIBC_2.16`, and accepts `username` and `password` as ADBC options distinct from `uri`. Every one of those is a value in a TOML row or an arm in an existing `match`. Nothing needs a Cargo dependency on `exarrow-rs`, so `deny.toml` is untouched and the crate's `aws-lc-sys` and Thrift-advisory exposure stays outside adbc-vs's dependency graph.

**The throughput goal is not achievable and is dropped.** Neither Exasol-to-Exasol Virtual Schema adapter moves rows through its sandbox. Both return an `IMPORT INTO (...) FROM EXA AT "<connection>" STATEMENT '<select>'` string, which the Exasol engine executes through its native bulk loader; the Lua adapter's `ImportQueryBuilder` defaults its source type to `EXA`, and its same-database variant generates a plain local `SELECT` with no transfer at all. Exasol documents that `IMPORT ... FROM EXA` "is always parallelized" across cluster nodes on the existing database port. adbc-vs moves the same rows through one UDF instance over one WebSocket connection, and this driver buffers the whole result before the scan sees a batch. A bulk-throughput win over that path is not credible.

What the feature does win is stated instead, and each half is documented rather than asserted. The built-in `EXA` and `JDBC` variants advertise `ORDER_BY_COLUMN` and `LIMIT` over a transport whose own documentation says "the outermost order of the imported result rows is not guaranteed", and prescribe an `ORDER BY FALSE` sub-query as the workaround; `vs-adapter/result-ordering` withholds both capabilities and returns a stable order without one. The same `IMPORT` route maps `HASHTYPE` to `VARCHAR` and then fails a join on incomparable types; `vs-adapter/exasol-type-mapping` omits the column instead of mismapping it. Beyond that, one adapter reaches Exasol the way it reaches every other ADBC source, on no JVM.

- **Goals** — add `exasol` as a supported `DRIVER` value with a complete, tested set of the three per-driver tables; state the technology's real limits, including its unbounded scan memory and its certificate requirement; correct `specs/mission.md` where it now carries a claim the research refutes.
- **Non-Goals** — a throughput or latency benchmark against either reference adapter; cross-cluster federation between two separate Exasol instances; exposing Exasol `CHAR`, `BOOLEAN`, `GEOMETRY`, `HASHTYPE`, `INTERVAL`, or sub-millisecond `TIMESTAMP`; widening the framework's five mapped Exasol types; parallelizing a scan across UDF instances or driver subconnections; any change to `deny.toml`, to the driver-provisioning mechanism, or to the `ManagedConnection` type the scan and metadata paths are written against.

### Decision

Treat `exasol` as an ordinary technology. Add one TOML row, one enum variant, and one arm in each of the four `match` tables that key on the identity.

#### Architecture

```
CREATE VIRTUAL SCHEMA ... DRIVER = 'exasol'
CONNECTION address exasol://host:8563/SCHEMA?validateservercertificate=0
                    │
                    ▼
        Driver::Exasol  (adbc-vs-scan::driver)
          │        │            │
          │        │            └─ package  ← drivers/exasol.toml
          │        │                 registry_path exasol
          │        │                 library      libadbc_driver_exasol.so
          │        │                 entry_point  AdbcDriverInit
          │        │                 version_range >=0.13.0, <0.14.0
          │        │
          │        └─ ExasolDialect          (adbc-vs-adapter)
          │             renders quoted identifiers, ''-doubled literals,
          │             typed DATE/TIMESTAMP literals, no COLLATE
          │
          └─ ExasolSource                    (adbc-vs-adapter)
               SYS.EXA_ALL_SCHEMAS  → precondition
               SYS.EXA_ALL_TABLES   → table names
               SYS.EXA_ALL_COLUMNS  → columns, mapped by exasol_type_mapping
                    │
                    ▼
        connection::open  →  ManagedDriver::load_dynamic_from_filename
                             uri + username + password
                    │
                    ▼
        libadbc_driver_exasol.so  →  Exasol source over its native protocol
```

#### Patterns

| Pattern | Where | Why |
|---------|-------|-----|
| One arm per identity in a closed `match` | `driver.rs`, `connection.rs`, `metadata.rs`, `sql_dialect.rs` | The build fails at every table missing a row, so the technology cannot half-exist |
| Per-technology TOML as the only data edit | `drivers/exasol.toml` | Package row and capability list are values; the build generates the tables |
| Precondition owned by the metadata source | `ExasolSource::check_source` | The schema check is Exasol's own concern and no other source has the concept |
| Measurement gate before a claim | tasks 2.2, 2.4, 5.3 | Credential bytes, certificate parameters, and collation identity are measured against the driver rather than reasoned from its grammar |
| Refuse rather than compensate | task 2.2 | Pre-encoding a credential to match a driver defect would double-encode against a fixed release |

### Consequences

| Decision | Alternatives Considered | Rationale |
|----------|------------------------|-----------|
| Load the registry-published `libadbc_driver_exasol.so` through the driver manager | Add `exarrow-rs` as a Cargo dependency and implement an in-process driver; require `DRIVER_PATH` and no registry entry | The registry publishes the package, so the existing self-install path works unchanged. A Cargo dependency would pull `aws-lc-sys`, `tokio`, and an unpatched Thrift advisory into adbc-vs's own graph and would force the scan and metadata paths off `ManagedConnection` |
| `entry_point = "AdbcDriverInit"` | Derive the symbol from the library filename; use `AdbcDriverExasolInit` | The published `v0.13.0` package exports exactly one init symbol and it is the unqualified one. The qualified name exists only from crate `0.15.1`, which the registry has not published |
| `version_range = ">=0.13.0, <0.14.0"` | Admit every `0.x`; pin `=0.13.0` | `0.13.0` is the newest published version and carries the zero-row-schema fix this scan depends on. `0.14.0` is a breaking release nothing has measured, and the init symbol changed after it |
| Source and target are the same Exasol instance | A second `exasol/docker-db` container as the source | CI runners already exhaust disk with three containers, and intra-instance federation is a first-class case. Cross-cluster is a Non-Goal, not a claim |
| Omit Exasol `CHAR` | Map it to Exasol `VARCHAR`; map it to Exasol `CHAR` | Four features hold the invariant that every delegated character operand is Exasol `VARCHAR`, and measured, `CHAR` comparison is blank-padded where `VARCHAR` is not. Mapping it to `CHAR` would reopen the invariant for one technology |
| Omit `TIMESTAMP(p)` above precision 3 | Map it to a bare `TIMESTAMP` | The declarable set carries no fractional-seconds precision, so mapping would drop digits silently — the outcome the framework's decimal rule refuses |
| No benchmark in this plan | A `make bench-exasol` target beside `make bench` | The reference path is native parallel `IMPORT FROM EXA`, or a plain local `SELECT` in the same-instance case. A measurement harness is worth building, and it belongs in its own plan rather than bundled with the technology |

## Features

| Feature | Status | Spec |
|---------|--------|------|
| vs-adapter/exasol-pushdown | NEW | `specs/_plans/add-exasol-technology/vs-adapter/exasol-pushdown/spec.md` |
| vs-adapter/exasol-collation | NEW | `specs/_plans/add-exasol-technology/vs-adapter/exasol-collation/spec.md` |
| vs-adapter/exasol-type-mapping | NEW | `specs/_plans/add-exasol-technology/vs-adapter/exasol-type-mapping/spec.md` |
| scan-udf/exasol-scan | NEW | `specs/_plans/add-exasol-technology/scan-udf/exasol-scan/spec.md` |
| scan-udf/arrow-scan | CHANGED | `specs/_plans/add-exasol-technology/scan-udf/arrow-scan/spec.md` |
| vs-adapter/driver-selection | CHANGED | `specs/_plans/add-exasol-technology/vs-adapter/driver-selection/spec.md` |
| vs-adapter/type-mapping | CHANGED | `specs/_plans/add-exasol-technology/vs-adapter/type-mapping/spec.md` |
| vs-adapter/create-virtual-schema | CHANGED | `specs/_plans/add-exasol-technology/vs-adapter/create-virtual-schema/spec.md` |
| driver-provisioning/driver-version-resolution | CHANGED | `specs/_plans/add-exasol-technology/driver-provisioning/driver-version-resolution/spec.md` |
| driver-provisioning/driver-self-install | CHANGED | `specs/_plans/add-exasol-technology/driver-provisioning/driver-self-install/spec.md` |

## Impact

Operators gain `DRIVER = 'exasol'`, addressed as `exasol://host:8563/SCHEMA`. The path component names a schema rather than a database, and that schema must exist, be visible, and not itself be virtual.

Three limits are operator-facing and none of them is a regression, because no earlier release supported this source.

- **The address must decide how the source certificate is checked.** The driver validates against the operating system's trust store and the Script Language Container ships no CA bundle, so the driver's default fails. The address must carry `certificate_fingerprint=<lower-case hex>` for a production source or `validateservercertificate=0` for a test stack. Pinning a fingerprint replaces hostname and chain verification rather than adding to it, so a replaced source certificate needs a re-pin.
- **An Exasol-source scan's memory is proportional to its result.** The driver collects every page before the scan reads a batch, so a result larger than the UDF's memory limit fails instead of streaming. Every other supported driver streams.
- **Six Exasol types are not exposed.** `CHAR`, `BOOLEAN`, `GEOMETRY`, `HASHTYPE`, both `INTERVAL` types, `TIMESTAMP WITH LOCAL TIME ZONE`, and `TIMESTAMP` above precision 3 are omitted with a recorded reason. `CHAR` is the one that costs most in practice, and it is omitted to hold an adapter invariant rather than because the values would not survive.

A credential containing a percent sign is refused with a named error, because the driver's own DSN round trip alters it.

No existing virtual schema changes behavior. No property, `adapterNotes` field, or install path changes shape. `specs/mission.md` changes one claim about the JVM Virtual Schemas that the research refutes; that is a documentation correction, not a behavior change.

## Requirements

| Requirement | Details |
|-------------|---------|
| Driver package | Registry path `exasol`, `libadbc_driver_exasol.so`, entry point `AdbcDriverInit`, range `>=0.13.0, <0.14.0` |
| Advertised capabilities | Same membership as the PostgreSQL set, declared independently in `drivers/exasol.toml` |
| Mapped source types | `DECIMAL(p,s)`, `DOUBLE`, `VARCHAR(n) UTF8`, `VARCHAR(n) ASCII`, `DATE`, `TIMESTAMP` at precision 3 or below |
| Source precondition | The address's schema exists, is visible, and is not virtual |
| Test tiers | `make test-it-exasol` (ADBC only, Exasol container), `make test-e2e-exasol` (full stack) |

## Dependencies

- The `dbc` registry entry for `exasol` must still publish a `linux_amd64` package satisfying `>=0.13.0, <0.14.0`. `make test-registry-live` is the check.
- `make test-e2e-exasol` needs the `exasol` compose service reachable from inside a UDF sandbox, which needs a static address on the `adbc-vs` network.
- A follow-up plan should carry the throughput measurement this plan declines, comparing adbc-vs against `exasol-virtual-schema-lua` and against a plain local `SELECT`, framed the way `make bench`'s framing correction already frames the MySQL comparison.
- `exarrow-rs` should fix the credential round trip its ADBC database options go through. Task 2.2 records the defect; adbc-vs refuses the affected bytes until it is fixed.

## Implementation Tasks

### 1. Declare the driver package

- [ ] 1.1 Add `crates/adbc-vs-scan/drivers/exasol.toml` with the `[package]` row and the `[capabilities]` list, each field commented with the evidence for its value
- [ ] 1.2 Add `Driver::Exasol` to `crates/adbc-vs-scan/src/driver.rs` — the variant, `ALL`, `as_str`, `package`, and `capabilities`
- [ ] 1.3 Extend `crates/adbc-vs-scan/tests/driver_toml_test.rs` and `crates/adbc-vs-scan/src/driver_tests.rs` for the third declaration
- [ ] 1.4 Resolve the declared range against the live registry with `make test-registry-live`

### 2. Reach the source

- [ ] 2.1 Add `exasol_target` to `crates/adbc-vs-scan/src/connection.rs` and its `database_target` arm, passing the address verbatim as `uri` beside separate `username` and `password`
- [ ] 2.2 Measure which credential bytes survive the driver's DSN round trip, then refuse the rest by name at connect time [expert]
- [ ] 2.3 Add the `exasol-it` cargo feature, `crates/adbc-vs-scan/tests/exasol_it_test.rs`, `make stack-up-exasol`, and `make test-it-exasol`
- [ ] 2.4 Measure the certificate parameters and the empty-versus-non-empty schema divergence in the integration tier
- [ ] 2.5 Assert the driver's Arrow types for every mapped Exasol type coerce to their declared output, adding no coercion rule unless a measurement demands one

### 3. Read the source catalog

- [ ] 3.1 Add `crates/adbc-vs-adapter/src/exasol_type_mapping.rs` — a total function over the `COLUMN_TYPE` spelling that maps six forms and omits every other by name [expert]
- [ ] 3.2 Add `crates/adbc-vs-adapter/src/exasol_source.rs` — `namespace`, `check_source` over `SYS.EXA_ALL_SCHEMAS`, and `discover_tables` over `SYS.EXA_ALL_TABLES` and `SYS.EXA_ALL_COLUMNS`, casting the catalog's booleans so the existing cell readers serve
- [ ] 3.3 Add the `metadata::source_for` arm and make `create_schema`'s missing-path-component message name a schema for this driver

### 4. Render the source SQL

- [ ] 4.1 Add `crates/adbc-vs-adapter/src/exasol_dialect.rs` — quoted identifiers, single-quote-doubled literals with a verbatim backslash, typed `DATE` and `TIMESTAMP` literals, `<>`, `LIKE ... ESCAPE`, `FULL OUTER JOIN`, and no collation suffix anywhere [expert]
- [ ] 4.2 Add the `sql_dialect::dialect_for` arm and extend `crates/adbc-vs-adapter/src/capabilities_tests.rs` to assert the Exasol set equals the PostgreSQL set and differs from MySQL's by one entry

### 5. Prove it end to end

- [ ] 5.1 Give the `exasol` compose service a static address on the `adbc-vs` network and make `make stack-up` depend on a new `stack-up-exasol`
- [ ] 5.2 Add `crates/adbc-vs-adapter/tests/e2e_exasol_test.rs` and its shared setup, creating one source schema and one virtual schema over the same instance [expert]
- [ ] 5.3 Add the differential assertions — collation identity, grouping, `MIN`/`MAX`, full outer join, and every omission — each against a local Exasol mirror of the same values
- [ ] 5.4 Add the `integration-tests-exasol` and `e2e-exasol` CI jobs, extend `release`'s `needs`, and update `crates/adbc-vs-adapter/tests/workflow_test.rs` and `tests/make_targets_test.rs`

### 6. Update the record

- [ ] 6.1 Update the Background prose of `specs/scan-udf/arrow-scan`, `specs/vs-adapter/driver-selection`, `specs/vs-adapter/type-mapping`, `specs/vs-adapter/create-virtual-schema`, `specs/vs-adapter/pushdown-dialect`, `specs/vs-adapter/result-ordering`, `specs/driver-provisioning/driver-self-install`, and `specs/driver-provisioning/driver-version-resolution` to name the third technology
- [ ] 6.2 Update `specs/mission.md` — add Exasol to Core Capability 5, correct the Problem Statement's claim about how the JVM Virtual Schemas move data, and add the scan-memory and certificate constraints
- [ ] 6.3 Update `CLAUDE.md` — the new test tiers, the new CI jobs, the compose static address, and the framing correction for any future Exasol benchmark
- [ ] 6.5 Re-check `CLAUDE.md`'s `exapump` bullet against the installed version: it states that `exapump` 0.9.2 exposes no `wait` subcommand, and 0.12.0 exposes one. Decide whether `make stack-up`'s bounded poll is replaced by `exapump wait` or the bullet is corrected to name the version it applied to
- [ ] 6.4 Bump the workspace version in `Cargo.toml` to `0.4.0`

## Parallelization

| Parallel Group | Tasks |
|----------------|-------|
| Group A | 1.1, 1.2, 1.3, 1.4 |
| Group B | 2.1, 2.2, 2.3, 2.4, 2.5 |
| Group C | 3.1, 3.2, 3.3 |
| Group D | 4.1, 4.2 |
| Group E | 5.1, 5.2, 5.3, 5.4 |
| Group F | 6.1, 6.2, 6.3, 6.4, 6.5 |

Sequential dependencies:

- Group A → Group B, Group C, Group D (every table keys on the identity Group A adds)
- Group B, Group C, Group D → Group E (the end-to-end tier needs all three tables)
- Group A → Group F (6.1 and 6.2 name values Group A fixes)

Groups B, C, and D are independent of each other: they are the three per-driver tables, and no one of them reads another.

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| None | — | Adding a technology is additive by construction: every per-driver fact is a new row in an existing table or a new arm in an existing `match`, and no existing arm becomes unreachable |

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| exasol-type-mapping: DECIMAL and DOUBLE map to the types the source column declares | Unit | `crates/adbc-vs-adapter/src/exasol_type_mapping_tests.rs` | `decimal_and_double_map_to_declared_types` |
| exasol-type-mapping: VARCHAR keeps its declared length and is declared UTF8 whichever character set the source uses | Unit | `crates/adbc-vs-adapter/src/exasol_type_mapping_tests.rs` | `varchar_keeps_length_and_declares_utf8` |
| exasol-type-mapping: DATE and millisecond TIMESTAMP map to their counterparts | Unit | `crates/adbc-vs-adapter/src/exasol_type_mapping_tests.rs` | `date_and_millisecond_timestamp_map` |
| exasol-type-mapping: TIMESTAMP the adapter cannot declare is unmapped | Unit | `crates/adbc-vs-adapter/src/exasol_type_mapping_tests.rs` | `undeclarable_timestamp_is_unmapped` |
| exasol-type-mapping: CHAR is unmapped because Exasol compares it blank-padded | Unit | `crates/adbc-vs-adapter/src/exasol_type_mapping_tests.rs` | `char_is_unmapped_for_blank_padding` |
| exasol-type-mapping: BOOLEAN, GEOMETRY, HASHTYPE, and INTERVAL are unmapped | Unit | `crates/adbc-vs-adapter/src/exasol_type_mapping_tests.rs` | `non_core_types_are_unmapped` |
| exasol-type-mapping: An unrecognized type spelling is unmapped rather than guessed | Unit | `crates/adbc-vs-adapter/src/exasol_type_mapping_tests.rs` | `unrecognized_spelling_is_unmapped_by_name` |
| exasol-type-mapping: Virtual schema exposes the schema the connection address names | Integration | `crates/adbc-vs-adapter/tests/e2e_exasol_test.rs` | `virtual_schema_exposes_only_the_addressed_schema` |
| exasol-type-mapping: A source schema that is missing or virtual is rejected at creation | Integration | `crates/adbc-vs-adapter/tests/e2e_exasol_test.rs` | `missing_or_virtual_source_schema_is_rejected` |
| exasol-collation: No rendered operand carries a collation suffix | Unit | `crates/adbc-vs-adapter/src/exasol_dialect_tests.rs` | `no_operand_carries_a_collation_suffix` |
| exasol-collation: Delegated comparison and ordering reproduce Exasol's own answers | Integration | `crates/adbc-vs-adapter/tests/e2e_exasol_test.rs` | `delegated_comparison_matches_local_mirror` |
| exasol-collation: Delegated grouping and DISTINCT return Exasol's own groups | Integration | `crates/adbc-vs-adapter/tests/e2e_exasol_test.rs` | `delegated_grouping_matches_local_mirror` |
| exasol-collation: A suffix rendered by mistake fails the source rather than returning wrong rows | Integration | `crates/adbc-vs-scan/tests/exasol_it_test.rs` | `collate_clause_is_refused_by_the_source` |
| exasol-pushdown: Capability set advertises the fragments this dialect renders | Unit | `crates/adbc-vs-adapter/src/capabilities_tests.rs` | `exasol_capability_set_matches_its_renderer` |
| exasol-pushdown: The advertised set matches the PostgreSQL dialect's membership and is stated independently | Unit | `crates/adbc-vs-adapter/src/capabilities_tests.rs` | `exasol_set_equals_postgres_set_and_differs_from_mysql_by_one` |
| exasol-pushdown: Identifiers are quoted unconditionally and an embedded double quote is doubled | Unit | `crates/adbc-vs-adapter/src/exasol_dialect_tests.rs` | `identifiers_are_quoted_and_doubled` |
| exasol-pushdown: String literal is rendered by doubling the single quote and nothing else | Unit | `crates/adbc-vs-adapter/src/exasol_dialect_tests.rs` | `string_literal_doubles_the_quote_only` |
| exasol-pushdown: LIKE, IS NULL, BETWEEN, IN, and notequal render in their standard forms | Unit | `crates/adbc-vs-adapter/src/exasol_dialect_tests.rs` | `predicates_render_in_standard_forms` |
| exasol-pushdown: Projection pushdown reaches the source Exasol rather than the querying Exasol | Integration | `crates/adbc-vs-adapter/tests/e2e_exasol_test.rs` | `projection_pushdown_reaches_the_source` |
| exasol-pushdown: Filter pushdown reaches the source Exasol rather than the querying Exasol | Integration | `crates/adbc-vs-adapter/tests/e2e_exasol_test.rs` | `filter_pushdown_reaches_the_source` |
| exasol-pushdown: Aggregation pushdown reaches the source Exasol rather than the querying Exasol | Integration | `crates/adbc-vs-adapter/tests/e2e_exasol_test.rs` | `aggregation_pushdown_reaches_the_source` |
| exasol-pushdown: Delegated full outer join is rendered and executed | Integration | `crates/adbc-vs-adapter/tests/e2e_exasol_test.rs` | `full_outer_join_is_delegated_and_executed` |
| exasol-pushdown: The generated statement carries no ordering or limiting clause | Integration | `crates/adbc-vs-adapter/tests/e2e_exasol_test.rs` | `generated_statement_carries_no_order_or_limit` |
| exasol-scan: Credentials pass as separate driver options and the address carries none | Unit | `crates/adbc-vs-scan/src/connection_tests.rs` | `exasol_target_sets_three_options` |
| exasol-scan: A credential the driver's own DSN round trip cannot carry is refused | Integration | `crates/adbc-vs-scan/tests/exasol_it_test.rs` | `credential_bytes_the_dsn_cannot_carry_are_refused` |
| exasol-scan: The address's query string reaches the driver, so certificate handling is operator-set | Integration | `crates/adbc-vs-scan/tests/exasol_it_test.rs` | `certificate_parameters_reach_the_driver` |
| exasol-scan: The whole result is buffered by the driver, and the scan states what that caps | Integration | `crates/adbc-vs-scan/tests/exasol_it_test.rs` | `multi_batch_result_is_emitted_in_full` |
| exasol-scan: An empty result set is emitted despite the driver reporting a different schema for it | Integration | `crates/adbc-vs-scan/tests/exasol_it_test.rs` | `empty_and_non_empty_results_both_coerce` |
| exasol-scan: Source failure surfaces the driver's diagnosis without the credential | Integration | `crates/adbc-vs-scan/tests/exasol_it_test.rs` | `source_failures_surface_no_credential` |
| arrow-scan: Scan connects using credentials resolved at run time | Integration | `crates/adbc-vs-adapter/tests/e2e_exasol_test.rs` | `scan_resolves_credentials_at_run_time` |
| arrow-scan: Scan streams every result batch to Exasol | Integration | `crates/adbc-vs-scan/tests/exasol_it_test.rs` | `every_batch_is_released_before_the_next` |
| arrow-scan: Provisioned driver loads inside the UDF sandbox | Integration | `crates/adbc-vs-adapter/tests/e2e_exasol_test.rs` | `exasol_driver_loads_by_its_named_entry_point` |
| driver-selection: Each supported DRIVER value resolves to its own dialect and package | Unit | `crates/adbc-vs-adapter/src/driver_selection_tests.rs` | `three_driver_values_resolve_to_distinct_tables` |
| type-mapping: DECIMAL beyond Exasol's maximum precision is unmapped | Unit | `crates/adbc-vs-adapter/src/exasol_type_mapping_tests.rs` | `exasol_source_cannot_reach_the_precision_bound` |
| type-mapping: Each technology states its own mapping table | Unit | `crates/adbc-vs-adapter/src/metadata_source_tests.rs` | `each_technology_maps_its_own_catalog_row` |
| create-virtual-schema: Unsupported driver value fails with a named error | Unit | `crates/adbc-vs-adapter/src/create_schema_tests.rs` | `unsupported_driver_names_three_supported_values` |
| create-virtual-schema: Connection address without a database fails with a named error | Unit | `crates/adbc-vs-adapter/src/create_schema_tests.rs` | `missing_path_component_names_a_schema_for_exasol` |
| create-virtual-schema: Columns of unmappable type are omitted and recorded | Integration | `crates/adbc-vs-adapter/tests/e2e_exasol_test.rs` | `hashtype_column_is_omitted_and_recorded` |
| driver-version-resolution: Version range resolves to the highest published version satisfying it | Unit | `crates/adbc-vs-adapter/src/registry_index_tests.rs` | `exasol_v_prefixed_versions_resolve_within_range` |
| driver-self-install: Two drivers install side by side without collision | Integration | `crates/adbc-vs-adapter/tests/e2e_exasol_test.rs` | `three_drivers_install_without_collision` |

### Manual Testing

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| vs-adapter/exasol-type-mapping | `make stack-up && exapump sql --dsn "$EXA_DSN" "CREATE VIRTUAL SCHEMA VS_EXA USING ADBC_VS.ADAPTER WITH DRIVER = 'exasol' CONNECTION = 'EXA_SRC' BUCKETFS_CONNECTION = 'BFS'"` | Statement succeeds; `DESCRIBE VS_EXA.<table>` lists the mapped columns and no `CHAR`, `BOOLEAN`, or `HASHTYPE` column |
| vs-adapter/exasol-type-mapping | `exapump sql --dsn "$EXA_DSN" "SELECT ADAPTER_NOTES FROM SYS.EXA_ALL_VIRTUAL_SCHEMAS WHERE SCHEMA_NAME = 'VS_EXA'"` | Notes record driver `exasol`, the resolved version, the source schema, and one omission per unmapped column with its `COLUMN_TYPE` spelling |
| vs-adapter/exasol-pushdown | `exapump sql --dsn "$EXA_DSN" "EXPLAIN VIRTUAL SELECT COUNT(*) FROM VS_EXA.T WHERE S = 'a''b'"` | Generated statement quotes every identifier, renders `'a''b'`, and carries the `COUNT` and the `WHERE` clause |
| vs-adapter/exasol-collation | `exapump sql --dsn "$EXA_DSN" "EXPLAIN VIRTUAL SELECT MIN(S) FROM VS_EXA.T"` | Generated statement contains no `COLLATE`; `SELECT MIN(S) FROM VS_EXA.T` returns the same value as the same query over a local mirror |
| scan-udf/exasol-scan | `exapump sql --dsn "$EXA_DSN" "SELECT COUNT(*) FROM VS_EXA.T"` | Row count matches the source table; no log line or error carries the source password |
| scan-udf/exasol-scan | `exapump sql --dsn "$EXA_DSN" "CREATE OR REPLACE CONNECTION EXA_BAD TO 'exasol://172.28.5.12:8563/SRC' USER 'sys' IDENTIFIED BY 'wrong'"`, then select through a virtual schema using it | Error names the address and the ADBC status, and contains neither `wrong` nor any escaped form of it |
| driver-provisioning/driver-self-install | `exapump bucketfs ls --dsn "$BFS_DSN" adbc-drivers/exasol/` | Lists `0.13.0/libadbc_driver_exasol.so` beside the MySQL and PostgreSQL driver directories |

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Build | `make cross-udf-build` | Exit 0; `target/release/libadbc_vs_adapter.so` revalidated |
| Unit tests | `make test` | 0 failures, no container needed |
| Integration (Exasol) | `make test-it-exasol` | 0 failures against the `exasol` container |
| Integration (MySQL) | `make test-it` | 0 failures, unchanged |
| Integration (PostgreSQL) | `make test-it-postgres` | 0 failures, unchanged |
| Artifact | `make test-artifact` | 0 failures against the built `.so` |
| E2E (Exasol) | `make test-e2e-exasol` | 0 failures, fails rather than skips when the stack is down |
| E2E (MySQL) | `make test-e2e` | 0 failures, unchanged |
| E2E (PostgreSQL) | `make test-e2e-postgres` | 0 failures, unchanged |
| Registry | `make test-registry-live` | The declared `>=0.13.0, <0.14.0` range resolves against the index published today |
| Policy | `make deny` | 0 findings; `deny.toml` unchanged by this plan |
| Lint | `make lint` | 0 warnings |
| Format | `make fmt-check` | No changes |
| Specs | `speq feature validate && speq plan validate add-exasol-technology` | Pass |
