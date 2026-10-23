# Plan: add-arm64-support

## Summary

adbc-vs gains a Linux aarch64 build: the adapter library requests the registry driver package of its own architecture, and CI builds, tests, and publishes `adbc-vs-linux-arm64.tar.gz` beside the unchanged x86_64 asset. The E2E tiers run against a matrix of Docker Exasol 2025 and 2026 on x86_64 and Exasol Personal on x86_64 and aarch64 (issue https://github.com/exasol-labs/adbc-vs-rs/issues/13).

## Context

- `crates/adbc-vs-scan/src/driver.rs:177` fixes `PLATFORM` to `linux_amd64`, so an aarch64 library would download x86_64 drivers it cannot load.
- The dbc registry publishes a `linux_arm64` package for every published MySQL, PostgreSQL, and Trino version, including the in-range versions 0.6.1, 1.12.0, and 0.5.3 (index read on 2026-10-23).
- language-container-rs publishes an aarch64 SLC since v0.22.0. For the pinned 0.23.1 the assets are `lc-rust-0.23.1.tar.gz` (x86_64) and `lc-rust-0.23.1-aarch64.tar.gz` (aarch64). It builds the aarch64 SLC natively on `ubuntu-24.04-arm`, from `rust:1.94-trixie` with a glibc floor of 2.41.
- `cargo exasol-udf validate` loads the `.so` with `dlopen`, so it runs only on a host of the `.so`'s architecture (`crates/cargo-exasol-udf/src/validate.rs:131,220-223` in language-container-rs).
- `exasol/docker-db` publishes amd64 images only. The tags `latest-2025.2` and `latest-2026.1` exist, and no `latest-2025` or `latest-2026` tag exists. `docker-compose.yml:14` pins `2025.2.1`.
- The E2E matrix uses the moving tags `latest-2025.2` and `latest-2026.1`. Docker Hub showed on 2026-10-23 that a moving tag can lag its line: `latest-2026.1` was last pushed on 2026-05-22 with 2026.1.0, although 2026.1.2 was pushed on 2026-09-14. `latest-2025.1` (2025.1.16) also carries a newer push than `latest-2025.2` (2025.2.1, pushed 2026-03-20). A moving tag can change the tested Exasol version without a change in this repository.
- Exasol Personal v2.3.0 runs a Linux local deployment directly in Podman on AMD64 or ARM64, without a VM. Each launcher release pulls its own pinned `exasol/nano` image tag. On 2026-10-23 that tag was `nano.7`, published for amd64 and arm64. Exasol Personal is the only aarch64 Exasol the tests use. Its local deployments have no BucketFS HTTP endpoint, take an SLC through `exasol slc custom install` instead of BucketFS, and report their address through `exasol info --json` and their password in the deployment's `secrets.json`.
- Exasol Personal documents that `localhost` and `127.0.0.1` inside its runtime refer to the runtime, not to the host (exasol-personal user docs `virtual-schemas.md:113-114`). On macOS, Exasol Personal runs in a VM that the launcher manages (`local-deployment.md:5-6`), and Docker Desktop cannot route to a container's own IP address (Docker Desktop networking docs, Known limitations).
- The E2E harness reads every Exasol address and credential from the Compose container (`crates/adbc-vs-adapter/tests/common/exasol.rs:19-87`, `mod.rs:3-6`), checks BucketFS through `docker exec`, and self-installs each driver through the BucketFS HTTPS endpoint.
- `crates/adbc-vs-adapter/tests/workflow_test.rs:279` forbids `arm64`, `aarch64`, and `ubuntu-24.04-arm` in `ci.yml`, and `:251-259` requires every job to run on `ubuntu-latest`.
- `exapump`'s installer supports `aarch64`, and the MySQL, PostgreSQL, and Trino images publish `linux/arm64`.
- Docs state x86_64-only support at `docs/install.md:14` and `docs/how-it-works.md:33`. The issue names `docs/limits.md`, which does not exist; the limits live in `docs/how-it-works.md` § Limits.
- The architecture changes are in `architecture.md` in this plan directory.
- The driver-version-resolution Background is condensed from 12 bullets to the 4 that its scenarios depend on, and the removed rationale is not restated elsewhere. The driver-path-override Background keeps its 12 recorded bullets and changes only the phrase that named `linux_amd64`.

## Features

| Feature | Status | Spec |
|---------|--------|------|
| driver-version-resolution | CHANGED | `driver-provisioning/driver-version-resolution/spec.md` |
| driver-self-install | CHANGED | `driver-provisioning/driver-self-install/spec.md` |
| driver-path-override | CHANGED | `driver-provisioning/driver-path-override/spec.md` |

## Impact

- Operators of Linux aarch64 clusters can install adbc-vs. They download `adbc-vs-linux-arm64.tar.gz` and the `lc-rust-0.23.1-aarch64.tar.gz` SLC.
- The x86_64 release asset keeps the name `adbc-vs.tar.gz`, and x86_64 behavior does not change. No change is breaking.
- An aarch64 adapter library self-installs `linux_arm64` drivers. An operator-supplied driver must match the cluster's architecture, as before.
- Exasol Personal users get a complete install path in `docs/install.md`. § 1 installs the pinned SLC with `exasol slc custom install`, § 2 copies the library into the deployment's BucketFS directory, and § 4 places the driver at the `supplied` location, because self-install cannot write to a Personal deployment.
- Contributors gain the make variables `EXASOL_TARGET` (`docker` or `personal`) and `EXASOL_IMAGE`. The default Docker Exasol tag moves from the pinned `2025.2.1` to `latest-2025.2`, so a CI or local E2E run can test a newer Exasol after Docker Hub moves the tag, with no commit in this repository.
- If fallback (a) applies (decision-log [3]), a CI run with `EXASOL_TARGET=personal` publishes the test databases and their fixed passwords on the runner's primary IPv4 address. Local runs keep the source ports on `127.0.0.1`, unless a contributor opts in for one run with `SOURCE_BIND_ADDRESS=<address>` on the make command line. The opt-in publishes the same fixed-password databases on that interface.
- A pull request runs 25 CI jobs instead of 10, and the E2E jobs grow from 3 to 12. A push to main adds two `package` jobs and the `release` job.
- If fallback (b) applies (decision-log [3]), a pull request runs 22 CI jobs with 9 E2E jobs, and CI runs no aarch64 E2E. A contributor runs `make test-e2e-arm64` on an aarch64 host before each release. On Apple silicon that run needs the `SOURCE_BIND_ADDRESS` opt-in, and the Apple silicon path stays unverified until its E2E test binaries have been built there.

## Implementation Tasks

### Group A: Registry platform from the compile target

- [ ] 1.1 Write the failing test `self_install_downloads_the_package_of_the_compile_targets_platform` in `crates/adbc-vs-adapter/tests/provisioning_test.rs`. The stub index publishes a `linux_amd64` and a `linux_arm64` package for the resolved version, each a tarball whose library bytes name its platform. The test asserts the installed library carries the bytes of the expected platform, which the test spells as a literal per `#[cfg(target_arch)]`.
- [ ] 1.2 In `crates/adbc-vs-scan/src/driver.rs`, select `PLATFORM` with `#[cfg(target_arch = "x86_64")]` (`linux_amd64`) and `#[cfg(target_arch = "aarch64")]` (`linux_arm64`), plus a `compile_error!` for any other target. Rewrite its doc comment and the one at `crates/adbc-vs-adapter/src/provisioning.rs:770` to name the compile target instead of `linux_amd64`.
- [ ] 1.3 Make every test that serves or expects a `linux_amd64` package pass on both architectures:
  - Change `missing_platform_names_driver_and_version` and `missing_platform_names_driver_and_version_for_postgresql` (`tests/provisioning_test.rs:306,345`). The stub publishes only the other Linux architecture's package plus `darwin_arm64`. The tests assert that the error names `PLATFORM` and that no package request reached the stub.
  - Change `version_without_the_platform_package_names_platform_and_version` (`src/provisioning_tests.rs:180`) the same way.
  - Add a `linux_arm64` package beside each `linux_amd64` package in `tests/fixtures/index.yaml`.
  - Build stub package paths from `PLATFORM` in `src/provisioning_tests.rs`, `src/create_schema_tests.rs:964,981`, and `tests/provisioning_test.rs:1090-1155`.
  - Leave `src/registry_index_tests.rs` alone where it passes the platform to `resolve_package` explicitly.
- [ ] 1.4 Replace `PAGE_BYTES = 4096` in `crates/adbc-vs-scan/tests/mysql_it_test.rs:122-124` with the page size `getconf PAGESIZE` reports, so the memory bound holds on 16 KiB and 64 KiB page kernels.
- [ ] 1.5 Run `make test`, `make test-it`, `make test-it-postgres`, and `make test-it-trino` on the x86_64 host.

### Group B: Exasol target selection for the E2E tiers

- [ ] 2.1 Spike on the local x86_64 host, keeping no code (decision-log [3]):
  - Install the Exasol Personal launcher and run `exasol install local --auto-approve`.
  - Read the host, port, and deployment directory from `exasol info --json`, and read the password from `<deploymentDir>/secrets.json` without printing it.
  - Install `lc-rust-0.23.1.tar.gz` with `exasol slc custom install --source <file> --alias RUST --language rust --auto-approve`.
  - Copy the `make cross-udf-build` library and the MySQL `linux_amd64` driver under `<deploymentDir>/local/runtime/exa/bucketfs/bfsdefault/default/`, then create the scripts and a MySQL virtual schema against `172.28.5.10:3306` with no `ADBC_VS_BUCKETFS` connection.
  - Record in the implementation summary whether the SLC loaded, whether the UDF read the copied files, and whether it reached the static IP. A failed static-IP check applies fallback (a) in task 2.5, which the user confirmed for CI runners only.
- [ ] 2.2 Add the Exasol target to the `Makefile`:
  - `EXASOL_TARGET ?= docker` accepts `docker` and `personal`, and any other value fails with a message that names both.
  - `export EXASOL_IMAGE`, and set the `exasol` service image in `docker-compose.yml` to `${EXASOL_IMAGE:-exasol/docker-db:latest-2025.2}`. That default is the "2025-latest" row, because no `latest-2025` tag exists.
  - With `personal`, `stack-up-exasol` runs `exasol install local --auto-approve` unless `exasol info --json` already reports a deployment, then runs a bounded readiness poll that never echoes the password.
  - With `personal`, `stack-down` also runs `exasol destroy --remove --auto-approve`, and `test-e2e`, `test-e2e-postgres`, and `test-e2e-trino` add the `exasol-personal` cargo feature to `exasol-e2e`.
  - Declare `exasol-personal = []` in `crates/adbc-vs-adapter/Cargo.toml`.
  - Split `lint` (`Makefile:82`) into two clippy runs, because `--all-features` would enable `exasol-personal` and compile out the Docker backend and every self-install test. The first run is `cargo clippy --workspace --all-targets --features adbc-vs-adapter/built-artifact,adbc-vs-adapter/exasol-e2e,adbc-vs-adapter/policy-check,adbc-vs-adapter/bench,adbc-vs-adapter/registry-live,adbc-vs-scan/mysql-it,adbc-vs-scan/postgres-it,adbc-vs-scan/trino-it -- -D warnings`. The second run is `cargo clippy -p adbc-vs-adapter --all-targets --features exasol-e2e,exasol-personal -- -D warnings`.
  - Add `compile_error!` to `tests/common/mod.rs` for `all(feature = "bench", feature = "exasol-personal")`, because the benchmarks measure only the Docker stack and report its Compose image. The bench modules still compile under `exasol-personal`, because `mod.rs` includes them in every test binary.
  - Update `lint_fmt_test_targets_use_expected_flags` (`make_targets_test.rs:303`) to expect both clippy commands. Add a test that reads the `[features]` tables of both crates and asserts that the first run lists every feature except `exasol-personal`, so a new feature cannot go unlinted.
  - Extend `crates/adbc-vs-adapter/tests/make_targets_test.rs` (`stack_targets_start_expected_services` :337 and the `test_e2e*` tests :505, :524, :737) with dry runs for both targets.
  - Make `tests/common/bench.rs:339` read the resolved image from `docker compose config --images` instead of the literal image line.
- [ ] 2.3 Give `tests/common/exasol.rs` one `Exasol` type with two backends selected by `cfg(feature = "exasol-personal")`. [expert]
  - The Docker backend keeps today's Compose discovery, the EXAConf write password, BucketFS over HTTPS, and the `docker exec` checks.
  - The Personal backend reads the address and deployment directory from `exasol info --json` and the password from `secrets.json`. It writes and checks BucketFS objects as files under `<deploymentDir>/local/runtime/exa/bucketfs/bfsdefault/default/`.
  - The callers' methods (`execute`, `query_rows`, `put_file`, `put_library`, `copy_bucket_object`, `delete_file`, `await_bucket_path`, `bucket_path_exists`, `bucket_entries`) keep their signatures on both backends. `bucketfs_write_url` and `bucketfs_write_password` exist only on the Docker backend.
  - Rewrite the `mod.rs:1-6` doc comment: the backend is a compile-time choice, and each backend discovers its own instance.
- [ ] 2.4 In `tests/common/slc.rs`, choose the SLC asset by `target_arch` (`lc-rust-<V>.tar.gz` or `lc-rust-<V>-aarch64.tar.gz`) and update `release_url_names_the_asset_of_the_pinned_version`. On the Personal backend, read the `RUST` entry from `exasol slc list --json` and compare it with the pinned SLC version `slc.rs` already uses:
  - No `RUST` alias: run `exasol slc custom install --source <cached tarball> --alias RUST --language rust --auto-approve`.
  - A custom `RUST` container whose source is the pinned asset of the host's architecture: skip the install.
  - A custom `RUST` container from another source, such as an older `SLC_VERSION`: run `exasol slc custom update --source <cached tarball> --alias RUST --language rust --auto-approve`.
  - A `RUST` container that `exasol slc install rust` installed, which tracks the latest language-container-rs release rather than the pinned one: fail with a message that names `exasol slc remove`, because `exasol slc custom update` replaces only custom containers and the harness does not remove a container it did not install.
  - After an install or update, wait for SQL with a bounded poll.
- [ ] 2.5 Give the harness one owner of the driver route (decision-log [2] and Review Findings):
  - Add `provide_drivers(exasol: &Exasol, drivers: &[Driver])` to `tests/common/schema.rs`. On the Docker backend it creates `ADBC_VS_BUCKETFS` through `create_bucketfs_connection`, which becomes Docker-only, and self-install then fetches each driver. On the Personal backend it calls `driver_fetch::fetch_driver_library` for each driver and writes the returned library with `put_library` to the bucket path that `adbc_vs_adapter::provisioning::default_driver_path` returns for an empty mount, `adbc-drivers/<driver>/supplied/<library>`.
  - Include the scan crate's downloader in `tests/common/mod.rs` with `#[path = "../../../adbc-vs-scan/tests/common/driver_fetch.rs"] pub mod driver_fetch;`. Write no second downloader. Extend the `driver_fetch.rs` doc comment to name the E2E harness as its second caller.
  - Replace each `schema::create_bucketfs_connection` call in a bring-up with `schema::provide_drivers`. `tests/common/stack.rs` passes MySQL. `tests/common/postgres_stack.rs` passes PostgreSQL and MySQL. `tests/common/trino_stack.rs` passes Trino and MySQL. Each of the three `tests/common/bench_stack.rs` bring-ups (:113, :175, :237) passes the drivers of the virtual schemas it creates.
  - On the Personal backend, `create_virtual_schema` leaves `BUCKETFS_CONNECTION` unset.
  - If task 2.1 showed the static IPs unreachable, add the source bind address for fallback (a) (decision-log [3]). Task 3.5 adds the same mechanism under fallback (b) if this task did not:
    - Change the source port lines in `docker-compose.yml` (:33, :54, :79-80) to `${SOURCE_BIND_ADDRESS:-127.0.0.1}:<port>:<port>`.
    - The `Makefile` computes `SOURCE_BIND_ADDRESS` as the host's primary IPv4 address only when `EXASOL_TARGET=personal` and `GITHUB_ACTIONS` is set. It passes the value to the Compose and `cargo test` commands.
    - A local run opts in per run with `SOURCE_BIND_ADDRESS=<address>` on the make command line, for example `make test-e2e EXASOL_TARGET=personal SOURCE_BIND_ADDRESS=<address>`. The address is one the Exasol Personal runtime reaches, such as the host's LAN IPv4 address. A command-line value wins over the computed one. Without the opt-in, local runs keep the source ports on `127.0.0.1`.
    - The opt-in and the CI value both publish the test databases and their fixed passwords on that interface, to every machine that can reach it.
    - When `SOURCE_BIND_ADDRESS` is non-empty, the Personal backend addresses each source by that address and its published port. Otherwise it keeps the static IPs. The Docker backend ignores the variable.
    - Extend `make_targets_test.rs` with dry runs of `test-e2e`. With `EXASOL_TARGET=personal GITHUB_ACTIONS=true`, the Compose and `cargo test` commands carry a non-empty `SOURCE_BIND_ADDRESS`. With either variable alone and no command-line value, they carry an empty one. With a command-line `SOURCE_BIND_ADDRESS=192.0.2.10`, they carry `192.0.2.10` with and without `GITHUB_ACTIONS`. The test removes `GITHUB_ACTIONS` from the child environment for the cases that need it unset, because CI sets it. Every source port line in `docker-compose.yml` reads `${SOURCE_BIND_ADDRESS:-127.0.0.1}`.
- [ ] 2.6 In `e2e_mysql_test.rs`, `e2e_postgres_test.rs`, and `e2e_trino_test.rs`, mark each test that needs a BucketFS write, the registry route, the reserved connection (including `ReservedConnectionGuard` in `e2e_mysql_test.rs`), or a recorded driver version with `#[cfg(not(feature = "exasol-personal"))]` (decision-log [2]).
- [ ] 2.7 Run `make test-e2e`, `make test-e2e-postgres`, and `make test-e2e-trino` three times on the local x86_64 host: with the defaults, with `EXASOL_IMAGE=exasol/docker-db:latest-2026.1`, and with `EXASOL_TARGET=personal`. If a 2026.1 or Personal run fails for a product reason, meaning a pushdown, metadata, or protocol difference rather than the environment, stop. Record the failing test and the measured behavior in the implementation summary, and ask the user before changing product code or specs. If fallback (a) applies, the local Personal run reaches the sources only with the `SOURCE_BIND_ADDRESS` opt-in, because the static IPs are unreachable and `127.0.0.1` refers to the Personal runtime. The opt-in exposes fixed-password databases on the user's network, so the implementer does not set it. The implementer runs the local Personal pass only with an address the user names for that run. Otherwise the implementation summary records that the pass did not run, and the `personal-x86_64` CI rows prove the Personal path.

### Group C: Per-architecture build, release, and CI

- [ ] 3.1 In the `Makefile`, read `uname -m` into `HOST_ARCH`, which a make command line can override, and name `RELEASE_TARBALL` after it: `dist/adbc-vs.tar.gz` for `x86_64`, and `dist/adbc-vs-linux-arm64.tar.gz` for `aarch64` or `arm64`. Any other value fails with a message. Update `release_tarball_depends_on_cross_udf_build` (`make_targets_test.rs:455`) to expect the host's name.
- [ ] 3.2 Add `aarch64-unknown-linux-gnu` to `targets` in `about.toml:23`, so one `THIRD-PARTY-LICENSES.md` covers both architectures.
- [ ] 3.3 Rework `.github/workflows/ci.yml`:
  - `build-so`, `lint`, `unit-tests`, `integration-tests`, `integration-tests-postgres`, and `integration-tests-trino` run on a `runner` matrix of `ubuntu-latest` and `ubuntu-24.04-arm`. `lint` runs on both, so clippy checks the `#[cfg(target_arch = "aarch64")]` branches from tasks 1.1, 1.2, and 2.4. `build-so` uploads its `.so` as an artifact named after `runner.arch`.
  - `e2e`, `e2e-postgres`, and `e2e-trino` set `fail-fast: false` and four `include` rows. The rows are `docker-2025` (`ubuntu-latest`, `docker`, `exasol/docker-db:latest-2025.2`), `docker-2026` (`ubuntu-latest`, `docker`, `exasol/docker-db:latest-2026.1`), `personal-x86_64` (`ubuntu-latest`, `personal`), and `personal-arm64` (`ubuntu-24.04-arm`, `personal`). Each row calls `make test-e2e*` with `EXASOL_TARGET` and `EXASOL_IMAGE` from the row.
  - Every row of a job that runs cargo on the host installs Rust 1.94 with `dtolnay/rust-toolchain@1.94`, as the release job does, because no source confirms that the `ubuntu-24.04-arm` image ships rustup.
  - The runner preparation of the Personal rows installs Podman when it is absent and installs the Exasol Personal launcher from `https://downloads.exasol.com/exasol-personal/installer.sh`. The launcher pulls the `exasol/nano` image it pins, so no CI file, make target, or test names a nano tag. The failure-log steps dump the Compose `exasol` log or the Personal container log, depending on the row's target.
  - A new `package` job runs on both runners under the release job's `if`. Its `needs` are `build-so`, `lint`, `unit-tests`, `deny`, `integration-tests`, `integration-tests-postgres`, `integration-tests-trino`, `e2e`, `e2e-postgres`, and `e2e-trino`. It downloads its architecture's `.so` into `target/release`, installs Rust 1.94 and cargo-about, runs `make third-party-licenses release-tarball`, and uploads the tarball. The x86_64 run also uploads `THIRD-PARTY-LICENSES.md`.
  - `release` needs `package`, keeps the tag check, downloads both package artifacts into `dist/`, and publishes `dist/adbc-vs.tar.gz`, `dist/adbc-vs-linux-arm64.tar.gz`, and `dist/THIRD-PARTY-LICENSES.md`.
- [ ] 3.4 Update `crates/adbc-vs-adapter/tests/workflow_test.rs`:
  - Replace the `ubuntu-latest` check (:251-259): each job's `runs-on` is `ubuntu-latest`, `ubuntu-24.04-arm`, or `${{ matrix.runner }}`, and every matrix value is one of those two runners.
  - Let the job-to-target pairing (:215-249) accept the matrix jobs.
  - Rename `workflow_has_no_cloud_or_arm_job` to `workflow_has_no_cloud_job`, keeping the cloud check and the single-workflow check (:292).
  - Assert that `build-so`, `lint`, `unit-tests`, and the three `integration-tests*` jobs cover both runners, and that each E2E job has the four rows.
  - Assert in `release_job_is_gated_and_packages_artifacts` that `package` covers both runners, needs each of the ten jobs task 3.3 lists, and calls `third-party-licenses` and `release-tarball`, that `release` needs `package` and calls neither target, and that `release` uploads both tarballs.
- [ ] 3.5 Run `make test`, `make cross-udf-build`, `make test-artifact`, and `make third-party-licenses release-tarball` on the x86_64 host. Push the branch, and confirm that the aarch64 `build-so`, `lint`, `unit-tests`, `integration-tests*`, and `personal-arm64` rows pass. Task 2.7's product-failure rule applies to every CI row. If Exasol Personal does not start on a `personal-arm64` row, apply fallback (b) (decision-log [3]):
  - Remove the `personal-arm64` row from `e2e`, `e2e-postgres`, and `e2e-trino` in `ci.yml`. Each job keeps `docker-2025`, `docker-2026`, and `personal-x86_64`. The `package` job keeps its ten `needs`, so the release stays gated on the aarch64 `build-so` (including `cargo exasol-udf validate`), `lint`, `unit-tests`, and `integration-tests*` rows.
  - Add `test-e2e-arm64` to the `Makefile`. A make-level `$(error)` fails the target unless `HOST_ARCH` (task 3.1) is `aarch64` or `arm64`, and the message names the value it read. The recipe then runs `$(MAKE) test-e2e test-e2e-postgres test-e2e-trino EXASOL_TARGET=personal`. `uname -m` reports `arm64` on Apple silicon and `aarch64` on Linux. No CI job calls the target.
  - If task 2.5 did not add the source bind address, add it now as task 2.5 describes, without the `GITHUB_ACTIONS` computation. Apple silicon needs it in every case: the Exasol Personal VM reaches neither the Docker Desktop container IPs nor the Mac's `127.0.0.1`. On Apple silicon the command is `make test-e2e-arm64 SOURCE_BIND_ADDRESS=<the Mac's LAN IPv4 address>`, and the opt-in publishes the fixed-password databases on that interface.
  - Add `test_e2e_arm64_runs_every_tier_against_personal` to `make_targets_test.rs`. Dry runs with `HOST_ARCH=aarch64` and `HOST_ARCH=arm64` list `cross-udf-build`, the Personal `stack-up-exasol` recipe, and the three `cargo test` commands with `exasol-e2e,exasol-personal`. A dry run with `HOST_ARCH=arm64 SOURCE_BIND_ADDRESS=192.0.2.10` passes `192.0.2.10` to the Compose and `cargo test` commands of all three tiers. A dry run with `HOST_ARCH=x86_64` exits non-zero and names `x86_64`.
  - In `workflow_test.rs`, change the task 3.4 assertion to the three E2E rows, assert that no E2E row runs on `ubuntu-24.04-arm`, and assert that no job calls `test-e2e-arm64`. `release_job_is_gated_and_packages_artifacts` keeps the ten `needs`.
  - The Apple silicon path is unverified. On Apple silicon the E2E test binaries build for `aarch64-apple-darwin`, a target no CI job and no Linux run compiles. No Linux-only gating was found in `exasol-udf-sdk` 0.23.1 or `exasol-udf-macros` 0.23.0. `ring`'s build script compiles C for the target, so the Linux host cannot check the darwin build. On an Apple silicon host, run `cargo test --no-run -p adbc-vs-adapter --features exasol-e2e,exasol-personal --test e2e_mysql_test --test e2e_postgres_test --test e2e_trino_test`. If no Apple silicon host is available, the implementation summary states the Apple silicon path as unverified.
  - Push again, and confirm that all 22 jobs pass. Record in the implementation summary that `make test-e2e-arm64` has not run if no aarch64 host was available.

### Group D: Docs, rules, and version

- [ ] 4.1 Update `docs/install.md`:
  - § Requirements: a Linux x86_64 or aarch64 cluster, using the release asset and the SLC asset of the cluster's architecture.
  - § 1: name both SLC assets. Add the Exasol Personal steps, because a Personal local deployment has no BucketFS HTTP endpoint: install the SLC with `exasol slc custom install --source lc-rust-0.23.1.tar.gz --alias RUST --language rust` (`lc-rust-0.23.1-aarch64.tar.gz` on aarch64) instead of the upload and `ALTER SYSTEM` steps. Warn that `exasol slc install rust` installs the latest language-container-rs release rather than the pinned 0.23.1.
  - § 2: name both release assets. Add the Exasol Personal step: copy `udf/libadbc_vs_adapter.so` to `<deploymentDir>/local/runtime/exa/bucketfs/bfsdefault/default/udf/`, where `exasol info --json` reports `deploymentDir`.
  - § 4 "Install the driver yourself": pick the `linux_amd64` or `linux_arm64` driver package. On an Exasol Personal local deployment, copy the driver to `<deploymentDir>/local/runtime/exa/bucketfs/bfsdefault/default/adbc-drivers/<driver>/supplied/<library>`, and state that the automatic install route is not available there.
- [ ] 4.2 Replace the x86_64-only line in `docs/how-it-works.md:33` (§ Limits, § Scope): the release has one library per architecture, x86_64 and aarch64, and the automatic install requests the driver package of the library's architecture. State that the aarch64 library is tested against Exasol Personal only, because no other aarch64 Exasol is available to the tests.
- [ ] 4.3 Change `AGENTS.md`:
  - Build: `make cross-udf-build` builds for the host's architecture. The aarch64 `.so` is built on an aarch64 host, never cross-compiled, because `cargo exasol-udf validate` loads the `.so`.
  - Build: `make release-tarball` names the tarball after the host architecture. CI packages each tarball on a runner of its architecture.
  - Code quality: `make lint` covers both Exasol backends with two clippy runs. The first enables every feature except `exasol-personal` and covers the Docker backend. The second enables `exasol-e2e` and `exasol-personal`. CI runs `make lint` on both architectures. A new cargo feature goes into the first run's list.
  - Testing: `make test` and the `test-it*` tiers pass on x86_64 and aarch64. Test code takes the registry platform from `adbc_vs_scan::driver::PLATFORM` and never spells it as a literal for the running path, except `self_install_downloads_the_package_of_the_compile_targets_platform`, which spells `linux_amd64` and `linux_arm64` per `#[cfg(target_arch)]` so that the mapping itself is under test.
  - Testing: `EXASOL_TARGET` selects the E2E Exasol, `docker` (the Compose `exasol` service, tag from `EXASOL_IMAGE`) or `personal` (an Exasol Personal local deployment, which needs Podman). No arm64 Docker Exasol image exists, so aarch64 E2E runs only against Exasol Personal.
  - Testing: Exasol Personal local has no BucketFS HTTP endpoint. Under `personal`, the harness writes BucketFS objects as files in the deployment directory and loads drivers from the `supplied` location. Tests that need self-install are compiled out with `#[cfg(not(feature = "exasol-personal"))]`, which is not a run-time skip.
  - Testing: the harness reads Exasol Personal's address and password from `exasol info --json` and the deployment's secrets file. No Exasol credential goes into an env var, a make variable, a CI file, or a log.
  - Testing: if task 2.5 or 3.5 added the source bind address, the static-IP rule names the Personal exception. Under `EXASOL_TARGET=personal`, the harness addresses the sources at `SOURCE_BIND_ADDRESS` when it is set. Under fallback (a), the Makefile sets it to the runner's primary IPv4 address when `GITHUB_ACTIONS` is set. A local run opts in per run with `SOURCE_BIND_ADDRESS=<address>` on the make command line, and that publishes the test databases and their fixed passwords on that interface. Without the opt-in, local runs keep the source ports on `127.0.0.1`. An agent never sets the opt-in without an address the user names for that run.
  - Testing: if fallback (b) applies, CI runs no aarch64 E2E. Before each release, run `make test-e2e-arm64` on an aarch64 host. It runs the three E2E tiers against Exasol Personal. On Apple silicon, add `SOURCE_BIND_ADDRESS=<the Mac's LAN IPv4 address>`. If task 3.5 did not verify the Apple silicon build, the rule says the Apple silicon path is unverified.
  - Release: the release publishes both architecture tarballs and needs every job on both architectures.
- [ ] 4.4 In `specs/mission.md`, list Linux x86_64 and aarch64 clusters under Core Capabilities, change the Tech Stack testing row to "E2E against a local Exasol Docker container or Exasol Personal", and say "release artifacts" in the bundling constraint.
- [ ] 4.5 Bump the version from 0.6.0 to 0.7.0 in the root `Cargo.toml` and in the `adbc-vs-scan` dependency of `crates/adbc-vs-adapter/Cargo.toml`, and refresh `Cargo.lock`.

## Parallelization

| Group | Tasks | Depends on | Knowledge |
|-------|-------|------------|-----------|
| A: Registry platform from the compile target | 1.1-1.5 | none | spec delta `driver-provisioning/driver-version-resolution`; `crates/adbc-vs-scan/src/driver.rs`, `crates/adbc-vs-adapter/src/provisioning.rs`, `src/provisioning_tests.rs`, `src/create_schema_tests.rs`, `src/registry_index_tests.rs`, `tests/provisioning_test.rs`, `tests/fixtures/index.yaml`, `crates/adbc-vs-scan/tests/mysql_it_test.rs` |
| B: Exasol target selection for the E2E tiers | 2.1-2.7 | none | decision-log [2], [3], Review Findings; spec delta `driver-provisioning/driver-path-override`; `Makefile` (`lint`, stack lifecycle, `test-e2e*`), `docker-compose.yml`, `crates/adbc-vs-adapter/Cargo.toml`, `crates/adbc-vs-scan/Cargo.toml` (`[features]`), `crates/adbc-vs-adapter/tests/common/` (`exasol.rs`, `slc.rs`, `stack.rs`, `postgres_stack.rs`, `trino_stack.rs`, `bench_stack.rs`, `schema.rs`, `mod.rs`, `bench.rs`), `crates/adbc-vs-scan/tests/common/driver_fetch.rs`, `tests/e2e_*_test.rs`, `tests/make_targets_test.rs`; `~/code/exa-personal/user-docs/content/` (`local-deployment.md`, `udfs.md`, `virtual-schemas.md`, `cli-reference.md`) |
| C: Per-architecture build, release, and CI | 3.1-3.5 | A (aarch64 `make test`), B (shares `Makefile` and `make_targets_test.rs`, and the matrix calls `EXASOL_TARGET`) | architecture delta § Interfaces, § Constraints; decision-log [3] (fallback (b)); spec delta `driver-provisioning/driver-self-install`; `Makefile` (release section, `test-e2e*`), `docker-compose.yml` and `crates/adbc-vs-adapter/tests/common/` (`mysql.rs`, `postgres.rs`, `trino.rs` source addresses, for the source bind address under fallback (b)), `about.toml`, `.github/workflows/ci.yml`, `tests/workflow_test.rs`, `tests/make_targets_test.rs`, `tests/artifact_test.rs` |
| D: Docs, rules, and version | 4.1-4.5 | A, B, C | `docs/install.md`, `docs/how-it-works.md`, `AGENTS.md`, `specs/mission.md`, `Cargo.toml`, `crates/adbc-vs-adapter/Cargo.toml`; `~/code/exa-personal/user-docs/content/` (`udfs.md`, `virtual-schemas.md`, `local-deployment.md`) |

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| Assertion | `crates/adbc-vs-adapter/tests/workflow_test.rs:279` (ARM strings in `workflow_has_no_cloud_or_arm_job`) | CI now runs on `ubuntu-24.04-arm` |
| Assertion | `crates/adbc-vs-adapter/tests/workflow_test.rs:251-259` (`runs-on` must equal `ubuntu-latest`) | Replaced by the two-runner check |
| Constant | `crates/adbc-vs-scan/tests/mysql_it_test.rs:122-124` `PAGE_BYTES` | Replaced by the page size the host reports |
| Command | `Makefile:82` `cargo clippy --workspace --all-targets --all-features -- -D warnings` | Replaced by the two clippy runs of task 2.2 |
| Literal | `docker-compose.yml:14` `exasol/docker-db:2025.2.1` | Replaced by `${EXASOL_IMAGE:-exasol/docker-db:latest-2025.2}` |
| Parser | `crates/adbc-vs-adapter/tests/common/bench.rs:339` literal image-line parse | Replaced by `docker compose config --images` |
| CI artifact | `.github/workflows/ci.yml` single `adbc-vs-so` artifact | Replaced by one artifact per architecture |

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| driver-version-resolution: Missing platform package fails with a named error | Integration | `crates/adbc-vs-adapter/tests/provisioning_test.rs` | `missing_platform_names_driver_and_version`, `missing_platform_names_driver_and_version_for_postgresql` |
| driver-version-resolution: Missing platform package fails with a named error | Unit | `crates/adbc-vs-adapter/src/provisioning_tests.rs` | `version_without_the_platform_package_names_platform_and_version` |
| driver-version-resolution: Package platform follows the architecture the adapter library was compiled for | Integration | `crates/adbc-vs-adapter/tests/provisioning_test.rs` | `self_install_downloads_the_package_of_the_compile_targets_platform`, run by `make test` on the `ubuntu-latest` and `ubuntu-24.04-arm` `unit-tests` rows |
| driver-self-install: Release artifact bundles no driver binary | Integration | `crates/adbc-vs-adapter/tests/artifact_test.rs` | `no_bundled_driver_binary`, run by `make test-artifact` on both `build-so` rows |

### Manual Testing

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| driver-version-resolution | On an aarch64 host: `make cross-udf-build && file target/release/libadbc_vs_adapter.so && grep -c linux_arm64 target/release/libadbc_vs_adapter.so; grep -c linux_amd64 target/release/libadbc_vs_adapter.so` | `ELF 64-bit LSB shared object, ARM aarch64`, a count of at least 1 for `linux_arm64`, and `0` for `linux_amd64` |
| driver-version-resolution | On the x86_64 host: `make test-e2e` | 0 failures. The MySQL driver that `driver_lands_in_bucketfs` self-installed loads in the x86_64 UDF sandbox |
| driver-version-resolution | On an aarch64 host: `make test-it` | 0 failures, with the MySQL driver downloaded as the `linux_arm64` package and loaded through ADBC |
| driver-self-install | On each architecture: `make third-party-licenses release-tarball && tar -tzf dist/adbc-vs*.tar.gz` | `udf/`, `udf/libadbc_vs_adapter.so`, and `THIRD-PARTY-LICENSES.md` only. The file name is `adbc-vs.tar.gz` on x86_64 and `adbc-vs-linux-arm64.tar.gz` on aarch64 |
| driver-path-override | On an aarch64 host: `make test-e2e EXASOL_TARGET=personal` | 0 failures, against the aarch64 library, the aarch64 SLC, and the `linux_arm64` driver at the `supplied` location |

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Build | `make cross-udf-build` | Exit 0, including `cargo exasol-udf validate` |
| Test | `make test` | 0 failures |
| Artifact | `make test-artifact` | 0 failures |
| Integration | `make test-it`, `make test-it-postgres`, `make test-it-trino` | 0 failures |
| E2E | `make test-e2e`, `make test-e2e-postgres`, `make test-e2e-trino` with the defaults, with `EXASOL_IMAGE=exasol/docker-db:latest-2026.1`, and with `EXASOL_TARGET=personal` | 0 failures |
| Policy | `make deny` | Exit 0 |
| Lint | `make lint` | 0 warnings from both clippy runs |
| Format | `make fmt-check` | No changes |
