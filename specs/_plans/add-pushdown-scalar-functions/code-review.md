# Code Review Findings: add-pushdown-scalar-functions

## Summary
- Files reviewed: 32
- Total findings: 10

## Findings

### crates/adbc-vs-adapter/src/pushdown_query_scalar.rs

#### [ARITHMETIC_UNDERFLOW] An exact class can carry a scale above its precision
- Location: `Arithmetic::value_class` (`Operator::Multiply` arm and the `Add | Subtract` arm), `ValueClass::of_literal`
- Issue: `Multiply` caps `precision` at `MAX_PRECISION` but leaves `scale` at `s1 + s2`, so `DECIMAL(36,20) * DECIMAL(36,20)` yields `Exact { precision: 36, scale: 40 }`. `of_literal` does the same for a literal with more than 36 fractional digits. A later `+` or `-` over that class evaluates `p1 - s1` on `u32`. That panics in a debug build and wraps in release, because the workspace sets no `overflow-checks`. The Trino `exact` then renders `CAST(... AS DECIMAL(36,40))`, which Trino rejects.
- Fix: In `Arithmetic::value_class`, cap the scale with `scale.min(MAX_PRECISION)` and set `precision` to `precision.min(MAX_PRECISION).max(scale)`. Clamp `scale` the same way in `of_literal`. Replace `p1 - s1` and `p2 - s2` with `p1.saturating_sub(s1)` and `p2.saturating_sub(s2)`. Add a unit test in `pushdown_query_tests.rs` that reads `("a" * "b") + 1` over two `DECIMAL(36,20)` columns and asserts that every `ValueClass::Exact` has `scale <= precision`. Before you pick the capped scale, measure the type that Exasol declares for `DECIMAL(36,20) * DECIMAL(36,20)` on the test Exasol and use that type.

#### [UNMEASURED_OVERLOAD] A character operand beside a date resolves as a number
- Location: `resolve_arithmetic`, first arm and the `l.is_temporal() && r.is_number()` arm
- Issue: `ValueClass::is_number` includes `Character`. So `"dt" - "dtext"` resolves to `ShiftDays`, which renders `CAST("dtext" AS DOUBLE)` and fails at the source for date text. `"dtext" - "dt"` matches no arm and fails in the adapter. Because `FN_SUB` is now advertised, Exasol delegates both expressions. If Exasol reads the text as a date, which is how the implicit conversion of `YEAR` reads it, each of these queries fails where it used to succeed. No test covers a character operand beside a temporal one.
- Fix: On the test Exasol, run `SELECT DATE '2024-02-01' - '2024-01-01'` and `SELECT '2024-02-01' - DATE '2024-01-01'`. If Exasol returns a day count, add arms to `resolve_arithmetic` before the number arms that read a `Character` operand beside a `Date` or `Timestamp` as that temporal class, rendered through `text_to_timestamp`. In both cases, add `"dt" - "dtext"` to `Scenario::date_arithmetic` in `tests/common/scalars.rs` so that the mirror comparison pins the behavior.

#### [BOOLEAN_FLAG_PARAMETER] `subtract: bool` selects the operation
- Location: `Arithmetic::ShiftDays`, `Arithmetic::ShiftInterval`, `Arithmetic::IntervalSum`; `StatementRenderer::shift_days` and `StatementRenderer::shift_interval` in `dialect_common.rs` and their three implementations
- Issue: A boolean argument chooses between addition and subtraction. The guardrails forbid selector booleans.
- Fix: Add `enum Direction { Forward, Backward }` to `pushdown_query_scalar.rs`, re-export it next to `Operator`, and replace each `subtract: bool` field and parameter with `direction: Direction`. Update the `if subtract` branches in `mysql_dialect.rs`, `postgres_dialect.rs`, `trino_dialect.rs`, and the `IntervalSum` arm of `render_arithmetic` to match on it.

### crates/adbc-vs-adapter/src/dialect_common.rs

#### [DEAD_CODE] The character arm of `number` is unreachable
- Location: `StatementRenderer::number`, the `ValueClass::Character => self.text_double(&rendered)` arm
- Issue: `number` is called only from the `Arithmetic::Exact` arm. `resolve_arithmetic` routes every operation with a `Character` operand to `Arithmetic::Double`, so this arm never runs.
- Fix: Delete the `ValueClass::Character` arm from `number` and update its doc comment to "an operand as Exasol reads it in exact arithmetic: a boolean as 1 or 0, every other number as itself".

### crates/adbc-vs-adapter/src/trino_dialect.rs

#### [CONVERSION_MISMATCH] Trino converts every `DOUBLE` to text in scientific notation
- Location: `StatementRenderer::text` in `dialect_common.rs`, the `ValueClass::Double` arm, which renders `CAST(x AS VARCHAR)` for Trino
- Issue: Trino's `CAST(1.5E0 AS VARCHAR)` returns `1.5E0`. Every double operand of `LENGTH`, `SUBSTR`, `UPPER`, `LOWER`, or `TRIM` at Trino therefore differs from Exasol's text form, not only extreme values. `docs/pushdown.md` gives only `1e20` as an example of a difference, so it understates this. No E2E test passes a `DOUBLE` to a string function.
- Fix: In `docs/pushdown.md` § Database differences › Trino, add the bullet: "Trino converts every `DOUBLE` value to text in scientific notation, for example `1.5E0`, so `LENGTH`, `SUBSTR`, `UPPER`, `LOWER`, and `TRIM` over a `DOUBLE` value return a result that differs from Exasol's." In `Scenario::implicit_conversions` in `tests/common/scalars.rs`, add a Trino-only assertion that `LENGTH("f")` is delegated and returns `5` for row `id = 1`.

### crates/adbc-vs-adapter/src/mysql_dialect.rs

#### [UNDOCUMENTED_DIFFERENCE] Invalid date text reads as null in MySQL
- Location: `text_to_timestamp`
- Issue: In a `SELECT`, MySQL's `CAST(<text> AS DATETIME(6))` returns null with a warning for text that is not a date. Exasol fails `YEAR('abc')`. Plan direction D3 and the docs list the matching difference for arithmetic over a character operand, but not this one.
- Fix: In `docs/pushdown.md` § Database differences › MySQL, add the bullet: "`YEAR`, `MONTH`, and `DAY` over text that is not a timestamp return null. Exasol fails the query for such a value."

### crates/adbc-vs-adapter/tests/common/scalars.rs

#### [UNTESTED_BOUNDARY] Millisecond truncation of a timestamp difference is never exercised
- Location: `COLUMNS` (`ts`, `ts2` declared `TIMESTAMP(3)`), `Scenario::timestamp_difference`; `mysql_script`, `docker/postgres/init.sql`
- Issue: `TRUNCATE(..., -3)` (MySQL) and `date_trunc('milliseconds', ...)` (PostgreSQL) exist only for sub-millisecond differences. All seeded timestamps have millisecond precision, so no E2E run changes a value through either expression. No E2E run covers a negative sub-millisecond difference either, where the truncation direction matters.
- Fix: For MySQL and PostgreSQL, add two microsecond-precision columns `us1` and `us2` to `scalars` (`DATETIME(6)` and `timestamp(6)`, mirrored as `TIMESTAMP(6)`). Seed one row with `us1 - us2 = 0.0015 s` and one row with `us1 - us2 = -0.0015 s`. Add `"us1" - "us2" AS U` to a MySQL- and PostgreSQL-only case in `Scenario::timestamp_difference`. Update both `init.sql` files and `ROWS` together so that `postgresql_and_trino_seed_the_same_rows` still passes.

#### [UNTESTED_PATH] Interval scaling and interval sums have no E2E coverage, and Trino may round
- Location: `Scenario::timestamp_difference`; `interval_scale` in `trino_dialect.rs`
- Issue: `Arithmetic::IntervalScale` and `Arithmetic::IntervalSum` are rendered in all three dialects but appear in no E2E query. MySQL and PostgreSQL truncate a scaled interval to milliseconds. Trino renders a bare `interval * double`, and Trino may round that product instead of truncating it.
- Fix: Add `("ts" - "ts2") * 1.5 AS S` and `("ts" - "ts2") + ("ts" - "dt") AS T` to the `assert_delegated` cases of `Scenario::timestamp_difference`. If the Trino result differs from the mirror, change the Trino `interval_scale` to `(CAST(truncate(to_milliseconds({interval}) * {factor}) AS BIGINT) * INTERVAL '0.001' SECOND)`.

#### [MISSING_SCENARIO_ASSERTION] The case-mapping half of "String function results follow the source" is untested
- Location: `Scenario::string_function_results_follow_the_source`; `e2e_trino_test.rs`
- Issue: The scenario requires `UPPER` over a non-ASCII letter with a locale-dependent upper case to return the source's result. The test checks only `LENGTH` over `CHAR`, and `s` holds only `é`, whose upper case does not depend on the locale. Trino maps no `CHAR` column, so it has no test for this scenario at all, although the `UPPER` half applies to Trino.
- Fix: Add a row whose `s` is `'ß'` to `ROWS` and both `init.sql` files. Split the `UPPER` half into a new `Scenario::upper_follows_the_source(&self, expected: &str)`. It asserts that `UPPER("s")` for that row is delegated and returns `expected`, the value the source's own `UPPER('ß')` returns, which you measure once per source. Call it from all three E2E files.

#### [WEAK_ASSERTION] The overflow case does not prove the product was delegated
- Location: `Scenario::exact_arithmetic`, the `"i" * "i" * "i"` loop
- Issue: The case asserts only that the query fails over both schemas. It would also pass if the product stayed in Exasol, or if the virtual query failed for an unrelated reason. So it does not show that a delegated result beyond the declared type fails instead of being truncated. The scenario's `WHERE` arm for `"d" * "d"` and `"d" - "d"` is also missing.
- Fix: Before the loop, assert that `self.generated_statement("\"i\" * \"i\" * \"i\"", "")` contains `* CAST(`. Add `("\"d\"", "WHERE \"d\" * \"d\" > \"d\" - \"d\"", ["* CAST(", "* CAST(", "* CAST("])` to the `assert_delegated` cases.
