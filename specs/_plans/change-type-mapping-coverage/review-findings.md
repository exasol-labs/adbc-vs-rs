# Code Review Findings: change-type-mapping-coverage

## Summary
- Files reviewed: 55
- Total findings: 18 (standard: 16, expert: 2)

Every item in plan.md § Dead Code Removal is gone (`MappedType` and its `From` impl, the listed PostgreSQL and MySQL reasons, `MySqlColumn.character_set`, `CHARACTER_SET_NAME`, and every listed test). `make lint` passes with zero warnings. The MySQL `ANY_VALUE` wrapping applies to grouping keys only: `Statement::render_column` wraps a read expression only when it is one of `any_value_keys`, and aggregate operands, `WHERE`, `ON`, and `GROUP BY` render through `per_row()`. No new error message carries a credential: the new errors name a column, its `adapterNotes` text, or a `dataType` JSON value. The `docs/type-mapping.md` rows match the three `*_type_mapping.rs` modules, except the `money` note below.

## Standard fixes

### crates/adbc-vs-adapter/src/postgres_type_mapping.rs

#### [UNTESTED_ERROR_PATH] Array omission reason carries a run of ten spaces
- Location: line 211, `array_reason`
- Issue: The format string reads `"... names no element type and          the Exasol PostgreSQL JDBC Virtual Schema leaves arrays out too"`. A `\` line continuation was lost, so every recorded array omission in a schema's `adapterNotes` carries ten spaces in the middle of the sentence. `array_columns_are_unmapped_naming_array` (postgres_type_mapping_tests.rs:415) asserts only `reason.contains("ARRAY")`, so the malformed text passes.
- Fix: In crates/adbc-vs-adapter/src/postgres_type_mapping.rs, rewrite the string in `array_reason` as `"PostgreSQL type {data_type} is not mapped, because the catalog names no element type and \` followed on the next line by `the Exasol PostgreSQL JDBC Virtual Schema leaves arrays out too"`, so a single space joins the two halves. In crates/adbc-vs-adapter/src/postgres_type_mapping_tests.rs, change `array_columns_are_unmapped_naming_array` to `assert_eq!` the full reason `"PostgreSQL type ARRAY is not mapped, because the catalog names no element type and the Exasol PostgreSQL JDBC Virtual Schema leaves arrays out too"`.

### crates/adbc-vs-adapter/src/postgres_type_mapping_tests.rs

#### [MISSING_BOUNDARY_TEST] Absent timestamp precision and absent or zero character length are untested
- Location: `map_data_type` timestamp arm (postgres_type_mapping.rs:152-156) and `map_character` (postgres_type_mapping.rs:189-198)
- Issue: No test maps a `timestamp` whose `datetime_precision` is `None`, so the `DEFAULT_POSTGRES_TIMESTAMP_PRECISION` branch is unexercised. `character_maps_to_char_up_to_2000_and_varchar_above` covers only `Some(length)` of 5 and above. A `bpchar` column without a length reports `data_type` `character` with a null `character_maximum_length`, which takes the `VARCHAR(2000000)` branch and reads as `CAST(... AS TEXT)`, and a length of 0 takes the `CHAR(1)` branch. Neither branch has a test.
- Fix: In crates/adbc-vs-adapter/src/postgres_type_mapping_tests.rs, add `a_timestamp_without_a_reported_precision_declares_postgresqls_default_of_six`, asserting `mapped(&column(data_type))` is `ExasolType::Timestamp { precision: 6 }` for `timestamp without time zone` and `timestamp with time zone`. Add `unbounded_and_zero_length_character_columns_map`, asserting that `character_column("character", None)` maps to `ExasolType::Varchar { size: 2_000_000 }` and reads `CAST("c" AS TEXT)`, and that `character_column("character", Some(0))` maps to `ExasolType::Char { size: 1 }` and reads `"c"`.

### crates/adbc-vs-adapter/src/postgres_dialect_tests.rs

#### [SHRINKABLE] Closure restates `fixtures::aggregate_over` with identical arguments
- Location: line 1743, `min_and_max_over_boolean_render_as_bool_and_and_bool_or`
- Issue: `let aggregate_of = |function, column_name, distinct| fixtures::aggregate_over(function, column_name, distinct);` adds a name and nothing else.
- Fix: In crates/adbc-vs-adapter/src/postgres_dialect_tests.rs, delete the `aggregate_of` closure in `min_and_max_over_boolean_render_as_bool_and_and_bool_or` and replace each `aggregate_of(` call in that test with `fixtures::aggregate_over(`.

### crates/adbc-vs-adapter/src/mysql_type_mapping.rs

#### [VAGUE_NAME] `SUPPORTED_CHARACTER_SET` lives in the mapping and names a restriction the mapping dropped
- Location: line 66
- Issue: The mapping no longer reads a character set, because every character set maps to `UTF8`. The constant's only reader is `render_string_literal` in mysql_dialect.rs:722, where it names the introducer of a hex string literal. The name `SUPPORTED_CHARACTER_SET` still states that the mapping supports one character set, and the dialect's literal decision sits in the type mapping module.
- Fix: Delete `SUPPORTED_CHARACTER_SET` and its doc comment from crates/adbc-vs-adapter/src/mysql_type_mapping.rs. In crates/adbc-vs-adapter/src/mysql_dialect.rs, add a private `const LITERAL_CHARACTER_SET: &str = "utf8mb4";` with the doc comment `/// The character set a string literal's hex introducer names: the literal carries UTF-8 bytes whatever the column's character set.`, use it in `render_string_literal`, and change the import on line 63 to `use crate::mysql_type_mapping;`.

#### [INLINE_COMMENT] Inline comment in `character_size` runs four lines
- Location: lines 215-218
- Issue: AGENTS.md § Code quality limits a comment to 1 or 2 lines. The comment above the `Some(length)` arm runs four.
- Fix: In crates/adbc-vs-adapter/src/mysql_type_mapping.rs, replace the four comment lines in `character_size` with `// Exasol rejects a zero-length declaration, so a reported 0 becomes 1` and `// rather than failing CREATE VIRTUAL SCHEMA for a column MySQL accepts.`

### crates/adbc-vs-adapter/src/mysql_type_mapping_tests.rs

#### [DUPLICATE_TEST] Source-type recording test repeats the cross-driver test
- Location: line 411, `the_mapped_column_records_the_source_type_as_the_catalog_spelled_it`
- Issue: The test asserts that a `json` column records `source_type` `"json"`. `every_mapped_column_records_the_catalog_spelling_of_its_type` in metadata_source_tests.rs asserts the same property for every column of `one_column_of_every_mapped_type()`, which includes `column("json")`.
- Fix: In crates/adbc-vs-adapter/src/mysql_type_mapping_tests.rs, delete `the_mapped_column_records_the_source_type_as_the_catalog_spelled_it`.

### crates/adbc-vs-adapter/src/trino_type_mapping.rs

#### [INLINE_COMMENT] Timestamp arm comment narrates the release history
- Location: lines 167-168
- Issue: The comment reads "Every precision declares what a Trino virtual table declared before this release, which carries fractional seconds for PostgreSQL and MySQL only." AGENTS.md § Code quality forbids narrating history in a comment, and "this release" goes stale with the next one. The reason that holds is that a refresh keeps an existing Trino column's declared type.
- Fix: In crates/adbc-vs-adapter/src/trino_type_mapping.rs, replace the two comment lines above the `timestamp` arm of `map_data_type` with `// Exasol's default precision, so a refresh leaves an existing Trino column's type unchanged.`

### crates/adbc-vs-scan/src/coerce_tests.rs

#### [MISSING_BOUNDARY_TEST] Integer coercion cases pair widths with precisions no mapping declares
- Location: lines 747, 762, 787, 792, 822, `narrow_and_unsigned_integers_coerce_into_their_declared_decimal`
- Issue: The test pairs `Int8` 127 with `DECIMAL(5,0)`, `Int16` 32767 with `DECIMAL(8,0)`, and signed `i64::MIN` and `i64::MAX` with `DECIMAL(20,0)`. MySQL declares `TINYINT` `DECIMAL(3,0)`, `SMALLINT` `DECIMAL(5,0)`, and signed `BIGINT` `DECIMAL(19,0)`. The signed `BIGINT` extremes therefore never reach the `Decimal128(19, 0)` path at exactly 19 digits, which is the boundary of the declaration MySQL and PostgreSQL produce. The `MEDIUMINT` declarations `DECIMAL(7,0)` and `DECIMAL(8,0)` have no case.
- Fix: In crates/adbc-vs-scan/src/coerce_tests.rs, in `narrow_and_unsigned_integers_coerce_into_their_declared_decimal`, change the `Int8Array::from(vec![127])` case to `declared_decimal_3`, the `Int16Array::from(vec![32_767])` case to `declared_decimal_5`, and both `Int64Array` cases to a new `let declared_decimal_19 = decimal_of(19);`. Add `Int32Array::from(vec![-8_388_608])` and `Int32Array::from(vec![8_388_607])` into `decimal_of(7)`, and `UInt32Array::from(vec![16_777_215])` into `decimal_of(8)`. Change the `DataType::Decimal128(20, 0)` match arm to `DataType::Decimal128(_, 0)` so both 19- and 20-digit results are read.

### crates/adbc-vs-adapter/tests/e2e_postgres_test.rs

#### [ASSERTION_FREE_TEST] Oversize text test accepts any failure
- Location: lines 2316-2323, `text_value_beyond_the_varchar_maximum_fails_the_query`
- Issue: `execute_expecting_failure` already panics when the statement succeeds and returns the failure text otherwise, so `assert!(!error.is_empty())` cannot fail. A failure for an unrelated cause, such as a driver load error or a missing table, passes the test as well as the length overflow the `vs-adapter/type-mapping` scenario names.
- Fix: In crates/adbc-vs-adapter/tests/e2e_postgres_test.rs, run `text_value_beyond_the_varchar_maximum_fails_the_query` once with `make cross-udf-build` and `make test-e2e-postgres`, read the error text Exasol returns for the 2000001-character value, and replace `assert!(!error.is_empty(), ...)` with an assertion that `error` contains the substring that names the overflow (the size violation or truncation wording Exasol reports), with a message that prints `error`.

#### [REDUNDANT_COMMENT] New section banner
- Location: line 1427
- Issue: `// --- type coverage ---...` is a banner. AGENTS.md § Code quality says never add banners.
- Fix: In crates/adbc-vs-adapter/tests/e2e_postgres_test.rs, delete line 1427, `// --- type coverage ----------------------------------------------------------`.

### crates/adbc-vs-adapter/tests/e2e_mysql_test.rs

#### [ASSERTION_FREE_TEST] Oversize text test accepts any failure
- Location: lines 2357-2363, `text_value_beyond_the_varchar_maximum_fails_the_query`
- Issue: Same defect as the PostgreSQL test: `assert!(!error.trim().is_empty())` holds for every failure `execute_expecting_failure` returns, so any failure cause passes.
- Fix: In crates/adbc-vs-adapter/tests/e2e_mysql_test.rs, run the test once with `make cross-udf-build` and `make test-e2e`, read the error text for the 2000001-character value, and replace the `!error.trim().is_empty()` assertion with an assertion that `error` contains the substring that names the overflow, with a message that prints `error`.

#### [SELECTOR_ARGUMENT] `assert_matches_mirror` swaps the mirror table on one table name
- Location: lines 1211-1213 (doc comment) and 1233-1237, `assert_matches_mirror`
- Issue: The shared helper compares `virtual_items` against the mirror table of the same name, except that `mysql::CONVERTED_REGION_VIEW` silently selects `ITEMS`. The argument picks a branch, and a reader of a call site cannot see which mirror table is compared.
- Fix: In crates/adbc-vs-adapter/tests/e2e_mysql_test.rs, add `fn assert_matches_mirror_of(virtual_table: &str, mirror_table: &str, query: impl Fn(&str, &str) -> String) -> Vec<serde_json::Map<String, serde_json::Value>>` holding the body of `assert_matches_mirror` with `mirror_table` in place of `mirrored` and no branch. Make `assert_matches_mirror(virtual_items, query)` return `assert_matches_mirror_of(virtual_items, virtual_items, query)`. In `latin1_column_converted_to_utf8mb4_0900_bin_in_a_view_returns_exasols_answers`, call `assert_matches_mirror_of(view, ITEMS, ...)` for every comparison. Delete the clause "except for [`mysql::CONVERTED_REGION_VIEW`], whose rows are those of [`ITEMS`]" from the doc comment.

### crates/adbc-vs-adapter/tests/common/mysql.rs

#### [IMPLEMENTATION_COUPLED_TEST] New fixture tests assert the text of the constant beside them
- Location: lines 586, 617, 627, 637: `fixtures_declare_one_column_per_mapped_type`, `fixtures_expose_the_converted_region_view_over_items`, `fixtures_hold_no_unmapped_json_column`, `the_code_point_table_alone_holds_the_two_like_rows`
- Issue: Each test asserts that `fixture_script()` or a `CREATE TABLE` statement contains a literal that the same file spells in its fixture constant, which is a source-text tautology (constitution: test behavior, not source text or configuration tautologies). The E2E tier asserts the behavior these fixtures exist for: `assert_declares` over every `type_coverage` column, `latin1_column_converted_to_utf8mb4_0900_bin_in_a_view_returns_exasols_answers` over the view, `unmappable_column_omitted_and_recorded` and `fully_unmappable_table_omitted` over `photo` and `shape`, and `filter_matches_local_exasol_result` over the two `LIKE` rows.
- Fix: In crates/adbc-vs-adapter/tests/common/mysql.rs, delete the tests `fixtures_declare_one_column_per_mapped_type`, `fixtures_expose_the_converted_region_view_over_items`, `fixtures_hold_no_unmapped_json_column`, and `the_code_point_table_alone_holds_the_two_like_rows`, and any helper, such as `create_statement_of`, that only they call.

### crates/adbc-vs-adapter/tests/common/mirror.rs

#### [IMPLEMENTATION_COUPLED_TEST] Mirror tests restate the mirror constants
- Location: lines 414 and 423: `the_float_is_mirrored_as_its_single_precision_value_widened`, `the_bit_columns_are_mirrored_as_whether_any_bit_is_set`
- Issue: The first asserts that `TYPE_COVERAGE_VALUES` contains `0.100000001490116119384765625`, a string that constant spells. The second asserts that the DDL built from `TYPE_COVERAGE_COLUMNS` declares the three `BIT` columns `BOOLEAN`, which that constant spells. The E2E differential tests compare the virtual table with this mirror and fail on any disagreement.
- Fix: In crates/adbc-vs-adapter/tests/common/mirror.rs, delete `the_float_is_mirrored_as_its_single_precision_value_widened` and `the_bit_columns_are_mirrored_as_whether_any_bit_is_set`. Keep `every_mirrored_row_carries_one_value_per_declared_column`, which checks two constants against each other.

### crates/adbc-vs-adapter/tests/common/postgres_mirror.rs

#### [IMPLEMENTATION_COUPLED_TEST] Mirror declaration test restates `ITEM_COLUMNS`
- Location: line 406, `the_mirrored_items_declare_every_mapped_column_and_no_array`
- Issue: The test asserts that the DDL built from `ITEM_COLUMNS` contains declarations that constant spells. `mirror_declares_the_same_columns_and_types_as_the_virtual_table` in e2e_postgres_test.rs asserts the property that matters, that the mirror declares what the virtual table declares.
- Fix: In crates/adbc-vs-adapter/tests/common/postgres_mirror.rs, delete `the_mirrored_items_declare_every_mapped_column_and_no_array`, and delete `item_declarations` if no other code calls it.

### docs/type-mapping.md

#### [OUTDATED_COMMENT] The `money` note claims independence from `lc_monetary`
- Location: line 29
- Issue: The note says "The value does not depend on `lc_monetary`". PostgreSQL's `money` to `numeric` cast (`cash_numeric` in `src/backend/utils/adt/cash.c`) divides the stored integer by 10 to the power of the session `lc_monetary` `frac_digits`, and the PostgreSQL manual states that the fractional precision of `money` is determined by `lc_monetary`. The read expression drops the currency symbol and separators, but not that scale. The claim is also unexercised: the test container offers only the `C`, `C.utf8`, `en_US.utf8`, and `POSIX` locales (measured with `locale -a`), all of which give 2 digits. The spec delta step at `vs-adapter/postgresql-type-mapping/spec.md:130` repeats the claim.
- Fix: In docs/type-mapping.md, replace the `money` row's note "The value does not depend on `lc_monetary`" with "The currency symbol and separators of `lc_monetary` do not affect the value. PostgreSQL scales the stored amount by the fractional digits of the session `lc_monetary`, 2 under the `C` and `en_US` locales".

## Expert fixes

### crates/adbc-vs-adapter/src/postgres_dialect.rs, mysql_dialect.rs, trino_dialect.rs

#### [DEAD_FLEXIBILITY] The `read_sql` parameter exists only for tests
- Location: `render_reading_columns_through` and `Statement.read_sql` in postgres_dialect.rs:161-182 and :290, mysql_dialect.rs:138-167 and :285, trino_dialect.rs:175-195 and :286; `ColumnReadSql` in sql_dialect.rs:36; `CONVERTED_SOURCE_TYPE`, `stub_read_sql`, the `converted_*` request builders, and `render_through_stub` in dialect_test_fixtures.rs:148-271; `render_through_stub` in mysql_dialect_tests.rs:1541, postgres_dialect_tests.rs:1599, trino_dialect_tests.rs:1007
- Issue: Each dialect's `render` passes one constant function, its own technology's `column_read_sql`, and only tests pass another (`stub_read_sql`). The function pointer is pure, not a seam over I/O, nondeterminism, or a third party, so it is a production parameter that production never varies. Real converted source types already exist for every position test: PostgreSQL `uuid` reads as `CAST(... AS TEXT)` and `bit` as `(position(B'1' IN ...) > 0)`, and MySQL `json` reads as `CAST(... AS CHAR)` and `bit` as `(... <> 0)`. postgres_dialect_tests.rs already asserts `MAX(CAST("uid" AS TEXT))` and a full outer join on `CAST("t1"."uid" AS TEXT)` through the real `render`. Driving the position tests through the real conversion tests the plumbing and the conversion together and removes the test-only path from production code. The fix changes three dialects, `sql_dialect.rs`, the shared fixtures, and three test modules.
- Fix: (1) In crates/adbc-vs-adapter/src/postgres_dialect.rs, mysql_dialect.rs, and trino_dialect.rs, delete `render_reading_columns_through`, move its body into `SqlDialect::render`, delete the `read_sql` field of `Statement`, and make `render_column` call the technology's function directly: `postgres_type_mapping::column_read_sql`, `mysql_type_mapping::column_read_sql`, and `trino_type_mapping::column_read_sql`. Keep the MySQL `any_value_keys` logic unchanged. Remove the `ColumnReadSql` imports. (2) Delete `ColumnReadSql` and its doc comment from crates/adbc-vs-adapter/src/sql_dialect.rs, and the `ExasolType` import if nothing else uses it. (3) In crates/adbc-vs-adapter/src/dialect_test_fixtures.rs, delete `CONVERTED_SOURCE_TYPE`, `stub_read_sql`, and `render_through_stub`. Give `converted_table`, `converted_request`, and the four public `converted_*` builders a `notes: ConvertedNotes` argument, a new `pub(crate) struct ConvertedNotes { pub uid: &'static str, pub flag: &'static str }`, and write `{"sourceType": notes.uid}` on `uid` and `{"sourceType": notes.flag}` on `flag`. (4) In mysql_dialect_tests.rs, postgres_dialect_tests.rs, and trino_dialect_tests.rs, delete `render_through_stub` and render with `render_statement`. Pass `ConvertedNotes { uid: "uuid", flag: "bit" }` for PostgreSQL, `ConvertedNotes { uid: "json", flag: "bit" }` for MySQL, and `ConvertedNotes { uid: "varchar(36)", flag: "boolean" }` for Trino. Rewrite each expected statement: in PostgreSQL `CONVERTED("uid")` becomes `CAST("uid" AS TEXT)` and `CONVERTED("flag")` becomes `(position(B'1' IN "flag") > 0)`; in MySQL `` CONVERTED(`uid`) `` becomes `` CAST(`uid` AS CHAR) `` and `` CONVERTED(`flag`) `` becomes `` (`flag` <> 0) ``, including inside `ANY_VALUE(...)`; in Trino every `CONVERTED(x)` becomes the bare quoted `x`. Keep every test name, because plan.md § Scenario Coverage names `column_reference_renders_identically_in_every_position` and `boolean_column_predicate_renders_through_its_read_expression`. Run `make test` and `make lint`.

### crates/adbc-vs-adapter/src/pushdown_query.rs

#### [MISSING_DESIGN_INTENT] The empty-string column note tolerance has no measured source
- Location: lines 430-451, `read_source_type`, the arm `Some(Value::String(text)) if text.is_empty() => return Ok(None)`; test `an_empty_column_note_records_no_source_type` in pushdown_query_tests.rs:489
- Issue: The doc comment justifies both the absent and the empty note with one reason, that a column declared before columns carried notes renders bare. The `vs-adapter/type-mapping` delta covers only the absent case ("a column without `adapterNotes` ... MUST render as the bare quoted column"). Nothing in plan.md § Context, the E2E tests, or the code records that Exasol echoes `"adapterNotes": ""` for a column declared without notes. The arm also exempts `""` from the rule that `malformed_column_note_fails_naming_the_column` enforces for every other note this adapter never writes. If Exasol omits the member, the arm is unreachable. If Exasol sends `""`, the arm is load-bearing for every pre-release schema, and removing it without that measurement would fail every pushdown over those schemas with no unit test noticing. The probe adapter already declares columns without notes, and `probe::measured_pushdown_request` returns `involvedTables`, so the answer is one assertion away.
- Fix: In crates/adbc-vs-adapter/tests/e2e_postgres_test.rs, extend `exasol_sends_the_measured_boolean_and_timestamp_filter_shapes` (line 2640) to read `involvedTables[0].columns` from the request measured for `WHERE b`, find the entry named `b`, and assert what its `adapterNotes` member is. Run it with `make cross-udf-build` and `make test-e2e-postgres`. If the member is absent, assert `entry.get("adapterNotes").is_none()`. Then, in crates/adbc-vs-adapter/src/pushdown_query.rs, delete the empty-string arm of `read_source_type` and change its doc comment to "An absent note gives `None`, because a column declared before columns carried notes renders bare rather than failing every query over it." In crates/adbc-vs-adapter/src/pushdown_query_tests.rs, delete `an_empty_column_note_records_no_source_type` and add `json!("")` to the cases of `malformed_column_note_fails_naming_the_column`. If the member is the empty string, assert `entry["adapterNotes"] == json!("")`, keep the arm and its test, and change the doc comment of `read_source_type` to state that Exasol echoes an empty `adapterNotes` for a column declared without notes, citing `exasol_sends_the_measured_boolean_and_timestamp_filter_shapes`.
