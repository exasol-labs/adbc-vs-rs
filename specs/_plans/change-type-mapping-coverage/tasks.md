# Tasks: change-type-mapping-coverage

Full task text lives in plan.md § Implementation Tasks under the same numbers.

## Phase 2: Implementation (Group A: Type model, column notes, and seam)

- [x] 1.1 Timestamp precision in `ExasolType` (gate, runs first)
- [x] 1.2 `from_json` fails on `withLocalTimeZone: true`
- [x] 1.3 Remove `MappedType`
- [x] 1.4 Record and read the column source type
- [x] 1.5 Read-expression seam `column_read_sql` [expert]
- [x] 1.6 `BOOLEAN` column as predicate
- [x] 1.7 Trino timestamp and boolean reason

## Phase 2: Implementation (Group B: PostgreSQL)

- [x] 2.1 Timestamp emit gate (after 1.1)
- [x] 2.2 Request-shape gate (after 2.1)
- [x] 2.3 `datetime_precision` in columns query
- [x] 2.4 Rewrite `map_column`
- [x] 2.5 PostgreSQL `column_read_sql`
- [x] 2.6 `MIN`/`MAX` over boolean, capabilities arm, join keys
- [x] 2.7 `docker/postgres/init.sql` fixtures
- [x] 2.8 PostgreSQL mirror
- [x] 2.9 PostgreSQL E2E tests
- [x] 2.10 Expression-index confirmation

## Phase 2: Implementation (Group D: Scan coercion)

- [x] 4.1 Integer coercion test
- [x] 4.2 Time zone timestamp coercion test

## Phase 2: Implementation (Group C: MySQL)

- [x] 3.1 `DATETIME_PRECISION`, drop `CHARACTER_SET_NAME`
- [x] 3.2 Rewrite `map_data_type`
- [x] 3.3 MySQL `column_read_sql`
- [x] 3.4 MySQL fixtures
- [x] 3.5 MySQL mirror
- [x] 3.6 MySQL E2E tests
- [x] 3.7 Zero `YEAR` and latin1 measurements

## Phase 2: Implementation (Group E: Documentation and reference)

- [x] 5.1 `docs/type-mapping.md`
- [x] 5.2 Link the page from docs
- [x] 5.3 Copy `specs/exasol-types.md`
- [x] 5.4 AGENTS.md bullets
- [x] 5.5 Version bump and PR description note

## Phase 3: Verification

- [x] 6.1 Run the plan's verification checklist
- [x] 6.2 Scenario coverage audit
- [x] 6.3 Verification report

## Phase 4: Review Fixes

- [x] 4.3 Join the `array_reason` string in `postgres_type_mapping.rs` with one space and `assert_eq!` the full reason in `array_columns_are_unmapped_naming_array`
- [x] 4.4 Add `a_timestamp_without_a_reported_precision_declares_postgresqls_default_of_six` and `unbounded_and_zero_length_character_columns_map` to `postgres_type_mapping_tests.rs`
- [x] 4.5 Delete the `aggregate_of` closure in `min_and_max_over_boolean_render_as_bool_and_and_bool_or` and call `fixtures::aggregate_over` directly
- [x] 4.6 Delete `SUPPORTED_CHARACTER_SET` from `mysql_type_mapping.rs` and add a private `LITERAL_CHARACTER_SET` in `mysql_dialect.rs` used by `render_string_literal`
- [x] 4.7 Shorten the `character_size` comment in `mysql_type_mapping.rs` to two lines
- [x] 4.8 Delete `the_mapped_column_records_the_source_type_as_the_catalog_spelled_it` from `mysql_type_mapping_tests.rs`
- [x] 4.9 Replace the Trino `timestamp` arm comment with the refresh-stability reason
- [x] 4.10 Pair the integer coercion cases in `coerce_tests.rs` with the declared MySQL precisions (3, 5, 7, 8, 19) and read `Decimal128(_, 0)`
- [x] 4.11 Assert the overflow wording in the PostgreSQL `text_value_beyond_the_varchar_maximum_fails_the_query` E2E test
- [x] 4.12 Delete the `// --- type coverage ---` banner in `e2e_postgres_test.rs`
- [x] 4.13 Assert the overflow wording in the MySQL `text_value_beyond_the_varchar_maximum_fails_the_query` E2E test
- [x] 4.14 Add `assert_matches_mirror_of` in `e2e_mysql_test.rs`, delegate `assert_matches_mirror` to it, and use it in the converted-view test
- [x] 4.15 Delete the four source-text fixture tests in `tests/common/mysql.rs` and helpers only they call
- [x] 4.16 Delete the two constant-restating tests in `tests/common/mirror.rs`
- [x] 4.17 Delete `the_mirrored_items_declare_every_mapped_column_and_no_array` (and `item_declarations` if unused) in `tests/common/postgres_mirror.rs`
- [x] 4.18 Replace the `money` row note in `docs/type-mapping.md` with the `lc_monetary` scale wording
- [x] 4.19 Remove the `read_sql` parameter and `ColumnReadSql`, render through each technology's `column_read_sql` directly, and drive the position tests through real converted source types via `ConvertedNotes` [expert]
- [x] 4.20 Measure Exasol's `adapterNotes` for a column declared without notes in `exasol_sends_the_measured_boolean_and_timestamp_filter_shapes` and settle the empty-string arm of `read_source_type` accordingly [expert]
