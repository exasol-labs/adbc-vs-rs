# Tasks: add-postgresql-support

## Phase 1: PostgreSQL test stack (Group A)
- [x] 1.1 Add `postgres` service to docker-compose.yml
- [x] 1.2 Add stack-up-postgres / test-it-postgres Makefile targets + make_targets_test.rs

## Phase 2: Gates (Group 0, B, C)
- [x] 2.2a Gate: fetch PostgreSQL package from registry, record version list, upload to BucketFS (Group 0)
- [x] 2.2b Gate: load PostgreSQL driver in Exasol UDF sandbox, record ldd output (Group 0, after 2.2a)
- [x] 2.1 [expert] Gate: measure getCapabilities adapterNotes/driver field presence (Group B, after Group 0)
- [x] 2.4 Gate: read MANIFEST [Files] driver value (Group B, after 2.2a)
- [x] 2.3 [expert] Gate: measure PostgreSQL dialect semantic assumptions vs postgres:18.6-trixie + Exasol (Group C, after Group A) — measured; (a)-(f) and (h) PASS, so **Phase 4 is unblocked and `JOIN_TYPE_FULL_OUTER` is advertised as planned**. Three Background contradictions found, none in Phase 4: (g1) `Infinity`/`-Infinity` are NOT storable in a `numeric(p,s)` column — only `NaN` is — so the infinities are reachable only from the unconstrained `numeric` this release leaves unmapped; (g2) the driver spells the specials `nan`/`inf`/`-inf`, not `NaN`/`Infinity`/`-Infinity`, and Rust's permissive float parse accepts all three silently; (g3) PostgreSQL 18.6 accepts `LIKE` over a nondeterministic collation rather than failing outright. Tasks 7.1 and 8.1 already implemented the correct behavior (constrained column seeds `NaN` only, unconstrained column carries the infinities, `coerce.rs` uses `cast_with_options` rather than a permissive float parse). `scan-udf/postgresql-scan`'s special-value Background bullet and scenario, and `vs-adapter/postgresql-type-mapping`'s nondeterministic-collation bullet, reworded to match measured facts. Evidence in decision-log.md § Gate Measurements.

## Phase 3: Driver identity foundation (Group D, after Group B)
- [x] 3.1 [expert] Driver enum, per-technology TOML mechanism, mysql.toml, build.rs
- [x] 3.2 [expert] Route dialect resolution through Driver identity (dialect_for, dispatch)
- [x] 3.3 Driver identity in ScanSpec/scan driver load (SourceConnection rename)
- [x] 3.4 Capability invariant test enumerates dialects from Driver type (after 3.2)
- [x] 3.5 [expert] Move type mapping behind MetadataSource (after 3.1)
- [x] 3.7 [expert] Move MySQL capability list into mysql.toml (after 3.1)
- [x] 3.8 Make build.rs/drivers/ prerequisites of $(SO_FILE), add make_targets_test (after 3.1)
- [x] 3.6 Generalize integration-tier driver-fetch helper over Driver identity (Group G, after 6.1)

## Phase 4: PostgreSQL dialect rendering (Group E, after Group C)
- [x] 4.1 postgres_dialect.rs: PostgresDialect skeleton, identifiers, projection
- [x] 4.2 [expert] Render PostgreSQL filters + collation placement (after 4.1)
- [x] 4.3 [expert] Render PostgreSQL aggregation + collation placement (after 4.2)
- [x] 4.4 Render FULL OUTER JOIN, JOIN_TYPE_FULL_OUTER capability, dialect capability-set-diff test (after 4.3)
- [x] 4.5 Unit tests for PostgresDialect (after 4.4)

## Phase 5: PostgreSQL metadata, mapping, connection (Group F, after Group D)
- [x] 5.1 PostgreSQL metadata source (information_schema queries)
- [x] 5.2 PostgreSQL type mapping + unit tests
- [x] 5.3 PostgreSQL source precondition check (UTF8 encoding) (after 5.1, 5.2)
- [x] 5.4 [expert] PostgreSQL libpq connection string + credential redaction (after 5.1, 5.2)

## Phase 6: Range resolution, driver path, PostgreSQL provisioning (Group G, after Group D)
- [x] 6.1 [expert] Range resolution in registry_index (semver dependency)
- [x] 6.6 Remove digest verification (immediately after 6.1, before 6.2)
- [x] 6.2 [expert] Versioned BucketFS install path, presence-check rebind (after 6.6)
- [x] 6.3 DRIVER_PATH property (after 6.2, after Group F's 3.5 work lands in create_schema.rs)
- [x] 6.4 crates/adbc-vs-scan/drivers/postgresql.toml (after 6.3)
- [x] 6.5 Extend provisioning tests for two drivers (after 6.4)

## Phase 7: Scan coercion and PostgreSQL integration tier (Group H, after Group D)
- [x] 7.1 Coerce numeric-as-string Arrow column to declared DECIMAL
- [x] 7.2 PostgreSQL integration tier (postgres-it feature, postgres_it_test.rs) (after 3.6, 6.4)

## Phase 8: End-to-end tier (Group I, after Groups E, F, G, H)
- [x] 8.1 Grow docker/postgres/init.sql into differential fixture + Exasol mirror tables
- [x] 8.2 [expert] Differential PostgreSQL E2E tests, shared setup module (after 8.1)
- [x] 8.3 Two-driver E2E test + every_supported_driver_creates_a_virtual_schema (after 8.2)
- [x] 8.4 PostgreSQL omission E2E test + credential-redaction E2E assertions (after 8.2)
- [x] 8.5 two_virtual_schemas_advertise_their_own_capability_sets (after 8.2)

## Phase 9: Operational surface and mission (Group J, after Group I)
- [x] 9.1 make test-e2e-postgres target
- [x] 9.2 CI workflow: test-it-postgres + e2e-postgres target, disk cleanup check
- [x] 9.3 Update CLAUDE.md dense bullets (postgres service, TOML mechanism, DRIVER_PATH, etc.)
- [x] 9.4 Update specs/mission.md
- [x] 9.5 Record framework/technology spec convention in CLAUDE.md Spec governance

## Phase 10: Review and verification (Group K, after Group J — orchestrator-run)
- [x] 10.1 Code review of all changed files
- [x] 10.2 Apply standard review fixes (if any)
- [x] 10.3 Apply expert review fixes (if any)
- [x] 10.4 Run automated checklist (build, test, artifact, integration both drivers, e2e both drivers, lint, format, deny)
- [x] 10.5 Scenario coverage audit
- [x] 10.6 Manual verification steps
- [x] 10.7 Confirm record-time Background/description rewrites owed; run post-record check (expect FAIL x11, exit 1, pre-record)
- [x] 10.8 Generate verification report

## Phase 12: Review Fixes (Standard)
- [x] 12.1 Delete `driver_load_probe.rs`/`driver_load_probe_tests.rs` (superseded by task 3.1's real entry-point resolution), remove their `lib.rs` wiring and the `driver-load-probe` Cargo feature
- [x] 12.2 Delete `the_manifest_filename_must_equal_the_compiled_in_filename` from `provisioning_tests.rs` (compares a value with itself)
- [x] 12.3 In `provisioning_tests.rs`, delete `each_driver_resolves_against_its_own_declared_range` and `a_diverging_manifest_filename_names_both_filenames`, and replace the path literal in `a_bucket_reports_where_its_driver_is_installed` with a call to `driver_install_path`
- [x] 12.4 In `tests/provisioning_test.rs`, rewrite `the_driver_path_route_makes_no_registry_request` to drive a route that can actually reach the stub registry, and fix `StubServer::serve` to log a request before invoking its handler
- [x] 12.5 Extend `every_supported_driver_creates_a_virtual_schema` in `tests/e2e_postgres_test.rs` to assert the created schema exposes a table and that `adapterNotes.driver` matches
- [x] 12.6 Make `tests/common/schema.rs`'s `quote_literal` `pub`, delete the duplicate local `quote` in `tests/e2e_postgres_test.rs`, and use `schema::quote_literal` at its call sites
- [x] 12.7 In `driver_selection_tests.rs`, replace the in-test runtime no-op signature check with a module-level `const _: fn(...) = metadata::source_for;` with a doc comment
- [x] 12.8 In `driver_selection_tests.rs`, delete the emptiness assertion duplicated between `each_driver_resolves_to_its_own_tables` and `every_identity_yields_every_table`
- [x] 12.9 Delete `the_exposed_namespace_is_the_public_schema` from `postgres_source_tests.rs`, folding its rationale into `both_catalog_queries_filter_on_the_exposed_namespace`
- [x] 12.10 Delete `the_precondition_query_reads_the_server_encoding` from `postgres_source_tests.rs` (subsumed by `metadata_source_tests.rs`)
- [x] 12.11 Extend `i64_cell_reads_signed_and_unsigned_integer_arrays` in `metadata_tests.rs` with a `UInt64Array` case, and add `i64_cell_reports_a_uint64_that_overflows_i64`
- [x] 12.12 Add an accepted-maximum case (`DECIMAL(36, s)`) to `oversize_decimal_unmapped` in `mysql_type_mapping_tests.rs`
- [x] 12.13 In `capabilities_tests.rs`, delete the seven membership tests subsumed by the two exact-set tests, moving their rationale into comments on the capability-list entries
- [x] 12.14 Delete `an_unmapped_type_states_a_reason_naming_it` from `postgres_type_mapping_tests.rs` (duplicates a loop case)
- [x] 12.15 Delete `select_list_entries_sharing_a_source_column_name_get_distinct_positional_names` from `pushdown_tests.rs` (implementation-coupled duplicate of a public-response test)
- [x] 12.16 Extract the shared mirror-table helpers and their four unit tests out of `mirror.rs`/`postgres_mirror.rs` into a new `tests/common/mirror_table.rs`
- [x] 12.17 Add `assert_container_address_invariants` to `tests/common/mod.rs` and use it from both `postgres.rs` and `mysql.rs` in place of their three duplicated tests
- [x] 12.18 In `tests/common/postgres_stack.rs`, remove the unused `postgres`/`mysql` fields from `Stack` and document why `bring_up` still constructs them
- [x] 12.19 Delete the unneeded `#![allow(dead_code)]` from `crates/adbc-vs-scan/tests/common/mod.rs`
- [x] 12.20 In `crates/adbc-vs-scan/tests/driver_toml_test.rs`, resolve the build script from this test's own `OUT_DIR` instead of scanning sibling fingerprint directories by mtime
- [x] 12.21 In `crates/adbc-vs-scan/build.rs`, reject a leading-digit driver-name stem and validate `[capabilities] advertised` is non-empty with no blank entries; add the three matching cases to `driver_toml_test.rs`
- [x] 12.22 Delete the redundant per-file `rerun-if-changed` loop in `crates/adbc-vs-scan/build.rs` (the directory-level one already covers it)
- [x] 12.23 Add `crates/adbc-vs-scan/drivers/postgresql.toml` to the staleness input array in `tests/make_targets_test.rs`
- [x] 12.24 Extend the `e2e` and `e2e-postgres` CI jobs' disk-cleanup step with `/opt/hostedtoolcache` and `/usr/local/.ghcup`, and update CLAUDE.md's matching bullet
- [x] 12.25 Reconcile CLAUDE.md's stale bullets (test tiers, CI job list, licensing scope) with the current config, accounting for the expert agent's CI-job split and `registry-live` feature
- [x] 12.26 Reword the four MySQL-only doc comments in driver-generic modules (`create_schema.rs`, `adapter_notes.rs`, `sql_dialect.rs`, and the type-mapping constant doc) to be source-agnostic, checking current file locations first

## Phase 11: Review Fixes (Expert)
- [x] 11.1 [expert] Move `reject_unsafe_database_name` out of `create_schema::open` into MySQL's own source so the guard runs only for `Driver::MySql`
- [x] 11.2 [expert] Rename `src/type_mapping.rs` to `src/mysql_type_mapping.rs` and leave a framework `type_mapping.rs` holding only `MappedColumn`, `MappedType`, its `ExasolType` conversion, and the three Exasol bounds
- [x] 11.3 [expert] Add a framework `metadata::assemble_tables` over a `CatalogRow` bound, delete both per-source copies, and split MySQL's source out of `metadata.rs` into `src/mysql_source.rs`
- [x] 11.4 [expert] Bound what `extract_driver` unpacks with `MAX_UNPACKED_BYTES` and a `ProvisioningError::PackageTooLarge` variant naming the URL and the limit
- [x] 11.5 [expert] Move `each_drivers_range_resolves_against_the_real_registry_index` into `tests/provisioning_test.rs` behind a default-off `registry-live` feature with a `make test-registry-live` target
- [x] 11.6 [expert] Add `adbc-vs-adapter` to `adbc-vs-scan`'s dev-dependencies and drive `tests/common/driver_fetch.rs` through the production index and MANIFEST readers
- [x] 11.7 [expert] Drop the credential-bearing conninfo keywords the PostgreSQL address's own query string carries in `connection::query_parameters`
- [x] 11.8 [expert] Give the three PostgreSQL E2E delegation assertions their missing half in `tests/e2e_postgres_test.rs`
- [x] 11.9 [expert] Add `driver_path_is_accepted_and_bypasses_provisioning` to `tests/e2e_postgres_test.rs`
- [x] 11.10 [expert] Drive `artifact_test::no_bundled_driver_binary` from `Driver::ALL` rather than MySQL alone
- [x] 11.11 [expert] Split the PostgreSQL tiers into their own `integration-tests-postgres` and `e2e-postgres` CI jobs and add both to `release`'s `needs`
