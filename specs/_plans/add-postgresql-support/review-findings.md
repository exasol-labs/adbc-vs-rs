# Code Review Findings: add-postgresql-support

## Summary
- Files reviewed: 74 (49 modified, 24 new, 1 deleted)
- Total findings: 37 (standard: 26, expert: 11)

## Expert fixes

### crates/adbc-vs-adapter/src/create_schema.rs

#### [BOUNDARY_VIOLATION] MySQL-only database-name guard now refuses legitimate PostgreSQL schemas
- Location: line 177 (`reject_unsafe_database_name(&database)?`), guard at lines 474-494
- Issue: `open` calls `parse_database` then `reject_unsafe_database_name` unconditionally for every driver. The guard exists solely because `metadata::quote_string_literal` interpolates the database name into MySQL's `information_schema` queries — its own doc comment (lines 474-482) says exactly that, and cites `mysql_dialect.rs`'s `NO_BACKSLASH_ESCAPES` reasoning. PostgreSQL interpolates nothing: `postgres_source.rs` lines 11-15 state "no value resolved from a `CONNECTION` object or typed by an operator reaches either query below: there is no string literal to escape". A PostgreSQL database named `it's` or `back\slash` is therefore refused with `SchemaError::UnsafeDatabaseName`, whose message ("the information_schema queries built from it cannot safely quote") is factually false for that source. This is a MySQL-only assumption that survived the driver-identity generalization.
- Fix: In crates/adbc-vs-adapter/src/create_schema.rs, move the `reject_unsafe_database_name` call out of `open` and into MySQL's own source: add a `MySqlSource::new` precondition or run it inside `MySqlSource::check_source`, so the check runs only for `Driver::MySql`. Add a unit test in `create_schema_tests.rs` asserting a PostgreSQL `CONNECTION` address whose database component contains `'` is accepted, and keep the existing MySQL rejection tests (`create_schema_tests.rs` lines 394 and 409) passing.

### crates/adbc-vs-adapter/src/type_mapping.rs

#### [INFORMATION_LEAKAGE] PostgreSQL's mapping depends on MySQL's module for Exasol-owned bounds
- Location: crates/adbc-vs-adapter/src/type_mapping.rs lines 1-2 and 18-31; crates/adbc-vs-adapter/src/postgres_type_mapping.rs lines 22-24
- Issue: `type_mapping.rs`'s own module doc reads "Converts a MySQL column definition into the Exasol data type the virtual table declares" — it is MySQL's mapping module — yet it also owns the framework-level `MappedColumn`, `MappedType`, `MAX_DECIMAL_PRECISION`, `MAX_VARCHAR_SIZE`, and `MIN_CHARACTER_SIZE`, and `postgres_type_mapping.rs` imports all five from it. Every other per-technology unit in this change carries its technology in the module name (`mysql_dialect.rs`, `postgres_dialect.rs`, `postgres_type_mapping.rs`, `postgres_source.rs`), and the plan's own § Features table splits `vs-adapter/type-mapping` (framework) from `vs-adapter/mysql-type-mapping` and `vs-adapter/postgresql-type-mapping`. The code split is half-done: the test file was renamed to `mysql_type_mapping_tests.rs` but the module was not, so MySQL remains the implicit default and a third technology's author will import Exasol's bounds out of MySQL's file.
- Fix: In crates/adbc-vs-adapter, rename `src/type_mapping.rs` to `src/mysql_type_mapping.rs` (keeping its `#[path = "mysql_type_mapping_tests.rs"]` test module), and create a new `src/type_mapping.rs` holding only the framework items — `MappedColumn`, `MappedType`, its `From<MappedType> for exa_type::ExasolType` conversion, `MAX_DECIMAL_PRECISION`, `MAX_VARCHAR_SIZE`, `MIN_CHARACTER_SIZE`. Update `lib.rs`, `metadata.rs`, `postgres_type_mapping.rs`, `create_schema.rs`, and `postgres_source.rs` imports accordingly. Leave `SUPPORTED_CHARACTER_SET`, `INT_PRECISION`, `BIGINT_PRECISION`, and `character_size` in the MySQL module.

### crates/adbc-vs-adapter/src/metadata.rs

#### [INFORMATION_LEAKAGE] `assemble_tables` is duplicated verbatim across the two sources
- Location: crates/adbc-vs-adapter/src/metadata.rs lines 268-305; crates/adbc-vs-adapter/src/postgres_source.rs lines 132-169
- Issue: The two bodies are identical apart from one line — `type_mapping::map_column(&mysql_column(row))` versus `postgres_type_mapping::map_column(&postgres_column(row))`. Everything else (the `sort_by_key` on `(table, ordinal_position)`, the two `BTreeMap`s, the `Omission { table, column, source_type, reason }` shape, the `table_names.into_iter()` join that yields an empty column list for an unknown table) is the framework's rule about how a discovered table is assembled, not any technology's. Changing that rule — the omission shape, the ordering guarantee, how a table with no rows is reported — now requires editing both modules, and the mission names three more technologies that would each copy it again. `metadata.rs` compounds this by holding the framework trait, `source_for`, the shared Arrow cell helpers, *and* `MySqlSource`, so PostgreSQL's source imports its framework from MySQL's module.
- Fix: In crates/adbc-vs-adapter/src/metadata.rs, add a framework function `pub(crate) fn assemble_tables<R>(table_names: Vec<String>, rows: Vec<R>, map: impl Fn(R) -> Result<MappedColumn, String>) -> Vec<DiscoveredTable>` taking a `pub(crate) trait CatalogRow { fn table(&self) -> &str; fn column(&self) -> &str; fn ordinal_position(&self) -> i64; fn source_type(&self) -> &str; }` bound on `R`, implement `CatalogRow` for both `ColumnRow` types, and delete the two private `assemble_tables` copies. Then split `MySqlSource`, `SourceCollations`, `check_required_collations`, `ColumnRow`, `mysql_column`, `quote_string_literal`, and the four `query_*` functions out of `metadata.rs` into a new `src/mysql_source.rs`, leaving `metadata.rs` holding only the trait, `source_for`, `DiscoveredColumn`/`DiscoveredTable`/`MetadataError`, the shared Arrow cell helpers, and the new `assemble_tables`. Update `lib.rs` and the `#[path]` test-module attributes.

### crates/adbc-vs-adapter/src/provisioning.rs

#### [TACTICAL_SHORTCUT] Removing the digest left the archive unpacked from unverified bytes with no decompressed bound
- Location: lines 503-507 (`download` then `extract_driver`), 644-702 (`extract_driver`), 78-79 (`MAX_PACKAGE_BYTES`)
- Issue: Release 0.2 called `verify_digest(&package, &registry.sha256, &package_url)?` before `extract_driver` (`git show HEAD:crates/adbc-vs-adapter/src/provisioning.rs` line 426). This change deletes that call, so `extract_driver` is now the first thing to touch bytes nobody has vouched for. `MAX_PACKAGE_BYTES` bounds only the *compressed* body; `GzDecoder` is unbounded, `member.unpack` writes to the temp directory unbounded, and the `MANIFEST` member is read with `read_to_string` into memory unbounded. A 256 MB response can expand to arbitrarily many gigabytes inside the UDF sandbox, filling the container's disk or exhausting its memory rather than failing with a named error. The module doc (lines 34-40) accepts TLS as the only integrity guarantee but says nothing about what the unpacking step now assumes.
- Fix: In crates/adbc-vs-adapter/src/provisioning.rs, add a `MAX_UNPACKED_BYTES` constant beside `MAX_PACKAGE_BYTES` and enforce it in `extract_driver`: wrap the `GzDecoder` in a `std::io::Read::take(MAX_UNPACKED_BYTES)`, accumulate the bytes written per member, and return a new `ProvisioningError::PackageTooLarge { url, limit }` variant naming the URL and the limit when the running total exceeds it; read the `MANIFEST` member through the same bound. Add unit tests in `provisioning_tests.rs` for a package whose members exceed the bound and for a `MANIFEST` that does, both asserting the error names the limit. Extend the module doc's integrity paragraph to state that the unpacking step bounds what it writes because nothing verifies the archive.

### crates/adbc-vs-adapter/src/provisioning_tests.rs

#### [NONDETERMINISTIC_TEST] A unit test makes a live HTTPS request to the public registry
- Location: lines 88-121, `each_drivers_range_resolves_against_the_real_registry_index`
- Issue: The test builds `registry_client()` and calls `download(&client, &index_url, MAX_INDEX_BYTES)` against `https://dbc-cdn.columnar.tech`, then panics if it is unreachable. It lives in `src/provisioning_tests.rs`, which is the default `cargo test --workspace` tier that `make test` runs and CI's `unit-tests` job invokes. `/speq:code-guardrails` requires unit tests to be "Independent and repeatable: no shared clock, network, filesystem, or unseeded randomness", and CLAUDE.md's test-tier bullets place every environment-dependent check behind a default-off feature. As written, a CDN outage or a slow DNS answer turns the fast tier red, and the CI job that gates `release` fails for a reason unrelated to the commit. The drift check itself is valuable and must not simply be deleted.
- Fix: Move `each_drivers_range_resolves_against_the_real_registry_index` out of crates/adbc-vs-adapter/src/provisioning_tests.rs into crates/adbc-vs-adapter/tests/provisioning_test.rs, and gate that one test with a new default-off cargo feature `registry-live` declared in crates/adbc-vs-adapter/Cargo.toml. Add a `make test-registry-live` target running `cargo test -p adbc-vs-adapter --features registry-live --test provisioning_test -- registry_live`, add the matching assertion to `crates/adbc-vs-adapter/tests/make_targets_test.rs`, and add the target's feature name to CLAUDE.md's "All non-unit tiers are behind default-off cargo features" bullet. Do not add it to any CI job.

### crates/adbc-vs-scan/tests/common/driver_fetch.rs

#### [INFORMATION_LEAKAGE] Registry-index and MANIFEST parsing reimplemented beside the production parser
- Location: lines 10-15 (module doc), 70-137 (`Resolved`, `resolve`, `published_version`), 145-176 (`extract_declared_library`), 209-246 (`Index`, `IndexDriver`, `VersionBlock`, `PlatformPackage`, `Manifest`, `ManifestFiles`)
- Issue: The registry `index.yaml` shape, the "normalize one optional leading `v`, take the highest in range" rule, and the `MANIFEST [Files] driver` shape are each defined twice — once in `adbc-vs-adapter`'s `registry_index.rs`/`provisioning.rs`, and again here. The module doc admits it: "resolution and manifest reading are reimplemented here against the same document shape, kept intentionally small so the two stay easy to compare by eye". That is a shortcut with no follow-up, and "compare by eye" is exactly the enforcement `/speq:design-philosophy` names as back-door leakage: if the registry changes its document shape, the integration tiers keep certifying against the old one. The stated justification — that the crate dependency runs adapter to scan — is only true of `[dependencies]`; cargo resolves `[dev-dependencies]` separately and permits a dev-dependency cycle between workspace members.
- Fix: In crates/adbc-vs-scan/Cargo.toml, add `adbc-vs-adapter = { path = "../adbc-vs-adapter" }` to `[dev-dependencies]` and run `cargo test -p adbc-vs-scan --features postgres-it --no-run` to confirm cargo resolves it. If it resolves, in crates/adbc-vs-scan/tests/common/driver_fetch.rs replace `resolve`, `published_version`, `Index`, `IndexDriver`, `VersionBlock`, and `PlatformPackage` with a call to `adbc_vs_adapter::registry_index::resolve_package`, and replace the `Manifest`/`ManifestFiles` structs with the adapter's manifest reading, deleting the duplicated declarations. If cargo refuses the cycle, leave the reimplementation in place and replace the module doc's "Reusing ... is not an option" sentence with the exact cargo error, so the constraint is recorded as measured rather than asserted.

### crates/adbc-vs-scan/src/connection.rs

#### [TACTICAL_SHORTCUT] Redaction covers only the resolved credential, not one the address carries
- Location: lines 298-315 (`query_parameters`), 218-235 (`conninfo`), 408-419 (`redact`)
- Issue: `PostgresAddress::parse` deliberately drops userinfo (lines 194-195) because "a user and password written into the address ... would put a secret into the subject an error names". The query string gets no such treatment: `query_parameters` copies every `name=value` pair across verbatim, so an address like `postgresql://host/db?password=hunter2` puts `password='hunter2'` into the conninfo string handed to libpq. `redact` strips only `connection.password` in its verbatim and conninfo-escaped forms, so a driver diagnostic echoing the connection string discloses the address-borne secret in full. The last-occurrence rule already makes such a parameter inert (the resolved credential is appended after it, lines 227-228), so carrying it across buys nothing and costs the one disclosure path this module was built to close.
- Fix: In crates/adbc-vs-scan/src/connection.rs, drop the conninfo keywords that carry a credential from the address's own query string: in `query_parameters`, filter out any parameter whose lower-cased name is `password`, `user`, or `sslpassword` before building the pair list, and state in the function's doc comment that the resolved `CONNECTION` object is the only source of a credential so an address-borne one is discarded rather than carried and overridden. Add unit tests in `connection_tests.rs` asserting that an address carrying `?password=...` produces a conninfo string that does not contain that value, and that the resolved credential still lands as the final `password=` keyword.

### crates/adbc-vs-adapter/tests/e2e_postgres_test.rs

#### [MISSING_BOUNDARY_TEST] Three delegation assertions verify only one of the two required halves
- Location: lines 187-198 (`filter_reaches_postgresql_and_matches_the_mirror`, ordering half), 420-459 (`uncollated_grouping_and_distinct_return_the_mirrors_group_count`), 342-412 (`collation_is_rendered_only_where_ordering_decides_the_answer`)
- Issue: CLAUDE.md requires every delegated fragment be verified twice — `EXPLAIN VIRTUAL` proving the fragment reached the generated source statement, and the rows proving it produced the right answer — because "asserting only one cannot tell a correct delegation from a silently dropped one". (a) The `ordering_query` half at line 187 calls only `assert_matches_mirror`; its own comment calls it "the assertion a missing COLLATE \"C\" fails", which only holds if the predicate actually reached PostgreSQL — if the filter were silently not delegated, Exasol would compute it locally and the row assertion would still pass. The first half of the same test does call `generated_statement`. (b) `uncollated_grouping_and_distinct_return_the_mirrors_group_count` asserts rows only; a `GROUP BY` Exasol computed itself passes it. (c) `collation_is_rendered_only_where_ordering_decides_the_answer` asserts generated statements only, and its `LIKE 'ab%'` case has no row-level counterpart anywhere in the file (the filter test uses `'ab!%'`).
- Fix: In crates/adbc-vs-adapter/tests/e2e_postgres_test.rs: (a) before line 189's `assert_matches_mirror(ordering_query)`, add `let ordering_statement = generated_statement(&ordering_query(VIRTUAL_SCHEMA));` and assert it contains both `"sort_key"` and `COLLATE "C"`; (b) in `uncollated_grouping_and_distinct_return_the_mirrors_group_count`, add `generated_statement` assertions that the grouped statement contains a bare `GROUP BY "category"` with no `COLLATE`, and that the distinct statement's `COUNT(DISTINCT ...)` operand carries no `COLLATE`; (c) in `collation_is_rendered_only_where_ordering_decides_the_answer`, add an `assert_matches_mirror` over `SELECT "id" FROM {schema}."items" WHERE "tag" LIKE 'ab%' ORDER BY "id"` and over `SELECT MIN("sort_key") AS LO, MAX("sort_key") AS HI FROM {schema}."items"`.

#### [MISSING_BOUNDARY_TEST] The `DRIVER_PATH` success path has no end-to-end coverage and its named test does not exist
- Location: crates/adbc-vs-adapter/tests/dispatch_test.rs line 273; specs/_plans/add-postgresql-support/plan.md lines 526 and 529
- Issue: `dispatch_test.rs`'s comment states "The success half is `e2e_postgres_test.rs::driver_path_is_accepted_and_bypasses_provisioning`", and plan.md's § Scenario Coverage maps the scenario "DRIVER_PATH is accepted and bypasses driver provisioning" to that same test name. `grep -rn driver_path_is_accepted_and_bypasses_provisioning crates/` returns only the comment — the test was never written. `DRIVER_PATH` is the release's answer to the egress-loss upgrade surprise the plan calls "the single most likely upgrade surprise in this release", and its success path is exercised only by a `#[cfg(test)]` string assertion on the generated DDL at `tests/common/schema.rs` line 271. `VirtualSchemaSpec.driver_path: Some(...)` never reaches a live stack.
- Fix: In crates/adbc-vs-adapter/tests/e2e_postgres_test.rs, add `#[test] fn driver_path_is_accepted_and_bypasses_provisioning()`: read the installed PostgreSQL driver's BucketFS mount path out of the shared schema's `adapterNotes.driverPath`, then create a second virtual schema through `schema::create_virtual_schema_through` with `driver_path: Some(<that path>)` and `bucketfs_connection: None`, assert the schema exposes the same table set as the shared one, assert its `adapterNotes` carries that exact `driverPath` and no `driverVersion` key, and drop it. Leave the `dispatch_test.rs` comment and the plan's coverage rows unchanged once the test exists.

### crates/adbc-vs-adapter/tests/artifact_test.rs

#### [MISSING_BOUNDARY_TEST] The bundled-driver assertions still cover MySQL only
- Location: line 29 (`DRIVER_INIT_SYMBOL`), lines 103-118 (tracked-file filter)
- Issue: `no_bundled_driver_binary` exists to prove the shipped `.so` neither exports a driver's init symbol nor ships a driver binary in the repo. After a second driver landed it still checks exactly one: `const DRIVER_INIT_SYMBOL: &str = "AdbcDriverMysqlInit";` and the tracked-file filter compares against `Driver::MySql.package().library` in three places. A statically linked or accidentally committed `libadbc_driver_postgresql.so` passes the test. Both facts are already on the identity — `DriverPackage::library` and `DriverPackage::entry_point` (`crates/adbc-vs-scan/src/driver.rs` lines 36-41) — so the assertion can be driven from `Driver::ALL` and inherit the third technology for free.
- Fix: In crates/adbc-vs-adapter/tests/artifact_test.rs, delete `DRIVER_INIT_SYMBOL` and rewrite `no_bundled_driver_binary` to loop `for driver in adbc_vs_scan::driver::Driver::ALL`, asserting for each that the built `.so`'s dynamic symbol table does not export `driver.package().entry_point` and that no tracked file equals or ends in `/{}` for `driver.package().library`. Keep the existing failure messages, naming the driver in each.

### .github/workflows/ci.yml

#### [SKIPPED_TEST] Both PostgreSQL tiers are silently skipped whenever the MySQL tier fails
- Location: lines 59-62 (`integration-tests`) and lines 78-81 (`e2e`)
- Issue: `make test-it-postgres` and `make test-e2e-postgres` were added as additional steps inside the existing `integration-tests` and `e2e` jobs, with GitHub Actions' default `if: success()`. Any MySQL-tier failure ends the job before the PostgreSQL step runs, so the technology this plan adds has no CI signal at exactly the moment the build is red — and `release`'s `needs: [..., integration-tests, e2e]` (line 95) cannot tell "PostgreSQL passed" from "PostgreSQL never ran". Both tiers are also serialized onto one runner, doubling the `e2e` job's wall clock against the same disk budget.
- Fix: In .github/workflows/ci.yml, split the two PostgreSQL steps into their own jobs: add `integration-tests-postgres` (checkout, then `run: make test-it-postgres`) and `e2e-postgres` (checkout, the same free-disk / AppArmor / exapump preparation steps the `e2e` job carries, then `run: make test-e2e-postgres`, plus the same two `if: failure()` container-log steps). Remove the two added steps from `integration-tests` and `e2e`. Add both new job names to `release`'s `needs:` list at line 95. Add the two jobs to `crates/adbc-vs-adapter/tests/workflow_test.rs`'s job-to-target assertions, and update CLAUDE.md's "Seven jobs" bullet list to nine.

## Standard fixes

### crates/adbc-vs-adapter/src/driver_load_probe.rs

#### [DEAD_FLEXIBILITY] The gate 2.2b probe is superseded and reachable from no check
- Location: crates/adbc-vs-adapter/src/driver_load_probe.rs (whole file), crates/adbc-vs-adapter/src/driver_load_probe_tests.rs (whole file), crates/adbc-vs-adapter/src/lib.rs lines 5-6 and 59-74, crates/adbc-vs-adapter/Cargo.toml lines 26-30
- Issue: The module's own doc says it is throwaway and that "the real per-driver entry-point resolution lands with task 3.1"; task 3.1 is complete and `connection.rs::open` line 367 now reads the entry point off `driver.package().entry_point`. The `driver-load-probe` feature's Cargo.toml comment states it is "Not wired into any `make` target", and it is referenced by no Make target, no CI job, and no test tier — so its two `#[test]` functions never execute (`make test` runs `cargo test --workspace` without `--all-features`; only `make lint`'s `cargo clippy --all-features` type-checks them). It also hard-codes `const ENTRY_POINT: &[u8] = b"AdbcDriverPostgresqlInit"`, a second spelling of the value that now lives in `drivers/postgresql.toml`, free to drift. `probe_reports_ok_for_a_path_holding_no_driver` asserts the opposite of what its name says. The measurement it produced is recorded in decision-log.md's gate 2.2b entry.
- Fix: Delete crates/adbc-vs-adapter/src/driver_load_probe.rs and crates/adbc-vs-adapter/src/driver_load_probe_tests.rs, remove lines 5-6 and 59-74 from crates/adbc-vs-adapter/src/lib.rs, and remove the `driver-load-probe = []` feature and its comment from crates/adbc-vs-adapter/Cargo.toml lines 26-30.

### crates/adbc-vs-adapter/src/provisioning_tests.rs

#### [ASSERTION_FREE_TEST] `the_manifest_filename_must_equal_the_compiled_in_filename` feeds the function its own input
- Location: lines 342-346
- Issue: The test calls `verify_library_filename(mysql_library(), Driver::MySql)`, where `mysql_library()` is `Driver::MySql.package().library` (lines 13-16) and the function under test is `if declared == driver.package().library { return Ok(()) }` (`provisioning.rs` lines 706-710). It compares a value with itself; no change to either side can make it fail.
- Fix: Delete `the_manifest_filename_must_equal_the_compiled_in_filename` from crates/adbc-vs-adapter/src/provisioning_tests.rs. The negative case at line 349 and the live-MANIFEST case at `tests/provisioning_test.rs` line 154 already cover the comparison.

#### [DUPLICATE_TEST] Three pairs of tests assert the same facts twice
- Location: lines 71-87 vs 134-152; lines 488-500 vs 365-374; lines 349-362 vs crates/adbc-vs-adapter/tests/provisioning_test.rs lines 154-181
- Issue: (a) `each_driver_resolves_against_its_own_declared_range` and `ranges_are_independent_rather_than_one_shared_minor_bump_rule` both resolve `FIXTURE_INDEX` through `Registry::public(Driver::MySql)` and `Registry::public(Driver::Postgres)` and both assert `("0.6.1", "1.13.0")`. (b) `a_bucket_reports_where_its_driver_is_installed` asserts the same literal install path `"/buckets/bfsdefault/default/adbc-drivers/mysql/0.6.1/libadbc_driver_mysql.so"` that `the_install_path_carries_the_driver_and_the_resolved_version` already asserts, while `Bucket::driver_path` is a one-line delegation to `driver_install_path` (`provisioning.rs` lines 181-183). (c) `a_diverging_manifest_filename_names_both_filenames` drives `"libadbc_driver_mysql_v2.so"` and asserts the message names both filenames — exactly what `tests/provisioning_test.rs::a_diverging_manifest_filename_is_rejected` asserts, plus a `bucketfs.requests().is_empty()` check the unit test lacks.
- Fix: In crates/adbc-vs-adapter/src/provisioning_tests.rs, delete `each_driver_resolves_against_its_own_declared_range` (lines 71-87) and `a_diverging_manifest_filename_names_both_filenames` (lines 349-362), and in `a_bucket_reports_where_its_driver_is_installed` replace the path literal with `assert_eq!(bucket.driver_path(Driver::MySql, &version), driver_install_path(Path::new(BUCKETFS_MOUNT), Driver::MySql, &version))`.

### crates/adbc-vs-adapter/tests/provisioning_test.rs

#### [ASSERTION_FREE_TEST] `the_driver_path_route_makes_no_registry_request` asserts against an unreachable stub
- Location: lines 622-651
- Issue: The test stands up a `StubServer` whose handler panics on any request, then calls `supplied_driver(path, Driver::Postgres)` — which takes no `Registry` and holds no reference to the stub — and asserts `registry.requests().is_empty()`. Nothing can ever reach that server, so the assertion is true by construction. It is also unfalsifiable a second way: `StubServer::serve` invokes `handler(&request)` at line 988 *before* pushing the request into the shared log at lines 990-993, and `Drop` swallows the worker's panic with `let _ = worker.join();` at line 1009, so even a real request would leave the log empty.
- Fix: In crates/adbc-vs-adapter/tests/provisioning_test.rs, rewrite `the_driver_path_route_makes_no_registry_request` to drive the route that can reach a registry: build a `Registry::at(&stub.base_url(), Driver::Postgres)` and a `Bucket`, assert `ensure_driver` reaches the stub, then assert `supplied_driver` on the same fixture path returns without the stub's request count increasing. Separately, in `StubServer::serve`, push the request into `shared.requests` before invoking `handler`, so a panicking handler still records what it was called with.

### crates/adbc-vs-adapter/tests/e2e_postgres_test.rs

#### [ASSERTION_FREE_TEST] `every_supported_driver_creates_a_virtual_schema` asserts nothing
- Location: lines 694-708
- Issue: The whole body is `schema::create_virtual_schema(...)` followed by `schema::drop_virtual_schema(...)` for each `Driver::ALL` entry. It proves only that the DDL did not make a helper panic; it never checks that the created schema exposes any table, that the right dialect was resolved, or that `adapterNotes.driver` records the driver the loop asked for. A `createVirtualSchema` that succeeded while discovering nothing passes.
- Fix: In crates/adbc-vs-adapter/tests/e2e_postgres_test.rs, extend `every_supported_driver_creates_a_virtual_schema` so that after each `create_virtual_schema` it asserts the schema exposes at least one table (query `EXA_ALL_TABLES` filtered on the schema name, or reuse the file's existing table-listing helper) and that the schema's `adapterNotes` parses with `driver` equal to `driver.as_str()`, before dropping the schema.

#### [SHRINKABLE] A local `quote` duplicates `schema::quote_literal`
- Location: lines 937-939
- Issue: `fn quote(value: &str) -> String { format!("'{}'", value.replace('\'', "''")) }` is byte-identical to the private `quote_literal` at `crates/adbc-vs-adapter/tests/common/schema.rs` lines 176-178, which already has its own unit tests for the doubling rule.
- Fix: In crates/adbc-vs-adapter/tests/common/schema.rs, change `fn quote_literal` to `pub fn quote_literal`, and in crates/adbc-vs-adapter/tests/e2e_postgres_test.rs delete the local `quote` (lines 937-939) and replace its call sites with `schema::quote_literal`.

### crates/adbc-vs-adapter/src/driver_selection_tests.rs

#### [UNUSED_VARIABLE] A compile-time signature check sits inside a `#[test]` body as a runtime no-op
- Location: lines 122-127
- Issue: `let source_table: fn(Driver, adbc_driver_manager::ManagedConnection, &str) -> Box<dyn metadata::MetadataSource> = metadata::source_for; let _ = source_table;` executes nothing at run time. It is a coercion that the compiler checks, dressed as a test statement, so a reader of `every_identity_yields_every_table` cannot tell which of its lines actually assert anything.
- Fix: In crates/adbc-vs-adapter/src/driver_selection_tests.rs, delete lines 122-127 from `every_identity_yields_every_table` and add a module-level `const _: fn(Driver, adbc_driver_manager::ManagedConnection, &str) -> Box<dyn metadata::MetadataSource> = metadata::source_for;` beside the module's other items, with a one-line doc comment saying the compiler checks `source_for`'s shape because constructing its result needs a live ADBC connection.

#### [DUPLICATE_TEST] Two tests in one file assert the same non-empty capability set
- Location: lines 27-31 and lines 145-150
- Issue: `each_driver_resolves_to_its_own_tables` asserts `!dialect_for(*driver).capabilities().is_empty()` over `Driver::ALL`, and `every_identity_yields_every_table` asserts the same predicate over the same loop with a different message. `each_driver_resolves_to_its_own_tables` is about the tables being distinct per driver, not about emptiness.
- Fix: In crates/adbc-vs-adapter/src/driver_selection_tests.rs, delete the emptiness assertion at lines 27-31 from `each_driver_resolves_to_its_own_tables`, leaving that test to assert only that no two drivers resolve the same package row and dialect.

### crates/adbc-vs-adapter/src/postgres_source_tests.rs

#### [IMPLEMENTATION_COUPLED_TEST] `the_exposed_namespace_is_the_public_schema` asserts a constant against its own literal
- Location: lines 142-145
- Issue: The whole body is `assert_eq!(EXPOSED_NAMESPACE, "public");` over the module-private constant declared as `const EXPOSED_NAMESPACE: &str = "public";` (`postgres_source.rs` line 45). It asserts internal state, not observable behaviour, and duplicates the observable half already covered by `both_catalog_queries_filter_on_the_exposed_namespace` (line 150), which asserts the queries contain `table_schema = 'public'`.
- Fix: Delete `the_exposed_namespace_is_the_public_schema` from crates/adbc-vs-adapter/src/postgres_source_tests.rs, moving its `vs-adapter/postgresql-type-mapping` rationale into the doc comment of `both_catalog_queries_filter_on_the_exposed_namespace`.

#### [DUPLICATE_TEST] `the_precondition_query_reads_the_server_encoding` restates a cross-source assertion
- Location: crates/adbc-vs-adapter/src/postgres_source_tests.rs lines 294-302; crates/adbc-vs-adapter/src/metadata_source_tests.rs lines 126-129 and 147-153
- Issue: The `Driver::Postgres` arm of `metadata_source_tests.rs` already sets `subjects: vec!["server_encoding"]` over `queries: vec![postgres_source::server_encoding_query()]` and asserts `own.queries.iter().any(|sql| sql.contains(subject))`, plus the negative half that no source's queries name another source's subject. The single-source test adds nothing beyond it.
- Fix: Delete `the_precondition_query_reads_the_server_encoding` from crates/adbc-vs-adapter/src/postgres_source_tests.rs.

### crates/adbc-vs-adapter/src/metadata_tests.rs

#### [UNTESTED_ERROR_PATH] `i64_cell`'s `UInt64` overflow arm has no test, and the test that claims it does not cover it
- Location: lines 218-223 (`i64_cell_reads_signed_and_unsigned_integer_arrays`); production path at crates/adbc-vs-adapter/src/metadata.rs lines 494-501
- Issue: The test's body constructs only `Int64Array::from(vec![Some(7), None])` — there is no unsigned array anywhere in it, so the name promises coverage the body does not deliver. `i64_cell` has eight integer arms plus a `Decimal128(_, 0)` arm, and its one failure path — `i64::try_from(value)` on a `UInt64` exceeding `i64::MAX`, producing "overflows i64 in an information_schema column" — is reached by no test.
- Fix: In crates/adbc-vs-adapter/src/metadata_tests.rs, extend `i64_cell_reads_signed_and_unsigned_integer_arrays` with a `UInt64Array::from(vec![7u64])` case asserting `Some(7)`, and add a separate test `i64_cell_reports_a_uint64_that_overflows_i64` constructing `UInt64Array::from(vec![u64::MAX])` and asserting the error message contains "overflows i64".

### crates/adbc-vs-adapter/src/mysql_type_mapping_tests.rs

#### [MISSING_BOUNDARY_TEST] `oversize_decimal_unmapped` never asserts the accepted maximum
- Location: lines 55-66
- Issue: The test asserts only that `DECIMAL(40,2)` is unmapped. The guard under test is `precision > MAX_DECIMAL_PRECISION` (`type_mapping.rs` line 163), so an off-by-one to `>=` would still pass — nothing asserts that `DECIMAL(36, s)` maps. The PostgreSQL twin does cover both sides (`postgres_type_mapping_tests.rs` lines 140-147, "Exasol's maximum precision itself must still map").
- Fix: In crates/adbc-vs-adapter/src/mysql_type_mapping_tests.rs, add to `oversize_decimal_unmapped` an accept case with `numeric_precision: Some(36)` asserting the column maps to `MappedType::Decimal { precision: 36, .. }`, mirroring the PostgreSQL test's structure.

### crates/adbc-vs-adapter/src/capabilities_tests.rs

#### [DUPLICATE_TEST] Six membership tests are subsumed by the two exact-set tests
- Location: lines 292-294 and 342-345 (`mysql_capability_set_is_exact`, `postgres_capability_set_is_exact`, both via `assert_set_is` at lines 363-386); subsumed tests at lines 382, 423, 451, 488, 506, 525, 586
- Issue: `assert_set_is` sorts both sides and does `assert_eq!(advertised, expected)` on the whole set. Over `Driver::ALL` — exactly `{MySql, Postgres}` — that equality already implies every presence and absence claim made by `dialect_sets_differ_only_in_full_outer_join`, `avg_and_sum_are_withheld_per_dialect`, `every_dialect_withholds_ordering`, `mysql_join_capabilities_are_exact`, `postgres_join_capabilities_add_full_outer`, `filter_capabilities_match_per_dialect`, and `aggregation_capabilities_match_per_dialect`. The two exact-set tests are new in this change; the membership tests predate it and now restate a subset of the same fact.
- Fix: In crates/adbc-vs-adapter/src/capabilities_tests.rs, delete `dialect_sets_differ_only_in_full_outer_join`, `avg_and_sum_are_withheld_per_dialect`, `every_dialect_withholds_ordering`, `mysql_join_capabilities_are_exact`, `postgres_join_capabilities_add_full_outer`, `filter_capabilities_match_per_dialect`, and `aggregation_capabilities_match_per_dialect`, moving each one's doc-comment rationale onto the corresponding group of entries inside `RELEASE_0_2_MYSQL_CAPABILITIES` and `POSTGRES_CAPABILITIES` as comments. Keep `advertised_set_matches_the_rendered_fragments` and both exact-set tests.

### crates/adbc-vs-adapter/src/postgres_type_mapping_tests.rs

#### [DUPLICATE_TEST] `an_unmapped_type_states_a_reason_naming_it` repeats a loop iteration
- Location: lines 368-376, duplicating lines 231-256
- Issue: The test does `let reason = unmapped(&column("jsonb")); assert!(reason.contains("jsonb"))`. `non_core_types_are_unmapped_with_the_reason_naming_the_type` already loops over a list including `"jsonb"` (line 242) making the identical assertion.
- Fix: Delete `an_unmapped_type_states_a_reason_naming_it` from crates/adbc-vs-adapter/src/postgres_type_mapping_tests.rs.

### crates/adbc-vs-adapter/src/pushdown_tests.rs

#### [IMPLEMENTATION_COUPLED_TEST] A 50-line hand-built `Query` asserts what the public response already proves
- Location: lines 138-189
- Issue: `select_list_entries_sharing_a_source_column_name_get_distinct_positional_names` constructs internal `pushdown_query::SelectItem`/`ColumnRef` literals, calls the private `emitted_columns(&query)`, and asserts only `columns[0].name != columns[1].name`. The same rule is proved through the public `pushdown_response` by `a_join_projecting_two_columns_of_the_same_name_gets_distinct_positional_names` (lines 75-136), which asserts the actual `c1`/`c2` names appear in the emitted SQL.
- Fix: Delete `select_list_entries_sharing_a_source_column_name_get_distinct_positional_names` from crates/adbc-vs-adapter/src/pushdown_tests.rs.

### crates/adbc-vs-adapter/tests/common/postgres_mirror.rs

#### [SHRINKABLE] The mirror helper and its tests are duplicated from `mirror.rs`
- Location: crates/adbc-vs-adapter/tests/common/postgres_mirror.rs lines 51-54, 132-160, 171-260; crates/adbc-vs-adapter/tests/common/mirror.rs lines 38-41, 113-141, 152-240
- Issue: `struct Column { name, declaration }`, `seed`, `create_statement`, and `insert_statement` are byte-identical between the two modules apart from the module-level `SCHEMA` constant they interpolate. Four unit tests are duplicated with them — `a_table_is_named_exactly_as_the_virtual_schema_names_it`, `a_table_replaces_the_one_an_earlier_run_left_behind`, `a_seeding_statement_names_every_column_the_table_was_created_with`, `every_column_is_seeded_with_a_value` — as is the private `item_declarations()` fixture.
- Fix: In crates/adbc-vs-adapter/tests/common, add `mirror_table.rs` holding `pub struct Column { pub name, pub declaration }`, `pub fn create_statement(schema: &str, table: &str, columns: &[Column]) -> String`, `pub fn insert_statement(schema: &str, table: &str, columns: &[Column], values: &[&str]) -> String`, `pub fn seed(...)`, and the four unit tests parameterised over a schema name; register it in `common/mod.rs`. Delete those items from `mirror.rs` and `postgres_mirror.rs`, leaving each holding only its own `SCHEMA` constant, column declarations, and row values, and have both call the shared functions.

### crates/adbc-vs-adapter/tests/common/postgres.rs

#### [DUPLICATE_TEST] Three container-address assertions are copied from `mysql.rs`
- Location: crates/adbc-vs-adapter/tests/common/postgres.rs lines 197, 207, 223; crates/adbc-vs-adapter/tests/common/mysql.rs lines 279, 288, 293
- Issue: `driver_uri_carries_no_credential`, `the_unreachable_uri_names_the_same_host_on_a_port_nothing_serves`, and `the_published_endpoint_and_the_in_network_address_share_a_port` exist in both modules; the third has an identical body (`assert!(PUBLISHED_ENDPOINT.ends_with(&format!(":{PORT}")))`). The helpers under test (`driver_uri`, `unreachable_uri`, `query_scalar`) are legitimately per-technology, but the three assertions are the same claim about a `(ADDRESS, PORT, PUBLISHED_ENDPOINT, DATABASE, PASSWORD)` tuple.
- Fix: In crates/adbc-vs-adapter/tests/common/mod.rs, add `pub fn assert_container_address_invariants(driver_uri: &str, unreachable_uri: &str, published_endpoint: &str, port: u16, password: &str)` holding the three assertions, and in both `postgres.rs` and `mysql.rs` replace the three tests with one test each that calls it with that module's constants.

### crates/adbc-vs-adapter/tests/common/postgres_stack.rs

#### [UNUSED_VARIABLE] Two `Stack` fields are written and never read
- Location: lines 50-54
- Issue: `pub struct Stack { pub exasol, pub postgres, pub mysql }`, but `e2e_postgres_test.rs` reads only `stack.exasol` — `grep -n "stack.postgres\|stack.mysql" crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` returns nothing. The two are constructed for their setup side effects and then kept alive as struct fields, which the crate-wide `#![allow(dead_code)]` at `common/mod.rs` line 12 hides.
- Fix: In crates/adbc-vs-adapter/tests/common/postgres_stack.rs, remove the `postgres` and `mysql` fields from `Stack`, bind their constructors to local variables inside `bring_up`, and add a doc comment on `bring_up` stating that constructing each is the contract — `Postgres::from_stack` runs the seed check and `MySql::from_stack` installs the fixtures — so the values are not kept.

### crates/adbc-vs-scan/tests/common/mod.rs

#### [SUPPRESSED_WARNING] A crate-wide `#![allow(dead_code)]` with nothing to silence
- Location: line 7
- Issue: The new harness module opens with `#![allow(dead_code)]`, but its only member, `driver_fetch::fetch_driver_library`, is used by both `mysql_it_test.rs` and `postgres_it_test.rs`. The attribute pre-emptively silences the warning that would tell a maintainer when a later helper stops being used.
- Fix: Delete `#![allow(dead_code)]` from crates/adbc-vs-scan/tests/common/mod.rs line 7 and run `cargo test -p adbc-vs-scan --features mysql-it --no-run` and `--features postgres-it --no-run` to confirm no warning appears.

### crates/adbc-vs-scan/tests/driver_toml_test.rs

#### [NONDETERMINISTIC_TEST] The build script under test is chosen by filesystem mtime
- Location: lines 43-48 (`build_script`)
- Issue: The helper scans sibling `adbc-vs-scan-*` fingerprint directories and picks the `build-script-build` binary with the newest `fs::metadata(&path).and_then(|m| m.modified())`, via `.max_by_key(|(built, _)| *built)`. After switching feature sets — `postgres-it` versus `mysql-it` versus default — several fingerprint directories exist, and which binary the eight refusal assertions run against then depends on mtime resolution and build order rather than on the `build.rs` in the working tree.
- Fix: In crates/adbc-vs-scan/tests/driver_toml_test.rs, resolve the build script from this test's own `OUT_DIR` instead of scanning siblings: take `env!("OUT_DIR")`, walk up to its fingerprint directory, and join `build-script-build`; panic naming the resolved path when it does not exist.

### crates/adbc-vs-scan/build.rs

#### [MISSING_BOUNDARY_TEST] Two validation gaps the generator's own doc claims it has closed
- Location: lines 138-141 (driver-name rule), lines 236-238 (capability list)
- Issue: (a) `stem.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())` accepts a leading digit, so `drivers/9db.toml` passes validation and line 244's `format!("{}_PACKAGE", driver.to_ascii_uppercase())` emits `9DB_PACKAGE` — not a valid Rust identifier. The maintainer gets a compile error inside `OUT_DIR/driver_packages.rs` instead of the named refusal every other bad input produces. (b) `[capabilities] advertised` is not validated at all, while every `[package]` field is checked for emptiness at line 165: `advertised = []` silently generates `pub const X_CAPABILITIES: &[&str] = &[];` (a dialect advertising nothing), and `advertised = [""]` emits a blank capability name into the `getCapabilities` response. `driver_toml_test.rs` has no case for either.
- Fix: In crates/adbc-vs-scan/build.rs, require the first character of the file stem to be `is_ascii_lowercase()` in the driver-name check at lines 138-141, and add a check in `read_declaration` that `declaration.capabilities.advertised` is non-empty and that no entry is empty or whitespace, both failing through `fail(...)` with a message naming the file and the offending value. Add three cases to crates/adbc-vs-scan/tests/driver_toml_test.rs — a leading-digit file name, an empty `advertised` list, and an `advertised` entry that is the empty string — each asserting the refusal message names the file.

#### [DUPLICATE_OPERATION] `rerun-if-changed` is emitted twice for every declaration file
- Location: lines 66-79
- Issue: Lines 66-69 already emit `cargo::rerun-if-changed=<drivers directory>`; cargo watches the whole directory, so a newly added `.toml` triggers a rebuild. Lines 71-79 then loop over `declarations.keys()` emitting a second `rerun-if-changed` per file, reconstructing each path as `declarations_directory.join(format!("{name}.toml"))` rather than reusing the `DirEntry` path already read.
- Fix: Delete lines 71-79 from crates/adbc-vs-scan/build.rs.

### crates/adbc-vs-adapter/tests/make_targets_test.rs

#### [MISSING_BOUNDARY_TEST] The staleness assertion never exercises `postgresql.toml`
- Location: lines 274-277
- Issue: The test's input array is `["crates/adbc-vs-scan/build.rs", "crates/adbc-vs-scan/drivers/mysql.toml"]`, so touching `postgresql.toml` is never asserted to force a rebuild of `$(SO_FILE)`. It passes today only because `Makefile` line 43 globs the directory; a future edit replacing the glob with an explicit prerequisite list naming only `mysql.toml` would still pass this test.
- Fix: In crates/adbc-vs-adapter/tests/make_targets_test.rs, add `"crates/adbc-vs-scan/drivers/postgresql.toml"` to the input array at lines 274-277.

### .github/workflows/ci.yml

#### [TACTICAL_SHORTCUT] The `e2e` disk cleanup was not adjusted for the third container image
- Location: lines 68-71
- Issue: The free-disk step is unchanged (`sudo rm -rf /usr/share/dotnet /usr/local/lib/android /opt/ghc /opt/hostedtoolcache/CodeQL` plus `docker image prune -af`) while the job now also pulls `postgres:18.6-trixie` on top of `exasol/docker-db:2025.2.1`, `mysql:8.4`, and `rust:1.94-trixie`. plan.md § Impact states "the CI `e2e` job's disk cleanup carries a PostgreSQL image on top of Exasol and MySQL", and tasks.md task 9.2 named a "disk cleanup check" — neither produced a change. CLAUDE.md line 103 records that a GitHub-hosted runner has roughly 21 GB free and that "Rust + Docker + Exasol exhausts it".
- Fix: In .github/workflows/ci.yml, extend the `Free runner disk space` step's `rm -rf` list with `/opt/hostedtoolcache` and `/usr/local/.ghcup`, and update CLAUDE.md's `e2e` MUST-free-disk bullet to name the same list in the same commit.

### CLAUDE.md

#### [OUTDATED_COMMENT] Six bullets no longer match the config they summarize
- Location: lines 44-76 (test tiers), 58, 64, 75-76, 98-99 (CI jobs), 111 (licensing scope)
- Issue: CLAUDE.md line 10 requires the matching bullet be updated in the same commit as the config it describes, and six are now stale. (a) `make test-e2e-postgres` (`Makefile` lines 111-113) appears nowhere in the file, though line 70 already speaks of "both E2E binaries". (b) Line 98's `` `integration-tests` → `make test-it` `` omits `make test-it-postgres` (`ci.yml` lines 61-62). (c) Line 99's `` `e2e` → `make test-e2e` `` omits `make test-e2e-postgres` (`ci.yml` lines 80-81). (d) Line 58's default-off feature list omits `driver-load-probe`. (e) Line 64 says the shared setup module "creates the one virtual schema every test shares" while `e2e_postgres_test.rs` line 24 states "two virtual schemas". (f) Lines 75-76 and line 111 still name MySQL alone ("the generated **MySQL** statement", "mirror the **MySQL** fixture", "**The MySQL ADBC driver** fetched at run time"), though a PostgreSQL mirror (`tests/common/postgres_mirror.rs`) and a second run-time-fetched driver (`drivers/postgresql.toml`) now exist.
- Fix: In CLAUDE.md: add a `make test-e2e-postgres` bullet beside line 63 stating its prereqs `cross-udf-build` + `stack-up` and that it runs `e2e_postgres_test`; append `, then make test-it-postgres` to line 98 and `, then make test-e2e-postgres` to line 99 (or the two new job names, if the CI-job split in this document's Expert section is applied first); reword line 64 to say each E2E binary's shared setup creates the virtual schemas its own tests share; make lines 75-76 source-agnostic ("the generated source statement", "mirror the source fixture"); and pluralize line 111 to "the ADBC drivers (`mysql`, `postgresql`) fetched at run time". Drop the `driver-load-probe` half of (d) by deleting the feature per this document's `[DEAD_FLEXIBILITY]` finding. Also update `deny.toml` line 5's comment to name both drivers.

### crates/adbc-vs-adapter/src/create_schema.rs

#### [OUTDATED_COMMENT] Four doc comments in driver-generic modules still name MySQL as the only source
- Location: crates/adbc-vs-adapter/src/create_schema.rs lines 46 and 458-461; crates/adbc-vs-adapter/src/adapter_notes.rs lines 8-11; crates/adbc-vs-adapter/src/sql_dialect.rs lines 14-16; crates/adbc-vs-adapter/src/type_mapping.rs lines 25-27
- Issue: (a) `CONNECTION_PROPERTY`'s doc reads "The property naming the Exasol `CONNECTION` object that addresses MySQL"; it now addresses whichever source the `DRIVER` property named. (b) `parse_database`'s doc reads "The database name is the path component of a MySQL `CONNECTION` address (`mysql://host:port/database`) — the live spike found ...", but the function runs for every driver, and "the live spike found" is a work-history reference the guardrails exclude from doc comments. (c) `adapter_notes.rs`'s module doc says notes never carry "the MySQL user and password, and the BucketFS write credentials", omitting PostgreSQL's. (d) `sql_dialect.rs` says "The trait exists for the second driver rather than for this one", written when one implementation existed; there are now two. (e) `MAX_VARCHAR_SIZE`'s doc says it is "read by every source's mapping", but only `postgres_type_mapping.rs` line 161 checks it — MySQL's `character_size` (`type_mapping.rs` lines 204-215) never does.
- Fix: In crates/adbc-vs-adapter/src/create_schema.rs, reword line 46 to "The property naming the Exasol `CONNECTION` object that addresses the source", and rewrite `parse_database`'s doc (lines 458-461) to state that the database name is the path component of the source `CONNECTION` address (`<driver>://host:port/database`), because the address must already be a full driver URI, dropping the "live spike" clause. In crates/adbc-vs-adapter/src/adapter_notes.rs, reword lines 8-11 to "the source user and password, and the BucketFS write credentials". In crates/adbc-vs-adapter/src/sql_dialect.rs, rewrite lines 14-16 to state that the trait has one implementation per technology and that mission.md's remaining v1 drivers each add one. In crates/adbc-vs-adapter/src/type_mapping.rs, either add the `MAX_VARCHAR_SIZE` check to `character_size` with a unit test for a MySQL length above it, or reword the constant's doc to say which mappings read it.
