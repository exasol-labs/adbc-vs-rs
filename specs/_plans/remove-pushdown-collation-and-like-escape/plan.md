# Plan: remove-pushdown-collation-and-like-escape

## Summary

Push-down stops emulating Exasol's string semantics: no dialect renders a collation, the MySQL source check stops requiring collations and a server version, and the Trino dialect withdraws `FN_PRED_LIKE_ESCAPE` and renders a plain `LIKE`. The specs and the README state that a delegated character comparison follows the source's collation. They give a user who needs Exasol's code-point semantics two remedies: a code-point collation at the source, or an operand wrapped in an expression no dialect advertises, which Exasol then evaluates itself.

## Context

- This is phase 1 of 3. Phase 2 refactors push-down into pluggable per-dialect code, and phase 3 adds `SUM`/`AVG` and a scalar-function allow-list. Neither is planned here, and nothing here blocks them.
- The requester's model: push-down is the first choice because it is fastest, the push-down layer stays simple, and where Exasol and the source differ with no straightforward rendering, the adapter passes the fragment through or withdraws the capability. It does not emulate.
- The comparison report (`pushdown-comparison-report.md`) found that collation compensation is the largest part of the push-down layer: about 350 lines in MySQL (dialect plus source check) and about 50 in PostgreSQL. Exasol's JDBC virtual schemas for MySQL and PostgreSQL render no collation.
- The MySQL compensation renders `COLLATE` on every character operand. It also forces a named error for one composed shape (a select-list predicate over the grouped character key) and a source check for two collations and MySQL 8.0.17.
- The Trino dialect renders `ESCAPE '\'` on every `LIKE` and validates each pattern, because Trino has no default escape character and rejects an escape character before an ordinary character.
- Exasol 2025.2.1 accepts no `COLLATE` clause in a query (measured on the local container), so a user chooses a collation only at the source.
- The other remedy keeps a predicate in Exasol. No dialect advertises a scalar-function capability (`drivers/*.toml`), so Exasol does not delegate a predicate over a wrapped operand such as `"name" || ''`. A throwaway Lua adapter probe on Exasol 2025.2.1 measured this for an equality, an ordering comparison, `GROUP BY`, and `MIN`. In a conjunction, Exasol still delegated the conjuncts that hold no wrapper. Decision [1] records the measurements.
- MySQL and PostgreSQL render a request's `escapeChar` verbatim, with no default clause and no validation.
- Withdrawing `FN_PRED_LIKE_ESCAPE` while keeping `FN_PRED_LIKE` relies on Exasol keeping a `LIKE ... ESCAPE` predicate for itself. Exasol's MySQL JDBC virtual schema relies on the same rule: `MySQLSqlDialect` declares `LIKE` and no `LIKE_ESCAPE`. The same Lua adapter probe measured the rule on Exasol 2025.2.1 with `FN_PRED_LIKE` advertised and `FN_PRED_LIKE_ESCAPE` withheld. `LIKE 'a%'` and `LIKE 'a\%'` arrived with no `escapeChar`, and Exasol delegated neither `LIKE 'a!%' ESCAPE '!'` nor `LIKE 'a\%' ESCAPE '\'`. Task 3.5 checks the same rule against the real adapter.
- Capability sets today: MySQL 34, PostgreSQL 35, Trino 36 (`drivers/*.toml`). After this plan: MySQL 34, PostgreSQL 35, Trino 35.
- `README.md` carries uncommitted edits by the user. Tasks edit it on top of the working tree.
- The architecture delta (`architecture.md` in this directory) records that `sql_dialect` renders no collation and adds the passthrough constraint.

## Features

| Feature | Status | Spec |
|---------|--------|------|
| mysql-collation | CHANGED | `vs-adapter/mysql-collation/spec.md` |
| postgresql-collation | CHANGED | `vs-adapter/postgresql-collation/spec.md` |
| trino-collation | CHANGED | `vs-adapter/trino-collation/spec.md` |
| pushdown-filter | CHANGED | `vs-adapter/pushdown-filter/spec.md` |
| pushdown-aggregation | CHANGED | `vs-adapter/pushdown-aggregation/spec.md` |
| pushdown-join | CHANGED | `vs-adapter/pushdown-join/spec.md` |
| pushdown-dialect | CHANGED | `vs-adapter/pushdown-dialect/spec.md` |
| trino-pushdown | CHANGED | `vs-adapter/trino-pushdown/spec.md` |
| mysql-pushdown | CHANGED | `vs-adapter/mysql-pushdown/spec.md` |
| postgresql-pushdown | CHANGED | `vs-adapter/postgresql-pushdown/spec.md` |
| mysql-type-mapping | CHANGED | `vs-adapter/mysql-type-mapping/spec.md` |
| postgresql-type-mapping | CHANGED | `vs-adapter/postgresql-type-mapping/spec.md` |
| trino-type-mapping | CHANGED | `vs-adapter/trino-type-mapping/spec.md` |
| type-mapping | CHANGED | `vs-adapter/type-mapping/spec.md` |
| create-virtual-schema | CHANGED | `vs-adapter/create-virtual-schema/spec.md` |
| driver-selection | CHANGED | `vs-adapter/driver-selection/spec.md` |

## Impact

Breaking changes to query results. They take effect when the new `.so` replaces the old one.

- **MySQL, results change.** Over a column under MySQL's default `utf8mb4_0900_ai_ci`, a delegated `=`, `LIKE`, and `IN` match case and accent variants. `GROUP BY` and `COUNT(DISTINCT)` fold those variants together, and `MIN`/`MAX` follow collation weight. Exasol computed each of these with code-point semantics before.
- **PostgreSQL, results change.** A delegated `<`, `<=`, `BETWEEN`, `MIN`, and `MAX` over a character column follow the column's collation, which is the database locale by default (`en_US.utf8` in the official image). Equality and grouping are unchanged for every built-in collation.
- **PostgreSQL, columns appear.** A `character varying` column with a nondeterministic collation was omitted. It is mapped after `ALTER VIRTUAL SCHEMA ... REFRESH`, and comparisons over it follow that collation. On PostgreSQL before 18, a delegated `LIKE` over such a column fails at the source: the PostgreSQL 17 documentation (§ 23.2.2.4) states that pattern matching is not possible under a nondeterministic collation, and the PostgreSQL 18 release notes list "Allow `LIKE` with nondeterministic collations".
- **MySQL, fewer refusals.** A server without `utf8mb4_0900_as_cs` and `utf8mb4_0900_bin` (MySQL older than 8.0.17, MariaDB) is no longer refused at creation. The adapter stays tested against MySQL 8.4 only.
- **MySQL, one error removed.** A select-list predicate over the grouped character column, which failed with a named error, is now delegated.
- **Trino, results change.** A plain `LIKE` whose pattern holds a backslash is matched by Trino's rule, where the backslash is an ordinary character. A `LIKE` with an explicit `ESCAPE` clause is now evaluated by Exasol after the scan, which is slower.
- **Remedies for Exasol's semantics.** A user who controls the source declares `utf8mb4_0900_bin` (MySQL 8.0.17 or newer; MariaDB has no equivalent `NO PAD` binary collation) or `"C"` (PostgreSQL) on the source column or in a source view, and the fragment stays delegated. A user who does not control the source wraps the operand in an expression no dialect advertises, such as `"name" || ''`, and Exasol evaluates that predicate, grouping, or `MIN`/`MAX` itself. The wrapper is slower, because Exasol fetches the rows the source would have filtered. Phase 3 adds a scalar-function allow-list, which can change which wrappers keep a predicate in Exasol. The README states both remedies.

## Migration

| Current | New |
|---------|-----|
| MySQL renders `COLLATE utf8mb4_0900_as_cs` / `utf8mb4_0900_bin` | No suffix; the column's MySQL collation applies |
| PostgreSQL renders `COLLATE "C"` on ordering operands | No suffix; the column's PostgreSQL collation applies |
| MySQL source check queries `information_schema.COLLATIONS` and `VERSION()` | No collation or version query |
| PostgreSQL nondeterministic-collation column omitted | Mapped |
| Trino advertises `FN_PRED_LIKE_ESCAPE`, renders `ESCAPE` on every `LIKE` | Withheld; plain `LIKE`; explicit `ESCAPE` evaluated by Exasol |

## Implementation Tasks

### 1. MySQL collation removal

- [ ] 1.1 Remove collation rendering from `crates/adbc-vs-adapter/src/mysql_dialect.rs` [expert]: delete `COMPARISON_COLLATION`, `ORDERING_COLLATION`, `ORDERING_FUNCTIONS`, the grouped-column coordination (`Statement.grouped`, the grouping branch of `render_select_item`, `groups`, `names_the_same_column`, `grouped_columns`, the suffix in `render_grouped_column`), the named error (`reject_collated_grouping_key`, `collated_grouping_key_read_by`, `collated_grouping_key_in`), the suffix branches of `render_compared_operand`, `render_aggregate`, and `render_ordering_operand`, the `is_character_*` helpers, and the module doc's collation part. Keep the alias qualification of grouped columns (`a_grouped_numeric_column_carries_the_alias_and_no_collation` stays green) and keep the verbatim `ESCAPE` rendering for a request's `escapeChar`.
- [ ] 1.2 Rewrite `mysql_dialect_tests.rs`: fold the tests that assert a `COLLATE` suffix into `no_operand_carries_a_collation_suffix_in_any_position`, which asserts no `COLLATE` in `WHERE` and select-list position, on a `GROUP BY` key and its select-list occurrence, on a `DISTINCT` operand, on `MIN`/`MAX`, and on a join key. Keep `a_grouping_key_is_recognized_however_the_request_qualifies_its_occurrences` and `a_joined_grouping_key_is_recognized_however_the_request_qualifies_its_occurrences` as separate tests, rewritten to assert that a grouping key and its select-list occurrence, qualified differently in the request, render identically and with no `COLLATE`, because MySQL rejects the statement with error 1055 when the two differ. Replace the three grouped-key error tests with `a_select_list_predicate_reading_the_grouped_character_column_renders`. Keep `like_renders_pattern_and_escape`, `between_and_in_keep_operand_order`, `a_select_list_predicate_renders_where_the_query_groups_by_nothing`, `a_select_list_predicate_over_an_aggregate_renders`, and `a_join_renders_one_statement_with_every_table_aliased_and_every_column_qualified`, each asserting no suffix.
- [ ] 1.3 Remove the MySQL server check from `mysql_source.rs`: `REQUIRED_COLLATIONS`, `MINIMUM_MYSQL_VERSION`, `COLLATION_NAMES_QUERY`, `SERVER_VERSION_QUERY`, `SourceCollations`, `check_required_collations`, `query_collation_names`, `query_server_version`. `check_source` keeps `reject_unsafe_database_name` only. Restate the `SUPPORTED_CHARACTER_SET` doc to the literal-form reason the `mysql-type-mapping` delta states, and restate the same reason in the `mysql_type_mapping.rs` doc near lines 26-29. Restate the `CHAR` reason near `mysql_type_mapping.rs:118` and the assertion message near `mysql_type_mapping_tests.rs:101` to the delta's reason, that MySQL returns a `CHAR` value with its trailing spaces stripped, with no reference to the collations pushdown renders. Delete the three collation tests in `mysql_source_tests.rs`, empty the MySQL arm of `metadata_source_tests.rs::precondition` and restate that file's doc comments, and change the refusal message of the fake source in `create_schema_tests.rs` (near line 1088) to name no collation.
- [ ] 1.4 MySQL E2E fixture: in `crates/adbc-vs-adapter/tests/common/mysql.rs`, create a table `items_bin` holding the `items` rows from `items.rs`, with every `utf8mb4` character column declared `COLLATE utf8mb4_0900_bin`. `region` stays `CHARACTER SET latin1` with no collation clause, because MySQL rejects `utf8mb4_0900_bin` for `latin1`, so `region` stays unmapped in `items_bin` as in `items`. Seed `items_bin` in the same step as `items` so it exists before the shared setup creates the virtual schema. Update any assertion over the exact table list of `MYSQLVS`. Restate the doc comments in `mysql.rs` and `items.rs` that describe the removed comparison collation.
- [ ] 1.5 MySQL E2E tests in `tests/e2e_mysql_test.rs`: add `comparison_and_grouping_follow_the_columns_own_collation` over `items`, asserting no `COLLATE` in `EXPLAIN VIRTUAL`, an equality against one value of a case pair and of an accent pair returning both values of that pair, one group per pair, and one distinct count per pair. The test asserts the group and distinct counts, and checks each group's reported label only for membership in its pair, because MySQL does not specify which member of a folded group it reports. Retarget `filter_matches_local_exasol_result`, `aggregation_matches_local_exasol_result`, `projected_predicate_matches_local_exasol_result`, and `grouping_over_character_variants_matches_local_exasol_result` to `items_bin` against the existing mirror, and drop their `COLLATE` assertions. To compare `items_bin` with `MIRROR."items"`, change `assert_matches_mirror` so that its query closure receives the table name beside the schema, and pass `items_bin` for the virtual side and `items` for the mirror side; existing callers pass `items` for both. Add `trailing_space_values_compare_and_order_as_in_exasol` over `items_bin`: `MIN("batch")`, `MAX("batch")`, and a filter `"batch" = 'ab'`, each compared with `MIRROR."items"` through `assert_matches_mirror`, with `EXPLAIN VIRTUAL` reporting the `MIN`, the `MAX`, and the equality in the generated statement. The fixture's `batch` values `'ab'` and `'ab '` differ only by a trailing space, so the filter returns the `'ab'` rows and not the `'ab '` rows on both sides. Add `a_comparison_over_a_wrapped_operand_is_evaluated_by_exasol` over `items`: `"category" || '' = 'widget'`, asserting that `EXPLAIN VIRTUAL` reports a generated statement with no comparison over `category` and that the rows equal the mirror's, which hold `widget` and not `Widget`. Replace `select_list_predicate_over_grouped_character_column_fails` with `select_list_predicate_over_grouped_character_column_delegates_and_matches_the_mirror` over `items_bin`. Move any other mirror comparison over a character column of `items` to `items_bin`. Keep `mirror_answers_under_exasols_own_comparison_and_ordering_rules`.

### 2. PostgreSQL collation removal

- [ ] 2.1 Remove `COLLATE "C"` from `crates/adbc-vs-adapter/src/postgres_dialect.rs`: `ORDERING_COLLATION`, `ORDERING_FUNCTIONS`, the suffix in `render_ordering` and the `BETWEEN` branch, `render_ordered_operand`, the suffix in `render_aggregate`, the `is_character_*` helpers, and the module doc's collation part.
- [ ] 2.2 Rewrite `postgres_dialect_tests.rs`: fold `collation_placement_matrix`, `a_character_join_key_renders_bare_beside_a_collated_ordering_comparison`, `grouping_and_distinct_render_uncollated`, `a_distinct_min_keeps_the_ordering_collation`, `only_min_and_max_over_a_character_operand_carry_the_ordering_collation`, and `numeric_and_temporal_operands_carry_no_collation_in_any_position` into `no_operand_carries_a_collation_suffix_in_any_position`. Keep `like_renders_its_escape_clause`, `between_and_in_keep_operand_order`, `select_list_predicate_renders_as_projected_boolean`, `select_list_predicate_over_an_aggregate_renders`, `inner_join_renders_one_statement`, and `a_select_list_predicate_reading_the_grouped_character_column_renders`, each asserting no suffix.
- [ ] 2.3 Remove the nondeterministic-collation guard and the collation read. After this task the adapter reads no PostgreSQL collation. In `postgres_type_mapping.rs` delete `ColumnCollation`, the `PostgresColumn.collation` field, `is_deterministic`, and `require_deterministic_collation`, and drop the per-column-collation sentence from the `PostgresColumn` doc. In `postgres_source.rs` delete `ColumnRow.collation_name`, `ColumnRow.collation_is_deterministic`, `determinism_flag`, `DETERMINISTIC`, `NONDETERMINISTIC`, and, in `columns_query`, the `collation_name` and `collisdeterministic` select items and the `pg_collation`/`pg_namespace` join. In `postgres_type_mapping_tests.rs` delete the `collated()` helper and the two collation tests, `a_character_column_of_a_nondeterministic_collation_is_unmapped` and `a_deterministic_or_unreported_collation_maps`, and add no replacement test. The removal breaks two tests in `postgres_source_tests.rs`, so edit them: drop `collation_name` and `collisdeterministic` from the field list of `the_columns_query_selects_every_field_the_mapping_reads`, delete its `collation_schema` assertion and the determinism paragraph of its doc comment, and drop `collation_name` and `collation_is_deterministic` from the `column_row` fixture. In the same file, reduce `a_character_columns_length_and_collation_reach_the_mapping` to its deterministic-length assertion and rename it `a_character_columns_length_reaches_the_mapping`. Restate the `REQUIRED_ENCODING` doc, the encoding error text in `postgres_source.rs`, and the doc comment of `postgres_source_tests.rs::a_database_of_another_encoding_is_refused_naming_it_and_utf8` to the `UTF8` declaration reason alone; that test stays green.
- [ ] 2.4 PostgreSQL E2E fixture: in `docker/postgres/init.sql`, add a table `items_c` holding the `items` rows, with every character column declared `COLLATE "C"`. Add `items_c` to `SEEDED_OBJECTS` in `tests/common/postgres.rs`, and update any assertion over the exact table list of `PGVS`. The running container applies it only after `docker compose rm -sfv postgres && make stack-up-postgres`.
- [ ] 2.5 PostgreSQL E2E tests in `tests/e2e_postgres_test.rs`: delete `ORDERING_COLLATION`. Replace `collation_is_rendered_only_where_ordering_decides_the_answer` with `ordering_and_min_max_follow_the_columns_own_collation` over `items`, asserting no `COLLATE`, the rows holding `a` for a comparison before `B`, `MIN` = `a`, and `MAX` = `C`. Add `no_generated_statement_carries_a_collation_keyword`. Retarget `filter_reaches_postgresql_and_matches_the_mirror`, `aggregation_reaches_postgresql_and_matches_the_mirror`, and `select_list_predicate_over_the_grouped_character_column_delegates_and_matches_the_mirror` to `items_c`, and drop their `COLLATE "C"` assertions. Add to the `items_c` tests a character `BETWEEN`, a `GROUP BY`, and a `COUNT(DISTINCT)`, each compared with the mirror. Change this file's `assert_matches_mirror` the same way task 1.5 changes the MySQL one, so a query over `items_c` compares with `PG_MIRROR."items"`. Add `an_ordering_comparison_over_a_wrapped_operand_is_evaluated_by_exasol` over `items`: `"sort_key" || '' < 'B'` and `MIN("sort_key" || '')`, asserting that `EXPLAIN VIRTUAL` reports a generated statement with neither the comparison nor the `MIN`, that the comparison returns no row holding `a`, that `MIN` returns `B`, and that both equal the mirror's answer. Keep `uncollated_grouping_and_distinct_return_the_mirrors_group_count` and `full_outer_join_is_delegated_and_returns_both_unmatched_rows` unchanged. Restate the module and `mirror_answers_under_exasols_own_comparison_and_ordering_rules` comments that cite `COLLATE "C"`, and the module doc near `tests/common/postgres_mirror.rs:16`, which names `COLLATE "C"` as what the delegated path reproduces.

### 3. Trino LIKE escape withdrawal

- [ ] 3.1 Remove `FN_PRED_LIKE_ESCAPE` from `crates/adbc-vs-scan/drivers/trino.toml` and restate the header comment: the set is PostgreSQL's minus `FN_PRED_LIKE_ESCAPE` plus `AGGREGATE_GROUP_BY_TUPLE`, 35 entries.
- [ ] 3.2 In `trino_dialect.rs`, render `predicate_like` as `LIKE <pattern>` with no `ESCAPE` clause, and fail a `predicate_like` that carries an `escapeChar` with an error naming `FN_PRED_LIKE_ESCAPE`. Delete `DEFAULT_ESCAPE_CHARACTER`, `default_escape_expression`, `literal_escape_char`, and `validate_like_pattern`, and restate the module doc. Restate the comment near `trino_source.rs:98-101`, which says MySQL needs a named collation.
- [ ] 3.3 In `trino_dialect_tests.rs`, replace the six escape tests with `like_renders_its_pattern_with_no_escape_clause` (patterns `ab%` and `ab\%`) and `a_like_carrying_an_escape_char_fails_naming_the_capability`. Change the `FN_PRED_LIKE` fixture (near line 961) to expect no `ESCAPE`, and delete the `"FN_PRED_LIKE_ESCAPE"` arm of `query_exercising` (near line 968), which no advertised capability reaches once Trino withholds it.
- [ ] 3.4 In `capabilities_tests.rs`, remove `FN_PRED_LIKE_ESCAPE` from the Trino constant and restate its doc. Add `trino_and_postgresql_sets_differ_by_two_named_entries`, which computes both set differences from the two generated tables and asserts exactly `AGGREGATE_GROUP_BY_TUPLE` one way and `FN_PRED_LIKE_ESCAPE` the other.
- [ ] 3.5 In `tests/e2e_trino_test.rs`, change `filter_reaches_trino_and_matches_the_mirror` to assert that the generated statement carries no `ESCAPE`. Add `like_with_an_explicit_escape_clause_is_evaluated_by_exasol`: `EXPLAIN VIRTUAL` shows no `LIKE` in the generated statement, and the rows equal the mirror's. Restate the doc comment near line 494, which says MySQL's dialect must render a collation for the select-list predicate shape.

### 4. Framework prose and user documentation

- [ ] 4.1 Restate the doc comments that give collation as a purpose: `ColumnRef.declared_type` in `pushdown_query.rs` (the field stays as the fallback `EMITS` type), `sql_dialect.rs`, and `tests/common/mirror.rs`.
- [ ] 4.2 `README.md` (keep the user's uncommitted edits): replace the MySQL 8.0.17/MariaDB prerequisite (line 69), the troubleshooting row for the version or collation error (line 137), and the Limits entry (line 143) with "tested against MySQL 8.4". Add a Limits entry stating that string comparison, grouping, and `MIN`/`MAX` follow the source column's collation while `ORDER BY` stays in Exasol and orders by code point, that a folded group, or a `MIN` or `MAX` tie between values of equal collation weight, reports one of the folded spellings and the source chooses which, that Exasol accepts no `COLLATE` in a query, and the two remedies for Exasol's code-point semantics. Remedy (a): declare `utf8mb4_0900_bin` (MySQL) or `"C"` (PostgreSQL) on the source column or in a source view; the fragment stays delegated; `utf8mb4_0900_bin` needs MySQL 8.0.17 or newer, and MariaDB has no equivalent `NO PAD` binary collation. Remedy (b): wrap the operand in an expression no dialect advertises, such as `"name" || ''`, so Exasol keeps the predicate, grouping, or `MIN`/`MAX` and evaluates it itself; this is slower, because Exasol fetches the rows the source would otherwise have filtered, and a later release that advertises scalar functions can change which wrappers work. State that on PostgreSQL before 18, a delegated `LIKE` over a column of a nondeterministic collation fails at the source. State that on Trino a backslash in a `LIKE` pattern is an ordinary character unless the query writes an explicit `ESCAPE` clause, which Exasol then evaluates.
- [ ] 4.3 `specs/mission.md` Core Capabilities item 3: replace "Each dialect also renders collation so that string comparison matches Exasol." with a sentence stating that string comparison follows the source's collation.
- [ ] 4.4 `CLAUDE.md`: restate the `LANG=en_US.utf8` bullet (E2E testing) so the locale is what lets `e2e_postgres_test.rs` show a delegated ordering comparison returning PostgreSQL's locale answer. Restate the differential-assertions bullet: a mirror comparison over a character column runs against `items_bin` (MySQL) or `items_c` (PostgreSQL), and a test over a default-collation column asserts the source's own answer. Leave unchanged, because both stay accurate: the `<technology>-collation` governance bullet (decision [5] keeps the three features) and the `test-e2e-trino` bullet's "collation differentials against live Trino".
- [ ] 4.5 Restate the comments near `metadata.rs:153` and `create_schema.rs:283`, which give MySQL's required collations and the source-collation check as the precondition's example. Then run `rg -n -i 'collat' crates/ docker/` and restate every remaining hit that does not state the passthrough rule, assert the absence of a suffix, or declare a fixture column's collation.

### 5. Verification (sequential, one shared Docker stack)

- [ ] 5.1 `make fmt-check`, `make lint`, `make test`, `make deny`.
- [ ] 5.2 `make cross-udf-build`, then `make test-artifact`.
- [ ] 5.3 `docker compose rm -sfv postgres`, then `make test-e2e`, `make test-e2e-postgres`, and `make test-e2e-trino`, one after another, never in parallel.

## Parallelization

| Group | Tasks | Depends on | Knowledge |
|-------|-------|------------|-----------|
| A: MySQL collation | 1.1-1.5 | none | spec deltas `vs-adapter/mysql-collation`, `vs-adapter/mysql-type-mapping`; `crates/adbc-vs-adapter/src/{mysql_dialect,mysql_dialect_tests,mysql_source,mysql_source_tests,mysql_type_mapping,mysql_type_mapping_tests,metadata_source_tests,create_schema_tests}.rs`, `crates/adbc-vs-adapter/tests/common/{mysql,items}.rs`, `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` |
| B: PostgreSQL collation | 2.1-2.5 | none | spec deltas `vs-adapter/postgresql-collation`, `vs-adapter/postgresql-type-mapping`; `crates/adbc-vs-adapter/src/{postgres_dialect,postgres_dialect_tests,postgres_type_mapping,postgres_type_mapping_tests,postgres_source,postgres_source_tests}.rs`, `docker/postgres/init.sql`, `crates/adbc-vs-adapter/tests/common/{postgres,postgres_mirror}.rs`, `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` |
| C: Trino LIKE escape | 3.1-3.5 | none | spec delta `vs-adapter/trino-pushdown`; `crates/adbc-vs-scan/drivers/trino.toml`, `crates/adbc-vs-adapter/src/{trino_dialect,trino_dialect_tests,trino_source,capabilities_tests}.rs`, `crates/adbc-vs-adapter/tests/e2e_trino_test.rs` |
| D: Framework prose and docs | 4.1-4.5 | A, B, C (4.5 sweeps their result, and the README states their behavior) | spec deltas `vs-adapter/{pushdown-filter,pushdown-aggregation,pushdown-join,pushdown-dialect,trino-collation,trino-type-mapping,type-mapping,create-virtual-schema,driver-selection,mysql-pushdown,postgresql-pushdown}`, `architecture.md` delta; `crates/adbc-vs-adapter/src/{pushdown_query,sql_dialect,metadata,create_schema}.rs`, `crates/adbc-vs-adapter/tests/common/mirror.rs`, `README.md`, `specs/mission.md`, `CLAUDE.md` |
| E: Verification | 5.1-5.3 | A, B, C, D | the Makefile targets in `CLAUDE.md` § Test tiers and § E2E testing |

- Groups A, B, and C share no file. Each runs `make test` on its own, and no group runs an E2E target: the three E2E binaries share one Exasol container and one BucketFS, so group E runs them in sequence.
- Groups A, B, and C implement the test assertions that the framework deltas in group D require (no suffix in `LIKE`, `IN`, select-list, aggregate-predicate, and join positions), and the mirror comparisons over `items_bin` and `items_c` that the CHANGED `mysql-pushdown`, `postgresql-pushdown`, and `pushdown-filter` scenarios restrict to code-point data. Their task lines name those tests, so the groups do not need to read the framework deltas. The join E2E tests join on the numeric `id`, so the CHANGED `pushdown-join` scenarios need no test change.

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| Constants | `mysql_dialect.rs`: `COMPARISON_COLLATION`, `ORDERING_COLLATION`, `ORDERING_FUNCTIONS` | No collation rendered |
| Functions | `mysql_dialect.rs`: `groups`, `names_the_same_column`, `grouped_columns`, `reject_collated_grouping_key`, `collated_grouping_key_read_by`, `collated_grouping_key_in`, `render_ordering_operand`, `is_character_*`; field `Statement.grouped` | Grouping coordination and the named error existed only for the collated key |
| Constants and functions | `mysql_source.rs`: `REQUIRED_COLLATIONS`, `MINIMUM_MYSQL_VERSION`, `COLLATION_NAMES_QUERY`, `SERVER_VERSION_QUERY`, `SourceCollations`, `check_required_collations`, `query_collation_names`, `query_server_version` | Server check removed |
| Constants and functions | `postgres_dialect.rs`: `ORDERING_COLLATION`, `ORDERING_FUNCTIONS`, `render_ordered_operand`, `is_character_*` | No collation rendered |
| Struct, fields, and functions | `postgres_type_mapping.rs`: `ColumnCollation`, `PostgresColumn.collation`, `is_deterministic`, `require_deterministic_collation`; `postgres_source.rs`: `ColumnRow.collation_name`, `ColumnRow.collation_is_deterministic`, `determinism_flag`, `DETERMINISTIC`, `NONDETERMINISTIC`, and the `collation_name`/`collisdeterministic` select items and `pg_collation`/`pg_namespace` join of `columns_query` | Nondeterministic guard and collation read removed |
| Constant and functions | `trino_dialect.rs`: `DEFAULT_ESCAPE_CHARACTER`, `default_escape_expression`, `literal_escape_char`, `validate_like_pattern` | `ESCAPE` rendering withdrawn |
| Capability | `drivers/trino.toml`: `FN_PRED_LIKE_ESCAPE` | Withheld |
| Tests | `mysql_source_tests.rs` (3 collation tests), the three MySQL grouped-key error tests, the six Trino escape tests, `postgres_type_mapping_tests.rs` (`collated()` helper and 2 collation tests) | Test removed behavior |
| Test edits | `postgres_source_tests.rs`: collation fields of `column_row`; collation items and `collation_schema` assertion of `the_columns_query_selects_every_field_the_mapping_reads`; collation half of `a_character_columns_length_and_collation_reach_the_mapping` (renamed `a_character_columns_length_reaches_the_mapping`) | The columns query and `ColumnRow` no longer carry a collation |
| Test fixture arm | `trino_dialect_tests.rs::query_exercising`: the `"FN_PRED_LIKE_ESCAPE"` arm | No advertised Trino capability reaches it |
| Test | `e2e_mysql_test.rs::select_list_predicate_over_grouped_character_column_fails`, `e2e_postgres_test.rs::collation_is_rendered_only_where_ordering_decides_the_answer` | Replaced by passthrough tests |

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| pushdown-filter: Conjunction, disjunction, and negation nest without losing grouping | Unit + Integration (E2E) | `src/mysql_dialect_tests.rs`, `tests/e2e_mysql_test.rs` | `nested_predicates_are_parenthesized`, `filter_matches_local_exasol_result` (over `items_bin`) |
| pushdown-filter: LIKE renders its pattern and its escape character | Unit | `src/mysql_dialect_tests.rs`, `src/postgres_dialect_tests.rs`, `src/trino_dialect_tests.rs` | `like_renders_pattern_and_escape`, `like_renders_its_escape_clause`, `like_renders_its_pattern_with_no_escape_clause` |
| pushdown-filter: BETWEEN and IN keep their operand order | Unit | `src/mysql_dialect_tests.rs`, `src/postgres_dialect_tests.rs` | `between_and_in_keep_operand_order` (each) |
| pushdown-filter: Predicate in select-list position renders as a projected boolean | Unit | `src/mysql_dialect_tests.rs`, `src/postgres_dialect_tests.rs` | `a_select_list_predicate_renders_where_the_query_groups_by_nothing`, `select_list_predicate_renders_as_projected_boolean` |
| pushdown-aggregation: Predicate over an aggregate is delegated as a BOOLEAN column | Unit | `src/mysql_dialect_tests.rs`, `src/postgres_dialect_tests.rs` | `a_select_list_predicate_over_an_aggregate_renders`, `select_list_predicate_over_an_aggregate_renders` |
| pushdown-join: Inner join of two virtual tables renders one statement | Unit | `src/mysql_dialect_tests.rs`, `src/postgres_dialect_tests.rs`, `src/trino_dialect_tests.rs` | `a_join_renders_one_statement_with_every_table_aliased_and_every_column_qualified`, `inner_join_renders_one_statement`, `inner_join_renders_positionally_aliased_and_qualified` |
| pushdown-join: Outer join emits nulls for the unmatched side | Integration (E2E) | `tests/e2e_mysql_test.rs`, `tests/e2e_postgres_test.rs`, `tests/e2e_trino_test.rs` | `join_matches_local_exasol_result`, `full_outer_join_is_delegated_and_returns_both_unmatched_rows`, `full_outer_join_reaches_trino_and_returns_both_unmatched_rows` (numeric join key, unchanged) |
| pushdown-join: Join composes with a delegated filter and aggregation | Unit | `src/mysql_dialect_tests.rs`, `src/postgres_dialect_tests.rs`, `src/trino_dialect_tests.rs` | `join_composes_with_filter_and_group_by`, `join_composes_with_filter_and_grouping`, `join_composes_with_filter_and_group_by` (unchanged) |
| mysql-pushdown: Aggregation pushdown reaches MySQL rather than Exasol | Integration (E2E) | `tests/e2e_mysql_test.rs` | `aggregation_matches_local_exasol_result` (over `items_bin`), `two_column_group_by_is_not_delegated` |
| postgresql-pushdown: Aggregation pushdown reaches PostgreSQL rather than Exasol | Integration (E2E) | `tests/e2e_postgres_test.rs` | `aggregation_reaches_postgresql_and_matches_the_mirror` (over `items_c`), `two_column_group_by_is_not_delegated` |
| mysql-collation: No rendered operand carries a collation suffix | Unit | `src/mysql_dialect_tests.rs` | `no_operand_carries_a_collation_suffix_in_any_position` |
| mysql-collation: A delegated comparison and grouping follow the column's own collation | Integration (E2E) | `tests/e2e_mysql_test.rs` | `comparison_and_grouping_follow_the_columns_own_collation` |
| mysql-collation: A column declaring utf8mb4_0900_bin returns Exasol's answers | Integration (E2E) | `tests/e2e_mysql_test.rs` | `filter_matches_local_exasol_result`, `aggregation_matches_local_exasol_result`, `grouping_over_character_variants_matches_local_exasol_result`, `trailing_space_values_compare_and_order_as_in_exasol` (all over `items_bin`) |
| mysql-collation: A select-list predicate over the grouped character column is delegated | Integration (E2E) + Unit | `tests/e2e_mysql_test.rs`, `src/mysql_dialect_tests.rs` | `select_list_predicate_over_grouped_character_column_delegates_and_matches_the_mirror`, `a_select_list_predicate_reading_the_grouped_character_column_renders` |
| mysql-collation: A comparison over a wrapped operand is evaluated by Exasol | Integration (E2E) | `tests/e2e_mysql_test.rs` | `a_comparison_over_a_wrapped_operand_is_evaluated_by_exasol` |
| postgresql-collation: No rendered operand carries a collation suffix | Unit | `src/postgres_dialect_tests.rs` | `no_operand_carries_a_collation_suffix_in_any_position` |
| postgresql-collation: A delegated ordering comparison and MIN or MAX follow the column's own collation | Integration (E2E) | `tests/e2e_postgres_test.rs` | `ordering_and_min_max_follow_the_columns_own_collation`, `no_generated_statement_carries_a_collation_keyword` |
| postgresql-collation: A column declaring the C collation returns Exasol's answers | Integration (E2E) | `tests/e2e_postgres_test.rs` | `filter_reaches_postgresql_and_matches_the_mirror`, `aggregation_reaches_postgresql_and_matches_the_mirror` |
| postgresql-collation: A select-list predicate over the grouped character column is delegated | Integration (E2E) | `tests/e2e_postgres_test.rs` | `select_list_predicate_over_the_grouped_character_column_delegates_and_matches_the_mirror` (over `items_c`) |
| postgresql-collation: An ordering comparison over a wrapped operand is evaluated by Exasol | Integration (E2E) | `tests/e2e_postgres_test.rs` | `an_ordering_comparison_over_a_wrapped_operand_is_evaluated_by_exasol` |
| postgresql-collation: Delegated grouping and DISTINCT over case-only variants return Exasol's groups | Integration (E2E) | `tests/e2e_postgres_test.rs` | `uncollated_grouping_and_distinct_return_the_mirrors_group_count` |
| mysql-type-mapping: Character types preserve the declared length and use UTF8 | Unit | `src/mysql_type_mapping_tests.rs` | `character_types_preserve_length` |
| mysql-type-mapping: Character column of a non-utf8mb4 character set is unmapped | Unit | `src/mysql_type_mapping_tests.rs` | `non_utf8mb4_character_columns_unmapped` |
| mysql-type-mapping: No server collation or version is checked for MySQL | Unit | `src/metadata_source_tests.rs`, `src/create_schema_tests.rs` | `each_sources_precondition_reads_only_its_own_catalog` (MySQL arm empty after task 1.3). No test can show that a query is never sent: task 1.3 deletes `COLLATION_NAMES_QUERY` and `SERVER_VERSION_QUERY`, and the task 4.5 sweep confirms no collation query remains |
| postgresql-type-mapping: character varying preserves the declared length and uses UTF8 | Unit | `src/postgres_type_mapping_tests.rs` | `character_varying_preserves_its_declared_length` |
| postgresql-type-mapping: Database whose encoding is not UTF8 is rejected at creation | Unit | `src/postgres_source_tests.rs` | `a_database_of_another_encoding_is_refused_naming_it_and_utf8` |
| trino-pushdown: Capability set advertises the fragments this dialect renders | Unit | `src/capabilities_tests.rs` | `trino_capability_set_is_exact`, `every_advertised_capability_renders_per_dialect` |
| trino-pushdown: Capability set withholds the fragments the Trino dialect does not delegate | Unit | `src/capabilities_tests.rs` | `trino_capability_set_is_exact` (the exact comparison fails when a withheld name is advertised) |
| type-mapping: Each technology states its own mapping table | Unit | `src/metadata_source_tests.rs` | `each_source_maps_its_own_catalog_row` (unchanged) |
| trino-pushdown: The Trino set differs from the PostgreSQL set by two named entries, and each is stated on its own | Unit | `src/capabilities_tests.rs` | `trino_and_postgresql_sets_differ_by_two_named_entries` |
| trino-pushdown: LIKE renders its pattern with no escape clause | Unit | `src/trino_dialect_tests.rs` | `like_renders_its_pattern_with_no_escape_clause`, `a_like_carrying_an_escape_char_fails_naming_the_capability` |
| trino-pushdown: A LIKE carrying an explicit ESCAPE clause is evaluated by Exasol | Integration (E2E) | `tests/e2e_trino_test.rs` | `like_with_an_explicit_escape_clause_is_evaluated_by_exasol` |

Test paths are under `crates/adbc-vs-adapter/`. The deltas for `pushdown-dialect`, `trino-collation`, `trino-type-mapping`, `create-virtual-schema`, and `driver-selection` change Background or description prose only; their scenarios and tests are unchanged. The `type-mapping` delta changes Background prose and the PostgreSQL example in one GIVEN step; the test for that scenario is unchanged. Unit tests cover rendering and type mapping, which are pure functions of the request or catalog row.

### Manual Testing

Run after group E's `make test-e2e*` targets, which leave the virtual schemas in place. `DSN` is `exasol://sys:exasol@localhost:8563?validateservercertificate=0`. Rows whose SQL holds single quotes, `||`, or `!` name a block in the fenced listing after the table. Each block passes its SQL through a quoted heredoc, which exapump reads from stdin, so the shell changes no character of it.

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| mysql-collation | `exapump sql --dsn "$DSN" 'EXPLAIN VIRTUAL SELECT "category", COUNT(*) FROM MYSQLVS."items" GROUP BY "category"'`, then the same `SELECT` without `EXPLAIN VIRTUAL` | Pushdown SQL holds `GROUP BY` and no `COLLATE`; 2 groups |
| mysql-collation | Same `SELECT` over `MYSQLVS."items_bin"` | 4 groups |
| mysql-collation | Blocks M3a and M3b | M3a: `widget` rows only. M3b: the pushdown SQL holds no comparison over `category` |
| postgresql-collation | `exapump sql --dsn "$DSN" 'SELECT MIN("sort_key"), MAX("sort_key") FROM PGVS."items"'` | `a`, `C` |
| postgresql-collation | Same `SELECT` over `PGVS."items_c"` | `B`, `a` |
| trino-pushdown | Block T6 | Pushdown SQL holds `LIKE 'a%'` and no `ESCAPE` |
| trino-pushdown | Block T7 | Pushdown SQL holds no `LIKE` |
| mysql-type-mapping | `exapump sql --dsn "$DSN" "SELECT COLUMN_NAME FROM EXA_ALL_COLUMNS WHERE COLUMN_SCHEMA = 'MYSQLVS' AND COLUMN_TABLE = 'items'"` | The `latin1` column `region` is absent |
| postgresql-type-mapping, pushdown-filter, pushdown-aggregation, pushdown-join | Covered by the `make test-e2e-postgres` and `make test-e2e` runs | 0 failures |

```bash
# M3a
exapump sql --dsn "$DSN" <<'SQL'
SELECT "category" FROM MYSQLVS."items" WHERE "category" || '' = 'widget'
SQL
# M3b
exapump sql --dsn "$DSN" <<'SQL'
EXPLAIN VIRTUAL SELECT "category" FROM MYSQLVS."items" WHERE "category" || '' = 'widget'
SQL
# T6
exapump sql --dsn "$DSN" <<'SQL'
EXPLAIN VIRTUAL SELECT "sort_key" FROM TRVS."items" WHERE "sort_key" LIKE 'a%'
SQL
# T7
exapump sql --dsn "$DSN" <<'SQL'
EXPLAIN VIRTUAL SELECT "sort_key" FROM TRVS."items" WHERE "sort_key" LIKE 'a!%' ESCAPE '!'
SQL
```

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Build | `make cross-udf-build` | Exit 0, `cargo exasol-udf validate` passes |
| Artifact | `make test-artifact` | 0 failures |
| Test | `make test` | 0 failures |
| Lint | `make lint` | 0 errors or warnings |
| Format | `make fmt-check` | No changes |
| Policy | `make deny` | Pass |
| E2E | `make test-e2e`, `make test-e2e-postgres`, `make test-e2e-trino` (in sequence) | 0 failures, no skipped test |
| Specs | `speq plan validate remove-pushdown-collation-and-like-escape` | Pass |
