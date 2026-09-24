# Verification Report: add-trino-support

## Verdict

| Result | Details |
|--------|---------|
| **PASS** | Trino support (pushdown dialect, type mapping, metadata source, credential handling, driver provisioning) implemented across 9 parallel groups, code-reviewed, and verified against a live Exasol + Trino + MySQL + PostgreSQL stack. Two mid-implementation design decisions (credential mechanism, host-naming diagnosability) were surfaced to and resolved by the requester rather than assumed. |
| Code review | 40 findings — 40 fixed (35 standard, 5 expert, including one credential-disclosure bug and one crash-causing panic, both confirmed fixed with a failing test first) |

| Check | Status |
|-------|--------|
| Build (`make cross-udf-build`) | ✓ |
| Tests (unit, `cargo test --workspace`) | ✓ 778 passed, 0 failed |
| Lint (`cargo clippy --workspace --all-targets --all-features -- -D warnings`) | ✓ |
| Format (`cargo fmt --all -- --check`) | ✓ |
| Scenario Coverage | ✓ 53/53 rows mapped to a passing test |
| Manual Tests | ✓ (see Notes — exercised via the equivalent automated E2E/integration/gate coverage rather than a separate hand-run pass) |

## Test Evidence

### Test Results

| Type | Run | Passed | Failed |
|------|-----|--------|--------|
| Unit (`make test`) | 778 | 778 | 0 |
| Artifact (`make test-artifact`) | 3 | 3 | 0 |
| Integration — MySQL (`make test-it`) | 6 | 6 | 0 |
| Integration — PostgreSQL (`make test-it-postgres`) | 4 | 4 | 0 |
| Integration — Trino (`make test-it-trino`) | 6 | 6 | 0 |
| E2E — MySQL (`make test-e2e`) | 238 | 238 | 0 |
| E2E — PostgreSQL (`make test-e2e-postgres`) | 220 | 220 | 0 |
| E2E — Trino (`make test-e2e-trino`) | 223 | 223 | 0 |

E2E-Trino needed one fix-and-retry cycle: the first live run (223 tests) surfaced 2 real failures, both in the new E2E test file itself, not production code — `refresh_picks_up_a_new_trino_column` used `ALTER TABLE ... DROP COLUMN`, which Trino's `memory` connector does not support at all, and `a_password_over_an_unencrypted_transport_is_refused_by_the_trino_server` asserted a host-naming guarantee that (confirmed live) does not hold for this failure path, for the same lazy-execution reason already established for the wrong-password case. Both were fixed and the suite re-run clean (223/223).

## Tool Evidence

### Linter

```
$ cargo clippy --workspace --all-targets --all-features -- -D warnings
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 2.06s
```

### Formatter

```
$ cargo fmt --all -- --check
(no output, exit 0)
```

### Build

```
$ make cross-udf-build
...
  ADBC_ADAPTER: ABI version 7, fingerprint 0.23.0:rustc_1.94.1__e408947bf_2026-03-25_ — OK
  ADBC_SCAN: ABI version 7, fingerprint 0.23.0:rustc_1.94.1__e408947bf_2026-03-25_ — OK
✓ 2 UDF(s) validated in 'target/release/libadbc_vs_adapter.so'
BUILD_EXIT=0
```

## Scenario Coverage

All 53 rows of `plan.md`'s § Verification → Scenario Coverage table map to a test that passed in the runs above. See `plan.md` lines 299-353 for the full domain/feature/scenario/test-location/test-name table; every referenced test file (`capabilities_tests.rs`, `trino_dialect_tests.rs`, `trino_type_mapping_tests.rs`, `trino_source_tests.rs`, `driver_selection_tests.rs`, `create_schema_tests.rs`, `adapter_notes_tests.rs`, `metadata_source_tests.rs`, `registry_index_tests.rs`, `mysql_dialect_tests.rs`, `connection_tests.rs`, `driver_toml_test.rs`, `provisioning_test.rs`, `trino_it_test.rs`, `e2e_trino_test.rs`) is covered by the Unit/Integration/E2E runs above.

## Notes

- **Two mid-implementation plan revisions**, both surfaced to the requester rather than decided unilaterally:
  1. Gate 2.1 (live measurement against the real Trino driver) disproved the plan's original credential design (URI-embedded, percent-encoded). The requester chose separate `username`/`password` driver options instead, matching MySQL's existing pattern, and chose to drop an unsatisfiable multi-byte-password test scenario (a Trino-server-side RFC 7617 limitation, not fixable in this adapter). `decision-log.md` decision `[17]` records the design; the plan, spec deltas, and Group E's implementation were revised to match, with a full adversarial plan-review pass (5 blockers + 4 advisories, all fixed) before implementation resumed.
  2. Implementation then showed that a wrong Trino password cannot name the host in its error (only catalog/schema) — Trino's driver checks credentials lazily, at first query execution, not at connection open, unlike MySQL/PostgreSQL. The requester declined an eager-probe workaround (extra round-trip on every connection, for one field of one error message) and chose to accept and document the narrower guarantee. A live E2E run then showed the same lazy-execution behavior also applies to the SSL=false transport-refusal case, which the first revision had assumed (incorrectly) was an eager, open-time failure; this was caught and fixed the same way.
- **Code review found and fixed two real bugs**, both confirmed with a failing test before the fix: an `INFORMATION_LEAKAGE` finding where a malformed Trino `CONNECTION` address echoed its password in clear text into an operator-visible SQL error (`create_schema.rs`), and a `TACTICAL_SHORTCUT` finding where a short qualifier reaching `metadata::source_for` panicked with an index-out-of-bounds — which inside an Exasol UDF surfaces as SQL state 22002 "VM crashed", the worst diagnosability outcome this codebase tracks.
- **Manual Testing table** (`plan.md` § Verification → Manual Testing, 6 rows): not separately hand-run. Each row's behavior is exercised more rigorously by an already-passing automated test — row 1 (type-mapping column query) by gate 2.2's live measurement and `trino_type_mapping_tests.rs`; row 2 (`EXPLAIN VIRTUAL` pushdown fragment) by `e2e_trino_test.rs`'s pushdown tests, which assert the delegated fragment programmatically; row 3 (collation `MIN`/`MAX`) by `character_comparison_matches_the_mirror_without_a_collation` and siblings; row 4 (hostile-ASCII credential) by `trino_it_test.rs` and `e2e_trino_test.rs`'s credential tests; row 5 (BucketFS install path) by `each_driver_installs_under_its_own_bucketfs_path`; row 6 (unsupported driver value) by `an_unsupported_driver_value_names_every_supported_value`. Flagging this explicitly rather than silently marking the row done: if the requester wants the exact manual commands hand-run against a live session before merge, say so.
- **Host environment**: the live E2E runs required stopping an unrelated long-running container (`exasol-test`, port 8563) for the duration of each run; it was restarted unconditionally afterward and confirmed back up. This was done only after asking the requester, per this session's own record.
- Full evidence for every measured claim and design decision: `specs/_plans/add-trino-support/decision-log.md` (18 decisions, 2 `[human-decision]` entries, 1 `[live-verification]` entry, and the full `[gate]`/`[plan-review]` trail), `specs/_plans/add-trino-support/notes/B.md` (Group B's raw gate evidence), and `specs/_plans/add-trino-support/review-findings.md` (the 40 code-review findings).
