# adbc-vs — Project Rules

Rust Exasol Virtual Schema over ADBC sources. Two crates (`adbc-vs-adapter`, `adbc-vs-scan`) link into one SLC-compatible `.so`. Product intent, scope, architecture, and constraints live in `specs/mission.md`.

## Spec governance

- E2E testing, integration testing, toolchain/build, CI, and licensing MUST NEVER be defined as specs. No `specs/<group>/<feature>/spec.md` may exist for them, and no speq plan may add one back.
- That knowledge is operational and environmental, not product behavior. It lives in this file, as dense bullets — never as prose, never as Gherkin scenarios.
- `specs/` defines only what the adapter does for a user: `vs-adapter/`, `scan-udf/`, `driver-provisioning/`. Behavior that is *verified* through a live stack still belongs to the feature spec that owns the behavior (e.g. "Filter pushdown reaches MySQL rather than Exasol" lives in `vs-adapter/pushdown-filter`), never to an `e2e/` group organized by test file.
- The real source of truth for everything below is the config itself: `Makefile`, `.github/workflows/ci.yml`, `deny.toml`, `rust-toolchain.toml`, `docker-compose.yml`. This file summarizes them and records *why*; it never restates them line by line. Change one of them, update the matching bullet here in the same commit.
- Executable enforcement lives in tests, not specs — keep these: `crates/adbc-vs-adapter/tests/make_targets_test.rs` (Makefile), `tests/workflow_test.rs` (ci.yml), `tests/policy_test.rs` (deny.toml).
- **`vs-adapter`'s feature count deliberately exceeds `/speq:spec-merge`'s generic >8-features-per-domain guideline, and MUST NOT be "fixed" by re-consolidating features.** The count is the intended result of splitting framework specs from per-technology specs (`pushdown-filter` vs. `mysql-pushdown`/`postgresql-pushdown`, and so on). Merging technology features back into their framework feature would restore exactly the mixing that split removed. The guideline is advisory, not a validator error; a human accepted this shape on the record. See decisions `[25]` and `[26]` in the `add-postgresql-support` decision log, promoted to ADRs in `specs/_decision/`.
- That acceptance is not a licence to skip the guideline elsewhere. The two thresholds were already jointly unsatisfiable for `vs-adapter` alone — roughly 96 scenarios cannot fit in 8 files of at most 10 — and the only lever that resolves them is a new top-level domain, which the rule above forbids. Every other domain still answers the guideline: `driver-provisioning` holds three features, none over 10 scenarios, and `scan-udf` three.
- `driver-provisioning` splits along **framework-internal** lines, never per technology: `driver-self-install` (install mechanism — download, `MANIFEST` check, BucketFS upload, visibility, transport policy), `driver-version-resolution` (registry index, version-range matching, which platform package, whether the resolved version is already installed), `driver-path-override` (the `DRIVER_PATH` air-gap mechanism). A new technology adds a row to `crates/adbc-vs-scan/drivers/<technology>.toml` and no provisioning feature, because provisioning's technology-dependence is data rather than behavior.
- **Every feature is either a framework feature or a technology feature.** A framework feature keeps the protocol's own vocabulary as its name (`pushdown-filter`, `type-mapping`, `create-virtual-schema`, `driver-self-install`, and so on). A technology feature is named `<technology>-<concern>` (`mysql-pushdown`, `postgresql-type-mapping`, and so on) and sits flat inside one of the three existing domains — never nested under its framework counterpart.
- Nesting is not used because `speq` represents exactly two path levels (`domain/feature`) and silently drops a nested feature from `feature list` rather than rejecting it — a nested spec would be invisible to search and validation, not merely disallowed.
- A new technology therefore adds three `vs-adapter` features (`<technology>-pushdown`, `<technology>-collation`, `<technology>-type-mapping`), one `scan-udf` feature (`<technology>-scan`), and no `driver-provisioning` feature — provisioning's technology-dependence is data in `drivers/<technology>.toml`, not behavior.
- See decisions `[25]` and `[26]` (`add-postgresql-support` decision log, promoted to ADRs in `specs/_decision/`) for why this shape was chosen; this file states the convention, not the reasoning.

## Toolchain & build

- `rust-toolchain.toml` pins `channel = "1.94"` with `rustfmt` + `clippy`, so host checks use the same compiler as the container build.
- `make cross-udf-build` is the ONLY way to build the UDF `.so`: `cargo build --release -p adbc-vs-adapter` inside `rust:1.94-trixie`. NEVER run host `cargo build --release` — the glibc fingerprint must match the `language-container-rs` SLC or the `.so` is rejected at load time.
- Output is `target/release/libadbc_vs_adapter.so`; both workspace crates link into that single `.so`.
- It is a real-file Make target with source prerequisites (both crates' `src/`, `Cargo.toml`, `Cargo.lock`). Unchanged workspace is a no-op; touching a crate source forces a rebuild. No test ever runs against a stale binary.
- The container runs as the invoking host UID/GID, so no artifact is root-owned, and mounts the named volume `adbc-vs-cargo-registry` so the download cache survives `--rm`.
- The target self-validates with `cargo exasol-udf validate` and exits non-zero on a fingerprint mismatch. `SLC_VERSION` in the `Makefile` MUST track `exasol-udf-sdk`'s version exactly — see `crates/adbc-vs-adapter/Cargo.toml` and `SLC_VERSION` in `tests/common` — or a correct `.so` reports a false mismatch.
- Every check goes through a Make target. CI calls `make <target>` and never re-spells a check as inline shell.

## Driver provisioning

- Each technology's driver package is declared in `crates/adbc-vs-scan/drivers/<technology>.toml` — `registry_path`, `library`, `entry_point`, `version_range`, and the dialect's advertised capability set. `crates/adbc-vs-scan/build.rs` generates the compiled-in `DriverPackage` table from every file in that directory at build time; nothing under `drivers/` is read at run time, so a new technology ships as a reviewed TOML file plus its dialect Rust code in a release, never as an operator-supplied file loaded live.
- `build.rs` validates only the `[package]` table (`deny_unknown_fields` on `registry_path`, `library`, `entry_point`, `version_range`). It checks no capability name against a list of valid names — an unknown or unrenderable capability name reaches the generated table and is caught by the capability invariant test, not the build.
- `crates/adbc-vs-scan/build.rs` and every file under `crates/adbc-vs-scan/drivers/` are `cross-udf-build` prerequisites (`SCAN_BUILD_INPUTS` in the Makefile) — editing a TOML file alone forces a `.so` rebuild, so no test tier ever certifies a stale binary against a changed driver declaration.
- Each driver's `version_range` is independent of every other driver's: MySQL `>=0.6.0, <0.7.0`, PostgreSQL `>=1.12.0, <2.0.0`. Neither is derived from the other or from a shared rule, and a range change to one never implies a change to the other.
- The BucketFS install path is versioned: `<mount>/adbc-drivers/<driver>/<version>/<library>`. Two in-range versions of the same driver, or two different drivers, install side by side without either overwriting or short-circuiting the other's presence check.
- The `DRIVER_PATH` property is the supported air-gapped route. When present, `createVirtualSchema`/`refresh`/`setProperties` skip `provisioning::ensure_driver` entirely: no registry read, no download, no BucketFS write, no integrity check. `BUCKETFS_CONNECTION` becomes optional in that case, since its only purpose is the self-install write credential.
- Self-install verifies NO checksum and NO signature — the download's TLS certificate verification is the only integrity guarantee, for MySQL too now. The compiled-in digest release 0.2 checked (`PACKAGE_SHA256`) has been removed outright (decision `[22]`); no replacement checksum, signature, or operator-supplied digest property was added in its place.
- PostgreSQL's `CONNECTION` value is a single libpq conninfo string carrying its own credential, unlike MySQL's separate `PASSWORD` option. Every error surfaced by `createVirtualSchema`, `refresh`, or `setProperties` MUST redact it — no password or conninfo string may appear — while still naming the host and the database.
- adbc-vs exposes PostgreSQL's `public` schema only. One virtual schema maps to one PostgreSQL database's `public` schema, never to every schema a connection can see.

## Test tiers

- `make test` — host unit tests, `cargo test --workspace`. Needs no Docker. A plain `cargo test` never touches a live stack.
- `make test-it` — integration tier. `cargo test -p adbc-vs-scan --features mysql-it --test mysql_it_test`, prereq `stack-up-mysql`.
  - Covers: the real ADBC path only — driver load, statement execution, Arrow batch streaming, type coercion — against the `mysql` container.
  - Does NOT cover: Exasol, the adapter script, pushdown planning, BucketFS, the SLC.
  - MUST NOT require an Exasol container (`stack-up-mysql`, never `stack-up`) and MUST NOT depend on `cross-udf-build` — these tests link the crate as an `rlib`.
  - This is the tier where the riskiest code is debuggable: a failure names an Arrow type or an ADBC status directly instead of surfacing as a UDF error.
- `make test-it-postgres` — the same integration tier for PostgreSQL. `cargo test -p adbc-vs-scan --features postgres-it --test postgres_it_test`, prereq `stack-up-postgres`.
  - Covers: driver load by the PostgreSQL entry point, statement execution, Arrow batch streaming across more than one batch, coercion of every mapped type including string-form `numeric` and a microsecond timestamp, and the ADBC status a rejected statement surfaces — the same ADBC-only scope `test-it` covers for MySQL.
  - Does NOT cover: Exasol, the adapter script, pushdown planning, BucketFS, the SLC.
  - MUST NOT require an Exasol container (`stack-up-postgres`, never `stack-up`) and MUST NOT depend on `cross-udf-build` — links the crate as an `rlib`, same as `test-it`.
- `make test-artifact` — `cargo test -p adbc-vs-adapter --features built-artifact --test artifact_test`, prereq `cross-udf-build`. Asserts against the real `.so`; MUST fail, not skip, when the `.so` is absent, and must run on the machine that holds it.
- `make deny` — `cargo deny check licenses advisories bans sources`, then `cargo test -p adbc-vs-adapter --features policy-check --test policy_test`.
- `make test-e2e` — full-stack tier, see below.
- `make bench` — transmission-throughput benchmark against the full stack, see "Benchmarking" below.
- `make test-registry-live` — `cargo test -p adbc-vs-adapter --features registry-live --test provisioning_test -- registry_live`, no prereq. The ONLY check that reaches the public driver registry: it resolves every `drivers/*.toml` version range against the index the CDN publishes today, so a range the registry has moved past is caught. Run by hand when a range is changed or suspected stale. In NO CI job — a CDN outage or a slow DNS answer is not a defect in the commit under test, and the fast tier must never depend on the network.
- All non-unit tiers are behind default-off cargo features: `mysql-it`, `postgres-it`, `built-artifact`, `policy-check`, `exasol-e2e`, `bench`, `registry-live`.
- Stack lifecycle: `make stack-up-mysql` (MySQL alone, `compose up -d --wait`), `make stack-up-postgres` (PostgreSQL alone, same pattern), `make stack-up` (depends on both, adds Exasol), `make stack-down` (removes every service). Never a fixed `sleep`.
- `docker/postgres/init.sql` seeds the PostgreSQL fixture only against a fresh volume — Postgres' own entrypoint skips it on a container reused from before the change. Editing that file requires `docker compose rm -sfv postgres && make stack-up-postgres` to apply it. `SEEDED_OBJECTS` in `crates/adbc-vs-adapter/tests/common/postgres.rs` is what turns a stale container into a named failure rather than a mystery one: `Postgres::check_fixture` asserts every name in that list exists before a test runs.

## E2E testing

- `make test-e2e` → prereqs `cross-udf-build` + `stack-up`, then `cargo test -p adbc-vs-adapter --features exasol-e2e --test e2e_mysql_test -- --test-threads=1`.
- `--test-threads=1` is mandatory: one shared setup module installs the SLC, uploads the `.so` to BucketFS, creates the scripts, and creates the one virtual schema every test shares.
- `make test-e2e-postgres` → same prereqs, then `cargo test -p adbc-vs-adapter --features exasol-e2e --test e2e_postgres_test -- --test-threads=1`. Its own shared setup module creates two virtual schemas rather than one — the PostgreSQL schema under test, plus a MySQL schema beside it — because the two-driver claims (each library installs under its own BucketFS path, each query reaches its own source, each virtual schema advertises its own dialect's capability set) are observable only from a stack carrying both. Also covers: PostgreSQL identifier case surviving discovery and rendering, a PostgreSQL view exposed alongside base tables, and a PostgreSQL `REFRESH` re-running metadata discovery after the source gains a column.
- Gated by the `exasol-e2e` feature, off by default.
- **Fail, never skip.** No environment variable, return value, or config may turn an unavailable container into a skipped or quietly-passing test. Bounded waits panic, naming the subject and the elapsed deadline.
- The stack is one `docker-compose.yml`: `exasol/docker-db:2025.2.1` + `mysql:8.4` + `postgres:18.6-trixie`. It is the only stack this project's tests need.
- Exasol's `exaconf` rewrites `/etc/hosts` and `/etc/resolv.conf` at startup, so a Compose service name does NOT resolve from inside a UDF sandbox. MySQL is addressed by the static IP `172.28.5.10`, PostgreSQL by `172.28.5.11`, both on the dedicated `adbc-vs` bridge network (subnet `172.28.5.0/24`).
- The `postgres` service keeps the image's default `LANG=en_US.utf8` cluster locale deliberately — no `POSTGRES_INITDB_ARGS` override. That locale's ordering disagrees with Exasol's byte-order comparison, which is what makes a delegated fragment missing `COLLATE "C"` fail a differential test instead of passing silently.
- Since task 1.2, `make stack-up` depends on `stack-up-postgres` alongside `stack-up-mysql`, so `make test-e2e` (`e2e_mysql_test.rs`, MySQL-only) now also starts a `postgres` container it never queries. Accepted rather than split further: `make stack-up` is the one "everything up" entry point both E2E binaries share.
- The Exasol service needs `privileged: true` and `shm_size: 2g`. Docker's 64 MB default `/dev/shm` is too small once a UDF sandbox starts.
- Every Exasol DSN carries `validateservercertificate=0` — the container serves a self-signed certificate.
- `exapump` 0.9.2 exposes NO `wait` subcommand. **This overrides the workspace-level "use `exapump wait`" rule for this repo.** Readiness is a bounded poll: `make stack-up` loops `exapump sql --dsn ... "SELECT 1"` every 5s up to `EXASOL_WAIT_TIMEOUT` (default 300s); the harness uses `SERVICE_DEADLINE` and `BUCKETFS_DEADLINE` of 300s, a 250ms poll interval, and a 2s connect timeout.
- BucketFS writes are asynchronous. Wait until the object (or an extracted archive's contents) is readable to a UDF; never assume it the moment the upload returns.
- Verify every delegated fragment twice: `EXPLAIN VIRTUAL` proves the fragment reached the generated source statement, and the returned rows prove it produced the right answer. Asserting only one cannot tell a correct delegation from a silently dropped one.
- Prefer differential assertions: mirror the source fixture into a local Exasol table holding identical values and run the same query over both. Collation, aggregate result type, and `MIN`/`MAX` ordering gaps then surface as a diff instead of having to be predicted case by case.

## Benchmarking

- Measures transmission throughput: wall-clock around a `CREATE ... AS SELECT` (CTAS) moving a MySQL table into Exasol, comparing this repo's ADBC-fronted virtual schema against Exasol's official JDBC-based `mysql-virtual-schema`.
- **Framing correction:** for this query shape, the JDBC adapter's Java script only does metadata/pushdown planning — the actual bulk transfer runs through Exasol's native ExaLoader (`IMPORT ... FROM JDBC`), entirely outside the Java UDF sandbox. So this compares "ExaLoader-over-JDBC" vs. "Rust scan UDF over ADBC/Arrow," never "Java vs Rust" adapter code.
- `make bench` → prereqs `cross-udf-build stack-up`, then `cargo test -p adbc-vs-adapter --features bench --test bench_mysql_test -- --test-threads=1 --nocapture`. Feature-gated (`bench`, default-off) so `make test` never needs a container.
- `--test-threads=1` is mandatory, same reason as `test-e2e`: one shared setup module provisions both virtual schemas and the `BENCH` sink schema, which every sample's CTAS reuses.
- No CI job. A shared GitHub runner is the wrong place to trust throughput numbers, and `workflow_test.rs` only asserts workflow→Makefile references, never the reverse, so an uninvoked `bench` target breaks nothing.
- Three data tiers, seeded into MySQL: `narrow_small` (100,000 rows, 8 narrow columns, ~10MB), `narrow_large` (1,000,000 rows, same 8 columns, ~100MB), `wide` (200,000 rows, ~1KB/row via wider `VARCHAR`/`DECIMAL` columns, ~200MB).
- Output: `dist/bench/results-<UTC-timestamp>.csv` (every raw sample) + `report-<UTC-timestamp>.md` (provenance header + summary table), gitignored via the existing `/dist` entry.
- Known asymmetry, not a bug: MySQL `CHAR` maps to ADBC `VARCHAR`/JDBC `CHAR`, and MySQL `DATETIME` maps to ADBC `TIMESTAMP(3)`/JDBC `TIMESTAMP(0)`. Both are each adapter's own intentional mapping — a small real bias in the CTAS write cost shared by both sides, not something the benchmark corrects for.
- Since task 1.2, `make bench`'s `stack-up` prereq also brings up `postgres` alongside `mysql` and `exasol`, so every throughput sample is taken on a host running a third database it never queries — not something the benchmark corrects for either.

## CI

- `.github/workflows/ci.yml`. Triggers: `push` to `main`, `pull_request` against `main`, `merge_group`, `workflow_dispatch`. Concurrency group is keyed on workflow + ref, with `cancel-in-progress` false for `main` and for tag refs so a merged run is never cancelled.
- Nine jobs, all `runs-on: ubuntu-latest`, x86_64 only:
  - `build-so` → `make cross-udf-build`, then `make test-artifact`, then uploads the built `.so` as the `adbc-vs-so` artifact for `release` to consume
  - `lint` → `make fmt-check`, then `make lint`
  - `unit-tests` → `make test`
  - `deny` → `cargo install cargo-deny --locked`, then `make deny`
  - `integration-tests` → `make test-it`
  - `integration-tests-postgres` → `make test-it-postgres`
  - `e2e` → `make test-e2e`
  - `e2e-postgres` → `make test-e2e-postgres`
  - `release` → needs all eight jobs above and only runs on a push to `main`. Reads the workspace version out of the root `Cargo.toml`, and is a no-op if the `v<version>` tag already exists — merging a PR that doesn't bump the version releases nothing. When the version was bumped, it downloads the `adbc-vs-so` artifact, runs `make release-tarball` and `make third-party-licenses`, tags and pushes `v<version>`, and creates a GitHub Release (`generate_release_notes: true`) carrying `dist/adbc-vs.tar.gz` and `dist/THIRD-PARTY-LICENSES.md`.
- Every job body invokes `make <target>`. The only permitted inline shell is runner *preparation*, which configures the runner rather than running a check: the disk cleanup, the apparmor sysctl, the `exapump` install, and `release`'s version-read/tag-gate/git-tag steps (deployment glue, not a check). Starting the stack is NOT an exception — `make stack-up` owns it.
- Each source's tier is its own job, never a second step in another source's. A shared job runs its later steps only while the earlier ones passed, so a red MySQL tier would leave PostgreSQL with no signal at the moment it is most needed, and `release`'s `needs` could not tell "passed" from "never ran".
- `e2e` and `e2e-postgres` MUST each run `sudo sysctl -w kernel.apparmor_restrict_unprivileged_userns=0` before the Exasol container starts. Exasol's UDF sandbox uses `nschroot`, which needs an unprivileged user namespace; Ubuntu 24.04 runners block that by default and every UDF then fails with **SQL state 22002 "VM crashed"**.
- `e2e` and `e2e-postgres` MUST each free disk before pulling images — `/usr/share/dotnet`, `/usr/local/lib/android`, `/opt/ghc`, `/opt/hostedtoolcache/CodeQL`, `/opt/hostedtoolcache`, `/usr/local/.ghcup`, plus `docker image prune -af`. A GitHub-hosted runner has roughly 21GB free and Rust + Docker + Exasol + a third container image (`postgres:18.6-trixie`, beside `exasol/docker-db` and `mysql`) exhausts it.
- `e2e` and `e2e-postgres` each dump `docker compose logs exasol` and upload it on failure, under artifact names of their own (`exasol-container-log`, `exasol-container-log-postgres`) — two jobs uploading one name collide.
- No `arm` / `aarch64` / `ubuntu-24.04-arm` runner and no matrix. No job may reference an Azure, AWS, Databricks, Snowflake, or Unity Catalog credential or secret. No second workflow that sweeps cloud resources.

## Licensing & dependency policy

- MIT, `Copyright (c) 2026 Exasol AG` in `LICENSE`. Root `Cargo.toml` `[workspace.package]` sets `license = "MIT"`; every member manifest sets `license.workspace = true`.
- `deny.toml` at the workspace root is the single source of truth. `cargo deny` is the sole enforcement mechanism, invoked through `make deny`.
- Scope is the Rust crate dependency tree only. The ADBC drivers (`mysql`, `postgresql`) fetched at run time from the driver registry are Apache-2.0 and are never bundled into a release artifact, so their license falls outside this policy.
- The allow list is closed and permissive-only — `cargo deny` rejects any identifier absent from it: `MIT`, `Apache-2.0`, `Apache-2.0 WITH LLVM-exception`, `BSD-2-Clause`, `BSD-3-Clause`, `ISC`, `Zlib`, `0BSD`, `Unlicense`, `CC0-1.0`, `BSL-1.0`, `Unicode-3.0`, `CDLA-Permissive-2.0`.
- NEVER add a `GPL`, `LGPL`, `AGPL`, `MPL`, or standalone `OpenSSL` identifier to `allow`.
- `[licenses.private] ignore = true` — the workspace's own crates are MIT but unpublished, so they are skipped rather than listed.
- `exceptions = []` today. `aws-lc-sys` (OpenSSL-derived file, inherited from that crate's OpenSSL fork history) is the only crate this policy has ever needed to except; reqwest's `rustls-tls-webpki-roots` feature and rustls's `ring` provider keep it out of the graph. If a TLS-provider change pulls it back in, add a scoped `[[licenses.exceptions]]` entry as a reviewed change — never widen `allow`.
- `[advisories] ignore` is empty. Every entry ever added MUST be preceded by a comment giving the ISO-8601 date the exception was granted and the condition under which it is re-evaluated (asserted by `policy_test.rs::advisory_ignores_are_dated`). The `serde_yaml` RUSTSEC-2024-0370 advisory was avoided outright by depending on `serde_yaml_ng`.
- `[bans] wildcards = "deny"`, `allow-workspace = true`. `[sources] unknown-registry = "deny"`, `unknown-git = "deny"`.
- `deny.toml` sets only the fields above — every field it doesn't set (`[graph]`, `confidence-threshold`, `multiple-versions`, `allow-registry`, etc.) is left at `cargo-deny`'s own default. Before adding a field back, confirm with `cargo deny init` (against a throwaway manifest) that its value actually differs from the tool's default; otherwise it's noise.
- `about.toml` + `about.hbs` drive `cargo about generate` (pinned `cargo-about@0.9.0`), invoked by `make third-party-licenses`. `about.toml`'s `accepted` list MUST mirror `deny.toml`'s `[licenses].allow` exactly. The output, `dist/THIRD-PARTY-LICENSES.md`, is generated fresh by the `release` CI job and attached to the GitHub Release — it is never committed (`dist/` is gitignored).

## Spec-driven development

- speq workflow: `/speq:plan` → `/speq:implement` → `/speq:record`, run from this directory.
- `specs/mission.md` is the mission. `specs/_decision/` holds ADRs (append-only history — do not rewrite past decisions). `specs/_plans/` holds in-flight plans. `specs/_recorded/` is a gitignored local archive of shipped plans.
- Live spec groups are `vs-adapter/`, `scan-udf/`, and `driver-provisioning/`. Creating an `e2e/`, `toolchain/`, `ci/`, or `licensing/` group violates Spec governance above.
