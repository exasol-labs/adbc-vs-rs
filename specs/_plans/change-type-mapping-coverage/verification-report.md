# Verification Report: change-type-mapping-coverage

## Verdict

| Result | Details |
|--------|---------|
| **PASS** | Every build, test, lint, format, license, integration, and end-to-end check exits 0. All 246 scenario rows have a test that exists and passes, and each manual test shows the expected output. |
| Code review | 18 findings (16 standard, 2 expert), 18 fixed |

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
| Unit | Not measured (no coverage tool in the project's make targets) |
| Integration | Not measured |

### Test Results

| Type | Run | Passed | Ignored |
|------|-----|--------|---------|
| Unit (`make test`) | 831 | 831 | 0 |
| Integration (`make test-it test-it-postgres test-it-trino`) | 16 | 16 | 0 |
| End-to-end MySQL (`make test-e2e`) | 282 | 282 | 0 |
| End-to-end PostgreSQL (`make test-e2e-postgres`) | 267 | 267 | 0 |
| End-to-end Trino (`make test-e2e-trino`) | 249 | 249 | 0 |

### Manual Tests

The plan's commands name tables and columns in upper case. The virtual schema exposes them in lower case, so the run quotes the lower-case names.

| Test | Result |
|------|--------|
| Distinct column types of `PGVS` and `MYSQLVS` are only `DECIMAL`, `VARCHAR`, `CHAR`, `DATE`, `TIMESTAMP(p)`, `DOUBLE`, `BOOLEAN` | ✓ |
| `PGVS.type_coverage` types: `unbounded_amount` `VARCHAR(2000000)`, `price` and `approx` `DOUBLE`, `flag`, `bit_flag`, `bits` `BOOLEAN`, `fixed_code` `CHAR(5)`, `wide_code` `VARCHAR(3000)`, `clock` `VARCHAR(100)`, `precise_at` and `happened_at` `TIMESTAMP(6)` | ✓ |
| `MYSQLVS.type_coverage` types: `ubig_value` `DECIMAL(20,0)`, `bit_flag` and `bits` `BOOLEAN`, `fixed` `CHAR(10)`, `kind` `CHAR(5)`, `elapsed` `TIMESTAMP(3)`, `y` `DATE`, `doc` `VARCHAR(2000000)`, latin1 columns `UTF8` | ✓ |
| Trino timestamp columns | ✓ every column `TIMESTAMP(3)` |
| `EXPLAIN VIRTUAL` over `PGVS.type_coverage` grouped by `uid` | ✓ `CAST("uid" AS TEXT)` in the select list, `WHERE`, and `GROUP BY` |
| `EXPLAIN VIRTUAL` over `MYSQLVS.type_coverage` grouped by `y` | ✓ `GROUP BY MAKEDATE(`y`, 1)` with `ANY_VALUE(MAKEDATE(`y`, 1))` in the select list |
| The same MySQL grouped query | ✓ returns `true`, `false`, and null rows, with no error 1055 |
| `MIN` and `MAX` over `flag` and `bit_flag` | ✓ `false`, `true`, `false`, `true` |
| `TO_CHAR(happened_at, '...FF6')` | ✓ `2024-01-01 12:00:00.123456` and the other values at six digits |

## Tool Evidence

### Linter

```
make lint (both clippy runs, -D warnings): exit 0
make deny: exit 0
```

### Formatter

```
make fmt-check: exit 0
```

## Scenario Coverage

| Domain | Feature | Scenario | Test Location | Test Name | Passes |
|--------|---------|----------|---------------|-----------|--------|
| vs-adapter | type-mapping | An exact numeric beyond Exasol's maximum precision is never declared DECIMAL | `crates/adbc-vs-adapter/src/postgres_type_mapping_tests.rs` | `numeric_beyond_precision_36_maps_to_varchar_read_as_text` | Pass |
| vs-adapter | type-mapping | An exact numeric beyond Exasol's maximum precision is never declared DECIMAL | `crates/adbc-vs-adapter/src/mysql_type_mapping_tests.rs` | `decimal_beyond_precision_36_maps_to_varchar_read_as_text` | Pass |
| vs-adapter | type-mapping | An exact numeric beyond Exasol's maximum precision is never declared DECIMAL | `crates/adbc-vs-adapter/src/trino_type_mapping_tests.rs` | `decimal_preserves_the_source_precision_and_scale` | Pass |
| vs-adapter | type-mapping | An exact numeric beyond Exasol's maximum precision is never declared DECIMAL | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `wide_and_unconstrained_numeric_read_as_text_like_the_mirror` | Pass |
| vs-adapter | type-mapping | A mapped column declares only an Exasol type this release produces | `crates/adbc-vs-adapter/src/metadata_source_tests.rs` | `every_mapped_column_declares_a_type_of_the_closed_set` | Pass |
| vs-adapter | type-mapping | A mapped column records its source type in its adapterNotes | `crates/adbc-vs-adapter/src/create_schema_tests.rs` | `mapped_columns_carry_their_source_type_in_adapter_notes` | Pass |
| vs-adapter | type-mapping | A mapped column records its source type in its adapterNotes | `crates/adbc-vs-adapter/src/pushdown_query_tests.rs` | `column_reference_carries_the_recorded_source_type` | Pass |
| vs-adapter | type-mapping | A mapped column records its source type in its adapterNotes | `crates/adbc-vs-adapter/src/pushdown_query_tests.rs` | `column_without_adapter_notes_has_no_source_type` | Pass |
| vs-adapter | type-mapping | A mapped column records its source type in its adapterNotes | `crates/adbc-vs-adapter/src/pushdown_query_tests.rs` | `projected_columns_without_a_select_list_carry_their_source_type` | Pass |
| vs-adapter | type-mapping | A mapped column records its source type in its adapterNotes | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `select_star_over_type_coverage_matches_the_mirror` | Pass |
| vs-adapter | type-mapping | A mapped column records its source type in its adapterNotes | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `values_read_as_text_compare_as_the_emitted_text` | Pass |
| vs-adapter | type-mapping | A TIMESTAMP column declares the source's fractional-second precision | `crates/adbc-vs-adapter/src/exa_type_tests.rs` | `timestamp_precision_round_trips_through_ddl_and_json` | Pass |
| vs-adapter | type-mapping | A TIMESTAMP column declares the source's fractional-second precision | `crates/adbc-vs-adapter/src/exa_type_tests.rs` | `timestamp_without_precision_reads_as_three` | Pass |
| vs-adapter | type-mapping | A TIMESTAMP column declares the source's fractional-second precision | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `scan_emits_every_fractional_digit_of_a_declared_timestamp` | Pass |
| vs-adapter | type-mapping | A TIMESTAMP column declares the source's fractional-second precision | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `exasol_sends_the_measured_boolean_and_timestamp_filter_shapes` | Pass |
| vs-adapter | type-mapping | A microsecond timestamp keeps every fractional digit through the scan and a delegated comparison | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `microsecond_timestamp_keeps_six_digits_like_the_mirror` | Pass |
| vs-adapter | type-mapping | A microsecond timestamp keeps every fractional digit through the scan and a delegated comparison | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `scan_emits_every_fractional_digit_of_a_declared_timestamp` | Pass |
| vs-adapter | type-mapping | A TIMESTAMP column declares the source's fractional-second precision | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` | `datetime_precision_carries_into_the_declared_timestamp` | Pass |
| vs-adapter | type-mapping | Each technology states its own mapping table | `crates/adbc-vs-adapter/src/metadata_source_tests.rs` | `each_source_maps_its_own_catalog_row` | Pass |
| vs-adapter | type-mapping | Each technology states its own mapping table | `crates/adbc-vs-adapter/src/metadata_source_tests.rs` | `each_sources_precondition_reads_only_its_own_catalog` | Pass |
| vs-adapter | type-mapping | A mapping reads one catalog row and performs no I/O | `crates/adbc-vs-adapter/src/metadata_source_tests.rs` | `each_source_maps_its_own_catalog_row` | Pass |
| vs-adapter | type-mapping | A mapping reads one catalog row and performs no I/O | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` | `datetime_precision_carries_into_the_declared_timestamp` | Pass |
| vs-adapter | type-mapping | Omission records the source type under a source-neutral field | `crates/adbc-vs-adapter/src/adapter_notes_tests.rs` | `an_omission_is_written_under_its_source_neutral_name` | Pass |
| vs-adapter | type-mapping | Omission records the source type under a source-neutral field | `crates/adbc-vs-adapter/src/adapter_notes_tests.rs` | `omission_source_type_accepts_legacy_alias` | Pass |
| vs-adapter | pushdown-dialect | Every advertised capability has a rendering path | `crates/adbc-vs-adapter/src/capabilities_tests.rs` | `every_advertised_capability_renders_per_dialect` | Pass |
| vs-adapter | pushdown-dialect | A withheld capability carries one of two reasons | `crates/adbc-vs-adapter/src/capabilities_tests.rs` | `{mysql,postgres,trino}_capability_set_is_exact` | Pass |
| vs-adapter | pushdown-dialect | A withheld capability carries one of two reasons | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` | `avg_and_sum_are_withdrawn_and_match_local_exasol_result` | Pass |
| vs-adapter | pushdown-dialect | Each dialect's capability set is stated once | `crates/adbc-vs-adapter/src/capabilities_tests.rs` | `advertised_set_matches_the_rendered_fragments` | Pass |
| vs-adapter | pushdown-dialect | Each dialect's capability set is stated once | `crates/adbc-vs-scan/tests/driver_toml_test.rs` | `a_declaration_with_no_capability_list_names_the_file_and_the_field` | Pass |
| vs-adapter | pushdown-dialect | Declared type outside the declarable set fails with a named error | `crates/adbc-vs-adapter/src/exa_type_tests.rs` | `local_time_zone_timestamp_fails_rather_than_reading_as_timestamp` | Pass |
| vs-adapter | pushdown-dialect | Declared type outside the declarable set fails with a named error | `crates/adbc-vs-adapter/src/exa_type_tests.rs` | `char_boolean_and_precise_timestamp_entries_read` | Pass |
| vs-adapter | pushdown-dialect | Declared type shapes the request carries are read without loss | `crates/adbc-vs-adapter/src/exa_type_tests.rs` | `timestamp_precision_and_absent_character_set_are_read` | Pass |
| vs-adapter | pushdown-dialect | A column reference renders through its read expression in every position | `crates/adbc-vs-adapter/src/{postgres,mysql,trino}_dialect_tests.rs` | `column_reference_renders_identically_in_every_position` | Pass |
| vs-adapter | pushdown-dialect | A column reference renders through its read expression in every position | `crates/adbc-vs-adapter/src/postgres_dialect_tests.rs` | `converted_join_keys_render_through_their_read_expressions_in_a_full_outer_join` | Pass |
| vs-adapter | pushdown-dialect | A column reference renders through its read expression in every position | `crates/adbc-vs-adapter/src/mysql_dialect_tests.rs` | `grouped_converted_column_in_the_select_list_renders_inside_any_value` | Pass |
| vs-adapter | pushdown-dialect | A column reference renders through its read expression in every position | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `converted_columns_group_join_and_aggregate_like_the_mirror` | Pass |
| vs-adapter | pushdown-dialect | A column reference renders through its read expression in every position | `crates/adbc-vs-adapter/src/pushdown_query_tests.rs` | `column_reference_carries_the_recorded_source_type` | Pass |
| vs-adapter | pushdown-dialect | Declared type outside the declarable set fails with a named error | `crates/adbc-vs-adapter/src/metadata_source_tests.rs` | `every_mapped_column_declares_a_type_of_the_closed_set` | Pass |
| vs-adapter | pushdown-dialect | Declared type shapes the request carries are read without loss | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` | `exasol_supplies_the_measured_type_shapes` | Pass |
| vs-adapter | pushdown-dialect | Emitted columns are named positionally | `crates/adbc-vs-adapter/src/pushdown_tests.rs` | `a_join_projecting_two_columns_of_the_same_name_gets_distinct_positional_names` | Pass |
| vs-adapter | pushdown-dialect | Missing declared types fail rather than being guessed | `crates/adbc-vs-adapter/src/emit_type_tests.rs` | `absent_select_list_data_types_named_in_error` | Pass |
| vs-adapter | pushdown-dialect | Declared types are read from the request positionally | `crates/adbc-vs-adapter/src/emit_type_tests.rs` | `declared_types_come_from_select_list_data_types` | Pass |
| vs-adapter | pushdown-dialect | Declared types are read from the request positionally | `crates/adbc-vs-adapter/src/emit_type_tests.rs` | `select_list_data_types_length_mismatch_named_in_error` | Pass |
| vs-adapter | pushdown-dialect | Declared types are read from the request positionally | `crates/adbc-vs-adapter/src/emit_type_tests.rs` | `absent_select_list_types_come_from_involved_tables` | Pass |
| vs-adapter | pushdown-dialect | BOOLEAN is declarable in the EMITS clause | `crates/adbc-vs-adapter/src/emit_type_tests.rs` | `boolean_declared_type_reaches_the_emits_clause` | Pass |
| vs-adapter | pushdown-dialect | Declared types are read from the request positionally | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `exasol_declares_the_measured_select_list_types` | Pass |
| vs-adapter | pushdown-dialect | BOOLEAN is declarable in the EMITS clause | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` | `projected_predicate_matches_local_exasol_result` | Pass |
| vs-adapter | pushdown-dialect | BOOLEAN is declarable in the EMITS clause | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `exasol_declares_the_measured_select_list_types` | Pass |
| vs-adapter | pushdown-dialect | The declarable set is closed under the advertised capabilities | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` | `exasol_supplies_the_measured_aggregate_types` | Pass |
| vs-adapter | pushdown-dialect | The declarable set is closed under the advertised capabilities | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` | `aggregation_not_delegable_whole_is_withdrawn_and_matches_local_exasol_result` | Pass |
| vs-adapter | pushdown-dialect | The declarable set is closed under the advertised capabilities | `crates/adbc-vs-adapter/src/capabilities_tests.rs` | `advertised_set_matches_the_rendered_fragments` | Pass |
| vs-adapter | pushdown-dialect | Comparison operands are read by position, not by assumed role | `crates/adbc-vs-adapter/src/pushdown_query_tests.rs` | `a_swapped_comparison_keeps_its_operand_order` | Pass |
| vs-adapter | pushdown-dialect | Comparison operands are read by position, not by assumed role | `crates/adbc-vs-adapter/src/mysql_dialect_tests.rs` | `swapped_comparison_keeps_operand_order` | Pass |
| vs-adapter | pushdown-dialect | Comparison operands are read by position, not by assumed role | `crates/adbc-vs-adapter/src/{postgres,trino}_dialect_tests.rs` | `comparison_operands_keep_request_order` | Pass |
| vs-adapter | pushdown-filter | BOOLEAN column is a predicate on its own | `crates/adbc-vs-adapter/src/pushdown_query_tests.rs` | `boolean_column_is_a_predicate_in_every_position` | Pass |
| vs-adapter | pushdown-filter | BOOLEAN column is a predicate on its own | `crates/adbc-vs-adapter/src/pushdown_query_tests.rs` | `non_boolean_column_in_predicate_position_fails_naming_its_type` | Pass |
| vs-adapter | pushdown-filter | BOOLEAN column is a predicate on its own | `crates/adbc-vs-adapter/src/{postgres,mysql,trino}_dialect_tests.rs` | `boolean_column_predicate_renders_through_its_read_expression` | Pass |
| vs-adapter | pushdown-filter | BOOLEAN column is a predicate on its own | `crates/adbc-vs-adapter/src/pushdown_query_tests.rs` | `predicate_operand_of_a_comparison_is_read` | Pass |
| vs-adapter | pushdown-filter | BOOLEAN column is a predicate on its own | `crates/adbc-vs-adapter/src/{postgres,mysql,trino}_dialect_tests.rs` | `predicate_operand_of_a_comparison_renders_parenthesized` | Pass |
| vs-adapter | pushdown-filter | BOOLEAN column is a predicate on its own | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `exasol_sends_the_measured_boolean_and_timestamp_filter_shapes` | Pass |
| vs-adapter | pushdown-filter | BOOLEAN column is a predicate on its own | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `boolean_and_bit_columns_filter_group_and_aggregate_like_the_mirror` | Pass |
| vs-adapter | pushdown-filter | BOOLEAN column is a predicate on its own | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` | `bit_columns_filter_group_and_aggregate_like_the_mirror` | Pass |
| vs-adapter | pushdown-filter | Numeric comparison is rendered as a WHERE clause | `crates/adbc-vs-adapter/src/mysql_dialect_tests.rs` | `each_comparison_operator_renders_its_mysql_counterpart` | Pass |
| vs-adapter | pushdown-filter | Numeric comparison is rendered as a WHERE clause | `crates/adbc-vs-adapter/src/postgres_dialect_tests.rs` | `standard_predicate_forms_render` | Pass |
| vs-adapter | pushdown-filter | Numeric comparison is rendered as a WHERE clause | `crates/adbc-vs-adapter/src/capabilities_tests.rs` | `every_advertised_capability_renders_per_dialect` | Pass |
| vs-adapter | pushdown-filter | Conjunction, disjunction, and negation nest without losing grouping | `crates/adbc-vs-adapter/src/{mysql,postgres}_dialect_tests.rs` | `nested_predicates_are_parenthesized` | Pass |
| vs-adapter | pushdown-filter | Conjunction, disjunction, and negation nest without losing grouping | `crates/adbc-vs-adapter/src/capabilities_tests.rs` | `every_advertised_capability_renders_per_dialect` | Pass |
| vs-adapter | pushdown-filter | LIKE renders its pattern and its escape character | `crates/adbc-vs-adapter/src/mysql_dialect_tests.rs` | `like_renders_pattern_and_escape` | Pass |
| vs-adapter | pushdown-filter | LIKE renders its pattern and its escape character | `crates/adbc-vs-adapter/src/postgres_dialect_tests.rs` | `like_renders_its_escape_clause` | Pass |
| vs-adapter | pushdown-filter | LIKE renders its pattern and its escape character | `crates/adbc-vs-adapter/src/trino_dialect_tests.rs` | `like_renders_its_pattern_with_no_escape_clause` | Pass |
| vs-adapter | pushdown-filter | A LIKE pattern without an ESCAPE clause follows the target's default escape rule | `crates/adbc-vs-adapter/src/trino_dialect_tests.rs` | `like_renders_its_pattern_with_no_escape_clause` | Pass |
| vs-adapter | pushdown-filter | A LIKE pattern without an ESCAPE clause follows the target's default escape rule | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` | `filter_matches_local_exasol_result` | Pass |
| vs-adapter | pushdown-filter | A LIKE pattern without an ESCAPE clause follows the target's default escape rule | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `filter_reaches_postgresql_and_matches_the_mirror` | Pass |
| vs-adapter | pushdown-filter | A delegated predicate follows the source's comparison rules | `crates/adbc-vs-adapter/src/{mysql,postgres,trino}_dialect_tests.rs` | `no_operand_carries_a_collation_suffix_in_any_position` | Pass |
| vs-adapter | pushdown-filter | A delegated predicate follows the source's comparison rules | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` | `comparison_and_grouping_follow_the_columns_own_collation` | Pass |
| vs-adapter | pushdown-filter | A delegated predicate follows the source's comparison rules | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `ordering_and_min_max_follow_the_columns_own_collation` | Pass |
| vs-adapter | pushdown-filter | A CHAR column compares as the source compares fixed-width values | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `character_column_emits_padded_and_compares_like_the_source` | Pass |
| vs-adapter | pushdown-filter | A CHAR column compares as the source compares fixed-width values | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` | `char_column_emits_padded_and_compares_like_the_source` | Pass |
| vs-adapter | pushdown-filter | A delegated predicate follows the source's comparison rules | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `values_read_as_text_compare_as_the_emitted_text` | Pass |
| vs-adapter | pushdown-filter | String, date, and timestamp literals render in a form no server setting can change | `crates/adbc-vs-adapter/src/mysql_dialect_tests.rs` | `string_literal_renders_as_hex` | Pass |
| vs-adapter | pushdown-filter | String, date, and timestamp literals render in a form no server setting can change | `crates/adbc-vs-adapter/src/postgres_dialect_tests.rs` | `string_literal_is_escape_string_constant` | Pass |
| vs-adapter | pushdown-filter | String, date, and timestamp literals render in a form no server setting can change | `crates/adbc-vs-adapter/src/trino_dialect_tests.rs` | `string_literal_doubles_only_the_single_quote` | Pass |
| vs-adapter | pushdown-filter | String, date, and timestamp literals render in a form no server setting can change | `crates/adbc-vs-adapter/src/{mysql,postgres,trino}_dialect_tests.rs` | `temporal_literals_render_typed` | Pass |
| vs-adapter | pushdown-filter | String, date, and timestamp literals render in a form no server setting can change | `crates/adbc-vs-adapter/src/{mysql,postgres,trino}_dialect_tests.rs` | `a_literal_that_could_change_the_statement_shape_fails_to_render` | Pass |
| vs-adapter | pushdown-filter | REGEXP_LIKE is evaluated by Exasol rather than delegated | `crates/adbc-vs-adapter/src/capabilities_tests.rs` | `{mysql,postgres,trino}_capability_set_is_exact` | Pass |
| vs-adapter | trino-type-mapping | char is unmapped because it is blank-padded | `crates/adbc-vs-adapter/src/trino_type_mapping_tests.rs` | `char_is_unmapped_naming_the_blank_padding` | Pass |
| vs-adapter | trino-type-mapping | Temporal and floating-point types map to their Exasol counterparts | `crates/adbc-vs-adapter/src/trino_type_mapping_tests.rs` | `temporal_and_floating_point_types_map_to_their_exasol_counterparts` | Pass |
| vs-adapter | trino-type-mapping | Non-core types are unmapped | `crates/adbc-vs-adapter/src/trino_type_mapping_tests.rs` | `boolean_is_unmapped_as_outside_the_trino_scope` | Pass |
| vs-adapter | trino-type-mapping | The column type is parsed out of the catalog's data_type string | `crates/adbc-vs-adapter/src/trino_type_mapping_tests.rs` | `an_unrecognized_data_type_is_named_in_full_rather_than_truncated_to_its_bare_name` | Pass |
| vs-adapter | trino-type-mapping | The column type is parsed out of the catalog's data_type string | `crates/adbc-vs-adapter/src/trino_type_mapping_tests.rs` | `decimal_and_map_disagree_on_whether_the_comma_carries_a_space` | Pass |
| vs-adapter | trino-type-mapping | No source-server precondition is checked for Trino | `crates/adbc-vs-adapter/src/metadata_source_tests.rs` | `each_sources_precondition_reads_only_its_own_catalog` | Pass |
| vs-adapter | trino-type-mapping | Virtual schema exposes one catalog and one schema | `crates/adbc-vs-adapter/src/trino_source_tests.rs` | `both_catalog_queries_qualify_the_named_catalog_and_filter_the_named_schema` | Pass |
| vs-adapter | trino-type-mapping | Virtual schema exposes one catalog and one schema | `crates/adbc-vs-adapter/tests/e2e_trino_test.rs` | `adapter_notes_record_the_two_segment_namespace` | Pass |
| vs-adapter | trino-type-mapping | Virtual schema exposes one catalog and one schema | `crates/adbc-vs-adapter/tests/e2e_trino_test.rs` | `views_are_exposed_alongside_base_tables` | Pass |
| vs-adapter | postgresql-type-mapping | Integer types map to DECIMAL with the matching digit count | `crates/adbc-vs-adapter/src/postgres_type_mapping_tests.rs` | `integer_types_map_to_decimal` | Pass |
| vs-adapter | postgresql-type-mapping | Integer types map to DECIMAL with the matching digit count | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `smallint_column_matches_the_mirror` | Pass |
| vs-adapter | postgresql-type-mapping | numeric preserves the source precision and scale | `crates/adbc-vs-adapter/src/postgres_type_mapping_tests.rs` | `numeric_preserves_the_source_precision_and_scale` | Pass |
| vs-adapter | postgresql-type-mapping | numeric preserves the source precision and scale | `crates/adbc-vs-adapter/src/postgres_type_mapping_tests.rs` | `numeric_beyond_precision_36_maps_to_varchar_read_as_text` | Pass |
| vs-adapter | postgresql-type-mapping | numeric without a declared precision maps to VARCHAR read as its text | `crates/adbc-vs-adapter/src/postgres_type_mapping_tests.rs` | `unconstrained_numeric_maps_to_varchar_read_as_text` | Pass |
| vs-adapter | postgresql-type-mapping | numeric without a declared precision maps to VARCHAR read as its text | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `wide_and_unconstrained_numeric_read_as_text_like_the_mirror` | Pass |
| vs-adapter | postgresql-type-mapping | character varying preserves the declared length and uses UTF8 | `crates/adbc-vs-adapter/src/postgres_type_mapping_tests.rs` | `character_varying_and_text_map_to_varchar_capped_at_the_maximum` | Pass |
| vs-adapter | postgresql-type-mapping | character varying preserves the declared length and uses UTF8 | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `text_value_beyond_the_varchar_maximum_fails_the_query` | Pass |
| vs-adapter | postgresql-type-mapping | character maps to CHAR | `crates/adbc-vs-adapter/src/postgres_type_mapping_tests.rs` | `character_maps_to_char_up_to_2000_and_varchar_above` | Pass |
| vs-adapter | postgresql-type-mapping | character maps to CHAR | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `character_column_emits_padded_and_compares_like_the_source` | Pass |
| vs-adapter | postgresql-type-mapping | Temporal and floating-point types map to their Exasol counterparts | `crates/adbc-vs-adapter/src/postgres_type_mapping_tests.rs` | `temporal_and_floating_point_types_map_to_their_exasol_counterparts` | Pass |
| vs-adapter | postgresql-type-mapping | Temporal and floating-point types map to their Exasol counterparts | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `real_column_holds_the_widened_single_precision_value` | Pass |
| vs-adapter | postgresql-type-mapping | boolean and bit map to BOOLEAN | `crates/adbc-vs-adapter/src/postgres_type_mapping_tests.rs` | `boolean_and_bit_of_every_width_map_to_boolean` | Pass |
| vs-adapter | postgresql-type-mapping | boolean and bit map to BOOLEAN | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `boolean_and_bit_columns_filter_group_and_aggregate_like_the_mirror` | Pass |
| vs-adapter | postgresql-type-mapping | money maps to DOUBLE | `crates/adbc-vs-adapter/src/postgres_type_mapping_tests.rs` | `money_maps_to_double_read_through_numeric` | Pass |
| vs-adapter | postgresql-type-mapping | money maps to DOUBLE | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `money_column_compares_as_double_like_the_mirror` | Pass |
| vs-adapter | postgresql-type-mapping | Types without an Exasol counterpart map to VARCHAR read as their text | `crates/adbc-vs-adapter/src/postgres_type_mapping_tests.rs` | `types_without_a_counterpart_map_to_varchar_read_as_text` | Pass |
| vs-adapter | postgresql-type-mapping | Types without an Exasol counterpart map to VARCHAR read as their text | `crates/adbc-vs-adapter/src/postgres_type_mapping_tests.rs` | `column_read_sql_follows_the_source_type` | Pass |
| vs-adapter | postgresql-type-mapping | Types without an Exasol counterpart map to VARCHAR read as their text | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `text_read_types_match_the_mirror` | Pass |
| vs-adapter | postgresql-type-mapping | A value read as text compares as the text the scan emits | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `values_read_as_text_compare_as_the_emitted_text` | Pass |
| vs-adapter | postgresql-type-mapping | timestamp with time zone maps to TIMESTAMP holding the UTC wall-clock value | `crates/adbc-vs-adapter/src/postgres_type_mapping_tests.rs` | `timestamp_with_time_zone_maps_to_timestamp_of_its_precision` | Pass |
| vs-adapter | postgresql-type-mapping | timestamp with time zone maps to TIMESTAMP holding the UTC wall-clock value | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `timestamptz_column_compares_groups_and_joins_like_the_mirror_under_a_utc_session` | Pass |
| vs-adapter | postgresql-type-mapping | Non-core types are unmapped | `crates/adbc-vs-adapter/src/postgres_type_mapping_tests.rs` | `array_columns_are_unmapped_naming_array` | Pass |
| vs-adapter | postgresql-type-mapping | Non-core types are unmapped | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `unmapped_items_columns_are_recorded_and_all_unmapped_table_is_absent` | Pass |
| vs-adapter | postgresql-type-mapping | The mapping is a pure function of the PostgreSQL catalog row | `crates/adbc-vs-adapter/src/postgres_source_tests.rs` | `the_columns_query_selects_every_field_the_mapping_reads` | Pass |
| vs-adapter | postgresql-type-mapping | The mapping is a pure function of the PostgreSQL catalog row | `crates/adbc-vs-adapter/src/postgres_type_mapping_tests.rs` | `a_data_type_is_matched_whatever_its_case` | Pass |
| vs-adapter | postgresql-type-mapping | Virtual schema exposes the public schema only | `crates/adbc-vs-adapter/src/postgres_source_tests.rs` | `both_catalog_queries_filter_on_the_exposed_namespace` | Pass |
| vs-adapter | postgresql-type-mapping | Virtual schema exposes the public schema only | `crates/adbc-vs-adapter/src/postgres_dialect_tests.rs` | `a_table_is_qualified_by_the_recorded_namespace` | Pass |
| vs-adapter | mysql-type-mapping | Signed integer types map to DECIMAL with the matching digit count | `crates/adbc-vs-adapter/src/mysql_type_mapping_tests.rs` | `narrow_signed_integers_map_to_decimal_with_matching_digits` | Pass |
| vs-adapter | mysql-type-mapping | Signed integer types map to DECIMAL with the matching digit count | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` | `narrow_integer_columns_match_the_mirror` | Pass |
| vs-adapter | mysql-type-mapping | DECIMAL preserves the source precision and scale | `crates/adbc-vs-adapter/src/mysql_type_mapping_tests.rs` | `decimal_beyond_precision_36_maps_to_varchar_read_as_text` | Pass |
| vs-adapter | mysql-type-mapping | DECIMAL preserves the source precision and scale | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` | `wide_decimal_reads_as_text_like_the_mirror` | Pass |
| vs-adapter | mysql-type-mapping | Character types preserve the declared length and use UTF8 | `crates/adbc-vs-adapter/src/mysql_type_mapping_tests.rs` | `character_types_preserve_the_declared_length` | Pass |
| vs-adapter | mysql-type-mapping | A CHAR value emits padded and compares as MySQL compares it | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` | `char_column_emits_padded_and_compares_like_the_source` | Pass |
| vs-adapter | mysql-type-mapping | Character columns of every character set map to UTF8 | `crates/adbc-vs-adapter/src/mysql_type_mapping_tests.rs` | `character_columns_map_whatever_their_character_set` | Pass |
| vs-adapter | mysql-type-mapping | Character columns of every character set map to UTF8 | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` | `latin1_columns_emit_and_compare_like_the_mirror` | Pass |
| vs-adapter | mysql-type-mapping | Temporal and floating-point types map to their Exasol counterparts | `crates/adbc-vs-adapter/src/mysql_type_mapping_tests.rs` | `temporal_and_floating_point_types_map_to_their_exasol_counterparts` | Pass |
| vs-adapter | mysql-type-mapping | Temporal and floating-point types map to their Exasol counterparts | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` | `float_column_holds_the_widened_single_precision_value` | Pass |
| vs-adapter | mysql-type-mapping | Unsigned integer types map to DECIMAL with the digits of their unsigned range | `crates/adbc-vs-adapter/src/mysql_type_mapping_tests.rs` | `unsigned_integers_map_to_decimal_with_unsigned_digits` | Pass |
| vs-adapter | mysql-type-mapping | Unsigned integer types map to DECIMAL with the digits of their unsigned range | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` | `unsigned_integer_columns_match_the_mirror` | Pass |
| vs-adapter | mysql-type-mapping | Text types map to VARCHAR bounded by their storage limit | `crates/adbc-vs-adapter/src/mysql_type_mapping_tests.rs` | `text_types_map_to_varchar_bounded_by_their_storage_limit` | Pass |
| vs-adapter | mysql-type-mapping | Text types map to VARCHAR bounded by their storage limit | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` | `text_value_beyond_the_varchar_maximum_fails_the_query` | Pass |
| vs-adapter | mysql-type-mapping | BIT of every width maps to BOOLEAN | `crates/adbc-vs-adapter/src/mysql_type_mapping_tests.rs` | `bit_of_every_width_maps_to_boolean` | Pass |
| vs-adapter | mysql-type-mapping | BIT of every width maps to BOOLEAN | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` | `bit_columns_filter_group_and_aggregate_like_the_mirror` | Pass |
| vs-adapter | mysql-type-mapping | A select-list predicate over a grouped converted column is delegated | `crates/adbc-vs-adapter/src/mysql_dialect_tests.rs` | `grouped_converted_column_in_the_select_list_renders_inside_any_value` | Pass |
| vs-adapter | mysql-type-mapping | A select-list predicate over a grouped converted column is delegated | `crates/adbc-vs-adapter/src/mysql_dialect_tests.rs` | `grouped_bare_column_in_the_select_list_renders_without_any_value` | Pass |
| vs-adapter | mysql-type-mapping | A select-list predicate over a grouped converted column is delegated | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` | `select_list_predicates_over_grouped_converted_columns_match_the_mirror` | Pass |
| vs-adapter | mysql-type-mapping | ENUM and SET map to CHAR | `crates/adbc-vs-adapter/src/mysql_type_mapping_tests.rs` | `enum_and_set_map_to_char_up_to_2000` | Pass |
| vs-adapter | mysql-type-mapping | ENUM and SET map to CHAR | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` | `enum_and_set_columns_emit_padded_and_compare_like_the_source` | Pass |
| vs-adapter | mysql-type-mapping | TIME maps to a TIMESTAMP on 1970-01-01 and YEAR to a DATE | `crates/adbc-vs-adapter/src/mysql_type_mapping_tests.rs` | `column_read_sql_follows_the_source_type` | Pass |
| vs-adapter | mysql-type-mapping | TIME maps to a TIMESTAMP on 1970-01-01 and YEAR to a DATE | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` | `time_and_year_columns_read_and_compare_like_the_mirror` | Pass |
| vs-adapter | mysql-type-mapping | JSON maps to VARCHAR read as its text | `crates/adbc-vs-adapter/src/mysql_type_mapping_tests.rs` | `json_maps_to_varchar_read_as_text` | Pass |
| vs-adapter | mysql-type-mapping | JSON maps to VARCHAR read as its text | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` | `json_column_compares_as_its_text_like_the_mirror` | Pass |
| vs-adapter | mysql-type-mapping | TIMESTAMP maps to TIMESTAMP in the session time zone | `crates/adbc-vs-adapter/src/mysql_type_mapping_tests.rs` | `timestamp_maps_to_timestamp_of_its_precision` | Pass |
| vs-adapter | mysql-type-mapping | TIMESTAMP maps to TIMESTAMP in the session time zone | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` | `timestamp_column_compares_groups_and_joins_like_the_mirror_under_a_utc_session` | Pass |
| vs-adapter | mysql-type-mapping | Non-core MySQL types are unmapped | `crates/adbc-vs-adapter/src/mysql_type_mapping_tests.rs` | `non_core_types_unmapped` | Pass |
| vs-adapter | mysql-type-mapping | Non-core MySQL types are unmapped | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` | `fully_unmappable_table_omitted` | Pass |
| vs-adapter | mysql-type-mapping | The mapping is a pure function of the MySQL catalog row | `crates/adbc-vs-adapter/src/mysql_source_tests.rs` | `the_columns_query_selects_every_field_the_mapping_reads` | Pass |
| vs-adapter | mysql-type-mapping | The mapping is a pure function of the MySQL catalog row | `crates/adbc-vs-adapter/src/metadata_source_tests.rs` | `each_source_maps_its_own_catalog_row` | Pass |
| vs-adapter | mysql-type-mapping | No server collation or version is checked for MySQL | `crates/adbc-vs-adapter/src/metadata_source_tests.rs` | `each_sources_precondition_reads_only_its_own_catalog` | Pass |
| vs-adapter | postgresql-pushdown | MIN and MAX over a BOOLEAN column render as bool_and and bool_or | `crates/adbc-vs-adapter/src/postgres_dialect_tests.rs` | `min_and_max_over_boolean_render_as_bool_and_and_bool_or` | Pass |
| vs-adapter | postgresql-pushdown | MIN and MAX over a BOOLEAN column render as bool_and and bool_or | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `boolean_and_bit_columns_filter_group_and_aggregate_like_the_mirror` | Pass |
| vs-adapter | create-virtual-schema | Columns of unmappable type are omitted and recorded | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `unmapped_items_columns_are_recorded_and_all_unmapped_table_is_absent` | Pass |
| vs-adapter | create-virtual-schema | Columns of unmappable type are omitted and recorded | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` | `unmappable_column_omitted_and_recorded` | Pass |
| vs-adapter | pushdown-join | A CHAR join key pairs rows as the source pairs them | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `char_key_joins_pair_rows_as_postgresql_pairs_them` | Pass |
| vs-adapter | pushdown-join | Every dialect advertises JOIN with an equality condition only | `crates/adbc-vs-adapter/src/capabilities_tests.rs` | `advertised_set_matches_the_rendered_fragments` | Pass |
| vs-adapter | pushdown-join | Every dialect advertises JOIN with an equality condition only | `crates/adbc-vs-adapter/src/capabilities_tests.rs` | `{mysql,postgres,trino}_capability_set_is_exact` | Pass |
| vs-adapter | pushdown-join | Every dialect advertises JOIN with an equality condition only | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `full_outer_join_is_delegated_and_returns_both_unmatched_rows` | Pass |
| vs-adapter | pushdown-join | Every dialect advertises JOIN with an equality condition only | `crates/adbc-vs-adapter/tests/e2e_trino_test.rs` | `full_outer_join_reaches_trino_and_returns_both_unmatched_rows` | Pass |
| vs-adapter | pushdown-join | Only a dialect whose target implements FULL OUTER JOIN advertises it | `crates/adbc-vs-adapter/src/capabilities_tests.rs` | `{mysql,postgres,trino}_capability_set_is_exact` | Pass |
| vs-adapter | pushdown-join | Only a dialect whose target implements FULL OUTER JOIN advertises it | `crates/adbc-vs-adapter/src/postgres_dialect_tests.rs` | `unsupported_join_type_named_per_dialect` | Pass |
| vs-adapter | pushdown-join | Only a dialect whose target implements FULL OUTER JOIN advertises it | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `full_outer_join_is_delegated_and_returns_both_unmatched_rows` | Pass |
| vs-adapter | pushdown-join | Only a dialect whose target implements FULL OUTER JOIN advertises it | `crates/adbc-vs-adapter/tests/e2e_trino_test.rs` | `full_outer_join_reaches_trino_and_returns_both_unmatched_rows` | Pass |
| vs-adapter | pushdown-join | Aliases derive from table position, never from the request | `crates/adbc-vs-adapter/src/{mysql,postgres}_dialect_tests.rs` | `a_request_alias_resolves_the_reference_without_reaching_the_statement` | Pass |
| vs-adapter | pushdown-join | Aliases derive from table position, never from the request | `crates/adbc-vs-adapter/src/{mysql,trino}_dialect_tests.rs` | `a_self_join_resolves_each_copy_to_its_own_alias` | Pass |
| vs-adapter | pushdown-join | Aliases derive from table position, never from the request | `crates/adbc-vs-adapter/src/{mysql,postgres,trino}_dialect_tests.rs` | `a_single_table_query_renders_no_alias_even_when_the_request_assigned_one` | Pass |
| vs-adapter | pushdown-join | Aliases derive from table position, never from the request | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` | `join_matches_local_exasol_result` | Pass |
| vs-adapter | pushdown-join | Outer join emits nulls for the unmatched side | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` | `join_matches_local_exasol_result` | Pass |
| vs-adapter | pushdown-join | Outer join emits nulls for the unmatched side | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `full_outer_join_is_delegated_and_returns_both_unmatched_rows` | Pass |
| vs-adapter | pushdown-join | Outer join emits nulls for the unmatched side | `crates/adbc-vs-adapter/tests/e2e_trino_test.rs` | `full_outer_join_reaches_trino_and_returns_both_unmatched_rows` | Pass |
| vs-adapter | pushdown-join | Each join type renders its counterpart | `crates/adbc-vs-adapter/src/pushdown_query_tests.rs` | `every_join_type_spelling_parses_verbatim` | Pass |
| vs-adapter | pushdown-join | Each join type renders its counterpart | `crates/adbc-vs-adapter/src/postgres_dialect_tests.rs` | `each_join_type_renders` | Pass |
| vs-adapter | pushdown-join | Each join type renders its counterpart | `crates/adbc-vs-adapter/src/mysql_dialect_tests.rs` | `each_join_type_renders_its_mysql_keyword_in_the_requests_operand_order` | Pass |
| vs-adapter | pushdown-join | Each join type renders its counterpart | `crates/adbc-vs-adapter/src/trino_dialect_tests.rs` | `each_join_type_renders_its_own_keyword` | Pass |
| vs-adapter | pushdown-join | Inner join of two virtual tables renders one statement | `crates/adbc-vs-adapter/src/postgres_dialect_tests.rs` | `inner_join_renders_one_statement` | Pass |
| vs-adapter | pushdown-join | Inner join of two virtual tables renders one statement | `crates/adbc-vs-adapter/src/mysql_dialect_tests.rs` | `a_join_renders_one_statement_with_every_table_aliased_and_every_column_qualified` | Pass |
| vs-adapter | pushdown-join | Inner join of two virtual tables renders one statement | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `join_reaches_postgresql_and_matches_the_mirror` | Pass |
| vs-adapter | pushdown-join | Inner join of two virtual tables renders one statement | `crates/adbc-vs-adapter/tests/e2e_trino_test.rs` | `join_reaches_trino_and_matches_the_mirror` | Pass |
| vs-adapter | pushdown-join | Inner join of two virtual tables renders one statement | `crates/adbc-vs-adapter/tests/e2e_{postgres,trino}_test.rs` | `no_generated_statement_carries_a_collation_keyword` | Pass |
| vs-adapter | pushdown-aggregation | Declared type entries carry the shapes this release measured | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` | `exasol_supplies_the_measured_type_shapes` | Pass |
| vs-adapter | pushdown-aggregation | MIN and MAX over a CHAR operand are declared CHAR | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `min_and_max_over_char_are_declared_char_like_the_mirror` | Pass |
| vs-adapter | pushdown-aggregation | Aggregation is delegated only beside SELECTLIST_EXPRESSIONS | `crates/adbc-vs-adapter/src/capabilities_tests.rs` | `advertised_set_matches_the_rendered_fragments` | Pass |
| vs-adapter | pushdown-aggregation | Aggregation is delegated only beside SELECTLIST_EXPRESSIONS | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` | `aggregation_matches_local_exasol_result` | Pass |
| vs-adapter | pushdown-aggregation | HAVING and grouping by an expression are withheld from every dialect | `crates/adbc-vs-adapter/src/capabilities_tests.rs` | `{mysql,postgres,trino}_capability_set_is_exact` | Pass |
| vs-adapter | pushdown-aggregation | A multi-column GROUP BY is delegated only by the dialect advertising AGGREGATE_GROUP_BY_TUPLE | `crates/adbc-vs-adapter/src/capabilities_tests.rs` | `trino_and_postgresql_sets_differ_by_two_named_entries` | Pass |
| vs-adapter | pushdown-aggregation | A multi-column GROUP BY is delegated only by the dialect advertising AGGREGATE_GROUP_BY_TUPLE | `crates/adbc-vs-adapter/tests/e2e_{mysql,postgres}_test.rs` | `two_column_group_by_is_not_delegated` | Pass |
| vs-adapter | pushdown-aggregation | A multi-column GROUP BY is delegated only by the dialect advertising AGGREGATE_GROUP_BY_TUPLE | `crates/adbc-vs-adapter/tests/e2e_trino_test.rs` | `two_column_group_by_is_delegated_and_matches_the_mirror` | Pass |
| vs-adapter | pushdown-aggregation | Single-group aggregation renders without a GROUP BY clause | `crates/adbc-vs-adapter/src/{mysql,postgres,trino}_dialect_tests.rs` | `single_group_renders_without_group_by` | Pass |
| vs-adapter | pushdown-aggregation | Each aggregate declares the type the request supplies for it | `crates/adbc-vs-adapter/src/emit_type_tests.rs` | `declared_types_come_from_select_list_data_types` | Pass |
| vs-adapter | pushdown-aggregation | Each aggregate declares the type the request supplies for it | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` | `exasol_supplies_the_measured_aggregate_types` | Pass |
| vs-adapter | pushdown-aggregation | Aggregate function name case does not change the rendered statement | `crates/adbc-vs-adapter/src/pushdown_query_tests.rs` | `a_function_aggregate_name_is_matched_case_insensitively` | Pass |
| vs-adapter | pushdown-aggregation | Aggregate function name case does not change the rendered statement | `crates/adbc-vs-adapter/src/{mysql,postgres}_dialect_tests.rs` | `mixed_case_function_names_render_identically` | Pass |
| vs-adapter | pushdown-aggregation | Character grouping keys and aggregate operands carry no collation suffix | `crates/adbc-vs-adapter/src/{mysql,postgres,trino}_dialect_tests.rs` | `no_operand_carries_a_collation_suffix_in_any_position` | Pass |
| vs-adapter | pushdown-aggregation | Character grouping keys and aggregate operands carry no collation suffix | `crates/adbc-vs-adapter/tests/e2e_{postgres,trino}_test.rs` | `uncollated_grouping_and_distinct_return_the_mirrors_group_count` | Pass |
| vs-adapter | pushdown-aggregation | Character grouping keys and aggregate operands carry no collation suffix | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` | `grouping_over_character_variants_matches_local_exasol_result` | Pass |
| vs-adapter | pushdown-aggregation | An aggregation Exasol cannot delegate whole is computed by Exasol | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` | `aggregation_not_delegable_whole_is_withdrawn_and_matches_local_exasol_result` | Pass |
| vs-adapter | pushdown-aggregation | AVG and SUM are computed by Exasol rather than delegated | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` | `avg_and_sum_are_withdrawn_and_match_local_exasol_result` | Pass |
| vs-adapter | pushdown-aggregation | AVG and SUM are computed by Exasol rather than delegated | `crates/adbc-vs-adapter/tests/e2e_{postgres,trino}_test.rs` | `avg_and_sum_are_withdrawn_and_match_the_mirror` | Pass |
| vs-adapter | pushdown-aggregation | AVG and SUM are computed by Exasol rather than delegated | `crates/adbc-vs-adapter/src/{mysql,postgres,trino}_dialect_tests.rs` | `unrenderable_aggregate_named_in_error` | Pass |
| vs-adapter | pushdown-aggregation | AVG stays withheld because no rendering reproduces Exasol's value | `crates/adbc-vs-adapter/src/capabilities_tests.rs` | `{mysql,postgres,trino}_capability_set_is_exact` | Pass |
| vs-adapter | pushdown-aggregation | AVG stays withheld because no rendering reproduces Exasol's value | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` | `avg_and_sum_are_withdrawn_and_match_local_exasol_result` | Pass |
| vs-adapter | pushdown-aggregation | SUM stays withheld because a floating-point sum depends on the source's row order | `crates/adbc-vs-adapter/src/capabilities_tests.rs` | `{mysql,postgres,trino}_capability_set_is_exact` | Pass |
| vs-adapter | pushdown-aggregation | SUM stays withheld because a floating-point sum depends on the source's row order | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` | `avg_and_sum_are_withdrawn_and_match_local_exasol_result` | Pass |
| vs-adapter | pushdown-aggregation | Every dialect delegates the same aggregate set | `crates/adbc-vs-adapter/src/capabilities_tests.rs` | `{mysql,postgres,trino}_capability_set_is_exact` | Pass |
| vs-adapter | pushdown-aggregation | Every dialect delegates the same aggregate set | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` | `aggregation_matches_local_exasol_result` | Pass |
| scan-udf | arrow-scan | Narrow and unsigned integers coerce into their declared DECIMAL | `crates/adbc-vs-scan/src/coerce_tests.rs` | `narrow_and_unsigned_integers_coerce_into_their_declared_decimal` | Pass |
| scan-udf | arrow-scan | Narrow and unsigned integers coerce into their declared DECIMAL | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` | `unsigned_integer_columns_match_the_mirror` | Pass |
| scan-udf | arrow-scan | A time-zone-aware timestamp is emitted as its UTC wall-clock value | `crates/adbc-vs-scan/src/coerce_tests.rs` | `time_zone_aware_timestamp_coerces_to_its_utc_wall_clock_value` | Pass |
| scan-udf | arrow-scan | A time-zone-aware timestamp is emitted as its UTC wall-clock value | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `timestamptz_column_compares_groups_and_joins_like_the_mirror_under_a_utc_session` | Pass |
| scan-udf | arrow-scan | Scan connects using credentials resolved at run time | `crates/adbc-vs-scan/src/connection_tests.rs` | `credentials_are_resolved_per_open_and_never_persisted` | Pass |
| scan-udf | arrow-scan | One scan instance streams the whole result as Arrow IPC bytes | `crates/adbc-vs-adapter/src/pushdown_tests.rs` | `the_scan_call_selects_the_scan_script_over_one_row` | Pass |
| scan-udf | arrow-scan | Emitted columns match the declared output | `crates/adbc-vs-scan/src/coerce_tests.rs` | `the_coerced_batch_carries_the_declared_names_in_the_declared_order` | Pass |
| scan-udf | arrow-scan | Emitted columns match the declared output | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `character_column_emits_padded_and_compares_like_the_source` | Pass |
| scan-udf | arrow-scan | A declared TIMESTAMP(p) is emitted at microsecond resolution | `crates/adbc-vs-scan/src/coerce_tests.rs` | `timestamp_units_normalize` | Pass |
| scan-udf | arrow-scan | A declared TIMESTAMP(p) is emitted at microsecond resolution | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `scan_emits_every_fractional_digit_of_a_declared_timestamp` | Pass |
| scan-udf | arrow-scan | A declared TIMESTAMP(p) is emitted at microsecond resolution | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` | `datetime_precision_carries_into_the_declared_timestamp` | Pass |
| scan-udf | arrow-scan | Declared BOOLEAN column emits every representation the driver returns | `crates/adbc-vs-scan/src/coerce_tests.rs` | `declared_boolean_accepts_driver_representations` | Pass |
| scan-udf | arrow-scan | Declared BOOLEAN column emits every representation the driver returns | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `boolean_and_bit_columns_filter_group_and_aggregate_like_the_mirror` | Pass |
| scan-udf | arrow-scan | Provisioned driver loads inside the UDF sandbox | `crates/adbc-vs-scan/tests/{postgres,trino}_it_test.rs` | `driver_loads_via_declared_entry_point_and_executes_a_statement` | Pass |
| scan-udf | arrow-scan | Provisioned driver loads inside the UDF sandbox | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `every_driver_this_stack_provisions_creates_a_virtual_schema` | Pass |
| scan-udf | postgresql-scan | Connection string is built from the address and escapes every credential byte | `crates/adbc-vs-scan/src/connection_tests.rs` | `postgres_escapes_every_credential_byte` | Pass |
| scan-udf | postgresql-scan | Connection string is built from the address and escapes every credential byte | `crates/adbc-vs-scan/src/connection_tests.rs` | `postgres_carries_every_address_parameter_across` | Pass |
| scan-udf | postgresql-scan | The credential travels only inside the driver's uri option for one connection attempt | `crates/adbc-vs-scan/src/connection_tests.rs` | `postgres_passes_one_conninfo_option_and_no_credential_option` | Pass |
| scan-udf | postgresql-scan | The credential travels only inside the driver's uri option for one connection attempt | `crates/adbc-vs-scan/src/connection_tests.rs` | `credentials_are_resolved_per_open_and_never_persisted` | Pass |
| scan-udf | postgresql-scan | The credential travels only inside the driver's uri option for one connection attempt | `crates/adbc-vs-adapter/tests/dispatch_test.rs` | `pushdown_sql_has_no_credential` | Pass |
| scan-udf | postgresql-scan | A numeric arriving as a string is coerced into its declared DECIMAL | `crates/adbc-vs-scan/src/coerce_tests.rs` | `string_form_numeric_coerces_to_declared_decimal` | Pass |
| scan-udf | postgresql-scan | Non-finite numeric fails with a named error | `crates/adbc-vs-scan/src/coerce_tests.rs` | `non_finite_string_form_numeric_fails_naming_the_value_and_declared_type` | Pass |
| scan-udf | postgresql-scan | Non-finite numeric fails with a named error | `crates/adbc-vs-scan/src/coerce_tests.rs` | `a_finite_value_coerces_after_a_non_finite_value_in_the_same_column` | Pass |
| scan-udf | postgresql-scan | Non-finite numeric fails with a named error | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `wide_and_unconstrained_numeric_read_as_text_like_the_mirror` | Pass |
| vs-adapter | mysql-collation | No rendered operand carries a collation suffix | `crates/adbc-vs-adapter/src/mysql_dialect_tests.rs` | `no_operand_carries_a_collation_suffix_in_any_position` | Pass |
| vs-adapter | mysql-collation | A delegated comparison and grouping follow the column's own collation | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` | `comparison_and_grouping_follow_the_columns_own_collation` | Pass |
| vs-adapter | mysql-collation | A column declaring utf8mb4_0900_bin returns Exasol's answers | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` | `trailing_space_values_compare_and_order_as_in_exasol` | Pass |
| vs-adapter | mysql-collation | A column declaring utf8mb4_0900_bin returns Exasol's answers | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` | `latin1_column_converted_to_utf8mb4_0900_bin_in_a_view_returns_exasols_answers` | Pass |
| vs-adapter | mysql-collation | A select-list predicate over the grouped character column is delegated | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` | `select_list_predicate_over_grouped_character_column_delegates_and_matches_the_mirror` | Pass |
| vs-adapter | postgresql-collation | Delegated grouping and DISTINCT over case-only variants return Exasol's groups | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `uncollated_grouping_and_distinct_return_the_mirrors_group_count` | Pass |
| vs-adapter | postgresql-collation | A column under a nondeterministic collation is mapped and compares as PostgreSQL compares | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `nondeterministic_collation_column_maps_and_compares_as_postgresql_compares` | Pass |
| vs-adapter | postgresql-collation | A select-list predicate over the grouped character column is delegated | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `select_list_predicate_over_the_grouped_character_column_delegates_and_matches_the_mirror` | Pass |
| vs-adapter | postgresql-collation | No rendered operand carries a collation suffix | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `no_generated_statement_carries_a_collation_keyword` | Pass |
| vs-adapter | postgresql-collation | A delegated ordering comparison and MIN or MAX follow the column's own collation | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `ordering_and_min_max_follow_the_columns_own_collation` | Pass |
| vs-adapter | postgresql-collation | A column declaring the C collation returns Exasol's answers | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `filter_reaches_postgresql_and_matches_the_mirror` | Pass |
| vs-adapter | postgresql-collation | A column declaring the C collation returns Exasol's answers | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `aggregation_reaches_postgresql_and_matches_the_mirror` | Pass |
| vs-adapter | postgresql-collation | An ordering comparison over a wrapped operand is evaluated by Exasol | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `an_ordering_comparison_over_a_wrapped_operand_is_evaluated_by_exasol` | Pass |

## Notes

- Two measurements changed documented differences. PostgreSQL does not match a `character(5)` value against `LIKE 'ab'`, so docs list no PostgreSQL `LIKE` difference. MySQL does match a `CHAR` value, so docs list it for MySQL.
- A MySQL `YEAR` of `0000` emits `2000-01-01`, and a `TIME` of `25:00:00` reads as `1970-01-02 01:00:00`. Both are in `docs/type-mapping.md`.
- A text longer than 2000000 characters fails the query with `string data, right truncation` (SQL state 22002), measured on both PostgreSQL and MySQL.
- Exasol sends no `adapterNotes` member for a column declared without notes. An E2E assertion pins this.
- Coverage percentages are not measured: the project has no coverage make target.
