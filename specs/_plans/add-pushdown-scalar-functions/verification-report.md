# Verification Report: add-pushdown-scalar-functions

## Verdict

| Result | Details |
|--------|---------|
| **PASS** | Every dialect delegates the eleven scalar functions (no `TRIM` at MySQL) and grouping by an expression; unit, integration, and all three E2E suites pass |
| Code review | 10 findings — 10 fixed |

| Check | Status |
|-------|--------|
| Build | ✓ |
| Tests | ✓ |
| Lint | ✓ |
| Format | ✓ |
| Scenario Coverage | ✓ |
| Manual Tests | ✓ |

## Test Evidence

### Coverage

| Type | Coverage % |
|------|------------|
| Unit | not measured (no coverage tool in the toolchain) |
| Integration | not measured |

### Test Results

| Type | Run | Passed | Ignored |
|------|-----|--------|---------|
| Unit (`make test`) | 882 | 882 | 0 |
| Integration (`make test-it test-it-postgres test-it-trino`) | 16 | 16 | 0 |
| E2E MySQL (`make test-e2e`) | 299 | 299 | 0 |
| E2E PostgreSQL (`make test-e2e-postgres`) | 287 | 287 | 0 |
| E2E Trino (`make test-e2e-trino`) | 267 | 267 | 0 |

### Manual Tests

| Test | Result |
|------|--------|
| `EXPLAIN VIRTUAL SELECT COUNT(*) FROM PGVS."scalars" WHERE "i" * 2 > 10` | ✓ statement `SELECT COUNT(*) FROM "public"."scalars" WHERE (10 < (CAST("i" AS numeric) * CAST(2 AS numeric)))`; the query returns one row (2) |
| `EXPLAIN VIRTUAL SELECT YEAR("dt"), COUNT(*) FROM MYSQLVS."scalars" GROUP BY YEAR("dt")` | ✓ statement carries `GROUP BY YEAR(`dt`)`; rows equal the mirror's (E2E `group_by_year_reaches_mysql_and_matches_the_mirror`) |
| `SELECT LENGTH("s"), "dt" + 1 FROM MYSQLVS."scalars"` for `s = 'é'` and `dt = 2024-02-29` | ✓ `1` and `2024-03-01` |
| `SELECT TRIM("s", 'x') FROM PGVS."scalars"` for `s = 'xxaxx'` | ✓ `a` |
| `EXPLAIN VIRTUAL SELECT TRIM("s") FROM MYSQLVS."scalars"` | ✓ statement `SELECT `s` FROM ...` carries no `TRIM` |
| `SELECT "ts" + 1.5 FROM TRVS."scalars"` for `ts = 2024-01-01 00:00:00` | ✓ `2024-01-03 00:00:00`, as Exasol rounds 1.5 days to 2 |
| `SELECT "ts" - "ts2" FROM PGVS."scalars"` against the mirror | ✓ `MINUS` over both returns 0 rows; `EXPLAIN VIRTUAL` shows `CAST(NUMTODSINTERVAL("c1", 'SECOND') AS INTERVAL DAY(9) TO SECOND(9))` around the scan |

## Tool Evidence

### Linter

```
make lint: both clippy runs (all features except exasol-personal; exasol-e2e,exasol-personal) with -D warnings
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 3.05s
exit 0
```

### Formatter

```
cargo fmt --all -- --check
exit 0
```

## Scenario Coverage

| Domain | Feature | Scenario | Test Location | Test Name | Passes |
|--------|---------|----------|---------------|-----------|--------|
| vs-adapter | pushdown-scalar-function | Each dialect advertises the scalar functions it renders | `src/capabilities_tests.rs` | `each_dialect_advertises_the_scalar_functions_it_renders` | Pass |
| vs-adapter | pushdown-scalar-function | A scalar function renders in every position Exasol delegates it to | `src/pushdown_query_tests.rs` | `scalar_function_renders_in_every_position_per_dialect`, `an_unrendered_scalar_function_is_reported_by_name` | Pass |
| vs-adapter | pushdown-scalar-function | Date parts match Exasol over DATE and TIMESTAMP columns | `tests/e2e_{mysql,postgres,trino}_test.rs` | `date_parts_reach_<source>_and_match_the_mirror` | Pass |
| vs-adapter | pushdown-scalar-function | Exact-numeric arithmetic is exact and does not overflow before Exasol does | `tests/e2e_{mysql,postgres,trino}_test.rs` | `exact_arithmetic_reaches_<source>_and_matches_the_mirror` | Pass |
| vs-adapter | pushdown-scalar-function | Floating-point and character arithmetic converts as Exasol converts | `tests/e2e_{mysql,postgres,trino}_test.rs` | `double_and_character_arithmetic_matches_the_mirror` | Pass |
| vs-adapter | pushdown-scalar-function | Date and timestamp arithmetic follows Exasol's day arithmetic | `tests/e2e_{mysql,postgres,trino}_test.rs` | `date_arithmetic_reaches_<source>_and_matches_the_mirror` | Pass |
| vs-adapter | pushdown-scalar-function | A select-list TIMESTAMP difference is converted to an interval in Exasol | `src/pushdown_tests.rs`, `tests/e2e_{mysql,postgres,trino}_test.rs` | `interval_entry_is_converted_in_an_outer_select`, `timestamp_difference_matches_the_mirror` | Pass |
| vs-adapter | pushdown-scalar-function | String functions match Exasol over VARCHAR text | `tests/e2e_{mysql,postgres,trino}_test.rs` | `string_functions_reach_<source>_and_match_the_mirror` | Pass |
| vs-adapter | pushdown-scalar-function | SUBSTR positions follow Exasol's rule | `tests/e2e_{mysql,postgres,trino}_test.rs` | `substr_positions_match_the_mirror` | Pass |
| vs-adapter | pushdown-scalar-function | An empty string function result is null, as in Exasol | `tests/e2e_{mysql,postgres,trino}_test.rs` | `empty_string_function_result_is_null_as_in_exasol` | Pass |
| vs-adapter | pushdown-scalar-function | String function results follow the source for padding and case mapping | `tests/e2e_{mysql,postgres,trino}_test.rs` | `string_function_results_follow_the_source` (MySQL, PostgreSQL), `upper_over_a_non_ascii_letter_follows_the_source` (all three) | Pass |
| vs-adapter | pushdown-scalar-function | An implicitly converted operand converts as Exasol converts it | `tests/e2e_{mysql,postgres,trino}_test.rs` | `implicitly_converted_operands_match_the_mirror` | Pass |
| vs-adapter | pushdown-declared-types | Declared type outside the declarable set fails with a named error | `src/exa_type_tests.rs` | `day_to_second_interval_select_list_entry_reads_as_an_interval`, `year_to_month_interval_select_list_entry_fails_naming_it` | Pass |
| vs-adapter | pushdown-aggregation-delegation | An aggregation Exasol cannot delegate whole is computed by Exasol | `tests/e2e_mysql_test.rs` | `aggregation_not_delegable_whole_is_withdrawn_and_matches_local_exasol_result` | Pass |
| vs-adapter | pushdown-aggregation-delegation | Grouping by a scalar expression is delegated by every dialect | `tests/e2e_{mysql,postgres,trino}_test.rs`, `src/mysql_dialect_tests.rs` | `group_by_year_reaches_<source>_and_matches_the_mirror`, `an_expression_over_a_grouped_expression_reads_through_any_value` | Pass |
| vs-adapter | pushdown-dialect | The declarable set is closed under the advertised capabilities | `tests/e2e_postgres_test.rs`, `src/capabilities_tests.rs` | `scalar_function_declared_types_stay_in_the_declarable_set`, `every_scalar_function_renders_over_each_admitted_operand_class` | Pass |
| vs-adapter | mysql-pushdown | Capability set advertises the fragments this dialect renders | `src/capabilities_tests.rs` | `mysql_capability_set_is_exact` | Pass |
| vs-adapter | mysql-pushdown | Aggregation capabilities cover single-group and column grouping | `src/capabilities_tests.rs` | `mysql_capability_set_is_exact` | Pass |
| vs-adapter | mysql-pushdown | String functions render with MySQL's character semantics | `src/mysql_dialect_tests.rs` | `string_functions_render_with_mysql_semantics` | Pass |
| vs-adapter | mysql-pushdown | Arithmetic renders with MySQL's day and decimal semantics | `src/mysql_dialect_tests.rs` | `arithmetic_renders_with_mysql_semantics` | Pass |
| vs-adapter | postgresql-pushdown | Capability set advertises the fragments this dialect renders | `src/capabilities_tests.rs` | `postgres_capability_set_is_exact` | Pass |
| vs-adapter | postgresql-pushdown | Scalar functions render in PostgreSQL's forms | `src/postgres_dialect_tests.rs` | `scalar_functions_render_in_postgresql_forms` | Pass |
| vs-adapter | trino-pushdown | Capability set advertises the fragments this dialect renders | `src/capabilities_tests.rs` | `trino_capability_set_is_exact` | Pass |
| vs-adapter | trino-pushdown | Capability set withholds the fragments the Trino dialect does not delegate | `src/capabilities_tests.rs` | `trino_capability_set_is_exact`, `each_dialect_advertises_the_scalar_functions_it_renders` | Pass |
| vs-adapter | trino-pushdown | Scalar functions render in Trino's forms | `src/trino_dialect_tests.rs` | `scalar_functions_render_in_trino_forms` | Pass |
| vs-adapter | mysql-collation | A comparison over a wrapped operand is evaluated by Exasol | `tests/e2e_mysql_test.rs` | `a_comparison_over_a_wrapped_operand_is_evaluated_by_exasol` | Pass |
| vs-adapter | postgresql-collation | An ordering comparison over a wrapped operand is evaluated by Exasol | `tests/e2e_postgres_test.rs` | `an_ordering_comparison_over_a_wrapped_operand_is_evaluated_by_exasol` | Pass |

## Notes

- Measured on the test Exasol, and corrected in the plan's spec deltas: Exasol rounds the number of days added to a `DATE` or `TIMESTAMP` half away from zero to whole days (`TIMESTAMP '2024-01-01 00:00:00' + 1.5` is `2024-01-03 00:00:00`), so every dialect renders whole days rather than fractional ones.
- Measured: Exasol simplifies `SUM("price" / 1)` to `SUM("price")` and delegates it, so `docs/pushdown.md` names `SUM(CAST("price" AS DOUBLE))` as the way to make Exasol compute a floating-point sum. A later `add-pushdown-cast` plan must keep a route.
- An Exasol interval holds milliseconds; MySQL and PostgreSQL truncate a timestamp difference to milliseconds to match it.
- Trino types a decimal product past 38 digits with a reduced scale; a product of factors whose precisions add up to more than 38 can lose fractional digits. `docs/pushdown.md` states it.
- `LENGTH`, `SUBSTR`, `UPPER`, `LOWER`, and `TRIM` over a `DOUBLE` value use the source's text form of the number; over an `INTERVAL` value they fail with a named error. `docs/pushdown.md` states both.
- Two existing MySQL E2E tests that measured `CHAR` padding with `LENGTH` now wrap the operand as `LENGTH("fixed" || '')`, because a delegated `LENGTH` counts the characters MySQL keeps, as the "follow the source" scenario requires.
- The MySQL `ANY_VALUE` wrapping also applies to a bare grouped expression in the select list (`SELECT ANY_VALUE(YEAR(`dt`)), COUNT(*) ... GROUP BY YEAR(`dt`)`); MySQL accepts it and the rows match the mirror.
