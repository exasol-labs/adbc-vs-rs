# Verification Report: remove-pushdown-collation-and-like-escape

## Verdict

| Result | Details |
|--------|---------|
| **PASS with one unrelated failing check** | All 22 plan tasks and 8 review fixes are done. Format, lint, unit tests, the UDF build, the artifact check, and all three E2E targets pass. `make deny` fails on five RustSec advisories against dependencies this change does not touch. |
| Code review | 8 findings, 8 fixed |

| Check | Status |
|-------|--------|
| Build (`make cross-udf-build`) | ✓ |
| Tests (`make test`, `make test-artifact`, three E2E targets) | ✓ |
| Lint (`make lint`) | ✓ |
| Format (`make fmt-check`) | ✓ |
| Policy (`make deny`) | ✗ advisories only, see Notes |
| Scenario Coverage | ✓ |
| Manual Tests | ✓ |

## Test Evidence

### Test Results

Counts are the sum of every `test result` line in each target's log, so they include every test binary the target runs.

| Target | Passed | Failed | Ignored |
|--------|--------|--------|---------|
| `make test` | 781 | 0 | 0 |
| `make test-artifact` | 3 | 0 | 0 |
| `make test-e2e` | 264 | 0 | 0 |
| `make test-e2e-postgres` | 245 | 0 | 0 |
| `make test-e2e-trino` | 247 | 0 | 0 |

The new E2E tests ran and passed:
- `comparison_and_grouping_follow_the_columns_own_collation`
- `a_comparison_over_a_wrapped_operand_is_evaluated_by_exasol`
- `trailing_space_values_compare_and_order_as_in_exasol`
- `select_list_predicate_over_grouped_character_column_delegates_and_matches_the_mirror`
- `ordering_and_min_max_follow_the_columns_own_collation`
- `an_ordering_comparison_over_a_wrapped_operand_is_evaluated_by_exasol`
- `no_generated_statement_carries_a_collation_keyword`
- `like_with_an_explicit_escape_clause_is_evaluated_by_exasol`

### Manual Tests

Run against the live virtual schemas left by the E2E targets.

| Test | Result |
|------|--------|
| MySQL `GROUP BY "category"` over `items` returns 2 groups (`Widget` 4, `Gizmo` 4) | ✓ |
| Same over `items_bin` returns 4 groups | ✓ |
| `WHERE "category" \|\| '' = 'widget'` over `items` returns only the `widget` rows | ✓ |
| PostgreSQL `MIN`/`MAX("sort_key")` over `items` returns `a`, `C` | ✓ |
| Same over `items_c` returns `B`, `a` | ✓ |
| Trino `LIKE 'a%'` pushdown SQL holds the `LIKE` and no `ESCAPE` | ✓ |
| Trino `LIKE 'a!%' ESCAPE '!'` pushdown SQL holds no `LIKE` | ✓ |

## Tool Evidence

### Linter

`make lint` (clippy with `-D warnings`, all features) exits 0.

### Formatter

`make fmt-check` exits 0.

## Scenario Coverage

Every scenario in `plan.md` § Verification › Scenario Coverage maps to a test in the table below, and each test passed in the runs above.

| Domain | Feature | Scenario | Test Location | Test Name | Passes |
|--------|---------|----------|---------------|-----------|--------|
| vs-adapter | mysql-collation | No rendered operand carries a collation suffix | `src/mysql_dialect_tests.rs` | `no_operand_carries_a_collation_suffix_in_any_position` | Pass |
| vs-adapter | mysql-collation | A delegated comparison and grouping follow the column's own collation | `tests/e2e_mysql_test.rs` | `comparison_and_grouping_follow_the_columns_own_collation` | Pass |
| vs-adapter | mysql-collation | A column declaring utf8mb4_0900_bin returns Exasol's answers | `tests/e2e_mysql_test.rs` | `filter_matches_local_exasol_result`, `aggregation_matches_local_exasol_result`, `grouping_over_character_variants_matches_local_exasol_result`, `trailing_space_values_compare_and_order_as_in_exasol` | Pass |
| vs-adapter | mysql-collation | A comparison over a wrapped operand is evaluated by Exasol | `tests/e2e_mysql_test.rs` | `a_comparison_over_a_wrapped_operand_is_evaluated_by_exasol` | Pass |
| vs-adapter | postgresql-collation | No rendered operand carries a collation suffix | `src/postgres_dialect_tests.rs` | `no_operand_carries_a_collation_suffix_in_any_position` | Pass |
| vs-adapter | postgresql-collation | A delegated ordering comparison and MIN or MAX follow the column's own collation | `tests/e2e_postgres_test.rs` | `ordering_and_min_max_follow_the_columns_own_collation`, `no_generated_statement_carries_a_collation_keyword` | Pass |
| vs-adapter | postgresql-collation | An ordering comparison over a wrapped operand is evaluated by Exasol | `tests/e2e_postgres_test.rs` | `an_ordering_comparison_over_a_wrapped_operand_is_evaluated_by_exasol` | Pass |
| vs-adapter | trino-pushdown | Capability set advertises and withholds the right fragments | `src/capabilities_tests.rs` | `trino_capability_set_is_exact`, `trino_and_postgresql_sets_differ_by_two_named_entries` | Pass |
| vs-adapter | trino-pushdown | LIKE renders its pattern with no escape clause | `src/trino_dialect_tests.rs` | `like_renders_its_pattern_with_no_escape_clause`, `a_like_carrying_an_escape_char_fails_naming_the_capability` | Pass |
| vs-adapter | trino-pushdown | A LIKE carrying an explicit ESCAPE clause is evaluated by Exasol | `tests/e2e_trino_test.rs` | `like_with_an_explicit_escape_clause_is_evaluated_by_exasol` | Pass |

## Notes

- **`make deny` fails.** Five advisories (RUSTSEC-2024-0449, RUSTSEC-2026-0346, -0347, -0348, -0349) hit dependencies such as rustls and `unsafe-libyaml`. This change touches no `Cargo.toml`, `Cargo.lock`, or `deny.toml`, so the failure comes from the advisory database. No ignore entry was added. Adding one is a policy decision for the maintainer.
- **Cross-collation comparison, measured on the local containers.** A comparison between two character columns that declare different collations now reaches the source without a `COLLATE` suffix. MySQL 8.4 accepted `utf8mb4_0900_ai_ci` column = `utf8mb4_0900_bin` column and returned 1. PostgreSQL rejected both `=` and `<` between an `en_US` column and a `C` column with "could not determine which collation to use for string comparison". The removed suffix used to hide this. No test covers it.
- **README.** The maintainer's uncommitted README rewrite was discarded at their request. The plan's Limits text was added to the committed README.
- **Local environment.** To run the E2E targets, the container `exasol-test` was stopped (not removed, not restarted) and the empty Docker network `labs-adbc-vs-4_adbc-vs` was removed, both with the maintainer's consent.
