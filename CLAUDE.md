# adbc-vs — Project Rules

Rust Exasol Virtual Schema over ADBC sources. Two crates (`adbc-vs-adapter`, `adbc-vs-scan`) link into one SLC-compatible `.so`. Product intent, scope, architecture, and constraints live in `specs/mission.md`.

## Spec governance

- E2E testing, integration testing, toolchain/build, CI, and licensing MUST NEVER be defined as specs. No `specs/<group>/<feature>/spec.md` may exist for them, and no speq plan may add one back.
- That knowledge is operational and environmental, not product behavior. It lives in this file, as dense bullets — never as prose, never as Gherkin scenarios.
- `specs/` defines only what the adapter does for a user: `vs-adapter/`, `scan-udf/`, `driver-provisioning/`. Behavior that is *verified* through a live stack still belongs to the feature spec that owns the behavior (e.g. "Filter pushdown reaches MySQL rather than Exasol" lives in `vs-adapter/pushdown-filter`), never to an `e2e/` group organized by test file.
- The real source of truth for everything below is the config itself: `Makefile`, `.github/workflows/ci.yml`, `deny.toml`, `rust-toolchain.toml`, `docker-compose.yml`. This file summarizes them and records *why*; it never restates them line by line. Change one of them, update the matching bullet here in the same commit.
- Executable enforcement lives in tests, not specs — keep these: `crates/adbc-vs-adapter/tests/make_targets_test.rs` (Makefile), `tests/workflow_test.rs` (ci.yml), `tests/policy_test.rs` (deny.toml).

## Toolchain & build

- `rust-toolchain.toml` pins `channel = "1.94"` with `rustfmt` + `clippy`, so host checks use the same compiler as the container build.
- `make cross-udf-build` is the ONLY way to build the UDF `.so`: `cargo build --release -p adbc-vs-adapter` inside `rust:1.94-trixie`. NEVER run host `cargo build --release` — the glibc fingerprint must match the `language-container-rs` SLC or the `.so` is rejected at load time.
- Output is `target/release/libadbc_vs_adapter.so`; both workspace crates link into that single `.so`.
- It is a real-file Make target with source prerequisites (both crates' `src/`, `Cargo.toml`, `Cargo.lock`, `.cargo/config.toml`). Unchanged workspace is a no-op; touching a crate source forces a rebuild. No test ever runs against a stale binary.
- The container runs as the invoking host UID/GID, so no artifact is root-owned, and mounts the named volume `adbc-vs-cargo-registry` so the download cache survives `--rm`.
- The target self-validates with `cargo exasol-udf validate` and exits non-zero on a fingerprint mismatch. `SLC_VERSION` in the `Makefile` MUST track `exasol-udf-sdk`'s version exactly — see `crates/adbc-vs-adapter/Cargo.toml` and `SLC_VERSION` in `tests/common` — or a correct `.so` reports a false mismatch.
- Every check goes through a Make target. CI calls `make <target>` and never re-spells a check as inline shell.

## Test tiers

- `make test` — host unit tests, `cargo test --workspace`. Needs no Docker. A plain `cargo test` never touches a live stack.
- `make test-it` — integration tier. `cargo test -p adbc-vs-scan --features mysql-it --test mysql_it_test`, prereq `stack-up-mysql`.
  - Covers: the real ADBC path only — driver load, statement execution, Arrow batch streaming, type coercion — against the `mysql` container.
  - Does NOT cover: Exasol, the adapter script, pushdown planning, BucketFS, the SLC.
  - MUST NOT require an Exasol container (`stack-up-mysql`, never `stack-up`) and MUST NOT depend on `cross-udf-build` — these tests link the crate as an `rlib`.
  - This is the tier where the riskiest code is debuggable: a failure names an Arrow type or an ADBC status directly instead of surfacing as a UDF error.
- `make test-artifact` — `cargo test -p adbc-vs-adapter --features built-artifact --test artifact_test`, prereq `cross-udf-build`. Asserts against the real `.so`; MUST fail, not skip, when the `.so` is absent, and must run on the machine that holds it.
- `make deny` — `cargo deny check licenses advisories bans sources`, then `cargo test -p adbc-vs-adapter --features policy-check --test policy_test`.
- `make test-e2e` — full-stack tier, see below.
- All non-unit tiers are behind default-off cargo features: `mysql-it`, `built-artifact`, `policy-check`, `exasol-e2e`.
- Stack lifecycle: `make stack-up-mysql` (MySQL alone, `compose up -d --wait`), `make stack-up` (adds Exasol), `make stack-down` (removes every service). Never a fixed `sleep`.

## E2E testing

- `make test-e2e` → prereqs `cross-udf-build` + `stack-up`, then `cargo test -p adbc-vs-adapter --features exasol-e2e --test e2e_mysql_test -- --test-threads=1`.
- `--test-threads=1` is mandatory: one shared setup module installs the SLC, uploads the `.so` to BucketFS, creates the scripts, and creates the one virtual schema every test shares.
- Gated by the `exasol-e2e` feature, off by default.
- **Fail, never skip.** No environment variable, return value, or config may turn an unavailable container into a skipped or quietly-passing test. Bounded waits panic, naming the subject and the elapsed deadline.
- The stack is one `docker-compose.yml`: `exasol/docker-db:2025.2.1` + `mysql:8.4`. It is the only stack this project's tests need.
- Exasol's `exaconf` rewrites `/etc/hosts` and `/etc/resolv.conf` at startup, so a Compose service name does NOT resolve from inside a UDF sandbox. MySQL is addressed by the static IP `172.28.5.10` on the dedicated `adbc-vs` bridge network (subnet `172.28.5.0/24`).
- The Exasol service needs `privileged: true` and `shm_size: 2g`. Docker's 64 MB default `/dev/shm` is too small once a UDF sandbox starts.
- Every Exasol DSN carries `validateservercertificate=0` — the container serves a self-signed certificate.
- `exapump` 0.9.2 exposes NO `wait` subcommand. **This overrides the workspace-level "use `exapump wait`" rule for this repo.** Readiness is a bounded poll: `make stack-up` loops `exapump sql --dsn ... "SELECT 1"` every 5s up to `EXASOL_WAIT_TIMEOUT` (default 300s); the harness uses `SERVICE_DEADLINE` and `BUCKETFS_DEADLINE` of 300s, a 250ms poll interval, and a 2s connect timeout.
- BucketFS writes are asynchronous. Wait until the object (or an extracted archive's contents) is readable to a UDF; never assume it the moment the upload returns.
- Verify every delegated fragment twice: `EXPLAIN VIRTUAL` proves the fragment reached the generated MySQL statement, and the returned rows prove it produced the right answer. Asserting only one cannot tell a correct delegation from a silently dropped one.
- Prefer differential assertions: mirror the MySQL fixture into a local Exasol table holding identical values and run the same query over both. Collation, aggregate result type, and `MIN`/`MAX` ordering gaps then surface as a diff instead of having to be predicted case by case.

## CI

- `.github/workflows/ci.yml`. Triggers: `push` to `main`, `pull_request` against `main`, `merge_group`, `workflow_dispatch`. Concurrency group is keyed on workflow + ref, with `cancel-in-progress` false for `main` and for tag refs so a merged run is never cancelled.
- Six jobs, all `runs-on: ubuntu-latest`, x86_64 only:
  - `build-so` → `make cross-udf-build`, then `make test-artifact`
  - `lint` → `make fmt-check`, then `make lint`
  - `unit-tests` → `make test`
  - `deny` → `cargo install cargo-deny --locked`, then `make deny`
  - `integration-tests` → `make test-it`
  - `e2e` → `make test-e2e`
- Every job body invokes `make <target>`. The only permitted inline shell is runner *preparation*, which configures the runner rather than running a check: the disk cleanup, the apparmor sysctl, and the `exapump` install. Starting the stack is NOT an exception — `make stack-up` owns it.
- `e2e` MUST run `sudo sysctl -w kernel.apparmor_restrict_unprivileged_userns=0` before the Exasol container starts. Exasol's UDF sandbox uses `nschroot`, which needs an unprivileged user namespace; Ubuntu 24.04 runners block that by default and every UDF then fails with **SQL state 22002 "VM crashed"**.
- `e2e` MUST free disk before pulling images — `/usr/share/dotnet`, `/usr/local/lib/android`, `/opt/ghc`, `/opt/hostedtoolcache/CodeQL`, plus `docker image prune -af`. A GitHub-hosted runner has roughly 21GB free and Rust + Docker + Exasol exhausts it.
- `e2e` dumps `docker compose logs exasol` and uploads it as the `exasol-container-log` artifact on failure.
- No `arm` / `aarch64` / `ubuntu-24.04-arm` runner and no matrix. No job may reference an Azure, AWS, Databricks, Snowflake, or Unity Catalog credential or secret. No second workflow that sweeps cloud resources.

## Licensing & dependency policy

- MIT, `Copyright (c) 2026 Exasol AG` in `LICENSE`. Root `Cargo.toml` `[workspace.package]` sets `license = "MIT"`; every member manifest sets `license.workspace = true`.
- `deny.toml` at the workspace root is the single source of truth. `cargo deny` is the sole enforcement mechanism, invoked through `make deny`.
- Scope is the Rust crate dependency tree only. The MySQL ADBC driver fetched at run time from the driver registry is Apache-2.0 and is never bundled into a release artifact, so its license falls outside this policy.
- The allow list is closed and permissive-only — `cargo deny` rejects any identifier absent from it: `MIT`, `Apache-2.0`, `Apache-2.0 WITH LLVM-exception`, `BSD-2-Clause`, `BSD-3-Clause`, `ISC`, `Zlib`, `0BSD`, `Unlicense`, `CC0-1.0`, `BSL-1.0`, `Unicode-3.0`, `CDLA-Permissive-2.0`. `confidence-threshold = 0.8`.
- NEVER add a `GPL`, `LGPL`, `AGPL`, `MPL`, or standalone `OpenSSL` identifier to `allow`.
- `[licenses.private] ignore = true` — the workspace's own crates are MIT but unpublished, so they are skipped rather than listed.
- `exceptions = []` today. `aws-lc-sys` (OpenSSL-derived file, inherited from that crate's OpenSSL fork history) is the only crate this policy has ever needed to except; reqwest's `rustls-tls-webpki-roots` feature and rustls's `ring` provider keep it out of the graph. If a TLS-provider change pulls it back in, add a scoped `[[licenses.exceptions]]` entry as a reviewed change — never widen `allow`.
- `[advisories] ignore` is empty. Every entry ever added MUST be preceded by a comment giving the ISO-8601 date the exception was granted and the condition under which it is re-evaluated (asserted by `policy_test.rs::advisory_ignores_are_dated`). The `serde_yaml` RUSTSEC-2024-0370 advisory was avoided outright by depending on `serde_yaml_ng`.
- `[bans]`: `multiple-versions = "warn"`, `wildcards = "deny"`, `allow-workspace = true`. `[sources]`: crates.io only; unknown registry and unknown git are denied.

## Spec-driven development

- speq workflow: `/speq:plan` → `/speq:implement` → `/speq:record`, run from this directory.
- `specs/mission.md` is the mission. `specs/_decision/` holds ADRs (append-only history — do not rewrite past decisions). `specs/_plans/` holds in-flight plans. `specs/_recorded/` is a gitignored local archive of shipped plans.
- Live spec groups are `vs-adapter/`, `scan-udf/`, and `driver-provisioning/`. Creating an `e2e/`, `toolchain/`, `ci/`, or `licensing/` group violates Spec governance above.
