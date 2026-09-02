# Plan: add-default-bucketfs-connection

> **Status:** blocked — see open-questions.md

## Summary

Make `BUCKETFS_CONNECTION` optional on every route: a statement that names none is answered by the reserved connection `ADBC_VS_BUCKETFS`, and a cluster where that connection does not resolve either falls to a version-independent default driver location an operator may have pre-placed a library at. Explicit `BUCKETFS_CONNECTION` and `DRIVER_PATH` keep working exactly as they do today, and neither ever falls back.

## Design

### Context

`createVirtualSchema` requires `BUCKETFS_CONNECTION` whenever `DRIVER_PATH` is absent, so every virtual schema on a cluster restates the same BucketFS write credential. Issue #4 asks for a conventional default plus a soft landing when neither route is configured.

Two forces shape the answer, and they pull against each other.

The first is release 0.3's own record. `driver-provisioning/driver-path-override` was introduced precisely to *remove* an informal "place the library at a fixed compiled-in path" fallback, and ADR 003 rejected keeping it on two grounds: the path was not a supported input, and it stopped being fixed once the install path carried a resolved version. Issue #4's point 3 asks for that fallback back. It is grantable only if both grounds are answered — the location must be a stated interface, and it must be version-independent by construction.

The second is why the install path carries a version at all. `provisioning.rs` states it: an unversioned path "would have made it answer for whichever version arrived first, so a range would resolve once per cluster and never again." Any location the presence check consults *before* the index read reintroduces exactly that defect.

Both are answered by keeping the two locations disjoint and consulting the new one last. Self-install keeps its versioned tree and its index-first presence check, untouched. The pre-placed location is a separate, version-free path reached only when no write credential exists at all, which is the one configuration in which self-install cannot run whatever it would have resolved.

- **Goals** — a `CREATE VIRTUAL SCHEMA` that needs no `BUCKETFS_CONNECTION` on the common cluster; a stated, version-independent location an air-gapped operator can pre-place a library at without setting `DRIVER_PATH`; both explicit properties unchanged, including their refusal to fall back.
- **Non-Goals** — no change to version resolution, to the presence check, or to what self-install downloads and writes. No new property. No configurable default-connection name and no configurable mount: both are conventions, and a property for either would be a decision the adapter declined to make. `driver-provisioning/driver-version-resolution` is deliberately untouched, including its unreachable-registry error.

### Decision

One precedence chain, four levels, each reached only by the absence of the one above it. The last two levels are new.

#### Architecture

```
CREATE VIRTUAL SCHEMA / refresh / setProperties
        │
        ▼
create_schema::driver_source(&properties)          ← pure, no I/O, unchanged shape
        │
        ├── DRIVER_PATH set ─────────────▶ Supplied(path)
        ├── BUCKETFS_CONNECTION set ─────▶ SelfInstall(name)
        └── neither ─────────────────────▶ DefaultBucketFs          (NEW)
        │
        ▼
create_schema::open_driver_library(ctx, source, driver)
        │
   Supplied ─────────▶ provisioning::supplied_driver(path)          exact path, readable only
   SelfInstall(name) ─▶ resolve_connection(name) ──┐                explicit: never falls back
   DefaultBucketFs ───▶ resolve_connection("ADBC_VS_BUCKETFS")
                         │  Ok  ─────────────────┤
                         │  Err(cause) ──────────┼──▶ default_driver_path(mount, driver)  (NEW)
                         │                       │       readable ? use it : NoDriverRoute
                         ▼                       ▼
                                    provisioning::ensure_driver(registry, bucket)
                                    index read ▸ resolve version ▸ presence check
                                    ▸ download ▸ PUT ▸ visibility wait

BucketFS mount, two disjoint trees:
  adbc-drivers/<driver>/<version>/<library>   adapter-owned, written by self-install
  adbc-drivers/<driver>/supplied/<library>    operator-owned, never written by the adapter   (NEW)
```

#### Patterns

| Pattern | Where | Why |
|---------|-------|-----|
| Route selection stays a pure function of properties | `create_schema::driver_source` | It runs before any `CONNECTION` is resolved, which is what keeps a statement that could never provision refusable without reaching outside the process. The default-connection probe is I/O, so it lives in `open_driver_library` where `ctx` already is |
| Sentinel path segment rather than a version | `adbc-drivers/<driver>/supplied/<library>` | The install path's third segment is always a parsed `semver::Version`, and no version parses as `supplied`, so the operator-owned and adapter-owned trees are provably disjoint rather than disjoint by convention |
| One definition of "readable driver library" | `provisioning::readable_library`, called by both routes | Two callers deciding independently what counts as readable is back-door leakage; each route phrases its own error over one shared answer |
| Fall through on resolution failure, carry the cause | `open_driver_library`'s `DefaultBucketFs` arm | `UdfError` reports every resolution failure as one opaque string, so "no such connection" cannot be told from "no access". Carrying the cause into the last-resort error keeps a permission misconfiguration diagnosable instead of silently reported as a missing file |
| Explicit values never fall back | `SelfInstall`, `Supplied` | Already the `DRIVER_PATH` rule; extending it to an explicit `BUCKETFS_CONNECTION` keeps one rule rather than two |

#### Key Interfaces

| Interface | Owner | Contract |
|-----------|-------|----------|
| `pub const DEFAULT_BUCKETFS_CONNECTION: &str` | `create_schema` | The reserved `CONNECTION` name. Lives beside `BUCKETFS_CONNECTION_PROPERTY` because `create_schema` owns every connection-name decision; `provisioning` is handed credentials and resolves nothing |
| `pub fn default_driver_path(mount: &Path, driver: Driver) -> PathBuf` | `provisioning` | The operator-owned location's layout. Takes the mount as an argument, exactly as `Bucket::new` does, so `provisioning` owns the layout and `create_schema` owns which mount is real |
| `pub fn readable_library(path: &Path) -> Result<(), String>` | `provisioning` | Whether a path is a driver library the adapter will use. `Err` carries the cause each route's own error message quotes |
| `SchemaError::NoDriverRoute { driver, path, connection, connection_cause, cause }` | `create_schema` | The error replacing today's missing-property error. Names the default location, the driver, `ADBC_VS_BUCKETFS` and why it did not resolve, and both properties |

Reviewed against `/speq:design-philosophy`'s Quick Diagnostic. No new module and no new boundary: three functions and one error variant join modules that already own the decisions behind them. `default_driver_path` sits beside `driver_install_path`, so BucketFS layout keeps its single owner; `DEFAULT_BUCKETFS_CONNECTION` sits beside the property constants, so connection naming keeps its single owner. `readable_library` exists to stop a second owner appearing for a decision `supplied_driver` already made. Dependencies stay inward: `provisioning` gains no knowledge of properties, connections, or the UDF context, and `create_schema` gains no knowledge of the path layout beyond calling for it. `driver_source` stays pure, so the module boundary between "which route the properties chose" and "what that route does" is unmoved.

### Consequences

| Decision | Alternatives Considered | Rationale |
|----------|------------------------|-----------|
| Reserve `ADBC_VS_BUCKETFS` as the default connection name | `ADBC_VS_BUCKETFS_CONNECTION`; a new `DEFAULT_BUCKETFS_CONNECTION` property naming it | The README, both E2E harnesses, and `tests/common/schema.rs` already use exactly this name, so the convention is the one already documented and a cluster that followed the README needs no change at all. A `..._CONNECTION` suffix restates the object's kind; a property naming the default is a default that is not one |
| Give the pre-placed route its own version-free location, `adbc-drivers/<driver>/supplied/<library>` | Reuse the versioned install path; add a `default` segment; put it under `adbc-drivers/supplied/<driver>/` | The versioned path cannot be pre-placed without resolving a version, which is what an air-gapped cluster cannot do — ADR 003's second ground for rejecting the old fallback. `default` reads as a version and the mount already ends `bfsdefault/default`. `supplied` names the owner, matches `supplied_driver`, and can never be mistaken for a version |
| Consult the default location last, never before the index read | Check it first as a cheap short-circuit; check it when the registry is unreachable | Checking first restores the "answers for whichever version arrived first" defect the versioned path removed. Falling back on an unreachable registry is a silent fallback, which `driver-path-override` forbids for exactly this reason |
| Fall through when `ADBC_VS_BUCKETFS` fails to resolve; never when an explicit property does | Fail on any resolution failure; distinguish not-found from no-access | `UdfError` exposes no structured variant, so the two cannot be told apart. Falling through is safe: Exasol enforces `CONNECTION` access at resolution, and the source `CONNECTION` is resolved first, so a broken connect-back has already failed the statement before the default is reached |
| Extend `driver-provisioning/driver-path-override` rather than add a fourth feature | A new `driver-preplaced-default` feature | Both routes reach the same three obligations — exact file, readability only, no fallback — and differ only in where the path came from. Two features would have to be kept in agreement about a decision neither owns alone |
| Leave `driver-provisioning/driver-version-resolution` untouched | Extend its unreachable-registry error to name the default location | Reaching the default location requires a cluster with no BucketFS connection at all, not a change one statement can make. Naming it there would be advice an operator cannot act on, and `DRIVER_PATH` remains the one-statement remedy that error already names |

## Features

| Feature | Status | Spec |
|---------|--------|------|
| `vs-adapter/create-virtual-schema` | CHANGED | `specs/_plans/add-default-bucketfs-connection/vs-adapter/create-virtual-schema/spec.md` |
| `driver-provisioning/driver-path-override` | CHANGED | `specs/_plans/add-default-bucketfs-connection/driver-provisioning/driver-path-override/spec.md` |
| `driver-provisioning/driver-self-install` | CHANGED | `specs/_plans/add-default-bucketfs-connection/driver-provisioning/driver-self-install/spec.md` |
| `driver-provisioning/driver-version-resolution` | UNCHANGED | `specs/driver-provisioning/driver-version-resolution/spec.md` |
| `scan-udf/arrow-scan` | UNCHANGED | `specs/scan-udf/arrow-scan/spec.md` |

Two merge notes for `/speq:record`.

`driver-path-override`'s Feature-description line changes, and the delta format carries no marker for it. Apply it with the Background block: the new line is the first line of the delta file.

`create-virtual-schema` reaches 15 scenarios, which trips `/speq:spec-merge`'s advisory >10 threshold. `CLAUDE.md` § Spec governance already records that `vs-adapter`'s shape is accepted and MUST NOT be re-consolidated, and decisions `[25]` and `[26]` of `add-postgresql-support` are the ADRs behind that. Answer the signal with that record rather than by splitting the feature. `driver-self-install` reaches exactly 10 and trips nothing; `driver-provisioning` keeps three features.

## Impact

`BUCKETFS_CONNECTION` becomes optional. A `CREATE VIRTUAL SCHEMA` on a cluster whose BucketFS connection is named `ADBC_VS_BUCKETFS` — the name the README has always given it — needs `DRIVER` and `CONNECTION_NAME` only. Every statement that works today keeps working unchanged, including both explicit routes and their refusal to fall back.

No breaking change. One behavior change: a statement omitting both `BUCKETFS_CONNECTION` and `DRIVER_PATH` used to fail naming the missing property, and now attempts the reserved connection and then the default driver location. It still fails when neither is there, with an error naming the location it looked at, the connection it tried, why that connection did not resolve, and both properties.

Two consequences worth stating for an operator. A session that can create a virtual schema and can resolve `ADBC_VS_BUCKETFS` can now trigger a driver download and a BucketFS write without naming that connection; Exasol still enforces access to the connection object, so this grants no credential the session did not already hold. And `adbc-drivers/<driver>/supplied/` becomes a reserved location on the BucketFS mount: adbc-vs reads it, never writes it, and a file placed there is loaded only when no BucketFS connection resolves.

## Dependencies

None. No new crate, no new Make target, no new CI job, no new container.

## Migration

| Current | New |
|---------|-----|
| `BUCKETFS_CONNECTION = 'ADBC_VS_BUCKETFS'` in every statement | Optional; omit it and the reserved connection answers |
| `BUCKETFS_CONNECTION` naming another connection | Unchanged — the explicit property still wins and still never falls back |
| Air gap: `DRIVER_PATH = '<any path>'` | Unchanged, and now also: place the library at `/buckets/bfsdefault/default/adbc-drivers/<driver>/supplied/<library>` and set neither property |
| Omitting both properties fails naming `BUCKETFS_CONNECTION` | Resolves `ADBC_VS_BUCKETFS`, then the default driver location; fails naming both properties, the connection, and the location |

## Implementation Tasks

- [ ] 1.1 Prove that a `CONNECTION` the adapter cannot resolve surfaces as the adapter's own error rather than killing the statement. The whole last-resort route rests on this, no test in this repository asserts it, and the implementation lives in the script language container's host bridge rather than in `exasol-udf-sdk` — `UdfContext::connection`'s default body only returns `Unimplemented`.
  - Add `create_virtual_schema_through_expecting_failure(exasol, spec) -> String` to `crates/adbc-vs-adapter/tests/common/schema.rs`, beside `create_virtual_schema_expecting_failure`, which hardcodes `bucketfs_connection: Some(BUCKETFS_CONNECTION)` and so cannot express the statement under test.
  - Add `an_explicit_bucketfs_connection_that_does_not_resolve_fails_naming_it` to `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs`: create a schema with `bucketfs_connection: Some("ADBC_VS_NO_SUCH_CONNECTION")`, `driver_path: None`, against a stack where `ADBC_VS_BUCKETFS` exists and the driver is already installed.
  - The surfaced SQL error MUST contain `ADBC_VS_NO_SUCH_CONNECTION`, MUST NOT contain `ADBC_VS_BUCKETFS`, and MUST NOT report a created schema. That absence is the "explicit properties never fall back" half of the precedence scenario, so this test is coverage rather than a throwaway probe.
  - Decision rule: if the error carries the adapter's own `resolving CONNECTION ...` text, the probe is recoverable and tasks 2.2 and 3.3 proceed. If instead the statement dies with an Exasol-level failure carrying no adapter text — SQL state 22002, a VM crash, or a session abort — STOP and report. Points 1, 2 and 4 of the issue stay implementable without it; point 3 does not, and no workaround may be improvised in its place.
  - Requires `make cross-udf-build` and `make stack-up`.

- [ ] 2.1 Default the BucketFS write credential to the reserved connection `ADBC_VS_BUCKETFS`, in `crates/adbc-vs-adapter/src/create_schema.rs`.
  - Add `pub const DEFAULT_BUCKETFS_CONNECTION: &str = "ADBC_VS_BUCKETFS";` beside `BUCKETFS_CONNECTION_PROPERTY`. Its doc comment states why the name is reserved rather than configurable, and why it lives here rather than in `provisioning`: this module owns every connection-name decision, and `provisioning` is handed credentials and resolves nothing.
  - Add a third `DriverSource` variant, `DefaultBucketFs`, carrying nothing. Rewrite `driver_source` to return it when neither property is set. `driver_source` stays pure — no `ctx` parameter — because it runs before any `CONNECTION` is resolved and that ordering is what keeps an unprovisionable statement refusable without reaching outside the process.
  - Replace the `required_property(properties, BUCKETFS_CONNECTION_PROPERTY)` call with `optional_property`. Rewrite `driver_source`'s doc comment: `DRIVER_PATH` no longer decides the route alone.
  - Handle `DefaultBucketFs` in `open_driver_library` by resolving `DEFAULT_BUCKETFS_CONNECTION` and self-installing through it, sharing one code path with `SelfInstall` so the two cannot drift. On resolution failure, return today's `SchemaError::MissingProperty(BUCKETFS_CONNECTION_PROPERTY)` for now; task 2.2 replaces it. The tree stays green at each step, and the delta's "MUST NOT produce a missing-property error" clause is met only after 2.2.
  - Red first, in `crates/adbc-vs-adapter/src/create_schema_tests.rs`: delete `bucketfs_connection_required_without_driver_path`, which asserts the behavior this task removes, and add `a_statement_naming_no_bucketfs_connection_takes_the_default_route` asserting `driver_source` returns `DefaultBucketFs`. Keep `a_statement_without_driver_path_self_installs_through_its_bucketfs_connection` and both `DRIVER_PATH`-precedence tests unchanged — they assert the levels above the new one.
  - In `crates/adbc-vs-adapter/tests/common/schema.rs`, replace the literal `pub const BUCKETFS_CONNECTION: &str = "ADBC_VS_BUCKETFS";` with a re-export of `adbc_vs_adapter::create_schema::DEFAULT_BUCKETFS_CONNECTION`, so the harness cannot create a connection under a name the product no longer reserves. Update the module doc comment's property list, which still calls `BUCKETFS_CONNECTION` one of the adapter's required properties.

- [ ] 2.2 Add the default driver location and the last-resort route. [expert]
  - In `crates/adbc-vs-adapter/src/provisioning.rs`: add `const SUPPLIED_DIRECTORY: &str = "supplied";` and `pub fn default_driver_path(mount: &Path, driver: Driver) -> PathBuf` returning `<mount>/adbc-drivers/<driver>/supplied/<library>`. Its doc comment states the disjointness argument — the install path's third segment is always a parsed `semver::Version` and no version parses as `supplied` — because that is the property the whole design rests on, not an incidental one.
  - Extract `pub fn readable_library(path: &Path) -> Result<(), String>` from `supplied_driver`'s body and have `supplied_driver` call it, so both routes share one definition of readable and each phrases its own error over the shared cause. Do NOT add a parameter to `supplied_driver` selecting which error to build: the two messages name different properties and different remedies, and a route-selecting parameter would be a decision the function declined to make.
  - In `create_schema.rs`: add `SchemaError::NoDriverRoute { driver, path, connection, connection_cause, cause }` and its `Display` arm. Replace 2.1's placeholder so the `DefaultBucketFs` arm, on resolution failure, computes `provisioning::default_driver_path(Path::new(BUCKETFS_MOUNT), driver)`, returns `DriverLibrary::supplied(path)` when `readable_library` accepts it, and `NoDriverRoute` otherwise.
  - Red first, in `crates/adbc-vs-adapter/src/provisioning_tests.rs`, beside `the_install_path_carries_the_driver_and_the_resolved_version`:
    - `the_default_location_carries_the_driver_and_no_version` — the exact path, for MySQL.
    - `the_default_location_is_distinct_per_driver` — over `Driver::ALL`, pairwise distinct paths.
    - `no_default_location_can_be_an_install_path` — for every `Driver::ALL` member, `default_driver_path` equals no `driver_install_path` over a set of versions, and `SUPPLIED_DIRECTORY` fails `semver::Version::parse`. Assert the parse failure, not just the inequality: the inequality holds for any sample of versions, and only the parse failure holds for all of them.
  - Red first, in `create_schema_tests.rs`: `no_driver_route_names_the_location_the_connection_and_both_properties` — the `NoDriverRoute` message contains the default path, the driver, `ADBC_VS_BUCKETFS`, the resolution cause it was given, `BUCKETFS_CONNECTION`, and `DRIVER_PATH`.
  - Depends on task 1.1's decision rule.

- [ ] 2.3 Prove at the provisioning tier that self-install ignores and preserves a pre-placed library. Add `self_install_writes_only_the_versioned_path_and_leaves_a_supplied_library_alone` to `crates/adbc-vs-adapter/tests/provisioning_test.rs`.
  - Write known bytes at `MountDirectory`'s `adbc-drivers/<driver>/supplied/<library>` before calling `ensure_driver` against the stub registry and the write-accepting BucketFS stub. Add a `supplied_path_for(driver)` helper to `MountDirectory` beside `driver_path_for`; do not spell the segment a second time in the test body.
  - Assert: the returned path is the versioned install path; the supplied file's bytes are byte-for-byte what the test wrote; and the stub recorded exactly one `PUT`, at the versioned URL. The last assertion is what separates "did not overwrite" from "overwrote with identical bytes".
  - Assert the same for the PostgreSQL driver through the existing two-driver setup, so `driver-self-install`'s per-driver clause is covered without a second E2E run.
  - This test covers `driver-path-override`'s no-short-circuit scenario at the provisioning level: `ensure_driver` still reads the index and installs, with a library sitting at the default location the whole time.

- [ ] 3.1 Prove end to end that a statement naming no `BUCKETFS_CONNECTION` self-installs through the reserved connection. Add `default_bucketfs_connection_needs_no_property` to `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs`.
  - Create `DEFAULTVS` with `bucketfs_connection: None` and `driver_path: None` against the shared stack, which already holds `ADBC_VS_BUCKETFS`.
  - `SELECT COUNT(*)` over one of its tables MUST return the shared schema's own count, which proves the schema reached the source rather than merely being created.
  - Its `adapterNotes["driverPath"]` MUST equal the shared schema's, and `driverVersion` MUST be present. Both together are what separate self-install through the default connection from the last-resort route, which records a different path and no version.
  - Drop the schema before asserting, following `first_use_installs_driver_from_registry`'s compute-then-assert order.

- [ ] 3.2 Prove end to end that a pre-placed library does not short-circuit self-install when a BucketFS connection resolves. Add `a_supplied_library_does_not_short_circuit_self_install` to `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs`, and `Exasol::copy_bucket_object(from, to)` to `crates/adbc-vs-adapter/tests/common/exasol.rs`.
  - `copy_bucket_object` downloads with `curl --fail --silent --show-error --insecure --user <credentials> --output <temp file>` from the same `https://{HOST}:{BUCKETFS_PORT}/{BUCKET}/{from}` URL shape `put_file` writes to, then calls `put_library(temp, to)` so the visibility wait is part of the copy rather than a second call a caller might forget. Its doc comment states that the 44 MB body goes through a temp file rather than memory.
  - Copy the installed driver to `adbc-drivers/mysql/supplied/libadbc_driver_mysql.so`, create a schema with neither property, read its notes, then delete the copied object and drop the schema before asserting.
  - `adapterNotes["driverPath"]` MUST be the versioned install path and MUST NOT be the supplied path; `driverVersion` MUST be present. Leave `ADBC_VS_BUCKETFS` in place throughout — this test needs the connection present, which is what makes it safe to run beside every other test in the binary.

- [ ] 3.3 Prove end to end that the default driver location answers when no BucketFS connection resolves, and that an empty one fails naming every route. Add `a_preplaced_library_answers_when_no_bucketfs_connection_resolves` to `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs`. [expert]
  - This is the only test in the binary that removes an object the shared setup created, so it must put the stack back before it asserts anything, exactly as `first_use_installs_driver_from_registry` does. Nothing here may depend on the order libtest happens to run these in.
  - Sequence, with every assertion held until the end: drop `ADBC_VS_BUCKETFS`; create a schema with neither property through `create_virtual_schema_through_expecting_failure` and capture the message; copy the installed driver to `adbc-drivers/mysql/supplied/libadbc_driver_mysql.so`; create the schema again and capture its row count and notes; drop the schema; delete the copied object; recreate `ADBC_VS_BUCKETFS` through `schema::create_bucketfs_connection`; then assert.
  - The captured failure message MUST contain the default location path, `ADBC_VS_BUCKETFS`, `BUCKETFS_CONNECTION`, and `DRIVER_PATH`.
  - The second creation MUST have succeeded, its row count MUST match the shared schema's, its `adapterNotes["driverPath"]` MUST be the default location path, and `driverVersion` MUST be absent — the same two-field signature `driver_path_is_accepted_and_bypasses_provisioning` reads for `DRIVER_PATH`.
  - Depends on task 1.1's decision rule: without a recoverable resolution failure this test cannot pass, and the failure is a finding rather than a bug to work around.

- [ ] 4.1 Update the two documents that state the properties.
  - `README.md`: drop `BUCKETFS_CONNECTION` from the quick-start statement, and rewrite the paragraph under it to state the precedence in one sentence each — the reserved `ADBC_VS_BUCKETFS`, the explicit property that overrides it, and the pre-placed `adbc-drivers/<driver>/supplied/<library>` location for a cluster with neither. Amend the "Self-installing drivers" bullet, which currently sends air-gapped clusters to `exapump` with no path to place the library at.
  - `CLAUDE.md` § Driver provisioning: add the reserved connection name and the default location as their own bullets, and amend the `DRIVER_PATH` bullet, which currently states that `BUCKETFS_CONNECTION` is optional only when `DRIVER_PATH` is present. State that `adbc-drivers/<driver>/supplied/` is read and never written by the adapter.

## Parallelization

| Parallel Group | Tasks |
|----------------|-------|
| Group A | 1.1 — alone, gates the rest |
| Group B | 2.1, then 2.2, then 2.3 — sequential, one module pair |
| Group C | 3.1, then 3.2, then 3.3 — sequential, one test binary |
| Group D | 4.1 |

Sequential dependencies:

- Group A → Group B — task 1.1 decides whether task 2.2 is implementable at all, and both 1.1 and 2.1 edit `tests/common/schema.rs`.
- Group B → Group C — every end-to-end test needs a `.so` carrying the new routes, so `make cross-udf-build` must run after 2.2.
- Group C → Group D — 4.1 documents behavior the tests prove.

Nothing overlaps. Groups B and C both run `cargo test` against one workspace and contend on the `target/` lock, and tasks 3.1 to 3.3 all edit one test binary that runs with `--test-threads=1`.

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| Call site | `create_schema::driver_source`'s `required_property(properties, BUCKETFS_CONNECTION_PROPERTY)` | The property is optional on every route; `SchemaError::MissingProperty` stays, still raised for `DRIVER` and `CONNECTION_NAME` |
| Test | `create_schema_tests.rs::bucketfs_connection_required_without_driver_path` | Asserts the missing-property error this plan removes |
| Constant | `tests/common/schema.rs::BUCKETFS_CONNECTION`'s literal value | Becomes a re-export of `create_schema::DEFAULT_BUCKETFS_CONNECTION`, so the harness cannot name a connection the product does not reserve |
| Doc prose | `tests/common/schema.rs` module comment; `README.md` quick start; `CLAUDE.md` § Driver provisioning | Each states `BUCKETFS_CONNECTION` as required, or optional only under `DRIVER_PATH` |

Nothing else. `supplied_driver`, `ensure_driver`, `Bucket`, and every version-resolution path are unchanged.

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| `create-virtual-schema` — Missing required property fails with a named error, `DRIVER` and `CONNECTION_NAME` halves | Unit | `crates/adbc-vs-adapter/src/create_schema_tests.rs` | `a_statement_naming_no_driver_reports_the_missing_property`, `a_property_the_request_sets_to_null_is_removed` — exist, unchanged |
| `create-virtual-schema` — Missing required property fails with a named error, `BUCKETFS_CONNECTION` half | Unit | `crates/adbc-vs-adapter/src/create_schema_tests.rs` | `a_statement_naming_no_bucketfs_connection_takes_the_default_route` |
| `create-virtual-schema` — The BucketFS write credential is resolved by precedence, default level | Integration | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` | `default_bucketfs_connection_needs_no_property` |
| `create-virtual-schema` — The BucketFS write credential is resolved by precedence, explicit level and no-fallback clause | Integration | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` | `an_explicit_bucketfs_connection_that_does_not_resolve_fails_naming_it` |
| `create-virtual-schema` — The BucketFS write credential is resolved by precedence, `DRIVER_PATH` level | Integration | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `driver_path_is_accepted_and_bypasses_provisioning` — exists, unchanged; creates its schema with no `BUCKETFS_CONNECTION` at all |
| `create-virtual-schema` — The BucketFS write credential is resolved by precedence, property-reading half | Unit | `crates/adbc-vs-adapter/src/create_schema_tests.rs` | `a_statement_setting_driver_path_supplies_its_own_library`, `a_driver_path_supplies_the_library_even_beside_a_bucketfs_connection`, `a_statement_without_driver_path_self_installs_through_its_bucketfs_connection` — exist, unchanged |
| `driver-path-override` — The default driver location is used when no BucketFS connection resolves | Integration | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` | `a_preplaced_library_answers_when_no_bucketfs_connection_resolves` |
| `driver-path-override` — The default driver location is used when no BucketFS connection resolves, per-driver layout clause | Unit | `crates/adbc-vs-adapter/src/provisioning_tests.rs` | `the_default_location_carries_the_driver_and_no_version`, `the_default_location_is_distinct_per_driver` |
| `driver-path-override` — A library at the default location does not short-circuit self-install | Integration | `crates/adbc-vs-adapter/tests/provisioning_test.rs`, `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` | `self_install_writes_only_the_versioned_path_and_leaves_a_supplied_library_alone`, `a_supplied_library_does_not_short_circuit_self_install` |
| `driver-path-override` — A library at the default location does not short-circuit self-install, disjointness clause | Unit | `crates/adbc-vs-adapter/src/provisioning_tests.rs` | `no_default_location_can_be_an_install_path` |
| `driver-path-override` — No driver on any route fails naming every route | Integration | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` | `a_preplaced_library_answers_when_no_bucketfs_connection_resolves` — its first phase |
| `driver-path-override` — No driver on any route fails naming every route, message-composition clause | Unit | `crates/adbc-vs-adapter/src/create_schema_tests.rs` | `no_driver_route_names_the_location_the_connection_and_both_properties` |
| `driver-path-override` — DRIVER_PATH skips self-install entirely | Integration | `crates/adbc-vs-adapter/tests/provisioning_test.rs`, `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `the_driver_path_route_makes_no_registry_request`, `driver_path_is_accepted_and_bypasses_provisioning` — exist, unchanged |
| `driver-path-override` — DRIVER_PATH naming no readable file fails without falling back | Unit, Integration | `crates/adbc-vs-adapter/src/provisioning_tests.rs`, `crates/adbc-vs-adapter/tests/provisioning_test.rs` | `a_supplied_path_holding_no_file_names_the_property_the_path_and_the_driver`, `a_supplied_path_naming_a_directory_is_refused` — exist, unchanged |
| `driver-self-install` — Self-install writes only the versioned install path | Integration | `crates/adbc-vs-adapter/tests/provisioning_test.rs` | `self_install_writes_only_the_versioned_path_and_leaves_a_supplied_library_alone` |

Two scenarios are covered by one test each at two tiers rather than one, and neither pairing is redundant. The no-short-circuit scenario needs the provisioning tier to prove `ensure_driver` never consults the supplied path and the end-to-end tier to prove the route selection wires that through a real statement. The no-driver-anywhere scenario needs the unit tier for the message's composition, which an end-to-end assertion can only substring-match, and the end-to-end tier to prove the failure is the adapter's rather than Exasol's.

Every unchanged scenario of all three features keeps the tests it has. This plan removes no coverage; the one deleted test asserted behavior the plan deliberately removes.

### Manual Testing

Run `make cross-udf-build && make stack-up` first. `DSN` below is `exasol://sys:exasol@localhost:8563?validateservercertificate=0`, and `MOUNT` is `/buckets/bfsdefault/default`.

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| `create-virtual-schema` — default connection | `exapump sql --dsn "$DSN" "CREATE VIRTUAL SCHEMA DEFVS USING ADBC_VS.ADBC_ADAPTER WITH DRIVER = 'mysql' CONNECTION_NAME = 'ADBC_VS_MYSQL'"` then `exapump sql --dsn "$DSN" 'SELECT COUNT(*) FROM DEFVS."orders"'` | The schema is created with no `BUCKETFS_CONNECTION` property, and the count is `3` |
| `create-virtual-schema` — explicit override never falls back | `exapump sql --dsn "$DSN" "CREATE VIRTUAL SCHEMA BADVS USING ADBC_VS.ADBC_ADAPTER WITH DRIVER = 'mysql' CONNECTION_NAME = 'ADBC_VS_MYSQL' BUCKETFS_CONNECTION = 'ADBC_VS_NO_SUCH_CONNECTION'"` | Fails naming `ADBC_VS_NO_SUCH_CONNECTION`; the message names no other connection |
| `driver-path-override` — no route at all | `exapump sql --dsn "$DSN" 'DROP CONNECTION ADBC_VS_BUCKETFS'`, then the `DEFVS` statement above under a fresh name | Fails naming `$MOUNT/adbc-drivers/mysql/supplied/libadbc_driver_mysql.so`, `ADBC_VS_BUCKETFS`, `BUCKETFS_CONNECTION`, and `DRIVER_PATH` |
| `driver-path-override` — pre-placed library | With `ADBC_VS_BUCKETFS` still dropped, find the installed object with `exapump bucketfs ls`, then round-trip it through a local file — `exapump bucketfs cp <installed path> /tmp/libadbc_driver_mysql.so` and `exapump bucketfs cp /tmp/libadbc_driver_mysql.so <bucket>/adbc-drivers/mysql/supplied/libadbc_driver_mysql.so`, because `cp` auto-detects direction and has no BucketFS-to-BucketFS form — then repeat the `DEFVS` statement | The schema is created, the count is `3`, and `SELECT ADAPTER_NOTES FROM SYS.EXA_ALL_VIRTUAL_SCHEMAS WHERE SCHEMA_NAME = 'DEFVS'` records the `supplied` path with no `driverVersion`. Restore the connection afterwards |
| `driver-self-install` — the supplied file is never written | With `ADBC_VS_BUCKETFS` restored and the supplied copy still in place, delete the installed object with `exapump bucketfs rm <installed path>` and create another schema | The versioned path is written again, and `exapump bucketfs ls` reports the `supplied` object at its original size |

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Build | `make cross-udf-build` | Exit 0 |
| Format | `make fmt-check` | No changes |
| Lint | `make lint` | 0 warnings |
| Unit tests | `make test` | 0 failures, including the new `provisioning_test.rs` and `create_schema_tests.rs` cases |
| Artifact | `make test-artifact` | 0 failures |
| Integration, MySQL | `make test-it` | 0 failures |
| Integration, PostgreSQL | `make test-it-postgres` | 0 failures |
| E2E, MySQL | `make test-e2e` | 0 failures, including the four new tests |
| E2E, PostgreSQL | `make test-e2e-postgres` | 0 failures — its `DRIVER_PATH` test already creates a schema with no `BUCKETFS_CONNECTION` and must stay on the `DRIVER_PATH` route |
| Dependency policy | `make deny` | 0 findings |
