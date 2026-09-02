# Verification Report: add-postgresql-test-coverage

## Verdict

| Result | Details |
|--------|---------|
| **PASS** | All 10 checklist steps green; all 6 named scenarios have a passing test; all 5 manual verification steps confirmed against the live stack. |
| Code review | 6 findings — 6 fixed (4 standard, 2 expert) |

| Check | Status |
|-------|--------|
| Build (`make cross-udf-build`) | ✓ |
| Tests (unit + both integration tiers + both e2e tiers) | ✓ |
| Lint (`make lint`) | ✓ |
| Format (`make fmt-check`) | ✓ |
| Scenario Coverage | ✓ |
| Manual Tests | ✓ |
| Dependency policy (`make deny`) | ✓ |

## Test Evidence

### Test Results

| Type | Run | Passed | Ignored |
|------|-----|--------|---------|
| Unit (`make test`) | 655 | 655 | 0 |
| Integration, MySQL (`make test-it`) | 6 | 6 | 0 |
| Integration, PostgreSQL (`make test-it-postgres`) | 4 | 4 | 0 |
| E2E, PostgreSQL (`make test-e2e-postgres`) | 202 | 202 | 0 |
| E2E, MySQL (`make test-e2e`) | 215 | 215 | 0 |
| Dependency policy (`make deny` policy_test) | 9 | 9 | 0 |

All five tiers: 0 failed.

### Manual Tests

| Test | Result |
|------|--------|
| Identifier case — `SELECT "ItemId", "Label" FROM PGVS."MixedCase" ORDER BY "ItemId"` returns 2 seeded rows in source case | ✓ |
| Identifier case — `SYS.EXA_ALL_TABLES` for schema `PGVS` includes `MixedCase`, `items`, `orders`, no upper-cased spelling | ✓ |
| View exposure — `SELECT COUNT(*) FROM PGVS."order_names"` returns `3` | ✓ |
| Refresh — add `extra NUMERIC(9,3)` to `refresh_target`, `ALTER VIRTUAL SCHEMA PGVS REFRESH`, columns become `id`, `name`, `extra DECIMAL(9,3)` | ✓ |
| Refresh — restore: drop `extra`, refresh again, columns return to `id`, `name` | ✓ |
| Source failure — `SELECT * FROM no_such_table` against PostgreSQL raises `ERROR: relation "no_such_table" does not exist`, the diagnosis the ADBC error carries through (confirmed via `source_failure_reports_adbc_status`) | ✓ |

## Tool Evidence

### Linter

```
cargo clippy --workspace --all-targets --all-features -- -D warnings
Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.96s
0 warnings
```

### Formatter

```
cargo fmt --all -- --check
No changes
```

## Scenario Coverage

| Domain | Feature | Scenario | Test Location | Test Name | Passes |
|--------|---------|----------|---------------|-----------|--------|
| vs-adapter | create-virtual-schema | Source identifiers keep their original case, PostgreSQL driver | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `identifiers_keep_their_case_through_discovery_and_rendering` | Pass |
| vs-adapter | create-virtual-schema | Views are exposed alongside base tables, PostgreSQL driver | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `views_are_exposed_alongside_base_tables` | Pass |
| vs-adapter | adapter-request-dispatch | Refresh re-runs metadata discovery, PostgreSQL driver | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `refresh_picks_up_a_new_postgresql_column` | Pass |
| vs-adapter | adapter-request-dispatch | Refresh re-runs metadata discovery, MySQL driver | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` | `refresh_picks_up_new_column` (pre-existing, unchanged) | Pass |
| vs-adapter | adapter-request-dispatch | setProperties re-runs metadata discovery | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` | `all_request_types_return_matching_type` (pre-existing, unchanged) | Pass |
| scan-udf | arrow-scan | Source failure surfaces the driver's own diagnosis, PostgreSQL driver, statement-rejection half | `crates/adbc-vs-scan/tests/postgres_it_test.rs` | `source_failure_reports_adbc_status` | Pass |

## Notes

- Test-only plan: no product code changed, only test files, fixtures, and CLAUDE.md.
- Code review found 6 issues, all fixed before this report: an assertion that checked only half a conjunction (`views_are_exposed_alongside_base_tables`), a refresh test comparing declared vs. non-declared column reads, a stale work-tracking comment in `docker/postgres/init.sql`, an out-of-date module doc in `postgres_mirror.rs`, a `SEEDED_TABLES` name that no longer matched its contents (renamed to `SEEDED_OBJECTS`, one of the three is a view), and a missing doc comment on `Stack::postgres`.
- One environment note surfaced during implementation, not a plan or code defect: a stale Docker Compose stack from the main (non-worktree) `labs-adbc-vs` checkout held the hardcoded `172.28.5.0/24` subnet and had to be brought down before this worktree's own stack could come up. Unrelated to this branch's changes.
- Per the plan's Impact section, a developer holding a `postgres` container from before this change must recreate it (`docker compose rm -sfv postgres && make stack-up-postgres`) — done here, and now documented in CLAUDE.md.
