# Plan: add-pushdown-scalar-functions

## Summary

Every dialect delegates `YEAR`, `MONTH`, `DAY`, `UPPER`, `LOWER`, `TRIM` (except MySQL), `LENGTH`, `SUBSTR`, `+`, `-`, and `*`, and grouping by one expression, so a filter, projection, or one-key grouping over them reads only the rows and groups it needs. Each function renders the source SQL that returns Exasol's result per operand type, and string functions follow the source for `CHAR` padding and case mapping.

## Context

- Issue #21: no dialect advertises a scalar function. In the 0.6.0 evaluation, a `GROUP BY YEAR(), MONTH()` fetched 1,000,000 rows (PostgreSQL JDBC VS: 33), and a `COUNT(*)` with an arithmetic filter fetched 5,000,000 rows (PostgreSQL JDBC VS: 1).
- Grouping by `YEAR("d")` needs `AGGREGATE_GROUP_BY_EXPRESSION`, which `vs-adapter/pushdown-aggregation-delegation` withholds today.
- Exasol does not re-apply a delegated fragment, so each advertised function must render for every operand type Exasol admits for it: `DATE + 1` is day arithmetic in Exasol and the number `YYYYMMDD + 1` in MySQL.
- The PostgreSQL JDBC VS delegates these functions as plain PostgreSQL functions. The MySQL JDBC VS withholds every string function, `ADD`, `SUB`, `MULT`, and `DAY`.
- `docs/pushdown.md` tells users to write `SUM("price" + 0)` to make Exasol compute a floating-point sum. Delegating `+` removes that route.
- Exasol types `TIMESTAMP - TIMESTAMP` as `INTERVAL DAY TO SECOND`, which the UDF SDK cannot emit, so that select-list entry fails once `FN_SUB` is advertised.
- Structural changes: see `architecture.md` in this plan directory.
- The MySQL JDBC VS withholds `TRIM`, because MySQL's `TRIM` removes a substring rather than a set of characters. This plan does the same.
- Follow-up plans:
  - `add-pushdown-cast`: delegate `CAST` once the target types Exasol sends are measured.
  - `change-pushdown-group-by-tuple`: advertise `AGGREGATE_GROUP_BY_TUPLE` for MySQL and PostgreSQL, so the issue's two-key `GROUP BY YEAR(), MONTH()` is delegated there too.

## Features

| Feature | Status | Spec |
|---------|--------|------|
| pushdown-scalar-function | NEW | `vs-adapter/pushdown-scalar-function/spec.md` |
| pushdown-aggregation-delegation | CHANGED | `vs-adapter/pushdown-aggregation-delegation/spec.md` |
| pushdown-aggregation | CHANGED | `vs-adapter/pushdown-aggregation/spec.md` |
| pushdown-dialect | CHANGED | `vs-adapter/pushdown-dialect/spec.md` |
| mysql-pushdown | CHANGED | `vs-adapter/mysql-pushdown/spec.md` |
| postgresql-pushdown | CHANGED | `vs-adapter/postgresql-pushdown/spec.md` |
| trino-pushdown | CHANGED | `vs-adapter/trino-pushdown/spec.md` |
| mysql-collation | CHANGED | `vs-adapter/mysql-collation/spec.md` |
| postgresql-collation | CHANGED | `vs-adapter/postgresql-collation/spec.md` |

## Impact

- Queries using the eleven scalar functions or grouping by an expression run at the source. They read fewer rows, and their results now come from the source.
- BREAKING CHANGE: `SELECT "t1" - "t2"` over two `TIMESTAMP` columns of a virtual table fails with a named error that names `SECONDS_BETWEEN` and `DAYS_BETWEEN` as the remedy. Before, Exasol computed it.
- `SUM("price" + 0)` no longer makes Exasol compute a floating-point sum. `docs/pushdown.md` names `SUM("price" / 1)` instead.
- `LENGTH` over a `CHAR` column and `UPPER` and `LOWER` over non-ASCII text return the source's result, which can differ from Exasol's. An implicit conversion follows Exasol's default date and timestamp formats, not a changed session format. `docs/pushdown.md` lists each difference per database.
- The two-key `GROUP BY YEAR(), MONTH()` of issue #21 stays undelegated for MySQL and PostgreSQL until `change-pushdown-group-by-tuple`.
- New ADR superseding `capability-negotiation-all-or-nothing` and `delegate-sum-avg-and-let-floating-point-sum-follow-source-order` (direction D7).

## Dependencies

- None. No new crate.

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| Function | `crates/adbc-vs-adapter/src/pushdown_query.rs` `read_group_by_column` | A `groupBy` entry is an expression, read through the expression parser |
| Type | `crates/adbc-vs-adapter/src/pushdown_query.rs` `Aggregation::GroupBy(Vec<ColumnRef>)` | Carries expressions, not columns only |
| Comment | `crates/adbc-vs-adapter/src/pushdown_query.rs:64` and `:711` | State the withholding of `AGGREGATE_GROUP_BY_EXPRESSION`, which ends |
| Logic | `crates/adbc-vs-adapter/src/mysql_dialect.rs` `any_value_keys` matching | Keyed on a column's read expression; a grouped expression key matches by rendered text |
| Test assertion | `crates/adbc-vs-adapter/src/capabilities_tests.rs:257` and the exact-set tests | Assert `AGGREGATE_GROUP_BY_EXPRESSION` is withheld |
| Docs | `docs/pushdown.md` `+ 0` advice in § Floating-point sums follow the source and § MySQL (replaced by `/ 1`), and the "What runs in Exasol" rows for scalar functions and `GROUP BY` over an expression | The `+ 0` route and the withholding end |

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| pushdown-scalar-function / Each dialect advertises the scalar functions it renders | Unit | `crates/adbc-vs-adapter/src/capabilities_tests.rs` | `each_dialect_advertises_the_scalar_functions_it_renders` |
| pushdown-scalar-function / A scalar function renders in every position Exasol delegates it to | Unit | `crates/adbc-vs-adapter/src/pushdown_query_tests.rs` | `scalar_function_renders_in_every_position_per_dialect` |
| pushdown-scalar-function / Date parts match Exasol over DATE and TIMESTAMP columns | E2E | `crates/adbc-vs-adapter/tests/e2e_{mysql,postgres,trino}_test.rs` | `date_parts_reach_<source>_and_match_the_mirror` |
| pushdown-scalar-function / Exact-numeric arithmetic is exact and does not overflow before Exasol does | E2E | `crates/adbc-vs-adapter/tests/e2e_{mysql,postgres,trino}_test.rs` | `exact_arithmetic_reaches_<source>_and_matches_the_mirror` |
| pushdown-scalar-function / Floating-point and character arithmetic converts as Exasol converts | E2E | `crates/adbc-vs-adapter/tests/e2e_{mysql,postgres,trino}_test.rs` | `double_and_character_arithmetic_matches_the_mirror` |
| pushdown-scalar-function / Date and timestamp arithmetic follows Exasol's day arithmetic | E2E | `crates/adbc-vs-adapter/tests/e2e_{mysql,postgres,trino}_test.rs` | `date_arithmetic_reaches_<source>_and_matches_the_mirror` |
| pushdown-scalar-function / A select-list TIMESTAMP difference fails with a named error | Unit + E2E | `crates/adbc-vs-adapter/src/pushdown_tests.rs`, `crates/adbc-vs-adapter/tests/e2e_{mysql,postgres,trino}_test.rs` | `select_list_timestamp_difference_fails_naming_interval`, `timestamp_difference_comparison_matches_the_mirror` |
| pushdown-scalar-function / String functions match Exasol over VARCHAR text | E2E | `crates/adbc-vs-adapter/tests/e2e_{mysql,postgres,trino}_test.rs` | `string_functions_reach_<source>_and_match_the_mirror` |
| pushdown-scalar-function / SUBSTR positions follow Exasol's rule | E2E | `crates/adbc-vs-adapter/tests/e2e_{mysql,postgres,trino}_test.rs` | `substr_positions_match_the_mirror` |
| pushdown-scalar-function / An empty string function result is null, as in Exasol | E2E | `crates/adbc-vs-adapter/tests/e2e_{mysql,postgres,trino}_test.rs` | `empty_string_function_result_is_null_as_in_exasol` |
| pushdown-scalar-function / String function results follow the source for padding and case mapping | E2E | `crates/adbc-vs-adapter/tests/e2e_{mysql,postgres,trino}_test.rs` | `string_function_results_follow_the_source` |
| pushdown-scalar-function / An implicitly converted operand converts as Exasol converts it | E2E | `crates/adbc-vs-adapter/tests/e2e_{mysql,postgres,trino}_test.rs` | `implicitly_converted_operands_match_the_mirror` |
| pushdown-aggregation-delegation / An aggregation Exasol cannot delegate whole is computed by Exasol | E2E | `crates/adbc-vs-adapter/tests/e2e_{mysql,postgres,trino}_test.rs` | existing test, updated to `ROUND(AVG(...))` |
| pushdown-aggregation-delegation / Grouping by a scalar expression is delegated by every dialect | E2E | `crates/adbc-vs-adapter/tests/e2e_{mysql,postgres,trino}_test.rs` | `group_by_year_reaches_<source>_and_matches_the_mirror` |
| pushdown-dialect / The declarable set is closed under the advertised capabilities | E2E | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `scalar_function_declared_types_stay_in_the_declarable_set` |
| mysql-pushdown / Capability set advertises the fragments this dialect renders, Aggregation capabilities cover single-group and column grouping | Unit | `crates/adbc-vs-adapter/src/capabilities_tests.rs` | `mysql_capability_set_is_exact` |
| mysql-pushdown / String functions render with MySQL's character semantics | Unit | `crates/adbc-vs-adapter/src/mysql_dialect_tests.rs` | `string_functions_render_with_mysql_semantics` |
| mysql-pushdown / Arithmetic renders with MySQL's day and decimal semantics | Unit | `crates/adbc-vs-adapter/src/mysql_dialect_tests.rs` | `arithmetic_renders_with_mysql_semantics` (includes a nested product `("a" * "b") * "c"`) |
| postgresql-pushdown / Capability set advertises the fragments this dialect renders | Unit | `crates/adbc-vs-adapter/src/capabilities_tests.rs` | `postgres_capability_set_is_exact` |
| postgresql-pushdown / Scalar functions render in PostgreSQL's forms | Unit | `crates/adbc-vs-adapter/src/postgres_dialect_tests.rs` | `scalar_functions_render_in_postgresql_forms` |
| trino-pushdown / Capability set advertises the fragments this dialect renders, Capability set withholds the fragments the Trino dialect does not delegate | Unit | `crates/adbc-vs-adapter/src/capabilities_tests.rs` | `trino_capability_set_is_exact` |
| trino-pushdown / Scalar functions render in Trino's forms | Unit | `crates/adbc-vs-adapter/src/trino_dialect_tests.rs` | `scalar_functions_render_in_trino_forms` |
| mysql-collation / A comparison over a wrapped operand is evaluated by Exasol | E2E | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` | `a_comparison_over_a_wrapped_operand_is_evaluated_by_exasol` |
| postgresql-collation / An ordering comparison over a wrapped operand is evaluated by Exasol | E2E | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `an_ordering_comparison_over_a_wrapped_operand_is_evaluated_by_exasol` |

`every_advertised_capability_renders_per_dialect` in `capabilities_tests.rs` gains one exercising fragment per scalar-function capability and `AGGREGATE_GROUP_BY_EXPRESSION`, with one arm per operand type class each function admits.

### Manual Testing

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| pushdown-scalar-function | `make stack-up-postgres && make test-e2e-postgres`, then in Exasol: `EXPLAIN VIRTUAL SELECT COUNT(*) FROM vs_pg."orders" WHERE "qty" * 2 > 10` | Generated statement carries `* 2` and the `WHERE` clause; the query returns one row |
| pushdown-aggregation-delegation | `EXPLAIN VIRTUAL SELECT YEAR("d"), COUNT(*) FROM vs_mysql."orders" GROUP BY YEAR("d")` | Generated statement carries `GROUP BY` over `YEAR`; result rows equal the mirror's |
| mysql-pushdown | `SELECT LENGTH('ä'), "d" + 1 FROM vs_mysql."orders"` against a row with `d = 2024-02-28` | `1` and `2024-02-29` |
| postgresql-pushdown | `SELECT TRIM('xxaxx', 'x') FROM vs_pg."orders"` | `a` |
| mysql-pushdown (TRIM) | `EXPLAIN VIRTUAL SELECT TRIM("name") FROM vs_mysql."orders"` | Generated statement carries no `TRIM` |
| trino-pushdown | `SELECT "ts" + 1.5 FROM vs_trino."orders"` against `2024-01-01 00:00:00` | `2024-01-02 12:00:00` |
| pushdown-dialect | `SELECT "t1" - "t2" FROM vs_pg."events"` | Error naming `INTERVAL DAY TO SECOND` and `SECONDS_BETWEEN` |

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Build | `make cross-udf-build` | Exit 0 |
| Test | `make test` | 0 failures |
| Integration | `make test-it test-it-postgres test-it-trino` | 0 failures |
| E2E | `make stack-up-exasol stack-up-mysql stack-up-postgres stack-up-trino && make test-e2e test-e2e-postgres test-e2e-trino` | 0 failures |
| Lint | `make lint` | 0 warnings |
| Format | `make fmt-check` | No changes |
