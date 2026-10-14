# Tasks: remove-pushdown-collation-and-like-escape

## PR Lifecycle
- [x] resolved
- [x] implemented
- [x] version-bumped
- [ ] tested-green
- [ ] recorded
- [ ] pr-ready

## Phase 2: Implementation (Group A: MySQL collation)
- [x] 1.1 Remove collation rendering from mysql_dialect.rs [expert]
- [x] 1.2 Rewrite mysql_dialect_tests.rs
- [x] 1.3 Remove the MySQL server check from mysql_source.rs and restate dependent docs and tests
- [x] 1.4 MySQL E2E fixture: items_bin
- [x] 1.5 MySQL E2E tests in tests/e2e_mysql_test.rs

## Phase 2: Implementation (Group B: PostgreSQL collation)
- [x] 2.1 Remove COLLATE "C" from postgres_dialect.rs
- [x] 2.2 Rewrite postgres_dialect_tests.rs
- [x] 2.3 Remove the nondeterministic-collation guard and the collation read
- [x] 2.4 PostgreSQL E2E fixture: items_c
- [x] 2.5 PostgreSQL E2E tests in tests/e2e_postgres_test.rs

## Phase 2: Implementation (Group C: Trino LIKE escape)
- [x] 3.1 Remove FN_PRED_LIKE_ESCAPE from drivers/trino.toml
- [x] 3.2 Render plain LIKE in trino_dialect.rs
- [x] 3.3 Rewrite trino_dialect_tests.rs
- [x] 3.4 Update capabilities_tests.rs
- [x] 3.5 Trino E2E test changes

## Phase 2: Implementation (Group D: Framework prose and docs, after A, B, C)
- [x] 4.1 Restate doc comments that give collation as a purpose
- [x] 4.2 README.md remedies and Limits (committed README; user's rewrite was discarded)
- [x] 4.3 specs/mission.md Core Capabilities item 3
- [x] 4.4 CLAUDE.md bullets
- [x] 4.5 Restate remaining collation comments and sweep with rg

## Phase 4: Review Fixes
- [x] 4.1 trino_dialect.rs: render_like names the escape character and uses `\` line continuations in its RenderError message; extend `a_like_carrying_an_escape_char_fails_naming_the_capability` to assert `'!'` and no double space
- [x] 4.2 trino_dialect.rs: delete the redundant "A collation could not be rendered here even if a rule wanted one." sentence from the module doc and rewrap the paragraph
- [x] 4.3 mysql_dialect.rs: replace `struct Statement` + `enum TableScope` with a single `enum Statement<'a> { OneTable, Joined }`
- [x] 4.4 mysql_dialect_tests.rs: delete `a_select_list_predicate_over_a_numeric_grouping_key_renders`
- [x] 4.5 e2e_postgres_test.rs: collapse the three mirror helpers into one `assert_matches_mirror(virtual_table, query)` and update every call site
- [x] 4.6 tests/common/mysql.rs: delete `fixtures_seed_the_code_point_items_with_the_items_rows`
- [x] 4.7 metadata.rs and metadata_source_tests.rs: replace the `check_source` doc with the generic contract and drop the "its own dialect" wording in the tests
- [x] 4.8 trino_source.rs: delete the PostgreSQL-precondition sentence from the `check_source` doc

## Phase 3: Verification (Group E, sequential)
- [ ] 5.1 make fmt-check, lint, test, deny
- [ ] 5.2 make cross-udf-build, make test-artifact
- [ ] 5.3 docker compose rm -sfv postgres, then make test-e2e, test-e2e-postgres, test-e2e-trino in sequence
