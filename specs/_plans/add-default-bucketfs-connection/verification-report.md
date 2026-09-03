# Verification Report: add-default-bucketfs-connection

## Verdict

| Result | Details |
|--------|---------|
| **PASS** | All 9 implementation tasks complete, all review findings resolved, every automated checklist item and all 6 manual-testing rows pass. |
| Code review | 11 findings — 11 fixed (8 standard, 3 expert) |

| Check | Status |
|-------|--------|
| Build (`make cross-udf-build`) | ✓ |
| Tests (`make test`, `test-artifact`, `test-it`, `test-it-postgres`, `test-e2e`, `test-e2e-postgres`) | ✓ |
| Lint (`make lint`) | ✓ |
| Format (`make fmt-check`) | ✓ |
| Dependency policy (`make deny`) | ✓ |
| Scenario Coverage | ✓ |
| Manual Tests | ✓ (6/6) |

## Test Evidence

### Test Results

| Type | Run | Passed | Failed |
|------|-----|--------|--------|
| Unit + host integration (`make test`, 18 binaries) | 678 | 678 | 0 |
| Artifact (`make test-artifact`, against real `.so`) | 3 | 3 | 0 |
| Integration, MySQL (`make test-it`) | 6 | 6 | 0 |
| Integration, PostgreSQL (`make test-it-postgres`) | 3 | 3 | 0 |
| E2E, MySQL (`make test-e2e`, `--test-threads=1`) | 223 | 223 | 0 |
| E2E, PostgreSQL (`make test-e2e-postgres`, `--test-threads=1`) | 202 | 202 | 0 |
| **Total** | **1115** | **1115** | **0** |

### Manual Tests

| Test | Result |
|------|--------|
| Default connection (`ADBC_VS_BUCKETFS`, no property set) | ✓ |
| Explicit override never falls back | ✓ |
| No route at all (connection dropped, no pre-placed library) | ✓ |
| Pre-placed library answers when no connection resolves | ✓ |
| Pre-placed library answers when the registry is genuinely unreachable (real `iptables`-blocked egress, not a stub) | ✓ |
| Self-install never overwrites or removes the pre-placed file | ✓ |

Full command/output evidence: `specs/_plans/add-default-bucketfs-connection/manual-testing-evidence.md`. Row 5 is the one scenario no automated tier can produce (the plan states this coverage boundary explicitly): a real registry unreachable from inside the UDF sandbox, verified by blocking the Exasol container's outbound HTTPS and confirming the fallback fires — not a stubbed HTTP server standing in for an air gap.

## Tool Evidence

### Linter

```
make lint: cargo clippy --workspace --all-targets --all-features -- -D warnings
exit 0, 0 warnings
```

### Formatter

```
make fmt-check: exit 0, no changes
```

### Dependency policy

```
make deny: cargo deny check licenses advisories bans sources, plus policy_test.rs
exit 0, 0 findings
```

## Scenario Coverage

All scenarios and test mappings are per plan.md's own Scenario Coverage table (specs/_plans/add-default-bucketfs-connection/plan.md, lines ~318-348), spanning 4 changed features. Every named test function was confirmed to exist in the codebase by name (`grep`-verified against `crates/adbc-vs-adapter/src` and `crates/adbc-vs-adapter/tests`) and passes in the suite runs above. Highlights:

| Domain | Feature | Scenario | Test Location | Test Name | Passes |
|--------|---------|----------|---------------|-----------|--------|
| vs-adapter | create-virtual-schema | Missing required property fails with a named error (BUCKETFS_CONNECTION half removed) | `src/create_schema_tests.rs` | `a_statement_naming_no_bucketfs_connection_takes_the_default_route` | Pass |
| vs-adapter | create-virtual-schema | BucketFS write credential resolved by precedence (default level) | `tests/e2e_mysql_test.rs` | `default_bucketfs_connection_needs_no_property` | Pass |
| vs-adapter | create-virtual-schema | Explicit connection never falls back | `tests/e2e_mysql_test.rs`, `src/create_schema_tests.rs` | `an_explicit_bucketfs_connection_that_does_not_resolve_fails_naming_it`, `an_explicit_bucketfs_connection_never_falls_back` | Pass |
| driver-provisioning | driver-path-override | Default location used when no connection resolves | `tests/e2e_mysql_test.rs` | `a_preplaced_library_answers_when_no_bucketfs_connection_resolves` | Pass |
| driver-provisioning | driver-path-override | Default location used when registry answers nothing (narrowness clauses) | `src/provisioning_tests.rs`, `tests/provisioning_test.rs` | `only_a_registry_reached_not_at_all_is_unreachable`, `registry_unreachability_is_told_from_a_registry_that_answered`, `a_package_download_that_failed_mid_transfer_is_not_an_unreachable_registry`, `an_index_read_that_failed_mid_transfer_is_not_an_unreachable_registry` | Pass |
| driver-provisioning | driver-path-override | No driver on any route fails naming every route | `tests/e2e_mysql_test.rs`, `src/create_schema_tests.rs` | `a_preplaced_library_answers_when_no_bucketfs_connection_resolves` (first phase), `no_driver_route_names_the_location_the_connection_and_both_properties` | Pass |
| driver-provisioning | driver-self-install | Self-install writes only the versioned install path | `tests/provisioning_test.rs` | `self_install_writes_only_the_versioned_path_and_leaves_a_supplied_library_alone` | Pass |
| driver-provisioning | driver-version-resolution | Unreachable registry names the offline fallback (scoped to explicit route) | `tests/provisioning_test.rs` | `offline_fallback_names_the_drivers_own_path`, `offline_fallback_names_the_drivers_own_path_for_postgresql` | Pass |

(Full table of ~28 scenario rows is in plan.md itself; this report highlights representative rows per feature rather than reproducing all of them.)

## Notes

**Design evolution.** This plan went through 3 rounds of adversarial spec review before implementation (`review/round-1.md` through `round-3.md`), each resolving a maintainer-answered open question. Implementation then surfaced one additional real defect the spec-level review could not have caught (a BucketFS 423-lock race between two E2E tests writing the same path, fixed in the shared test harness) and code review surfaced 3 more (a redirect-loop/malformed-request gap in the provisioning-error taxonomy, a discarded error cause in `NoDriverRoute`, and a double-panic/`abort()` risk in two E2E `Drop` guards under a failing cleanup). All were found via induced-failure verification (a real panic and a real cleanup failure reproduced, not just code inspection) and fixed with new regression tests.

**Deliberate deviations from the plan's exact wording, both justified in code:**
- `SchemaError::NoDriverRoute`'s new `unreadable` field is `String`, not `Option<String>` as the review finding proposed — `default_location_library` is its only constructor and always supplies a cause on failure, so `None` would be unconstructible dead flexibility.
- `write_no_driver_route` now takes 5 arguments, above the project's ≤3 guideline — the alternative (a payload struct) was judged a wider refactor than the finding's scope warranted.

**Coverage boundary, stated rather than hidden (per the plan itself):** no automated tier can produce a genuinely unreachable driver registry from inside a UDF sandbox (the CDN host is compiled in, the E2E stack has working egress). This is covered by decision and outcome at the unit tier, by which real failure produces which error variant at the provisioning tier, and — for the first time with a real network condition rather than a stub — by manual-testing row 5.

**Stack state:** left clean after all testing — no dropped connections, no leftover `supplied` BucketFS objects, no iptables rules, egress restored, confirmed by a final throwaway-schema sanity check.

Ready for: `/speq:record add-default-bucketfs-connection`
