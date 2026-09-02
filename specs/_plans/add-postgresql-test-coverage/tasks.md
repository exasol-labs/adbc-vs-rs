# Tasks: add-postgresql-test-coverage

## Group A
- [x] 1.1 Extend the PostgreSQL Compose seed with the three fixture objects (refresh_target, order_names, "MixedCase"), guarded by SEEDED_TABLES staleness check in crates/adbc-vs-adapter/tests/common/postgres.rs and docker/postgres/init.sql.

## Group B (sequential, one file: crates/adbc-vs-adapter/tests/e2e_postgres_test.rs)
- [x] 2.1 Add identifiers_keep_their_case_through_discovery_and_rendering
- [x] 2.2 Add Postgres::execute + Stack.postgres field + refresh_picks_up_a_new_postgresql_column
- [x] 2.3 Add views_are_exposed_alongside_base_tables

## Group B' (concurrent with Group B)
- [x] 2.4 Add source_failure_reports_adbc_status to crates/adbc-vs-scan/tests/postgres_it_test.rs

## Group C
- [x] 3.1 Update CLAUDE.md coverage bullets for test-it-postgres and test-e2e-postgres

## Phase 3: Verification
- [x] 3.2 Run full checklist (stack recreate, build, fmt, lint, test, test-it, test-it-postgres, test-e2e-postgres, test-e2e, deny) — all 10 steps green; manual verification steps also confirmed against the live stack. See verification-report.md.

## Phase 4: Review Fixes
- [x] 4.1 In crates/adbc-vs-adapter/tests/e2e_postgres_test.rs::views_are_exposed_alongside_base_tables, require the `tables` listing to contain both `order_names` and the base table `orders` (message naming the scenario's "both the base table and the view" obligation), assert `column_names(VIRTUAL_SCHEMA, "order_names")` equals `["id", "name"]`, and assert `column_values(&rows, "name")` equals `["Widget", "Gadget", "Gizmo"]` beside the existing `id` comparison — every expectation a literal traceable to docker/postgres/init.sql's `INSERT INTO orders`. [expert]
- [x] 4.2 In crates/adbc-vs-adapter/tests/e2e_postgres_test.rs::refresh_picks_up_a_new_postgresql_column, read `before` and `after_drop` through `declared_columns(VIRTUAL_SCHEMA, "refresh_target")` instead of `column_names` so all three reads are `Vec<(String, String)>`, change the `before` guard to `!before.iter().any(|(name, _)| name == "extra")` keeping its message, leave the `after_add` and final `assert_eq!(after_drop, before)` assertions as they are, and preserve the compute-then-assert ordering exactly. [expert]
- [x] 4.3 In docker/postgres/init.sql line 160, change the block header's first sentence from `-- End-to-end fixture, task 1.1 (add-postgresql-test-coverage). Each object` to `-- End-to-end fixture for the PostgreSQL tier. Each object`, leaving the rest of that comment paragraph and the pre-existing "task 8.1"/"task 7.2"/"task 7.1"/"task 8.2"/"task 8.4" references above line 160 unchanged.
- [x] 4.4 In crates/adbc-vs-adapter/tests/common/postgres_mirror.rs, extend the module doc paragraph beginning "`numeric_specials` and `all_unmapped` carry no mirror at all" to also account for `refresh_target`, `order_names`, and `"MixedCase"`, explaining that none of the three is read differentially — `refresh_target` is read for its declared column set before/after a `REFRESH`, `order_names` for its presence in the exposed table list, and `"MixedCase"` for identifier case — so a mirror column for any of them would be compared against nothing.
- [x] 4.5 In crates/adbc-vs-adapter/tests/common/postgres.rs, rename the constant `SEEDED_TABLES` to `SEEDED_OBJECTS` at all five sites (declaration, the `.iter()` call, and the three uses inside `check_fixture`'s `assert_eq!`), reword its doc comment to say "The objects `docker/postgres/init.sql` seeds beyond the `orders` table the integration tier reads" plus a sentence noting `order_names` is a view and `information_schema.tables` lists views alongside base tables, change `check_fixture`'s panic message from "of the {} seeded tables" to "of the {} seeded fixture objects", and update CLAUDE.md's "Test tiers" bullet to name `SEEDED_OBJECTS` instead of `SEEDED_TABLES`.
- [x] 4.6 In crates/adbc-vs-adapter/tests/common/postgres_stack.rs, add a doc comment to the `pub postgres: Postgres` field of `Stack` stating what it publishes (the handle a test uses to change the PostgreSQL source mid-run, kept on `Stack` rather than reconstructed per call because `Postgres::from_stack` re-runs the readiness wait and the seed check, mirroring `pub mysql: MySql`), then cut `bring_up`'s doc comment back to the `MySql` half only, removing the sentences beginning "`Postgres` is different:" and the reference to `refresh_picks_up_a_new_postgresql_column` by name.
