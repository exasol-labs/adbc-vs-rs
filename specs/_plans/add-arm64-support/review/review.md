# Plan Review Findings: add-arm64-support

## Summary
- Axes checked: 6/6
- Total findings: 14 (Blockers: 3, Findings: 11)
- Intent Fidelity blockers: 1

## Ways This Plan May Fail

- The new `exasol-personal` cargo feature removes code, and `make lint` turns on every feature. The CI `lint` job then stops checking the Docker backend and every self-install E2E test, and the `bench` plus `exasol-personal` combination does not compile. Routed to `[HIDDEN_DEPENDENCY]`.
- If the task 2.1 spike shows that a UDF inside Exasol Personal cannot reach the Compose static IPs, the predefined fallback (a) also fails, because every source port is published on `127.0.0.1` only. Routed to `[UNSTATED_ASSUMPTION]`.
- The issue's motivating user runs Exasol Personal Local on Apple silicon. That user cannot follow `docs/install.md` § 1 and § 2, which upload through a BucketFS HTTP endpoint that Personal local does not have. Routed to `[SCOPE_REDUCTION]`.

## Intent Fidelity

#### [SCOPE_REDUCTION] BLOCKER
- Location: plan.md § Implementation Tasks, task 4.1; plan.md § Impact
- Issue: Issue #13, which the user linked as the source, states the problem as: "Exasol Personal Local runs on macOS, and on Apple silicon it uses ARM." Its work item 6 is "Update `docs/install.md`". The plan names Personal users as an audience: "Exasol Personal users learn from `docs/install.md` that self-install cannot write to their deployment, and that they place the driver at the `supplied` location." Task 4.1 adds Personal guidance to § 4 only. `docs/install.md:31-33` (§ 1) tells the reader to upload the SLC to BucketFS as `slc/rustslc.tar.gz`, and `docs/install.md:44-48` (§ 2) uploads the library with `exapump bucketfs cp`. Exasol Personal local deployments publish no BucketFS HTTP service (exa-personal `user-docs/content/local-deployment.md:43`, `limitations.md:15-16`). The only aarch64 Exasol the plan names is Exasol Personal, so on aarch64 the documented install stops at § 1.
- Fix: Extend task 4.1 with Exasol Personal steps for § 1 and § 2. § 1: install the SLC with `exasol slc custom install --source lc-rust-0.23.1[-aarch64].tar.gz --alias RUST --language rust`, and warn that `exasol slc install rust` installs the latest language-container-rs release rather than the pinned 0.23.1. § 2: copy `udf/libadbc_vs_adapter.so` to `<deploymentDir>/local/runtime/exa/bucketfs/bfsdefault/default/udf/`. Keep the § 4 `supplied` copy step. Update the plan.md § Impact line to say that Personal users get a complete install path.

#### [SCOPE_CREEP] FINDINGS
- Location: specs/_plans/add-arm64-support/driver-provisioning/driver-version-resolution/spec.md § Background; plan.md § Features
- Issue: The `DELTA:CHANGED` Background holds 4 bullets. The recorded Background (`specs/driver-provisioning/driver-version-resolution/spec.md:7-18`) holds 12. Under deletion semantics, recording deletes 8 bullets. Only bullets 9 and 18 needed a change for this plan. The other deleted bullets are the `v`-prefix rule, the per-driver range rationale, the presence-check binding, and the route-scoping rationale at :14-17. plan.md never says that the Background is condensed, so the user and `recorder-agent` cannot see the deletion. The new bullets satisfy the 5-bullet Background limit, so the condensation itself is defensible.
- Fix: Keep the 4-bullet Background. Add one line to plan.md § Features or § Context: "The driver-version-resolution Background is condensed from 12 bullets to the 4 that its scenarios depend on, and the removed rationale is not restated elsewhere."

## Feasibility

#### [HIDDEN_DEPENDENCY] BLOCKER
- Location: plan.md § Implementation Tasks, tasks 2.2, 2.3, 2.6; decision-log.md [2]
- Issue: `make lint` runs `cargo clippy --workspace --all-targets --all-features -- -D warnings` (`Makefile:82`), and `make_targets_test.rs:303-308` pins that command. Task 2.2 adds `exasol-personal = []`, a feature that removes code: task 2.3 compiles the Docker backend only without it, and task 2.6 compiles the self-install E2E tests only without it. `--all-features` turns it on, which has two effects. First, the CI `lint` job never compiles the Docker backend of `tests/common/exasol.rs` or any test task 2.6 marks `#[cfg(not(feature = "exasol-personal"))]`. That is the main E2E path. Second, `--all-features` also enables `bench`. `tests/common/mod.rs` compiles `bench_stack.rs` into every test binary, and `bench_stack.rs:113,175,237` call `schema::create_bucketfs_connection`, which calls `bucketfs_write_url()` (`schema.rs:51-58`). Task 2.3 makes that method Docker-only, so `make lint` fails to compile. The plan addresses neither effect, and the Checklist requires `make lint` to report 0 warnings. The `lint` job also runs only on `ubuntu-latest`, so clippy never checks the `#[cfg(target_arch = "aarch64")]` branches from tasks 1.1 and 1.2.
- Fix: In task 2.2, change the `lint` target to run clippy twice. The first run uses an explicit feature list without the Personal feature (`--features built-artifact,exasol-e2e,policy-check,bench,registry-live`). The second run uses `--features exasol-e2e,exasol-personal`. Update `lint_fmt_test_targets_use_expected_flags` (`make_targets_test.rs:303`) to expect both commands. State what `bench` does under `exasol-personal`: either compile the bench modules out of `tests/common/mod.rs` or reject the combination with `compile_error!`. Add a task 4.3 bullet to AGENTS.md § Code quality saying that `make lint` covers both Exasol backends. Add `lint` to the task 3.3 runner matrix, or state in task 3.3 why the aarch64 `cfg` branches are left unlinted.

#### [UNSTATED_ASSUMPTION] BLOCKER
- Location: decision-log.md [3] fallback (a); plan.md task 2.5 bullet 3, task 4.3 bullet 6
- Issue: Fallback (a) reads: "the harness addresses each source by the host's primary IPv4 address and the source's published port under `EXASOL_TARGET=personal`". `docker-compose.yml` publishes every source on loopback only: `"127.0.0.1:3306:3306"` (:33), `"127.0.0.1:5432:5432"` (:54), and `"127.0.0.1:8080:8080"` and `"127.0.0.1:8443:8443"` (:79-80). Exasol Personal runs rootless Podman on its default network (`internal/localinstall/podman_install.go:349-357`). A connection from that container to the host's primary IPv4 reaches no listener, because nothing binds that address. The fallback therefore fails in exactly the case that triggers it. Making it work means publishing test databases with fixed passwords on a non-loopback interface. That is a security-relevant change the plan neither states nor asks the user about.
- Fix: Rewrite decision-log.md [3] fallback (a). Under `EXASOL_TARGET=personal` only, the Compose source ports bind to a make-controlled address, for example `${SOURCE_BIND_ADDRESS:-127.0.0.1}:3306:3306`, which `personal` sets to the host's primary IPv4. Name the consequence: the test databases and their fixed passwords are reachable on that interface. Require the user's confirmation before applying (a), as (b) already does. Add the `docker-compose.yml` port lines and the matching `make_targets_test.rs` expectation to task 2.5 bullet 3. In task 4.3 bullet 6, change the AGENTS.md exception text to name the bind address.

#### [UNSTATED_ASSUMPTION] FINDINGS
- Location: plan.md tasks 2.7 and 3.5
- Issue: The plan assumes that the existing E2E suite passes unchanged on `exasol/docker-db:latest-2026.1` and on Exasol Personal's `2026.2.0` nano image. Recorded specs state behavior measured on 2025.2.1 only. For example, `specs/vs-adapter/create-virtual-schema/spec.md:20` says that Exasol 2025.2.1 discards `schemaMetadata.properties`, and `specs/vs-adapter/pushdown-aggregation/spec.md:8,16,17,20` hold similar 2025.2.1 measurements. Task 3.5 has a stop rule only for an environmental `personal-arm64` failure. No task says what happens when a 2026 row fails because Exasol behaves differently.
- Fix: Add to task 2.7: "If a 2026.1 or Personal run fails for a product reason, meaning a pushdown, metadata, or protocol difference rather than the environment, stop, record the failing test and the measured behavior in the implementation summary, and ask the user before changing product code or specs."

#### [UNSTATED_ASSUMPTION] FINDINGS
- Location: plan.md task 2.4
- Issue: Task 2.4 installs the SLC on the Personal backend "only when `SCRIPT_LANGUAGES` has no `RUST` alias". Exasol Personal reserves the alias `rust`, and `exasol slc install rust` installs "the latest release of language-container-rs" under the alias `RUST` (exa-personal `user-docs/content/udfs.md:24-30`). A developer deployment that already holds such a container, or one left over from an older `SLC_VERSION`, keeps a `RUST` alias whose SDK does not match 0.23.1. Every UDF then fails to load. The Docker backend reinstalls on every run (`slc.rs:43-60`), so it does not have this defect.
- Fix: In task 2.4, check the installed `RUST` container's version (for example from `exasol slc list`) against `SLC_VERSION`. Skip the install only on a match, and run `exasol slc custom update --source <tarball> --alias RUST --auto-approve` when the versions differ.

## Requirement Quality

#### [REQUIREMENT_CONFLICT] FINDINGS
- Location: specs/driver-provisioning/driver-path-override/spec.md § Background, line 13 (no delta in this plan)
- Issue: After this plan merges, the recorded Background still lists "a resolved version with no `linux_amd64` package" as one of the registry-answered failures. On an aarch64 library, the missing package is `linux_arm64`. The plan removes every other `linux_amd64`-only statement from the specs and architecture. `notes/planning.md:15` records that the planner saw this line.
- Fix: Add `specs/_plans/add-arm64-support/driver-provisioning/driver-path-override/spec.md` with a `DELTA:CHANGED` `## Background` block. Copy all 12 recorded bullets and replace "a resolved version with no `linux_amd64` package" with "a resolved version with no package for the adapter library's platform". Include one recorded scenario unmarked so that `speq plan validate` accepts the file. Add the feature to plan.md § Features as CHANGED.

#### [COMPLETENESS_GAP] FINDINGS
- Location: specs/_plans/add-arm64-support/driver-provisioning/driver-self-install/spec.md § Scenario: Release artifact bundles no driver binary; plan.md § Verification, Scenario Coverage
- Issue: The changed WHEN step reads "the tracked files and each architecture's release artifact are inspected". The mapped test `no_bundled_driver_binary` (`tests/artifact_test.rs:93-141`) inspects `git ls-files` and `target/release/libadbc_vs_adapter.so`. It never reads `dist/adbc-vs*.tar.gz`. Task 3.3 builds the tarballs later, in the new `package` job, where no automated check lists their contents. Only the Manual Testing row runs `tar -tzf`.
- Fix: Choose one option. Either reword the WHEN step to "the tracked files and each architecture's built UDF object are inspected" and keep the tarball listing as a manual check. Or add a make target that lists the built tarball and asserts it holds only `udf/`, `udf/libadbc_vs_adapter.so`, and `THIRD-PARTY-LICENSES.md`, call it from the task 3.3 `package` job, and list it in § Scenario Coverage.

#### [REQUIREMENT_CONFLICT] FINDINGS
- Location: plan.md task 4.3 bullet 3; task 1.1
- Issue: The new AGENTS.md rule reads: "Test code takes the registry platform from `adbc_vs_scan::driver::PLATFORM` and never spells it as a literal for the running path." Task 1.1 requires the opposite for its own test: "the test spells as a literal per `#[cfg(target_arch)]`". The literal is deliberate, because it keeps the test from comparing `PLATFORM` with itself. A later contributor who applies the rule would still remove it.
- Fix: In task 4.3 bullet 3, add the exception: "except `self_install_downloads_the_package_of_the_compile_targets_platform`, which spells `linux_amd64` and `linux_arm64` per `#[cfg(target_arch)]` so that the mapping itself is under test."

#### [AMBIGUOUS_REQUIREMENT] FINDINGS
- Location: plan.md task 3.3 bullets 5 and 6; task 3.4 bullet 5
- Issue: The `package` job runs "after every test job", and `release` "needs `package`". Today `release` needs `lint` and `deny` directly (`ci.yml:161-173`). Whether `lint` and `deny` count as test jobs is not stated. If they do not, both jobs stop gating the release. Task 3.4 asserts only that `release` needs `package`.
- Fix: In task 3.3, list the `package` job's `needs` explicitly: `build-so`, `lint`, `unit-tests`, `deny`, `integration-tests`, `integration-tests-postgres`, `integration-tests-trino`, `e2e`, `e2e-postgres`, and `e2e-trino`. In task 3.4, assert that `package` needs each of them.

## Task Breakdown

#### [EFFORT_MISESTIMATION] FINDINGS
- Location: plan.md task 2.5; § Parallelization, Group B Knowledge
- Issue: Task 2.5 changes only `tests/common/stack.rs` and `schema.rs`. The PostgreSQL and Trino E2E binaries have their own setup functions. `postgres_stack.rs:74-102` and `trino_stack.rs:77-113` each call `schema::create_bucketfs_connection` and create two virtual schemas: PostgreSQL plus MySQL, and Trino plus MySQL. `bench_stack.rs:113,175,237` makes the same call. Under the Personal backend, none of these files compiles, and the two binaries place no PostgreSQL or Trino driver at `supplied`. Two of the three tiers the interview requires therefore fail on Personal. Task 2.5 also says only "Resolve the package with `registry_index::resolve_package`". That call returns a URL. Downloading the package, reading its `MANIFEST`, extracting the library, and caching it are unplanned work. The Group B Knowledge list omits all three stack files.
- Fix: Rewrite task 2.5 to name `stack.rs`, `postgres_stack.rs`, `trino_stack.rs`, and `bench_stack.rs`. State which drivers each binary places at `supplied`: MySQL; PostgreSQL and MySQL; Trino and MySQL. Name the download-and-extract mechanism, as the Design Depth finding below specifies. Add the three stack files and `crates/adbc-vs-scan/tests/common/driver_fetch.rs` to the Group B Knowledge cell.

No objection on traceability: each spec delta has a task (driver-version-resolution: 1.1-1.3; driver-self-install: 3.3 `build-so` matrix with `make test-artifact`), and every task traces to the intent or issue #13. No objection on cluster coherence: Groups A, B, and C each share a module set, and Group D comes last because it documents the final state.

## Design Depth

#### [INFORMATION_LEAKAGE] FINDINGS
- Location: plan.md tasks 2.3 and 2.5
- Issue: Each backend makes one decision: how a driver becomes loadable. On Docker, the harness creates the reserved connection and self-install writes the driver. On Personal, the harness places the library at `supplied`. The plan spreads that decision across `schema.rs` and four stack files, each with its own `cfg`. The Personal path also needs a second copy of the logic in `crates/adbc-vs-scan/tests/common/driver_fetch.rs`. That module's doc comment states its invariant: "This is the only route an integration tier has to a driver library outside BucketFS ... it agrees by calling production's own code rather than by restating it." A second downloader in the adapter harness breaks that invariant.
- Fix: In task 2.3, give the `Exasol` type one method that owns the driver route, for example `provide_drivers(&self, drivers: &[Driver])`. The Docker backend creates `ADBC_VS_BUCKETFS`. The Personal backend writes each library returned by `driver_fetch::fetch_driver_library` to `adbc-drivers/<driver>/supplied/<library>`. Each `bring_up` calls that method instead of `create_bucketfs_connection`. Reuse `driver_fetch.rs` from the adapter harness through `#[path = "../../../adbc-vs-scan/tests/common/driver_fetch.rs"]`, or move it to a module that both crates' tests include. Do not write a second copy.

No objection on the ADR checks. `[ADR_OVERPROMOTION]`: all three decision-log entries are `Promotes to ADR: no`. `[ARCHITECTURE_DRIFT]`: the BASE hash `2a3a0535` matches `git hash-object specs/architecture.md`; each CHANGED block matches the recorded section except for the intended lines; and no plan.md component, interface, constraint, or dependency is missing from the delta. `[ADR_CONFLICT]`: the Personal setup, which creates no reserved connection and uses `supplied`, matches `fallback-scoped-to-reserved-connection-route`, `version-free-supplied-driver-location`, and the path-override scenario "The default driver location is used when no BucketFS connection resolves".

## Prose Quality

#### [PROSE_UNCLEAR] FINDINGS
- Location: plan.md § Impact, bullet 6
- Issue: "CI runs about twice as many jobs" understates the change. A pull request runs 10 jobs today. Under task 3.3 it runs 24: `build-so` 2, `lint` 1, `unit-tests` 2, `deny` 1, the integration tiers 6, and the E2E tiers 12. The E2E jobs, which are the most expensive, grow from 3 to 12.
- Fix: Replace the bullet with: "A pull request runs 24 CI jobs instead of 10, and the E2E jobs grow from 3 to 12. A push to main adds two `package` jobs and the `release` job."

#### [PROSE_UNCLEAR] FINDINGS
- Location: specs/_plans/add-arm64-support/driver-provisioning/driver-version-resolution/spec.md § Scenario: Package platform follows the architecture the adapter library was compiled for
- Issue: The WHEN step names only an x86_64 library. The aarch64 case appears as a second condition inside a THEN step: "*AND* an adapter library compiled for aarch64 MUST download the `linux_arm64` package from the same index". A reader cannot tell which WHEN that step belongs to.
- Fix: Write one condition and one rule: "*WHEN* the adapter provisions that driver" / "*THEN* the adapter MUST download the package whose platform matches the architecture the adapter library was compiled for: `linux_amd64` for x86_64, `linux_arm64` for aarch64" / "*AND* it MUST NOT download the other package, because an adapter library runs only on the architecture it was compiled for".
