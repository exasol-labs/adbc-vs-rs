# Tasks: add-arm64-support

Full task text: plan.md § Implementation Tasks (same numbering).


## Phase 2: Implementation (Group A: Registry platform from the compile target)

- [x] 1.1 Write the failing test `self_install_downloads_the_package_of_the_compile_targets_platform` in `crates/adbc-vs-adapter/tests/provisioning_
- [x] 1.2 In `crates/adbc-vs-scan/src/driver.rs`, select `PLATFORM` with `#[cfg(target_arch = "x86_64")]` (`linux_amd64`) and `#[cfg(target_arch = "aa
- [x] 1.3 Make every test that serves or expects a `linux_amd64` package pass on both architectures:
- [x] 1.4 Replace `PAGE_BYTES = 4096` in `crates/adbc-vs-scan/tests/mysql_it_test.rs:122-124` with the page size `getconf PAGESIZE` reports, so the m
- [x] 1.5 Run `make test`, `make test-it`, `make test-it-postgres`, and `make test-it-trino` on the x86_64 host.

## Phase 2: Implementation (Group B: Exasol target selection for the E2E tiers)

- [x] 2.1 Spike on the local x86_64 host, keeping no code (decision-log [3]):
- [x] 2.2 Add the Exasol target to the `Makefile`:
- [x] 2.3 Give `tests/common/exasol.rs` one `Exasol` type with two backends selected by `cfg(feature = "exasol-personal")`. [expert]
- [x] 2.4 In `tests/common/slc.rs`, choose the SLC asset by `target_arch` (`lc-rust-<V>.tar.gz` or `lc-rust-<V>-aarch64.tar.gz`) and update `release_u
- [x] 2.5 Give the harness one owner of the driver route (decision-log [2] and Review Findings):
- [x] 2.6 In `e2e_mysql_test.rs`, `e2e_postgres_test.rs`, and `e2e_trino_test.rs`, mark each test that needs a BucketFS write, the registry route, th
- [x] 2.7 Run `make test-e2e`, `make test-e2e-postgres`, and `make test-e2e-trino` three times on the local x86_64 host:

## Phase 2: Implementation (Group C: Per-architecture build, release, and CI)

- [x] 3.1 In the `Makefile`, read `uname -m` into `HOST_ARCH`, which a make command line can override, and name `RELEASE_TARBALL` after it:
- [x] 3.2 Add `aarch64-unknown-linux-gnu` to `targets` in `about.toml:23`, so one `THIRD-PARTY-LICENSES.md` covers both architectures.
- [x] 3.3 Rework `.github/workflows/ci.yml`:
- [x] 3.4 Update `crates/adbc-vs-adapter/tests/workflow_test.rs`:
- [~] 3.5 Run `make test`, `make cross-udf-build`, `make test-artifact`, and `make third-party-licenses release-tarball` on the x86_64 host.
  - Local x86_64 part done: `make test`, `make -B cross-udf-build`, `make test-artifact`, and `make third-party-licenses release-tarball` pass. Remaining for the orchestrator: push the branch, confirm the aarch64 and `personal-arm64` rows in CI, and apply fallback (b) only if Exasol Personal does not start on `ubuntu-24.04-arm`.

## Phase 2: Implementation (Group D: Docs, rules, and version)

- [x] 4.1 Update `docs/install.md`:
- [x] 4.2 Replace the x86_64-only line in `docs/how-it-works.md:33` (§ Limits, § Scope):
- [x] 4.3 Change `AGENTS.md`:
- [x] 4.4 In `specs/mission.md`, list Linux x86_64 and aarch64 clusters under Core Capabilities, change the Tech Stack testing row to "E2E against a l
- [x] 4.5 Bump the version from 0.6.0 to 0.7.0 in the root `Cargo.toml` and in the `adbc-vs-scan` dependency of `crates/adbc-vs-adapter/Cargo.toml`, a

## Phase 4: Review Fixes

- [x] 4.6 In `.github/workflows/ci.yml`, add a step named `Verify the release assets` between `Download the release tarballs and third-party notices` and `Create and push tag`, with `if: steps.gate.outputs.release == 'true'` and `run: ls -l dist/adbc-vs.tar.gz dist/adbc-vs-linux-arm64.tar.gz dist/THIRD-PARTY-LICENSES.md`, and add `fail_on_unmatched_files: true` under `with:` of the `Create GitHub Release` step. In `crates/adbc-vs-adapter/tests/workflow_test.rs`, extend `release_job_is_gated_and_packages_artifacts` to assert that `with.fail_on_unmatched_files` of the `softprops/action-gh-release` step is the YAML boolean `true` and that a step whose `run` names all three asset paths comes before the step whose `run` contains `git push origin`.
- [x] 4.7 In `Makefile`, change `HOST_ARCH ?= $(shell uname -m)` to `HOST_ARCH := $(shell uname -m)`. In `crates/adbc-vs-adapter/tests/make_targets_test.rs`, add the test `release_tarball_ignores_a_host_arch_from_the_environment`: it runs `make -n -B release-tarball` with `MAKEFLAGS` and `EXASOL_TARGET` removed and `HOST_ARCH=amd64` set in the child environment, and asserts success and that stdout contains `tar -czf {expected}` for `release_tarball_for` of the trimmed `uname -m` output.
- [x] 4.8 In `Makefile`, replace the three comment lines above `HOST_ARCH` with two lines: `# The .so loads only on the architecture it was built for, so each architecture packs its own tarball.` and `# The x86_64 asset keeps its unsuffixed name, so existing download links keep working.`
- [x] 4.9 In `AGENTS.md` line 26, replace the two sentences after "pass on x86_64 and aarch64." with the finding's replacement text on `PLATFORM` use in tests, the one named exception, and the data that may spell either platform.
- [x] 4.10 In `crates/adbc-vs-scan/src/driver.rs`, replace ``/// See the `x86_64` declaration.`` above the `aarch64` `PLATFORM` declaration with the full two-paragraph doc comment of the `x86_64` declaration.
- [x] 4.11 In `crates/adbc-vs-adapter/tests/common/exasol/personal.rs`, delete the method `Exasol::launcher`, make the free function `launcher` `pub` carrying the method's doc comment, re-export it from `tests/common/exasol.rs`, and call `launcher(` in `tests/common/slc/personal.rs` `install` instead of `exasol.launcher(`.
- [x] 4.12 In `crates/adbc-vs-adapter/tests/common/exasol.rs`, replace the sentence starting "Each backend discovers its own instance" in the module doc with the finding's text naming what each backend alone exports.
- [x] 4.13 In `crates/adbc-vs-adapter/tests/common/slc/personal.rs` `container_step`, replace `source.rsplit('/').next().unwrap_or(source)` with `source.rsplit_once('/').map_or(source, |(_, asset)| asset)`.
- [x] 4.14 In `crates/adbc-vs-adapter/tests/make_targets_test.rs`, add the test `an_empty_or_two_word_exasol_target_fails`: for each of `EXASOL_TARGET=` and `EXASOL_TARGET=docker personal`, `run_make(&["-n", "test", value])` fails and stderr contains `EXASOL_TARGET must be`.
- [x] 4.15 In `crates/adbc-vs-scan/tests/mysql_it_test.rs` `page_bytes`, assert `output.status.success()` right after `.output()` with a message holding the exit status and stderr, and parse stdout only afterwards.
