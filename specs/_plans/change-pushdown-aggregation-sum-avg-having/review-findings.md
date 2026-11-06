# Code Review Findings: change-pushdown-aggregation-sum-avg-having

## Summary
- Files reviewed: 33
- Total findings: 12 (standard: 12, expert: 0)
- Verified before review: `make lint` passes, `cargo test -p adbc-vs-adapter --lib` passes 465 tests, and the `docs/pushdown.md` table matches the `advertised` list of each `crates/adbc-vs-scan/drivers/*.toml` (MySQL 39, PostgreSQL 40, Trino 40 entries; every Yes row fully advertised, every No row absent, every entry covered).

## Standard fixes

### docs/pushdown.md

#### [OUTDATED_COMMENT] The MySQL 8.0.17 requirement names only `AVG`
- Location: line 43
- Issue: The bullet states that only a delegated `AVG` needs MySQL 8.0.17. `mysql_dialect.rs` `summed` (line 678) also renders a character operand of `SUM` as `CAST(NULLIF(<operand>, '') AS DOUBLE)`, so `SUM` over a `VARCHAR` or `CHAR` column fails on MySQL before 8.0.17 too. The bullet also gives the reader no way to run such a query on an older server.
- Fix: In docs/pushdown.md, replace the bullet at line 43 with: "- A delegated `AVG`, and a delegated `SUM` over a character column, need MySQL 8.0.17 or later, because the query that `adbc-vs-rs` sends uses `CAST(... AS DOUBLE)`. On an older MySQL, the query fails. To run it there, wrap the operand in an expression that `adbc-vs-rs` does not send, for example `AVG("price" + 0)`, so that Exasol computes the aggregate."

#### [OUTDATED_COMMENT] The floating-point section omits character operands
- Location: line 34
- Issue: The section says only a `DOUBLE` column is summed in the source's own order. A `SUM` or `AVG` over a character column is also a double-precision sum: every dialect renders `CAST(NULLIF(<operand>, '') AS DOUBLE)` or `DOUBLE PRECISION`, and Exasol declares the result `DOUBLE`. Its result can differ in the low-order digits in the same way.
- Fix: In docs/pushdown.md line 34, replace "Over a `DOUBLE` column, the source adds the values in its own order." with "Over a `DOUBLE` column, and over a character column, which the source and Exasol both convert to `DOUBLE`, the source adds the values in its own order."

### crates/adbc-vs-adapter/src/postgres_dialect.rs

#### [SHALLOW_MODULE] `is_boolean_operand` is a one-line wrapper the plan removes
- Location: line 688, used at line 530
- Issue: The body of `is_boolean_operand` is only `operand.aggregate_operand_conversion() == Some(OperandConversion::Boolean)`, and its doc comment restates the classification that `Expression::aggregate_operand_conversion` already documents. plan.md § Dead Code Removal lists the function for removal.
- Fix: In crates/adbc-vs-adapter/src/postgres_dialect.rs, delete `is_boolean_operand` with its doc comment, and change the match guard in `Statement::render_aggregate` from `_ if is_boolean_operand(argument) =>` to `_ if argument.aggregate_operand_conversion() == Some(OperandConversion::Boolean) =>`. Run `cargo test -p adbc-vs-adapter --lib postgres_dialect` and confirm it passes.

### crates/adbc-vs-adapter/src/mysql_dialect.rs

#### [OUTDATED_COMMENT] The `ANY_VALUE` docs do not name the `HAVING` clause
- Location: lines 282-286 (`Statement` doc), lines 33-35 (module doc)
- Issue: `MySqlDialect::render` renders `HAVING` through the `select_list` statement, so a converted grouping key reads as `ANY_VALUE(<read expression>)` in `HAVING` too. The `Statement` doc names only "outside an aggregate in the select list", and the module doc names only the key's "select-list occurrence".
- Fix: In crates/adbc-vs-adapter/src/mysql_dialect.rs, in the `Statement` doc comment, change "so outside an aggregate in the select list a key grouped by" to "so outside an aggregate in the select list and in the `HAVING` clause a key grouped by". In the module doc, change "its select-list occurrence outside an aggregate renders as" to "its occurrence outside an aggregate in the select list or the `HAVING` clause renders as".

### crates/adbc-vs-adapter/src/capabilities_tests.rs

#### [OUTDATED_COMMENT] New capability comments exceed two lines and one misstates the release
- Location: lines 251-254, 259-261, 331-334, 407-410
- Issue: AGENTS.md limits a comment to 1 or 2 lines. The three comments above `"FN_AGG_SUM"` each take 4 lines. The comment at lines 259-261 says "this release groups by one column", but the same release advertises `AGGREGATE_GROUP_BY_TUPLE` for Trino (`TRINO_CAPABILITIES`), and its second clause about `SELECTLIST_EXPRESSIONS` does not explain the withholding.
- Fix: In crates/adbc-vs-adapter/src/capabilities_tests.rs, replace each of the three comments above `"FN_AGG_SUM"` (in `MYSQL_CAPABILITIES`, `POSTGRES_CAPABILITIES`, `TRINO_CAPABILITIES`) with the two lines `// Advertised although a floating-point sum follows the source's accumulation order:` and `// refusing a DOUBLE operand at render time would fail a query Exasol answers.`. Replace lines 259-261 with the one line `// AGGREGATE_GROUP_BY_EXPRESSION and AGGREGATE_GROUP_BY_TUPLE are withheld: MySQL groups by one column.`.

### crates/adbc-vs-scan/drivers/postgresql.toml

#### [OUTDATED_COMMENT] The comment gives the wrong reason for the `DOUBLE` cast
- Location: lines 55-59
- Issue: The comment says the dialect "casts a `double precision` operand ... so each returns the type Exasol declares". `postgres_dialect.rs` `summed` casts every operand declared `DOUBLE` because `sum` over `real` adds in single precision; casting a `double precision` operand changes nothing. The comment also takes 5 lines, where AGENTS.md allows 2.
- Fix: In crates/adbc-vs-scan/drivers/postgresql.toml, replace lines 55-59 with the two lines `# FN_AGG_SUM and FN_AGG_AVG cast a real operand to double precision, because sum(real) adds in` and `# single precision; a double precision sum follows the source's accumulation order (docs/pushdown.md).`.

### crates/adbc-vs-scan/drivers/trino.toml

#### [OUTDATED_COMMENT] The comment gives the wrong reason for the `DECIMAL(38, s)` widening
- Location: lines 60-66
- Issue: The comment says the dialect widens an exact-numeric `SUM` operand "so each returns the type Exasol declares". `trino_dialect.rs` `summed` widens it because Trino's `sum` returns its input type and a `bigint` sum overflows past the `bigint` range; Exasol declares `DECIMAL(min(p + 11, 36), s)`, not 38 digits. The comment also groups `AGGREGATE_HAVING` under "a capability admits every numeric operand", which does not apply to it, and takes 7 lines, where AGENTS.md allows 2.
- Fix: In crates/adbc-vs-scan/drivers/trino.toml, replace lines 60-66 with the two lines `# SUM widens an exact-numeric operand to DECIMAL(38, s), because sum(bigint) fails past the bigint range;` and `# a double sum follows Trino's distributed accumulation order (docs/pushdown.md).`.

### crates/adbc-vs-adapter/src/{mysql,postgres,trino}_dialect_tests.rs

#### [INFORMATION_COUPLING] Three copies of dialect-neutral request builders
- Location: mysql_dialect_tests.rs lines 603, 696, 701; postgres_dialect_tests.rs lines 1122, 1127, 1132; trino_dialect_tests.rs lines 556, 561, 566
- Issue: `sum_type`, `double_type`, and `distinct` are identical in all three files. `sum_type` encodes a fact of the shared `orders_columns` fixture (`total` is `DECIMAL(12,4)`). The `dialect_test_fixtures.rs` module doc states that dialect-neutral request builders live there so that three copies cannot drift apart.
- Fix: Move `sum_type`, `double_type`, and `distinct`, with their doc comments, into crates/adbc-vs-adapter/src/dialect_test_fixtures.rs as `pub(crate) fn`. Delete the copies in mysql_dialect_tests.rs, postgres_dialect_tests.rs, and trino_dialect_tests.rs, and add the three names to each file's `use crate::dialect_test_fixtures::{...}` list. Run `cargo test -p adbc-vs-adapter --lib` and confirm it passes.

### crates/adbc-vs-adapter/tests/common/trino_mirror.rs

#### [SHRINKABLE] A third, hand-written tuple counter
- Location: line 223
- Issue: `every_mirrored_measures_row_carries_one_value_per_declared_column` reimplements the per-tuple value count with its own quote and comma parsing. `mirror.rs` (line 420) and `postgres_mirror.rs` (line 464) already hold the same `values_per_tuple` helper, and `mirror_table.rs` is the module for mirror mechanics that the mirrors share.
- Fix: Move `values_per_tuple`, with its doc comment, from the `tests` module of crates/adbc-vs-adapter/tests/common/mirror.rs into crates/adbc-vs-adapter/tests/common/mirror_table.rs as `pub fn values_per_tuple(values: &str) -> Vec<usize>`. Delete the copies in mirror.rs and postgres_mirror.rs and call `mirror_table::values_per_tuple` from their `every_mirrored_row_carries_one_value_per_declared_column` tests. In trino_mirror.rs, replace the body of `every_mirrored_measures_row_carries_one_value_per_declared_column` with `let counts = mirror_table::values_per_tuple(MEASURES_VALUES);`, an assertion that `counts` is not empty, and an assertion that every count equals `MEASURES_COLUMNS.len()`, keeping the existing failure message.

### crates/adbc-vs-adapter/tests/e2e_mysql_test.rs

#### [ASSERTION_FREE_TEST] The predicate operand case never asserts delegation
- Location: lines 1535-1541
- Issue: In `sum_and_avg_over_boolean_and_character_columns_match_the_mirror`, the `SUM("big_value" > 5)` and `AVG("big_value" > 5)` case only compares rows with the mirror. The rows match whether or not Exasol delegates the aggregate, so the test passes even if the predicate operand is never sent to MySQL. AGENTS.md requires each delegated fragment to be verified in the statement and in the rows. The PostgreSQL and Trino versions of the test assert `CAST((5 < "big_value") AS INTEGER)` in the statement.
- Fix: In crates/adbc-vs-adapter/tests/e2e_mysql_test.rs, before `assert_matches_mirror(MEASURES, predicate);`, add `let statement = pushdown_statement(&predicate(VIRTUAL_SCHEMA, MEASURES));` and an assertion that `statement` contains ``SUM((5 < `big_value`))``, with the message "a predicate operand must render unchanged inside SUM: {statement}".

### crates/adbc-vs-adapter/tests/{e2e_postgres,e2e_trino}_test.rs

#### [REDUNDANT_COMMENT] New section banners
- Location: e2e_postgres_test.rs line 742, e2e_trino_test.rs line 397
- Issue: Each file gains the banner `// --- SUM, AVG, and HAVING ---...`. AGENTS.md states that a comment must never add a banner.
- Fix: Delete the line `// --- SUM, AVG, and HAVING ---------------------------------------------------` from crates/adbc-vs-adapter/tests/e2e_postgres_test.rs (line 742) and from crates/adbc-vs-adapter/tests/e2e_trino_test.rs (line 397).

### docker/{postgres,trino}/init.sql

#### [REDUNDANT_COMMENT] Fixture comments restate the rows below them
- Location: docker/postgres/init.sql lines 323-336 and 358-360; docker/trino/init.sql lines 190-201
- Issue: The `measures` comments take 14 and 12 lines and restate the seeded values (for example "`big_value` holds `9223372036854775807` and `1` in group `b`"), which the `INSERT` right below shows. AGENTS.md limits a comment to 1 or 2 lines that state a non-obvious why. The `nan_measures` comment takes 3 lines.
- Fix: In docker/postgres/init.sql, replace lines 323-336 with the two lines `-- measures feeds the SUM, AVG, and HAVING tests; postgres_mirror.rs restates its rows by hand. Floats are` and `-- binary fractions so no accumulation order changes a sum, and real 16777216 + 1 needs double precision.`, and replace lines 358-360 with the one line `-- nan_measures holds a NaN, which Exasol cannot store, so no mirror copies it.`. In docker/trino/init.sql, replace lines 190-201 with the two lines `-- measures feeds the SUM, AVG, and HAVING tests; trino_mirror.rs restates its rows by hand.` and `-- Floats are binary fractions so no accumulation order changes a sum, and big_value sums past bigint.`. Change no SQL statement.

## Expert fixes
[none]
