# Verification Report: change-pushdown-aggregation-sum-avg-having

## Verdict

| Result | Details |
|--------|---------|
| **PASS with one environmental exception** | MySQL, PostgreSQL, and Trino delegate `SUM`, `AVG`, and `HAVING`. All new and changed tests pass. One unrelated Trino e2e test fails on stale state in the shared local Trino. |
| Code review | 12 findings, 12 fixed |

| Check | Status |
|-------|--------|
| Build (`make cross-udf-build`, both UDFs validated) | ✓ |
| Tests (`make test`, `make deny`) | ✓ |
| E2E MySQL / PostgreSQL | ✓ (285 / 272 passed, 0 failed) |
| E2E Trino | ✗ 252 passed, 1 failed (see Notes) |
| Lint (`make lint`, both clippy runs) | ✓ |
| Format (`make fmt-check`) | ✓ |
| Scenario Coverage | ✓ |
| Manual Tests | ✓ covered by e2e fragment assertions (see Notes) |

## Test Evidence

### Test Results

| Type | Run | Passed | Failed |
|------|-----|--------|--------|
| Unit (adapter lib) | 465 | 465 | 0 |
| E2E MySQL | 285 | 285 | 0 |
| E2E PostgreSQL | 272 | 272 | 0 |
| E2E Trino | 253 | 252 | 1 (`refresh_picks_up_a_new_trino_column`) |

### Manual Tests

Each manual-testing row checks a delegated fragment through `EXPLAIN VIRTUAL` or the returned rows. The new e2e tests assert the same two things (`pushdown_statement` / `generated_statement` for the fragment, `assert_matches_mirror*` for the rows) against the live stack.

| Test | Result |
|------|--------|
| MySQL SUM, AVG, HAVING fragment and rows | ✓ |
| PostgreSQL SUM (DOUBLE cast), AVG quotient, HAVING, boolean and character operand conversion | ✓ |
| Trino SUM (DECIMAL(38) cast), AVG, HAVING | ✓ |
| Documented workaround `SUM("price" + 0)` stays in Exasol | not run as a separate command |

## Scenario Coverage

Every scenario in the plan's table maps to a test that passes: the dialect unit tests (`sum_renders_with_and_without_distinct`, `avg_renders_as_the_double_quotient_of_sum_and_count`, `sum_and_avg_convert_boolean_and_character_operands`, `having_renders_after_group_by_and_on_a_single_group`, `mixed_case_function_names_render_identically`), the exact-set tests in `capabilities_tests.rs`, and the e2e tests `sum_and_avg_are_delegated_and_match_the_mirror`, `sum_and_avg_over_boolean_and_character_columns_match_the_mirror`, `having_is_delegated_and_matches_the_mirror`, `sum_over_a_real_column_accumulates_in_double_precision`, `a_nan_operand_makes_the_delegated_sum_and_avg_null`, `sum_over_a_bigint_column_sums_past_the_bigint_range`.

## Notes

- `refresh_picks_up_a_new_trino_column` fails with "Column 'extra' already exists". An earlier interrupted run left the column in the shared Trino `memory.adbc_vs.refresh_target`, and the memory connector cannot drop columns. It does not touch this change. Clearing it needs the table recreated (`make stack-up-trino` on a fresh volume). CI starts from a fresh stack.
- Known gap, accepted by the user: a delegated `AVG` fails on MySQL older than 8.0.17 (`CAST(... AS DOUBLE)`). No test covers it (test MySQL is 8.4). It is documented in `docs/pushdown.md`.
- MySQL reads a non-numeric string as its numeric prefix or 0 in a delegated `SUM`/`AVG`, where Exasol fails. Documented.
- The `.speq` hook and the `docker compose` project name: e2e from this worktree needs `COMPOSE_PROJECT_NAME=labs-adbc-vs`.
