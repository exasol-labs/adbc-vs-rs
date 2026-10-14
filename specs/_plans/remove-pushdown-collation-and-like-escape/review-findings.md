# Code Review Findings: remove-pushdown-collation-and-like-escape

## Summary
- Files reviewed: 34
- Total findings: 8 (standard: 8, expert: 0)
- Evidence: `cargo clippy --workspace --all-targets --all-features -- -D warnings` exits 0, and `cargo fmt --all -- --check` exits 0. Neither tool checks string-literal content or comment wrapping, which is where findings 1 and 2 sit.
- Not fix tasks, for the orchestrator or the user:
  - `tasks.md` task 4.2 records "user's rewrite was discarded" for `README.md`, while `plan.md` task 4.2 required keeping the user's uncommitted edits. The working-tree `README.md` is the committed 63 lines plus the new Limits section. Confirm with the user that the discard was intended.
  - Unverified inference: a delegated comparison between two character columns that declare different collations now reaches the source bare. MySQL rejects `=` between two implicit, different, non-`_bin` collations with error 1267, and PostgreSQL rejects an ordering comparison between two different implicit collations. The removed `COLLATE` suffix used to resolve both. `plan.md` § Impact does not list this, and no test covers it.

## Standard fixes

### crates/adbc-vs-adapter/src/trino_dialect.rs

#### [CONTEXTLESS_ERROR] LIKE escape refusal carries runs of 18 spaces and names no escape character
- Location: lines 420-424, `Statement::render_like`
- Issue: the error literal lost its `\` line continuations, so the message an operator reads contains three runs of 18 spaces ("does not                  advertise"). The message also does not name the escape character the request carried, which is the input that failed. `a_like_carrying_an_escape_char_fails_naming_the_capability` checks only `contains("FN_PRED_LIKE_ESCAPE")`, so it passes over the malformed text.
- Fix: In crates/adbc-vs-adapter/src/trino_dialect.rs, replace the `if like.escape_char.is_some() { ... }` block in `Statement::render_like` with `if let Some(escape_char) = &like.escape_char { return Err(RenderError::new(format!(...))); }`, where the format string reads "a `predicate_like` node carries the `escapeChar` {}, and this dialect does not advertise `FN_PRED_LIKE_ESCAPE`: Trino defines no default escape character and rejects an escape character followed by anything other than `%`, `_`, or itself, so Exasol evaluates a `LIKE` with an `ESCAPE` clause itself", split across source lines that each end in `\` as the file's other multi-line `RenderError` messages are, with `self.render_expression(escape_char)?` as the argument. In crates/adbc-vs-adapter/src/trino_dialect_tests.rs, extend `a_like_carrying_an_escape_char_fails_naming_the_capability` with `assert!(error.contains("'!'"), "{error}")` and `assert!(!error.contains("  "), "{error}")`.

#### [REDUNDANT_COMMENT] Module doc states the absent suffix position twice
- Location: lines 20-31, module doc comment
- Issue: "it has no suffix position to render into at all" (lines 21-22) and "A collation could not be rendered here even if a rule wanted one" (lines 29-30) state the same fact in one paragraph. The edit also left lines 22 and 30 well past the roughly 78-column width the rest of the module doc uses.
- Fix: In crates/adbc-vs-adapter/src/trino_dialect.rs, delete the sentence "A collation could not be rendered here even if a rule wanted one." from the module doc, then rewrap the paragraph spanning lines 20-39 to the width the surrounding module doc uses, changing no other wording.

### crates/adbc-vs-adapter/src/mysql_dialect.rs

#### [SHALLOW_MODULE] `Statement` is a one-field wrapper around `TableScope`
- Location: lines 210-249 (`enum TableScope`, `struct Statement`), line 120, lines 253-258 (`render_from`), lines 487-498 (`render_column`)
- Issue: removing `Statement.grouped` left `struct Statement { tables: TableScope }`, whose `of` only calls `TableScope::of(&query.from)`. Every reader goes through `self.tables` to reach the one enum. `postgres_dialect.rs` and `trino_dialect.rs` declare the same concept as a single `enum Statement { OneTable, Joined }`.
- Fix: In crates/adbc-vs-adapter/src/mysql_dialect.rs, delete `struct Statement` and `enum TableScope` with its `impl`, and declare `enum Statement<'a> { OneTable(&'a SourceTable), Joined(AliasedFrom<'a>) }` in their place, keeping the existing `Statement` doc comment and the two variant doc comments from `TableScope`. Give `impl<'a> Statement<'a>` the method `fn of(from: &'a FromClause) -> Self` with `TableScope::of`'s body. In `MySqlDialect::render`, call `Statement::of(&query.from)`. In `render_from`, change `match &self.tables` to `match self` and the arms to `Statement::OneTable(table)` and `Statement::Joined(from)`. In `render_column`, change `let TableScope::Joined(from) = &self.tables else` to `let Statement::Joined(from) = self else`. Run `cargo test -p adbc-vs-adapter --lib mysql_dialect` and `cargo test -p adbc-vs-adapter --lib capabilities`.

### crates/adbc-vs-adapter/src/mysql_dialect_tests.rs

#### [DUPLICATE_TEST] The numeric grouping-key predicate test repeats the character one
- Location: lines 823-844, `a_select_list_predicate_over_a_numeric_grouping_key_renders`
- Issue: the dialect no longer reads any operand's declared type, so a numeric and a character grouping key take the same rendering path. The test existed as the control for the removed character-key error. Its own doc comment now points to `a_select_list_predicate_reading_the_grouped_character_column_renders` as covering the same shape. The change already deleted `numeric_grouping_key_carries_no_collation` for this reason.
- Fix: In crates/adbc-vs-adapter/src/mysql_dialect_tests.rs, delete the test `a_select_list_predicate_over_a_numeric_grouping_key_renders` together with its doc comment.

### crates/adbc-vs-adapter/tests/e2e_postgres_test.rs

#### [SHALLOW_MODULE] Three mirror helpers where the MySQL file has one
- Location: lines 1341-1386 (`assert_matches_mirror`, `assert_items_c_matches_mirror`, `assert_table_matches_mirror`); call sites at lines 161, 196, 217, 239, 277, 307, 321, 345, 378, 517, 533, 560, 596, 628, 671, 717, 754
- Issue: `assert_items_c_matches_mirror` is a pass-through that fixes one argument, and `assert_matches_mirror` adapts a one-argument closure to the same function. `an_ordering_comparison_over_a_wrapped_operand_is_evaluated_by_exasol` calls the third helper directly. `plan.md` task 2.5 required this file's helper to change the same way task 1.5 changed `e2e_mysql_test.rs::assert_matches_mirror`, which is one function taking the virtual table and a two-argument closure.
- Fix: In crates/adbc-vs-adapter/tests/e2e_postgres_test.rs, rename `assert_table_matches_mirror` to `assert_matches_mirror`, keeping its signature `(virtual_table: &str, query: impl Fn(&str, &str) -> String)` and its body, and merge the doc comment of the old `assert_matches_mirror` into it. Delete the old one-argument `assert_matches_mirror` and `assert_items_c_matches_mirror`. Change every call `assert_items_c_matches_mirror(q)` to `assert_matches_mirror(ITEMS_C, q)` and every call `assert_table_matches_mirror(ITEMS, q)` to `assert_matches_mirror(ITEMS, q)`. For every remaining call that passed a one-argument closure, change the closure to `|schema: &str, table: &str|`, replace `{schema}.\"items\"` inside it with `{schema}.\"{table}\"`, pass `ITEMS` as the first argument, and change each `query(VIRTUAL_SCHEMA)`-style call of that closure in the same test to pass `ITEMS` as the second argument.

### crates/adbc-vs-adapter/tests/common/mysql.rs

#### [IMPLEMENTATION_COUPLED_TEST] Fixture-row test restates the format string it checks
- Location: lines 440-455, `fixtures_seed_the_code_point_items_with_the_items_rows`
- Issue: `fixture_script` builds both `items` and `items_bin` through `items_table`, which formats one `INSERT ... VALUES {}` string from `items::source_values()`. The test rebuilds that same string from the same function and searches for it, so it is true by construction. It fails only when the SQL text changes, whatever the rows.
- Fix: In crates/adbc-vs-adapter/tests/common/mysql.rs, delete the test `fixtures_seed_the_code_point_items_with_the_items_rows`. Keep `create_statement_of`, which the two remaining fixture tests use.

### crates/adbc-vs-adapter/src/metadata.rs

#### [OUTDATED_COMMENT] `check_source` contract says the dialect states the precondition and lists every implementation
- Location: lines 149-154, `MetadataSource::check_source` doc comment
- Issue: the doc says the precondition "is stated by the dialect that renders for the source rather than by the source itself". MySQL's remaining check, the database-name guard, exists for `mysql_source`'s own catalog queries, not for the dialect. The doc also enumerates each implementation's precondition, so a change to any source's check edits this trait. This change had to edit it for that reason. `metadata_source_tests.rs` lines 93-94 and 116 repeat the "its own dialect" claim.
- Fix: In crates/adbc-vs-adapter/src/metadata.rs, replace the `check_source` doc comment with: "Refuses a source this release cannot discover from or render for." followed by a blank doc line and "Each implementation states and checks its own precondition and runs no other source's. An implementation whose discovery and rendering depend on nothing the server configures succeeds without a query." In crates/adbc-vs-adapter/src/metadata_source_tests.rs, change "has to state the precondition its own dialect needs rather than inherit one" (lines 93-94) to "has to state its own precondition rather than inherit one", and change "Each source checks the precondition its own dialect states, and reads no" (line 116) to "Each source checks its own precondition, and reads no".

### crates/adbc-vs-adapter/src/trino_source.rs

#### [INFORMATION_LEAKAGE] Trino's `check_source` doc restates PostgreSQL's precondition
- Location: lines 94-104, `TrinoSource::check_source` doc comment
- Issue: the doc opens by describing PostgreSQL's encoding check. That decision belongs to `postgres_source.rs`. This change had to rewrite the sentence because it previously described MySQL's check too, and a change to PostgreSQL's check would require editing it again.
- Fix: In crates/adbc-vs-adapter/src/trino_source.rs, delete the sentence that begins "PostgreSQL's check exists because its dialect's rendering depends on" and ends "before trusting it." from the `check_source` doc comment, and keep the remaining sentences from "Nothing Trino's rendering depends on is configurable" through "with no driver excepted." unchanged.

## Expert fixes
[none]
