# Verification Report: add-arm64-support

## Verdict

| Result | Details |
|--------|---------|
| **PASS (x86_64 host), aarch64 pending CI** | Every check that can run on the local x86_64 host passes. The aarch64 build, the aarch64 test rows, and `personal-arm64` run only in CI and are not yet confirmed. |
| Code review | 10 findings, 10 fixed |

| Check | Status |
|-------|--------|
| Build | ✓ (`make cross-udf-build`, including `cargo exasol-udf validate`) |
| Tests | ✓ |
| Lint | ✓ (both clippy runs) |
| Format | ✓ |
| Scenario Coverage | ✓ on x86_64, aarch64 run pending CI |
| Manual Tests | partial: x86_64 rows pass, aarch64 rows not run |

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
| aarch64: `make cross-udf-build` and the `linux_arm64` string check | not run, needs an aarch64 host or the CI `build-so` row |
| aarch64: `make test-it` downloads the `linux_arm64` driver | not run, CI `integration-tests` row |
| aarch64: `make test-e2e EXASOL_TARGET=personal` | not run, CI `personal-arm64` rows |
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
| driver-provisioning | driver-version-resolution | Package platform follows the architecture the adapter library was compiled for | `crates/adbc-vs-adapter/tests/provisioning_test.rs` | `self_install_downloads_the_package_of_the_compile_targets_platform` | Pass on x86_64, aarch64 pending CI |
| driver-provisioning | driver-self-install | Release artifact bundles no driver binary | `crates/adbc-vs-adapter/tests/artifact_test.rs` | `no_bundled_driver_binary` | Pass on x86_64, aarch64 pending CI |

## Open Items

- The first CI run decides the `personal-arm64` rows. If Exasol Personal does not start on `ubuntu-24.04-arm`, fallback (b) applies: remove those rows and add `make test-e2e-arm64`.
- `make test-e2e-arm64` and the Apple silicon path do not exist yet and are unverified.
- Hosted runners are assumed to ship `jq`, and Podman is installed when absent. Neither is confirmed.
