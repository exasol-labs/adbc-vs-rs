# Code Review Findings: add-arm64-support

## Summary
- Files reviewed: 38
- Total findings: 10 (standard: 10, expert: 0)

## Standard fixes

### .github/workflows/ci.yml

#### [SWALLOWED_ERROR] The release pushes a tag and publishes even when a tarball artifact did not arrive
- Location: lines 418-444 (`release` job: `Download the release tarballs and third-party notices`, `Create and push tag`, `Create GitHub Release`)
- Issue: The tarballs now reach `release` as artifacts that `package` uploads and `release` downloads by the pattern `package-*`. Before this change, `release` built the tarball in the same job, so the file always existed. `softprops/action-gh-release` defaults `fail_on_unmatched_files` to `false`, so a `files` entry that matches nothing only logs a warning. If an artifact name or upload path drifts, the job pushes the tag and publishes a release without `dist/adbc-vs.tar.gz` or `dist/adbc-vs-linux-arm64.tar.gz`, and CI stays green. The pushed tag then makes every later run's gate report "already released", so the missing asset is never published. `release_job_is_gated_and_packages_artifacts` checks only that the `files` list names the three assets.
- Fix: In `.github/workflows/ci.yml`, add a step named `Verify the release assets` between `Download the release tarballs and third-party notices` and `Create and push tag`, with `if: steps.gate.outputs.release == 'true'` and `run: ls -l dist/adbc-vs.tar.gz dist/adbc-vs-linux-arm64.tar.gz dist/THIRD-PARTY-LICENSES.md`. Add `fail_on_unmatched_files: true` under `with:` of the `Create GitHub Release` step. In `crates/adbc-vs-adapter/tests/workflow_test.rs`, extend `release_job_is_gated_and_packages_artifacts` to assert two things: the `softprops/action-gh-release` step's `with.fail_on_unmatched_files` is the YAML boolean `true` (`Value::as_bool` returns `Some(true)`), and a step whose `run` names all three asset paths comes before the step whose `run` contains `git push origin`.

### Makefile

#### [AMBIENT_STATE_READ] An exported `HOST_ARCH` overrides `uname -m` and breaks every target
- Location: line 195
- Issue: `HOST_ARCH ?= $(shell uname -m)` also takes `HOST_ARCH` from the environment, while plan task 3.1 names only the make command line. `HOST_ARCH=amd64 make -n fmt-check` fails with "HOST_ARCH must be `x86_64`, `aarch64`, or `arm64`, but is `amd64`", because the `$(error)` runs at parse time for every target. `?=` also defines a recursively expanded variable, so `uname -m` runs again at each expansion. A command-line value overrides a `:=` assignment too, so `?=` adds only the environment path.
- Fix: In `Makefile`, change `HOST_ARCH ?= $(shell uname -m)` to `HOST_ARCH := $(shell uname -m)`. In `crates/adbc-vs-adapter/tests/make_targets_test.rs`, add the test `release_tarball_ignores_a_host_arch_from_the_environment`. It runs `make -n -B release-tarball` in `workspace_root()` with `MAKEFLAGS` and `EXASOL_TARGET` removed from the child environment and `HOST_ARCH=amd64` set in it (`Command::env`). It asserts that the run succeeds and that stdout contains `tar -czf {expected}`, where `expected` is `release_tarball_for` of the trimmed `uname -m` output.

#### [REDUNDANT_COMMENT] The `HOST_ARCH` comment narrates history and restates make semantics
- Location: lines 192-194
- Issue: AGENTS.md limits a comment to 1 or 2 lines that state a non-obvious why, and it forbids restating code or narrating history. "The x86_64 name predates the aarch64 asset and stays unchanged" narrates history. "A make command line can override HOST_ARCH" restates how make treats every variable.
- Fix: In `Makefile`, replace the three comment lines above `HOST_ARCH` with these two lines: `# The .so loads only on the architecture it was built for, so each architecture packs its own tarball.` and `# The x86_64 asset keeps its unsuffixed name, so existing download links keep working.`

### AGENTS.md

#### [OUTDATED_COMMENT] The platform-literal rule forbids literals that the tests use
- Location: line 26
- Issue: The rule says that only `self_install_downloads_the_package_of_the_compile_targets_platform` and tests that pass a platform to `registry_index` spell `linux_amd64` or `linux_arm64`. Other tests spell them too. `OTHER_LINUX_PLATFORM` in `crates/adbc-vs-adapter/tests/provisioning_test.rs:55-57` feeds `ensure_driver`. `OTHER_LINUX_PLATFORM` in `crates/adbc-vs-adapter/src/provisioning_tests.rs:182-184` feeds `select_package`. `PACKAGE_URL` in `src/provisioning_tests.rs:67` labels an extraction error. `tests/fixtures/index.yaml` publishes both platforms. None of these is the platform the running binary requests, which is what plan task 4.3 restricts.
- Fix: In `AGENTS.md` line 26, replace the two sentences after "pass on x86_64 and aarch64." with: "Test code takes the platform the running binary requests from `adbc_vs_scan::driver::PLATFORM` and never spells it as a literal. The exception is `self_install_downloads_the_package_of_the_compile_targets_platform`, which spells `linux_amd64` and `linux_arm64` per `#[cfg(target_arch)]`, so that the mapping itself is under test. The other Linux platform, fixture indexes, and URLs that only label an error are data and may spell either platform."

### crates/adbc-vs-scan/src/driver.rs

#### [MISSING_DESIGN_INTENT] The aarch64 `PLATFORM` doc points at a declaration its build does not have
- Location: line 184
- Issue: rustdoc renders only the declaration of the compiled architecture. On an aarch64 build, the public `PLATFORM` doc reads "See the `x86_64` declaration.", and that build neither compiles nor renders the `x86_64` declaration. The rationale, that the compile target is already the cluster's architecture, appears only in x86_64 builds.
- Fix: In `crates/adbc-vs-scan/src/driver.rs`, replace ``/// See the `x86_64` declaration.`` above the `#[cfg(target_arch = "aarch64")]` declaration with the full two-paragraph doc comment of the `#[cfg(target_arch = "x86_64")]` declaration. Keep both `#[cfg]` declarations and the `compile_error!`.

### crates/adbc-vs-adapter/tests/common/exasol/personal.rs

#### [SHALLOW_MODULE] `Exasol::launcher` forwards to a free function and ignores `self`
- Location: lines 82-87 (method), 187-191 (free function)
- Issue: The public method `Exasol::launcher(&self, description, args)` has the body `launcher(description, args)` and never reads `self`. The deployment comes from the `DEPLOYMENT` constant inside the free function. The method makes `slc/personal.rs` read as if it addressed this instance. The free function already does the work, and `discover` must call it before any `Exasol` exists.
- Fix: In `crates/adbc-vs-adapter/tests/common/exasol/personal.rs`, delete the method `launcher` from the `impl Exasol` block, make the free function `launcher` `pub`, and move the method's doc comment onto it. In `crates/adbc-vs-adapter/tests/common/exasol.rs`, change `pub use personal::DEPLOYMENT as PERSONAL_DEPLOYMENT;` to `pub use personal::{launcher, DEPLOYMENT as PERSONAL_DEPLOYMENT};`. In `crates/adbc-vs-adapter/tests/common/slc/personal.rs`, change the import to `use super::super::exasol::{launcher, Exasol, PERSONAL_DEPLOYMENT};` and replace both `exasol.launcher(` calls in `install` with `launcher(`.

### crates/adbc-vs-adapter/tests/common/exasol.rs

#### [OUTDATED_COMMENT] The module doc says both backends serve the same methods and nothing names a backend
- Location: lines 7-10
- Issue: The doc says that both backends "serve the same methods, so nothing else in the harness spells out a host, a port, a credential, or a BucketFS layout, or names a backend". The Docker backend alone serves `bucketfs_write_url`, `bucketfs_write_password`, and `BUCKETFS_WRITE_USER`. The Personal backend alone serves `launcher` and `PERSONAL_DEPLOYMENT`. `schema.rs`, `slc.rs`, and the three `e2e_*_test.rs` files select code with `cfg(feature = "exasol-personal")`.
- Fix: In `crates/adbc-vs-adapter/tests/common/exasol.rs`, replace the sentence that starts "Each backend discovers its own instance" with: "Each backend discovers its own instance and owns its BucketFS route. Both serve the same SQL and BucketFS methods, so no caller spells out a host, a port, a credential, or a BucketFS layout. Each backend also exports what only its own route needs: the Docker backend the BucketFS write endpoint, user, and password that self-install writes through, and the Personal backend its deployment name and launcher."

### crates/adbc-vs-adapter/tests/common/slc/personal.rs

#### [UNREACHABLE_CODE] `unwrap_or(source)` after `rsplit('/').next()` never applies
- Location: line 91
- Issue: `str::rsplit` yields at least one item for every input, including the empty string. `next()` is therefore always `Some`, and the `unwrap_or(source)` branch cannot run.
- Fix: In `container_step`, replace `let asset = source.rsplit('/').next().unwrap_or(source);` with `let asset = source.rsplit_once('/').map_or(source, |(_, asset)| asset);`.

### crates/adbc-vs-adapter/tests/make_targets_test.rs

#### [MISSING_BOUNDARY_TEST] The empty and two-word cases of the `EXASOL_TARGET` guard are untested
- Location: line 535 (`an_unknown_exasol_target_fails_naming_both_targets`)
- Issue: `Makefile:18` concatenates the unknown words with the word count, so that only one known word passes. That construction exists for an empty value and for two known words, and no test covers either case. A dry run shows that both fail today: `make -n fmt-check EXASOL_TARGET=` and `make -n fmt-check "EXASOL_TARGET=docker personal"`. A rewrite to `ifeq ($(filter $(EXASOL_TARGET),docker personal),)` would accept `docker personal` and keep every current test green.
- Fix: In `crates/adbc-vs-adapter/tests/make_targets_test.rs`, add the test `an_empty_or_two_word_exasol_target_fails`. For each of `EXASOL_TARGET=` and `EXASOL_TARGET=docker personal`, passed as one argument each, call `run_make(&["-n", "test", value])` and assert that the exit status is not success and that stderr contains `EXASOL_TARGET must be`.

### crates/adbc-vs-scan/tests/mysql_it_test.rs

#### [CONTEXTLESS_ERROR] `page_bytes` ignores the exit status of `getconf`
- Location: lines 124-137
- Issue: `page_bytes` parses the standard output of `getconf PAGESIZE` without checking `output.status`. When `getconf` exits non-zero, stdout is empty and the panic reads "`getconf PAGESIZE` prints a number: ParseIntError { kind: Empty }". The panic drops the exit status and the standard error, which state why the call failed.
- Fix: In `page_bytes`, directly after `.output()`, assert `output.status.success()` with a message formatted as ``"`getconf PAGESIZE` exited with {}: {}"`` from `output.status` and `String::from_utf8_lossy(&output.stderr)`, and parse stdout only after that assertion.

## Expert fixes
[none]
