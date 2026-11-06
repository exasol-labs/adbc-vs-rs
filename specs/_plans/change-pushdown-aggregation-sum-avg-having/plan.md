# Plan: change-pushdown-aggregation-sum-avg-having

## Summary

The MySQL, PostgreSQL, and Trino dialects delegate `SUM`, `SUM(DISTINCT)`, `AVG`, `AVG(DISTINCT)`, and `HAVING` to the source, and a floating-point sum follows the source's accumulation order, as documented for users. A new documentation page lists what runs at the source for each database. This implements issue exasol-labs/adbc-vs-rs#19 with its option 1.

## Context

- Issue #19: in an evaluation over a 5,000,000-row table, `SUM(total) GROUP BY status` fetched 1,000,000 rows where the PostgreSQL JDBC Virtual Schema fetched 5, and `HAVING COUNT(*) > 12` fetched 1,000,000 rows where it fetched 0.
- The accepted ADR `capability-negotiation-all-or-nothing` withholds `FN_AGG_SUM`, `FN_AGG_SUM_DISTINCT`, `FN_AGG_AVG`, and `FN_AGG_AVG_DISTINCT` from every dialect, because a floating-point sum at the source depends on row order. Decision-log entry [1] supersedes it.
- The interview chose option 1 (advertise `SUM` and `AVG`, document the floating-point order dependence) and one plan for `SUM`, `AVG`, and `HAVING`.
- Exasol declares `SUM` over `DECIMAL(p,s)` as `DECIMAL(min(p + 11, 36), s)`, `SUM` over `DOUBLE` as `DOUBLE`, and `AVG` as `DOUBLE` (`vs-adapter/pushdown-aggregation-types`, measured).
- Measured on the test Exasol with a scratch adapter: the Lua source of `crates/adbc-vs-adapter/tests/common/probe.rs`, plus `AGGREGATE_SINGLE_GROUP`, `AGGREGATE_HAVING`, and `FN_AGG_AVG_DISTINCT`. Exasol delegates `SUM`, `SUM(DISTINCT)`, `AVG`, and `AVG(DISTINCT)` over a column declared `BOOLEAN`, `VARCHAR`, or `CHAR`, over a predicate, and over a string literal as the unconverted operand, inside `HAVING` too. It declares `SUM` over a `BOOLEAN` column or a predicate as `DECIMAL(12,0)`, `SUM` over a character column as `DOUBLE`, and every `AVG` as `DOUBLE`. It rejects `SUM` and `AVG` over a `DATE` or `TIMESTAMP` column before it sends a request.
- Over local values on the test Exasol, `SUM` reads true as 1 and false as 0, reads `''` as null, and converts `' 5 '` and `'1e3'`. A value that is not a number fails with SQL state 22018. Over `'5'`, `'5.0'`, and `'7'`, `SUM(DISTINCT)` returns `12` and `AVG(DISTINCT)` returns `4`: Exasol takes the distinct values after the conversion and divides by the count of distinct unconverted values.
- `pushdown_query.rs` refuses a `having` element through `UNDELEGATED_ELEMENTS` (`crates/adbc-vs-adapter/src/pushdown_query.rs:333-337`). The `Query` model has no `having` field.
- Each dialect renders aggregates in its own `render_aggregate` (`mysql_dialect.rs:499`, `postgres_dialect.rs:503`, `trino_dialect.rs:482`) and assembles the statement in its own `render` (`:132-157`, `:151-165`, `:165-179`). A column operand carries its declared Exasol type (`pushdown_query.rs:142`).
- The capability lists are data in `crates/adbc-vs-scan/drivers/{mysql,postgresql,trino}.toml`. They hold 34, 35, and 35 entries, and each grows by five.
- MySQL's `SUM` over an exact-numeric operand returns a `DECIMAL` wider than the operand. The scan converts each result column to its declared type with arrow's cast kernel (`crates/adbc-vs-scan/src/coerce.rs:178`), and no existing test reads a MySQL decimal wider than 38 digits.
- No mirrored test table holds an exact-numeric column, a `DOUBLE` column, and a grouping column together, and the Trino mirror holds no `DOUBLE` column.
- The deltas rewrite the Background of `mysql-pushdown`, `postgresql-pushdown`, `trino-pushdown`, and `pushdown-dialect`, whose bullets state the old withholding. Each rewrite is cut to five bullets. The behavior a dropped bullet described stays in that feature's scenarios.
- `docs/how-it-works.md:39` states what runs at the source in one sentence for every database. The three capability sets differ in three entries: `FN_PRED_LIKE_ESCAPE` (MySQL, PostgreSQL), `JOIN_TYPE_FULL_OUTER` (PostgreSQL, Trino), and `AGGREGATE_GROUP_BY_TUPLE` (Trino). PR #23 feedback asks for a per-database overview that each dialect group keeps equal to its driver capability set.
- Architecture: `architecture.md` adds the floating-point `SUM` and `AVG` rule to § Constraints, beside the collation rule. The query model gains an optional `having` predicate inside `pushdown_query`'s existing responsibility.

## Features

| Feature | Status | Spec |
|---------|--------|------|
| vs-adapter/pushdown-aggregation | CHANGED | `vs-adapter/pushdown-aggregation/spec.md` |
| vs-adapter/pushdown-aggregation-delegation | CHANGED | `vs-adapter/pushdown-aggregation-delegation/spec.md` |
| vs-adapter/pushdown-aggregation-types | CHANGED | `vs-adapter/pushdown-aggregation-types/spec.md` |
| vs-adapter/pushdown-dialect | CHANGED | `vs-adapter/pushdown-dialect/spec.md` |
| vs-adapter/mysql-pushdown | CHANGED | `vs-adapter/mysql-pushdown/spec.md` |
| vs-adapter/postgresql-pushdown | CHANGED | `vs-adapter/postgresql-pushdown/spec.md` |
| vs-adapter/trino-pushdown | CHANGED | `vs-adapter/trino-pushdown/spec.md` |

## Impact

- `SUM`, `SUM(DISTINCT)`, `AVG`, `AVG(DISTINCT)`, and `HAVING` run at the source for every driver. A grouped `SUM` or a `HAVING` query fetches the result groups instead of every row.
- Behavior change: a `SUM` or `AVG` over a `DOUBLE` column returns the source's floating-point result. It can differ from the value the previous release returned in its low-order digits, and two runs can differ. A `NaN` in a PostgreSQL or Trino floating-point column makes the delegated `SUM` and `AVG` null.
- A `SUM` or `AVG` over a `BOOLEAN` or character column is delegated, with the operand converted as Exasol converts it. Over a MySQL character column, a value that is not a number reads as its numeric prefix or as 0, where Exasol fails the query.
- Breaking for MySQL servers older than 8.0.17: a delegated `AVG` fails there, because `CAST(... AS DOUBLE)` needs 8.0.17.
- Exasol requests the capability set once per session, so sessions that start after the adapter library is upgraded use the new set.
- The documentation gains a Pushdown page, `docs/pushdown.md`, linked from every page's navigation header, from `docs/index.md`, and from `README.md`. It lists what runs at the source for MySQL, PostgreSQL, and Trino, and where a delegated result can differ from Exasol's.

## Implementation Tasks

### 1. Query model, shared tests, and user documentation

- [ ] 1.1 Parse the `having` element into the `Query` model in `pushdown_query.rs` with `read_predicate` (`:767`), and remove its entry from `UNDELEGATED_ELEMENTS` (`:336`) with the doc comment at `:322-323`. Replace `a_having_element_is_refused` (`pushdown_query_tests.rs:227`) with `a_having_element_parses_into_the_query_model`.
- [ ] 1.2 Add `AGGREGATE_HAVING` to `MAIN_CAPABILITY_CONSTANTS` (`capabilities_tests.rs:144`).
- [ ] 1.3 Extend `declared_types_come_from_select_list_data_types` (`emit_type_tests.rs:51`) with `SUM`, `SUM(DISTINCT)`, `AVG`, and `AVG(DISTINCT)` entries.
- [ ] 1.4 Create `docs/pushdown.md`, titled "Pushdown", with the navigation header and rule of `docs/how-it-works.md:1-3`. Add `[Pushdown](pushdown.md)` between How Does It Work and Type Mapping to that header in `docs/{install,create-virtual-schema,how-it-works,type-mapping}.md` and in the new page. Add the page to the list in `docs/index.md:7-10` and to the documentation list in `README.md:58-63`, and link the "SQL pushdown" bullet of `README.md:30` to it. Replace the two paragraphs of "What runs at the source" in `docs/how-it-works.md` (`:39-41`) with one sentence that links the page. The page holds, in this order:
  - One paragraph: the adapter sends the parts of a query that the source can run, and Exasol runs the rest on the returned rows.
  - The table of the rows listed after this task group, with the Capabilities column and one Yes or No cell per database. Each cell comes from the `advertised` list of `crates/adbc-vs-scan/drivers/<database>.toml` as it stands before group B: Yes when the list holds every capability of the row, No when it holds none. The `SUM`, `AVG`, and `HAVING` rows read No for every database until tasks 2.8, 3.7, and 4.7.
  - The parts Exasol runs for every database: `ORDER BY`, `LIMIT`, `OFFSET`, `GROUP BY` over an expression, a join on a condition other than equality, and scalar functions and arithmetic, such as `"price" + 0`. A query with such a part still works, but Exasol fetches more rows.
  - A section "Floating-point sums follow the source" for the databases whose `SUM` and `AVG` cells read Yes: an exact-numeric `SUM` or `AVG` equals Exasol's result; over a `DOUBLE` column the source adds in its own order, so the result can differ in the low-order digits, differ further where values cancel, and differ between runs; a `NaN` makes the result null; to get Exasol's result, wrap the operand in an expression the adapter does not send, such as `SUM("price" + 0)`. The section links "String comparison follows the source" in `docs/how-it-works.md`.
  - A section per database for its differences. The MySQL section states that MySQL has no `FULL OUTER JOIN`. The Trino section holds the `LIKE` backslash bullet, moved from `docs/how-it-works.md:82`.
- [ ] 1.5 In `pushdown_query.rs`, add one method on `Expression` (`:92`) that classifies a `SUM` or `AVG` operand by the conversion Exasol applies to it: `BOOLEAN` (a column declared `ExasolType::Boolean`, or a predicate), character (a column declared `Varchar` or `Char`, or a `Literal::String`), or none. Its doc comment states that it reproduces Exasol's implicit conversion of the emitted value and is not a read expression. Add `aggregate_operand_conversion_follows_the_exasol_type` to `pushdown_query_tests.rs`.

Rows of the `docs/pushdown.md` table, derived from the three `advertised` lists before group B. The rows cover all 34, 35, and 35 entries.

| Row | Capabilities | MySQL | PostgreSQL | Trino |
|-----|--------------|-------|------------|-------|
| Column selection | `SELECTLIST_PROJECTION`, `SELECTLIST_EXPRESSIONS` | Yes | Yes | Yes |
| Filters with comparisons, `IS NULL`, `IS NOT NULL`, `BETWEEN`, `IN` with a list of constants, `LIKE`, `AND`, `OR`, and `NOT` | `FILTER_EXPRESSIONS`, `FN_PRED_EQUAL`, `FN_PRED_NOTEQUAL`, `FN_PRED_LESS`, `FN_PRED_LESSEQUAL`, `FN_PRED_IS_NULL`, `FN_PRED_IS_NOT_NULL`, `FN_PRED_BETWEEN`, `FN_PRED_IN_CONSTLIST`, `FN_PRED_LIKE`, `FN_PRED_AND`, `FN_PRED_OR`, `FN_PRED_NOT` | Yes | Yes | Yes |
| `LIKE` with an `ESCAPE` clause | `FN_PRED_LIKE_ESCAPE` | Yes | Yes | No |
| `NULL`, number, string, date, and timestamp literals | `LITERAL_NULL`, `LITERAL_EXACTNUMERIC`, `LITERAL_DOUBLE`, `LITERAL_STRING`, `LITERAL_DATE`, `LITERAL_TIMESTAMP` | Yes | Yes | Yes |
| Aggregation without `GROUP BY` | `AGGREGATE_SINGLE_GROUP` | Yes | Yes | Yes |
| `GROUP BY` one column | `AGGREGATE_GROUP_BY_COLUMN` | Yes | Yes | Yes |
| `GROUP BY` several columns | `AGGREGATE_GROUP_BY_TUPLE` | No | No | Yes |
| `COUNT`, `COUNT(*)`, `COUNT(DISTINCT)` | `FN_AGG_COUNT`, `FN_AGG_COUNT_STAR`, `FN_AGG_COUNT_DISTINCT` | Yes | Yes | Yes |
| `MIN`, `MAX` | `FN_AGG_MIN`, `FN_AGG_MAX` | Yes | Yes | Yes |
| `SUM`, `SUM(DISTINCT)` | `FN_AGG_SUM`, `FN_AGG_SUM_DISTINCT` | No | No | No |
| `AVG`, `AVG(DISTINCT)` | `FN_AGG_AVG`, `FN_AGG_AVG_DISTINCT` | No | No | No |
| `HAVING` | `AGGREGATE_HAVING` | No | No | No |
| Inner, left outer, and right outer join on an equality | `JOIN`, `JOIN_TYPE_INNER`, `JOIN_TYPE_LEFT_OUTER`, `JOIN_TYPE_RIGHT_OUTER`, `JOIN_CONDITION_EQUI` | Yes | Yes | Yes |
| Full outer join on an equality | `JOIN_TYPE_FULL_OUTER` | No | Yes | Yes |

### 2. MySQL dialect

- [ ] 2.1 Add `AGGREGATE_HAVING`, `FN_AGG_SUM`, `FN_AGG_SUM_DISTINCT`, `FN_AGG_AVG`, and `FN_AGG_AVG_DISTINCT` to `crates/adbc-vs-scan/drivers/mysql.toml`. Update the MySQL constant (`capabilities_tests.rs:239`) and its comments (`:245-260`) so `mysql_capability_set_is_exact` pins 39 entries.
- [ ] 2.2 In `mysql_dialect.rs`, add `SUM` and `AVG` to `AGGREGATE_FUNCTIONS` (`:77`). Render `SUM` natively and `AVG` as `(CAST(SUM(x) AS DOUBLE) / COUNT(x))` in `render_aggregate` (`:499`), and update the error text of `renderable_function` (`:652`). With the classification of task 1.5, render a character operand as `CAST(NULLIF(<operand>, '') AS DOUBLE)` inside `SUM` and as `NULLIF(<operand>, '')` inside the `AVG` quotient's `COUNT`, and leave a `BOOLEAN` operand unchanged.
- [ ] 2.3 Render the `having` predicate as `HAVING` after the grouping in `render` (`:156`), in the select-list scope (`any_value_keys`, struct at `:273`), so a grouped converted column reads inside `ANY_VALUE(<read expression>)`.
- [ ] 2.4 In `mysql_dialect_tests.rs`, add `query_exercising` arms (`:949`) for the five capabilities. Add `sum_renders_with_and_without_distinct`, `avg_renders_as_the_double_quotient_of_sum_and_count`, `sum_and_avg_convert_boolean_and_character_operands`, `having_renders_after_group_by_and_on_a_single_group`, and `having_reads_a_grouped_converted_column_inside_any_value`. Extend `mixed_case_function_names_render_identically` (`:649`) with `sum` and `avg`. Remove the `AVG` and `SUM` cases from `unrenderable_aggregate_named_in_error` (`:678`).
- [ ] 2.5 Add a `measures` fixture table to the MySQL test source and to the `MIRROR` schema (`tests/common/mysql.rs`, `tests/common/mirror.rs`). It holds a grouping column, a `BIGINT` column, a `DECIMAL(30,2)` column, a `DOUBLE` column, and a `FLOAT` column across three groups. One group holds only null operands. The floating-point values are binary fractions whose partial sums are exact in double precision. The table also holds a `BIT(1)` column and a `VARCHAR` column whose values are binary-fraction decimals as text, with an empty string and `'5'` beside `'5.0'` in one group.
- [ ] 2.6 In `e2e_mysql_test.rs`, add `sum_and_avg_are_delegated_and_match_the_mirror`, `sum_and_avg_over_boolean_and_character_columns_match_the_mirror`, and `having_is_delegated_and_matches_the_mirror`. Each asserts the fragment through `pushdown_statement` (`:2964`) and the rows through `assert_matches_mirror_of` (`:1235`). The `BIGINT` and `DECIMAL(30,2)` sums read a MySQL `DECIMAL` wider than 38 digits; if the scan cannot cast the driver's Arrow type for it, extend `crates/adbc-vs-scan/src/coerce.rs` under the existing `scan-udf/arrow-coercion` scenario "Decimal width variation is normalized", with a test in `coerce_tests.rs`. Change the `SUM`-beside-`COUNT` case of `aggregation_not_delegable_whole_is_withdrawn_and_matches_local_exasol_result` (`:2708`) to `STDDEV` beside `COUNT`.
- [ ] 2.7 Update the stale comments that say the shipped dialect withholds `SUM` and `AVG`: `e2e_mysql_test.rs:535`, `tests/common/probe.rs:10-11` and `:204`.
- [ ] 2.8 In `docs/pushdown.md`, set the MySQL cells of the `SUM`, `AVG`, and `HAVING` rows to Yes, so every MySQL cell matches `mysql.toml` after task 2.1. Add to the MySQL section that a delegated `AVG` needs MySQL 8.0.17 or later, because `CAST(... AS DOUBLE)` needs it, and that a delegated `SUM` or `AVG` over a character column reads a value that is not a number as its numeric prefix or as 0, where Exasol fails the query.

### 3. PostgreSQL dialect

- [ ] 3.1 Add the five capabilities to `crates/adbc-vs-scan/drivers/postgresql.toml` and rewrite its comment (`:55`). Update the PostgreSQL constant (`capabilities_tests.rs:318`) so `postgres_capability_set_is_exact` pins 40 entries.
- [ ] 3.2 In `postgres_dialect.rs`, add `SUM` and `AVG` to `AGGREGATE_FUNCTIONS` (`:105`). In `render_aggregate` (`:503`), render `SUM` with an operand declared `DOUBLE` cast to `DOUBLE PRECISION`, and `AVG` as `(CAST(SUM(x) AS DOUBLE PRECISION) / COUNT(x))` over that `SUM`. With the classification of task 1.5, render a `BOOLEAN` operand as `CAST(<operand> AS INTEGER)` and a character operand as `CAST(NULLIF(<operand>, '') AS DOUBLE PRECISION)` inside `SUM`, and the `AVG` quotient's `COUNT` over the unconverted operand, `NULLIF(<operand>, '')` for a character one. Keep the `bool_and` and `bool_or` path for `MIN` and `MAX`, and have it use the same classification in place of `is_boolean_operand` (`:668-676`).
- [ ] 3.3 Render the `having` predicate as `HAVING` after the grouping in `render` (`:164`).
- [ ] 3.4 In `postgres_dialect_tests.rs`, add `query_exercising` arms (`:1374`) for the five capabilities. Add `sum_renders_with_and_without_distinct`, `sum_over_a_double_operand_casts_it_to_double_precision`, `avg_renders_as_the_double_quotient_of_sum_and_count`, `sum_and_avg_convert_boolean_and_character_operands`, and `having_renders_after_group_by_and_on_a_single_group`. Extend `mixed_case_function_names_render_identically` (`:1077`). Remove the `AVG` and `SUM` cases from `unrenderable_aggregate_named_in_error` (`:1105`) and the `having` arm from `order_by_and_limit_elements_fail_named` (`:1249`).
- [ ] 3.5 Add a `measures` fixture table to `docker/postgres/init.sql` and to `PG_MIRROR` (`tests/common/postgres_mirror.rs`) with the columns of task 2.5, a `boolean` column in place of `BIT(1)`, a `real` column in place of `FLOAT` holding `16777216` and `1` in one group, and a separate table holding a `double precision` `NaN` beside finite values, which the mirror does not copy.
- [ ] 3.6 In `e2e_postgres_test.rs`, add `sum_and_avg_are_delegated_and_match_the_mirror`, `sum_and_avg_over_boolean_and_character_columns_match_the_mirror`, `having_is_delegated_and_matches_the_mirror`, `sum_over_a_real_column_accumulates_in_double_precision`, and `a_nan_operand_makes_the_delegated_sum_and_avg_null`, asserting the fragment through `generated_statement` (`:2468`) and the rows through `assert_matches_mirror` (`:2442`). Update the comment at `:755`.
- [ ] 3.7 In `docs/pushdown.md`, set the PostgreSQL cells of the `SUM`, `AVG`, and `HAVING` rows to Yes, so every PostgreSQL cell matches `postgresql.toml` after task 3.1. Add to the PostgreSQL section that a `SUM` over a `real` column adds in double precision, so it returns the sum Exasol computes over the same values as `DOUBLE`.

### 4. Trino dialect

- [ ] 4.1 Add the five capabilities to `crates/adbc-vs-scan/drivers/trino.toml` and rewrite its comments (`:59-61`, `:79-89`). Update the Trino constant (`capabilities_tests.rs:384`) so `trino_capability_set_is_exact` pins 40 entries, and the shared-entry count in `trino_and_postgresql_sets_differ_by_two_named_entries` (`:434`).
- [ ] 4.2 In `trino_dialect.rs`, add `SUM` and `AVG` to `AGGREGATE_FUNCTIONS` (`:126`). In `render_aggregate` (`:482`), render `SUM` with an operand declared `DECIMAL(p,s)` cast to `DECIMAL(38,s)`, and `AVG` as `(CAST(SUM(x) AS DOUBLE) / COUNT(x))` over that `SUM`. With the classification of task 1.5, render a `BOOLEAN` operand, which for Trino is a predicate, as `CAST(<operand> AS INTEGER)` and a character operand as `CAST(NULLIF(<operand>, '') AS DOUBLE)` inside `SUM`, and the `AVG` quotient's `COUNT` over the unconverted operand, `NULLIF(<operand>, '')` for a character one.
- [ ] 4.3 Render the `having` predicate as `HAVING` after the grouping in `render` (`:178`).
- [ ] 4.4 In `trino_dialect_tests.rs`, add `query_exercising` arms (`:769`) for the five capabilities. Add `sum_renders_with_and_without_distinct`, `sum_over_a_decimal_operand_casts_it_to_decimal_38`, `avg_renders_as_the_double_quotient_of_sum_and_count`, `sum_and_avg_convert_boolean_and_character_operands`, `having_renders_after_group_by_and_on_a_single_group`, and `mixed_case_function_names_render_identically`. Change `unrenderable_aggregate_named_in_error` (`:509`) from `sum` to `stddev`.
- [ ] 4.5 Add a `measures` fixture table to `docker/trino/init.sql` and to `TR_MIRROR` (`tests/common/trino_mirror.rs`) with a grouping column, a `bigint` column holding `9223372036854775807` and `1` in one group, a `decimal(12,4)` column, a `double` column, and a `varchar` column holding the text values of task 2.5, under the value rules of task 2.5.
- [ ] 4.6 In `e2e_trino_test.rs`, add `sum_and_avg_are_delegated_and_match_the_mirror`, `sum_and_avg_over_boolean_and_character_columns_match_the_mirror` (over the `varchar` column and over a predicate on the `bigint` column), `having_is_delegated_and_matches_the_mirror`, and `sum_over_a_bigint_column_sums_past_the_bigint_range`, asserting the fragment through `generated_statement` (`:1250`) and the rows through `assert_matches_mirror` (`:1234`).
- [ ] 4.7 In `docs/pushdown.md`, set the Trino cells of the `SUM`, `AVG`, and `HAVING` rows to Yes, so every Trino cell matches `trino.toml` after task 4.1. Add to the Trino section that a `SUM` over a `bigint` or `decimal` column adds in a 38-digit decimal, so a sum beyond the `bigint` range returns its value instead of a Trino overflow error.

## Parallelization

| Group | Tasks | Depends on | Knowledge |
|-------|-------|------------|-----------|
| A: Query model and documentation | 1.1-1.5 | — | spec deltas `vs-adapter/pushdown-aggregation`, `vs-adapter/pushdown-aggregation-delegation`, `vs-adapter/pushdown-aggregation-types`, `vs-adapter/pushdown-dialect`; `crates/adbc-vs-adapter/src/pushdown_query.rs`, `pushdown_query_tests.rs`, `emit_type_tests.rs`, `capabilities_tests.rs`, `crates/adbc-vs-scan/drivers/{mysql,postgresql,trino}.toml` (read only), `docs/{pushdown,how-it-works,index,install,create-virtual-schema,type-mapping}.md`, `README.md` |
| B: MySQL dialect | 2.1-2.8 | A (uses the `having` model); shares `capabilities_tests.rs` | spec delta `vs-adapter/mysql-pushdown` plus group A's deltas; `crates/adbc-vs-scan/drivers/mysql.toml`, `crates/adbc-vs-adapter/src/mysql_dialect.rs`, `mysql_dialect_tests.rs`, `capabilities_tests.rs`, `tests/e2e_mysql_test.rs`, `tests/common/{mysql,mirror,probe}.rs`, `crates/adbc-vs-scan/src/coerce.rs`, `docs/pushdown.md` |
| C: PostgreSQL dialect | 3.1-3.7 | B (shares `capabilities_tests.rs`) | spec delta `vs-adapter/postgresql-pushdown` plus group A's deltas; `crates/adbc-vs-scan/drivers/postgresql.toml`, `crates/adbc-vs-adapter/src/postgres_dialect.rs`, `postgres_dialect_tests.rs`, `capabilities_tests.rs`, `tests/e2e_postgres_test.rs`, `tests/common/postgres_mirror.rs`, `docker/postgres/init.sql`, `docs/pushdown.md` |
| D: Trino dialect | 4.1-4.7 | C (shares `capabilities_tests.rs`, and the PostgreSQL set must be final for the set comparison) | spec delta `vs-adapter/trino-pushdown` plus group A's deltas; `crates/adbc-vs-scan/drivers/trino.toml`, `crates/adbc-vs-adapter/src/trino_dialect.rs`, `trino_dialect_tests.rs`, `capabilities_tests.rs`, `tests/e2e_trino_test.rs`, `tests/common/trino_mirror.rs`, `docker/trino/init.sql`, `docs/pushdown.md` |

- The groups run in sequence. `capabilities_tests.rs` runs every dialect's invariant and exact-set tests, so a dialect group mid-change turns another group's run red.
- `docs/pushdown.md` is shared the same way. Group A writes the page at the capability sets before group B. Each dialect group then changes only its own database's column and section, so the page matches the driver capability sets after every group.

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| Constant entry | `UNDELEGATED_ELEMENTS` `("having", "AGGREGATE_HAVING")`, `crates/adbc-vs-adapter/src/pushdown_query.rs:336`, comment `:322-323` | `having` is parsed and rendered |
| Test | `a_having_element_is_refused`, `crates/adbc-vs-adapter/src/pushdown_query_tests.rs:227` | Replaced by `a_having_element_parses_into_the_query_model` |
| Function | `is_boolean_operand`, `crates/adbc-vs-adapter/src/postgres_dialect.rs:668-676` | Replaced by the operand classification of task 1.5 |
| Test arm | `having` arm of `order_by_and_limit_elements_fail_named`, `crates/adbc-vs-adapter/src/postgres_dialect_tests.rs:1253-1259` | `having` is no longer refused |
| Test cases | `AVG` and `SUM` cases of `unrenderable_aggregate_named_in_error` in `mysql_dialect_tests.rs:682-683`, `postgres_dialect_tests.rs:1109-1110`, `trino_dialect_tests.rs:509` | Both functions render |
| Test | `avg_and_sum_are_withdrawn_and_match_local_exasol_result` and helper `assert_sum_over_a_double_column_answers`, `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs:1436`, `:1478` | Replaced by `sum_and_avg_are_delegated_and_match_the_mirror` |
| Test | `avg_and_sum_are_withdrawn_and_match_the_mirror`, `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs:766` | Replaced by `sum_and_avg_are_delegated_and_match_the_mirror` |
| Test | `avg_and_sum_are_withdrawn_and_match_the_mirror`, `crates/adbc-vs-adapter/tests/e2e_trino_test.rs:398` | Replaced by `sum_and_avg_are_delegated_and_match_the_mirror` |
| Comments | Withholding comments in `postgresql.toml:55`, `trino.toml:59-61,79-89`, `capabilities_tests.rs:245-260`, `probe.rs:10-11,204`, `e2e_mysql_test.rs:535,1431`, `e2e_postgres_test.rs:755` | They state the withdrawn withholding |

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| pushdown-aggregation: A having element renders as a HAVING clause | Unit | `crates/adbc-vs-adapter/src/{mysql,postgres,trino}_dialect_tests.rs`, `pushdown_query_tests.rs` | `having_renders_after_group_by_and_on_a_single_group`, `a_having_element_parses_into_the_query_model` |
| pushdown-aggregation: Aggregate function name case does not change the rendered statement | Unit | `crates/adbc-vs-adapter/src/{mysql,postgres,trino}_dialect_tests.rs` | `mixed_case_function_names_render_identically` |
| pushdown-aggregation: SUM renders as the source's SUM over the operand | Unit | `crates/adbc-vs-adapter/src/{mysql,postgres,trino}_dialect_tests.rs` | `sum_renders_with_and_without_distinct` |
| pushdown-aggregation: AVG renders as the double-precision quotient of SUM and COUNT | Unit + Integration | `crates/adbc-vs-adapter/src/{mysql,postgres,trino}_dialect_tests.rs`; `tests/e2e_{mysql,postgres,trino}_test.rs` | `avg_renders_as_the_double_quotient_of_sum_and_count`; `sum_and_avg_are_delegated_and_match_the_mirror` (null group) |
| pushdown-aggregation: A BOOLEAN or character SUM and AVG operand renders converted as Exasol converts it | Unit | `crates/adbc-vs-adapter/src/{mysql,postgres,trino}_dialect_tests.rs`, `pushdown_query_tests.rs` | `sum_and_avg_convert_boolean_and_character_operands`, `aggregate_operand_conversion_follows_the_exasol_type` |
| pushdown-aggregation-delegation: SUM and AVG over a BOOLEAN or character operand are delegated and match Exasol | Integration | `crates/adbc-vs-adapter/tests/e2e_{mysql,postgres,trino}_test.rs` | `sum_and_avg_over_boolean_and_character_columns_match_the_mirror` |
| pushdown-aggregation-delegation: SUM and AVG are delegated by every dialect and match Exasol over exact-numeric operands | Integration | `crates/adbc-vs-adapter/tests/e2e_{mysql,postgres,trino}_test.rs` | `sum_and_avg_are_delegated_and_match_the_mirror` |
| pushdown-aggregation-delegation: A floating-point SUM or AVG follows the source's accumulation order | Unit + Integration | `crates/adbc-vs-adapter/src/capabilities_tests.rs`; `tests/e2e_{mysql,postgres,trino}_test.rs`; `tests/e2e_postgres_test.rs` | `{mysql,postgres,trino}_capability_set_is_exact`; `sum_and_avg_are_delegated_and_match_the_mirror` (`DOUBLE` column delegated, no refusal); `a_nan_operand_makes_the_delegated_sum_and_avg_null` |
| pushdown-aggregation-delegation: HAVING is delegated by every dialect | Integration | `crates/adbc-vs-adapter/tests/e2e_{mysql,postgres,trino}_test.rs` | `having_is_delegated_and_matches_the_mirror` |
| pushdown-aggregation-delegation: Grouping by an expression is withheld from every dialect | Unit | `crates/adbc-vs-adapter/src/capabilities_tests.rs` | `mysql_capability_set_is_exact`, `postgres_capability_set_is_exact`, `trino_capability_set_is_exact` |
| pushdown-aggregation-delegation: An aggregation Exasol cannot delegate whole is computed by Exasol | Integration | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` | `aggregation_not_delegable_whole_is_withdrawn_and_matches_local_exasol_result` |
| pushdown-aggregation-delegation: Every dialect delegates the same aggregate set | Unit | `crates/adbc-vs-adapter/src/capabilities_tests.rs` | `{mysql,postgres,trino}_capability_set_is_exact`, `every_advertised_capability_renders_per_dialect` |
| pushdown-aggregation-types: Each aggregate declares the type the request supplies for it | Unit | `crates/adbc-vs-adapter/src/emit_type_tests.rs` | `declared_types_come_from_select_list_data_types` |
| pushdown-aggregation-types: Exasol's aggregate types match the values this release was measured against | Integration | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs`; `tests/e2e_{mysql,postgres,trino}_test.rs` | `exasol_supplies_the_measured_aggregate_types`; `sum_and_avg_are_delegated_and_match_the_mirror` (Exasol accepts the `EMITS` clause) |
| pushdown-dialect: The declarable set is closed under the advertised capabilities | Integration | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` | `exasol_supplies_the_measured_aggregate_types` |
| mysql-pushdown: Capability set advertises the fragments this dialect renders | Unit | `crates/adbc-vs-adapter/src/capabilities_tests.rs` | `mysql_capability_set_is_exact`, `advertised_set_matches_the_rendered_fragments` |
| mysql-pushdown: Aggregation capabilities cover single-group and column grouping | Unit | `crates/adbc-vs-adapter/src/capabilities_tests.rs` | `mysql_capability_set_is_exact` |
| mysql-pushdown: Aggregation pushdown reaches MySQL rather than Exasol | Integration | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` | `aggregation_matches_local_exasol_result`, `sum_and_avg_are_delegated_and_match_the_mirror` |
| mysql-pushdown: A HAVING clause reads a grouped converted column the way the select list does | Unit | `crates/adbc-vs-adapter/src/mysql_dialect_tests.rs` | `having_reads_a_grouped_converted_column_inside_any_value` |
| postgresql-pushdown: Capability set advertises the fragments this dialect renders | Unit | `crates/adbc-vs-adapter/src/capabilities_tests.rs` | `postgres_capability_set_is_exact` |
| postgresql-pushdown: Aggregation pushdown reaches PostgreSQL rather than Exasol | Integration | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `aggregation_reaches_postgresql_and_matches_the_mirror`, `sum_and_avg_are_delegated_and_match_the_mirror` |
| postgresql-pushdown: SUM over a real column accumulates in double precision | Unit + Integration | `crates/adbc-vs-adapter/src/postgres_dialect_tests.rs`; `tests/e2e_postgres_test.rs` | `sum_over_a_double_operand_casts_it_to_double_precision`; `sum_over_a_real_column_accumulates_in_double_precision` |
| trino-pushdown: Capability set advertises the fragments this dialect renders | Unit | `crates/adbc-vs-adapter/src/capabilities_tests.rs` | `trino_capability_set_is_exact` |
| trino-pushdown: Aggregation pushdown reaches Trino rather than Exasol | Integration | `crates/adbc-vs-adapter/tests/e2e_trino_test.rs` | `aggregation_reaches_trino_and_matches_the_mirror`, `sum_and_avg_are_delegated_and_match_the_mirror` |
| trino-pushdown: Capability set withholds the fragments the Trino dialect does not delegate | Unit | `crates/adbc-vs-adapter/src/capabilities_tests.rs` | `trino_capability_set_is_exact` |
| trino-pushdown: The Trino set differs from the PostgreSQL set by two named entries, and each is stated on its own | Unit | `crates/adbc-vs-adapter/src/capabilities_tests.rs` | `trino_and_postgresql_sets_differ_by_two_named_entries` |
| trino-pushdown: SUM over an exact-numeric column sums in Trino's 38-digit decimal | Unit + Integration | `crates/adbc-vs-adapter/src/trino_dialect_tests.rs`; `tests/e2e_trino_test.rs` | `sum_over_a_decimal_operand_casts_it_to_decimal_38`; `sum_over_a_bigint_column_sums_past_the_bigint_range` |

### Manual Testing

Run each command after `make stack-up` and `make cross-udf-build`, against virtual schemas `MYSQLVS`, `PGVS`, and `TRINOVS` created over the test sources per `docs/create-virtual-schema.md`. `$EXASOL_DSN` is the DSN the Makefile's `stack-up-exasol` target uses. `PUSHDOWN_SQL` embeds the source statement in the scan specification JSON, where each `"` appears as `\"`.

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| mysql-pushdown, pushdown-aggregation | `exapump sql --dsn "$EXASOL_DSN" 'SELECT PUSHDOWN_SQL FROM (EXPLAIN VIRTUAL SELECT "name", SUM("amount"), AVG("amount") FROM MYSQLVS."orders" GROUP BY "name" HAVING COUNT(*) > 0)'` | The statement carries ``SUM(`amount`)``, ``(CAST(SUM(`amount`) AS DOUBLE) / COUNT(`amount`))``, ``GROUP BY `name` ``, and `HAVING` over `COUNT(*)` |
| postgresql-pushdown | `exapump sql --dsn "$EXASOL_DSN" 'SELECT PUSHDOWN_SQL FROM (EXPLAIN VIRTUAL SELECT "name", SUM("price"), AVG("amount") FROM PGVS."orders" GROUP BY "name" HAVING SUM("price") > 0)'` | The statement carries `SUM(CAST("price" AS DOUBLE PRECISION))`, `(CAST(SUM("amount") AS DOUBLE PRECISION) / COUNT("amount"))`, and a `HAVING` clause |
| trino-pushdown | `exapump sql --dsn "$EXASOL_DSN" 'SELECT PUSHDOWN_SQL FROM (EXPLAIN VIRTUAL SELECT "name", SUM("big_id"), AVG("price") FROM TRINOVS."orders" GROUP BY "name" HAVING AVG("price") > 0)'` | The statement carries `SUM(CAST("big_id" AS DECIMAL(38,0)))`, `(CAST(SUM("price") AS DOUBLE) / COUNT("price"))`, and a `HAVING` clause |
| pushdown-aggregation-delegation | `exapump sql --dsn "$EXASOL_DSN" 'SELECT "name", SUM("amount"), AVG("amount") FROM PGVS."orders" GROUP BY "name"'` | Each name's `SUM` and `AVG` equal its seeded `amount` in `docker/postgres/init.sql`, and the name whose `amount` is null returns null for both |
| pushdown-aggregation (operand conversion) | `exapump sql --dsn "$EXASOL_DSN" 'SELECT PUSHDOWN_SQL FROM (EXPLAIN VIRTUAL SELECT SUM("flag"), AVG("label") FROM PGVS."type_coverage")'` | The statement carries `SUM(CAST("flag" AS INTEGER))` and `(CAST(SUM(CAST(NULLIF("label", '') AS DOUBLE PRECISION)) AS DOUBLE PRECISION) / COUNT(NULLIF("label", '')))` |
| pushdown-aggregation-delegation (operand conversion) | `exapump sql --dsn "$EXASOL_DSN" 'SELECT SUM("flag"), COUNT("flag") FROM PGVS."type_coverage"'` | `SUM` equals the number of rows whose `flag` is true in `docker/postgres/init.sql`, and no source error |
| pushdown-aggregation-delegation (documented workaround) | `exapump sql --dsn "$EXASOL_DSN" 'SELECT PUSHDOWN_SQL FROM (EXPLAIN VIRTUAL SELECT SUM("price" + 0) FROM PGVS."orders")'` | The statement carries no `SUM`, so Exasol computes the sum, as `docs/pushdown.md` states |
| pushdown-aggregation-types, pushdown-dialect | `exapump sql --dsn "$EXASOL_DSN" 'SELECT SUM("amount"), AVG("price") FROM MYSQLVS."orders"'` | One row and no declared-type error |

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Build | `make cross-udf-build` | Exit 0 |
| Unit tests | `cargo test` | 0 failures |
| E2E MySQL | `make test-e2e` | 0 failures |
| E2E PostgreSQL | `make test-e2e-postgres` | 0 failures |
| E2E Trino | `make test-e2e-trino` | 0 failures |
| Lint | `make lint` | 0 errors or warnings |
| Format | `make fmt-check` | No changes |
| Docs | Compare each database column of `docs/pushdown.md` with `advertised` in `crates/adbc-vs-scan/drivers/<database>.toml`, and follow every navigation link of `docs/*.md` and the documentation links of `README.md` | Each Yes row's capabilities are all in the list, each No row's are absent, the rows cover every list entry, and every link opens a page |
