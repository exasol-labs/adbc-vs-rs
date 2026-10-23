# Verification Report: add-arm64-support

## Verdict

| Result | Details |
|--------|---------|
| **PASS** | Local x86_64 checks pass, and all 25 CI jobs on commit 096e648 pass, including the aarch64 build, lint, unit and integration rows and the three `personal-arm64` E2E rows. Fallback (b) was not needed. |
| Code review | 10 findings, 10 fixed |

| Check | Status |
|-------|--------|
| Build | ✓ (`make cross-udf-build`, including `cargo exasol-udf validate`) |
| Tests | ✓ |
| Lint | ✓ (both clippy runs) |
| Format | ✓ |
| Scenario Coverage | ✓ on both architectures |
| Manual Tests | ✓ x86_64 locally, aarch64 through CI |

## Test Evidence

### Test Results

Run after the review fixes, on the x86_64 host.

| Type | Command | Passed | Failed |
|------|---------|--------|--------|
| Unit and host integration | `make test` | 795 | 0 |
| Artifact | `make test-artifact` | 3 | 0 |
| Integration MySQL | `make test-it` | 6 | 0 |
| Integration PostgreSQL | `make test-it-postgres` | 4 | 0 |
| Integration Trino | `make test-it-trino` | 6 | 0 |
| E2E MySQL, Docker `latest-2025.2` | `make test-e2e` | 264 | 0 |
| E2E MySQL, Exasol Personal | `make test-e2e EXASOL_TARGET=personal` | 266 | 0 |
| Policy | `make deny` | exit 0 | |

Run before the review fixes (Group B, task 2.7), not repeated afterwards because the fixes changed no harness behavior beyond a compile-time refactor of the launcher helper:

| Command | Passed | Failed |
|---------|--------|--------|
| `make test-e2e-postgres` (Docker `latest-2025.2`, Docker `latest-2026.1`, Personal) | 245, 245, 254 | 0 |
| `make test-e2e-trino` (Docker `latest-2025.2`, Docker `latest-2026.1`, Personal) | 247, 247, 256 | 0 |
| `make test-e2e` (Docker `latest-2026.1`) | 264 | 0 |

### Manual Tests

| Test | Result |
|------|--------|
| x86_64: `make cross-udf-build` validates the `.so` | ✓ |
| x86_64: `make third-party-licenses release-tarball` packs `udf/libadbc_vs_adapter.so` and `THIRD-PARTY-LICENSES.md` only | ✓ |
| x86_64: `make test-e2e` loads the self-installed MySQL driver in the UDF sandbox | ✓ |
| aarch64: `make cross-udf-build` validates the `.so` | ✓ CI `build-so (ubuntu-24.04-arm)`. The `linux_arm64` string check was not run separately |
| aarch64: `make test-it`, `test-it-postgres`, `test-it-trino` | ✓ CI `integration-tests*` (ubuntu-24.04-arm) |
| aarch64: E2E against Exasol Personal | ✓ CI `e2e`, `e2e-postgres`, `e2e-trino` (personal-arm64) |
| Spike: a Personal UDF reaches the Compose static IPs (x86_64) | ✓, so fallback (a) does not apply |

## Tool Evidence

### Linter

`make lint` exit 0: first clippy run with every feature except `exasol-personal`, second run with `exasol-e2e,exasol-personal`.

### Formatter

`make fmt-check` exit 0.

## Scenario Coverage

| Domain | Feature | Scenario | Test Location | Test Name | Passes |
|--------|---------|----------|---------------|-----------|--------|
| driver-provisioning | driver-version-resolution | Missing platform package fails with a named error | `crates/adbc-vs-adapter/tests/provisioning_test.rs` | `missing_platform_names_driver_and_version`, `missing_platform_names_driver_and_version_for_postgresql` | Pass |
| driver-provisioning | driver-version-resolution | Missing platform package fails with a named error | `crates/adbc-vs-adapter/src/provisioning_tests.rs` | `version_without_the_platform_package_names_platform_and_version` | Pass |
| driver-provisioning | driver-version-resolution | Package platform follows the architecture the adapter library was compiled for | `crates/adbc-vs-adapter/tests/provisioning_test.rs` | `self_install_downloads_the_package_of_the_compile_targets_platform` | Pass on both architectures |
| driver-provisioning | driver-self-install | Release artifact bundles no driver binary | `crates/adbc-vs-adapter/tests/artifact_test.rs` | `no_bundled_driver_binary` | Pass on both architectures |

## Open Items

- The CI `release` and `package` jobs were skipped on the PR run, as designed. The first push to main with a bumped version runs them for the first time.
- `make test-e2e-arm64` does not exist because fallback (b) was not needed.
- The CI `package` job name shows `package (${{ matrix.runner }})` while skipped. This is cosmetic.
