# Decision Log: add-postgresql-test-coverage

## Interview

Headless run. No live interview took place. The orchestrator supplied GitHub issue exasol-labs/adbc-vs-rs#2 and its own discovery notes; every question below was answered by reading the repository rather than by a human, and each answer names what was read.

**Q:** Do the four gaps issue #2 names already exist as spec scenarios, or does this plan need new ones?
**A:** All four exist. `vs-adapter/create-virtual-schema/Source identifiers keep their original case`, `vs-adapter/create-virtual-schema/Views are exposed alongside base tables`, and `scan-udf/arrow-scan/Source failure surfaces the driver's own diagnosis` each read "for each driver this release supports". `vs-adapter/adapter-request-dispatch/Refresh and setProperties re-run metadata discovery` names no driver and requires discovery to run "through the metadata source the request's recorded driver resolves to". No new scenario is needed.

**Q:** Which MySQL test covers each gap, and is the PostgreSQL counterpart genuinely absent?
**A:** Yes, absent in all four cases. `e2e_mysql_test.rs::identifiers_keep_original_case` (line 439), `::views_are_exposed` (line 454), `::refresh_picks_up_new_column` (line 380), and `mysql_it_test.rs::source_failure_reports_adbc_status` (line 374). `e2e_postgres_test.rs` and `postgres_it_test.rs` carry no counterpart to any of them.

**Q:** Is the view gap a product gap or only a test gap?
**A:** Only a test gap. `crates/adbc-vs-adapter/src/postgres_source.rs:188-190` already filters `table_type IN ('BASE TABLE', 'VIEW')`, and `postgres_source_tests.rs:165` asserts that filter. No PostgreSQL view has ever reached a live stack.

**Q:** Does the PostgreSQL fixture hold the objects the three end-to-end tests need?
**A:** No. `docker/postgres/init.sql` seeds `orders`, `items`, `item_notes`, `numeric_specials`, and `all_unmapped`. It holds no view, no mixed-case table, and no table dedicated to a refresh.

**Q:** Can Rust change the PostgreSQL fixture mid-run today?
**A:** No. `crates/adbc-vs-adapter/tests/common/postgres.rs` exposes only `query_scalar`, and its header states the read-only stance is deliberate: the fixture is authored in SQL and the module checks the seed rather than creating it.

**Q:** Would adding fixture objects break an existing PostgreSQL test?
**A:** No. No test asserts an exact table set or an exact table count. `driver_path_is_accepted_and_bypasses_provisioning` compares two PostgreSQL schemas against each other, so both gain the new objects. `postgres_mirror` mirrors only `items` and `item_notes`, both hardcoded. `unmapped_items_columns_are_recorded_and_all_unmapped_table_is_absent` uses `contains` assertions.

**Q:** Does anything outside the tests depend on `docker/postgres/init.sql`?
**A:** Only `docker-compose.yml:60`, which bind-mounts it into `/docker-entrypoint-initdb.d/`. No Makefile prerequisite and no assertion in `make_targets_test.rs` or `workflow_test.rs` references it.

## Design Decisions

### [1] Split the refresh scenario from its `setProperties` half rather than widening or leaving it

- **Decision:** Author one spec delta, `specs/_plans/add-postgresql-test-coverage/vs-adapter/adapter-request-dispatch/spec.md`. It removes `Refresh and setProperties re-run metadata discovery` and replaces it with two scenarios: `Refresh re-runs metadata discovery`, whose `GIVEN` carries "for each driver this release supports", and `setProperties re-runs metadata discovery`, which carries no per-driver phrase. `vs-adapter/create-virtual-schema` and `scan-udf/arrow-scan` stay UNCHANGED.
- **Alternatives:** (a) Add "for each driver this release supports" to the undivided scenario's `GIVEN`, matching its three sibling scenarios — rejected, because the scenario is titled for both request types, so the phrase would oblige a per-driver `setProperties` success test as well; MySQL covers that half in `e2e_mysql_test.rs::all_request_types_return_matching_type`, PostgreSQL does not, and issue #2 does not raise it. (b) Author no delta at all and rely on the scenario as it stands — rejected, because it names no driver, so `e2e_mysql_test.rs::refresh_picks_up_new_column` already satisfies it in full; task 2.2's test would then implement no outstanding obligation, and a later consolidation of `vs-adapter/adapter-request-dispatch` could delete it without violating anything in the library. (c) Keep the undivided scenario and record the unanchored coverage as a residual gap, in the form decision `[5]` uses for the MySQL mixed-case gap — rejected, because documenting the risk leaves the deletion path open, and the split closes it at the cost of one delta file.
- **Rationale:** The two request types carry different obligations, which is why one scenario could not take the per-driver phrase without over-obliging the other. Splitting them makes each obligation match the tests that exist: the refresh half is per-driver and gains PostgreSQL from task 2.2, the `setProperties` half is single-driver and keeps the MySQL test it already had. The split adds no obligation the library did not already carry, and it leaves the feature at eight scenarios, under `/speq:spec-merge`'s >10 signal.
- **Promotes to ADR:** no

### [2] Static fixture shape stays in the Compose seed; only mutation reaches Rust

- **Decision:** Add `refresh_target`, `order_names`, and `"MixedCase"` to `docker/postgres/init.sql`. Add exactly one mutation method, `Postgres::execute`, for the `ALTER TABLE` the refresh scenario requires, and document its boundary in its doc comment.
- **Alternatives:** (a) Create all three objects at run time from a new `Postgres` method, leaving `init.sql` untouched — rejected, because fixture shape would then be decided independently in two places, and a reviewer diffing `postgres_mirror.rs` against the seed would be reading half the fixture. (b) Route the `ALTER` through the existing `query_scalar` — rejected, because it asserts a single-value result, so `psql`'s DDL command tag would pass as a value and a rejected statement could read as success. (c) Keep `Postgres` read-only and seed the grown column in `init.sql` — impossible: the scenario requires a source that "has gained a column since creation".
- **Rationale:** Shape and mutation are different decisions. Keeping shape in one file preserves the module's stated design and extends `SEEDED_TABLES`' staleness check to the new objects at no cost. One narrowly documented method is the smallest interface that satisfies a requirement no static seed can meet.
- **Promotes to ADR:** yes

### [3] Extend `SEEDED_TABLES` before adding the objects, as the red step

- **Decision:** Task 1.1 adds the three names to `SEEDED_TABLES` first, watches `Postgres::check_fixture` fail, then adds the objects to `init.sql`.
- **Alternatives:** Add the objects first and the names afterwards. Rejected — it produces no failing test, and it leaves a window in which a stale container satisfies the check while presenting an older fixture.
- **Rationale:** The seed check is the only executable assertion available for a fixture-only change, so it is the change's red-green cycle. It also gives the three new tests a named failure with a recovery command instead of a mystery.
- **Promotes to ADR:** no

### [4] Prove mixed case with a quoted `"MixedCase"` table rather than mirroring MySQL's lower-case-only test

- **Decision:** Seed `"MixedCase" ("ItemId", "Label")` with every identifier quoted, and assert both discovery and rendering keep the case.
- **Alternatives:** Mirror `e2e_mysql_test.rs::identifiers_keep_original_case` exactly, which asserts only that a lower-case name stays lower case. Rejected for PostgreSQL: PostgreSQL folds unquoted identifiers to lower case itself, so a lower-case-only assertion cannot distinguish case preservation from case folding — it would pass against an adapter that upper-folds nothing and preserves nothing.
- **Rationale:** The scenario's third `THEN` clause — "a mixed-case source table name MUST be returned with its case intact" — has never been proven end to end for any driver. PostgreSQL is where the risk is real, and issue #2 asks for "discovery and rendering", which the `EXPLAIN VIRTUAL` half covers.
- **Promotes to ADR:** no

### [5] Leave the mixed-case gap open on the MySQL side

- **Decision:** Add no mixed-case fixture or assertion to the MySQL tier.
- **Alternatives:** Close the same clause for MySQL in this plan. Rejected as out of scope.
- **Rationale:** Issue #2 lists four PostgreSQL gaps. Widening the plan to a MySQL fixture change would put the MySQL end-to-end tier at risk for a claim nobody asked for. Recorded here so the residual gap is on the record rather than forgotten: after this plan, `vs-adapter/create-virtual-schema`'s mixed-case clause is proven for PostgreSQL and unproven for MySQL.
- **Promotes to ADR:** no

### [6] Assert the refreshed column's declared Exasol type, not just its name

- **Decision:** `refresh_picks_up_a_new_postgresql_column` adds `extra NUMERIC(9,3)` and asserts `declared_columns` returns `("extra", "DECIMAL(9,3)")`.
- **Alternatives:** Assert the column name alone, as the MySQL test does with `column_names`. Rejected — a name says nothing about which metadata source produced it.
- **Rationale:** The scenario requires that "the discovery MUST run through the metadata source the request's recorded driver resolves to". A `numeric(9,3)` rendered as `DECIMAL(9,3)` is PostgreSQL's own mapping running, which is the cheapest available evidence for that clause.
- **Promotes to ADR:** no

### [7] Two things decide parallelism here: the file a task edits, and the Compose stack it drives

- **Decision:** Task 1.1 runs alone. Tasks 2.1, 2.2, and 2.3 run in order as one group. Task 2.4 runs after 1.1, concurrently with that group.
- **Alternatives:** (a) Run 2.1, 2.2, and 2.3 concurrently — rejected, because all three edit `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs`, and concurrent agents editing one file conflict. (b) Run 2.4 beside 1.1, which file disjointness alone appears to permit — rejected, because task 1.1's own steps run `docker compose rm -sfv postgres && make stack-up-postgres`, destroying the container 2.4's tier queries at `postgresql://127.0.0.1:5432/adbc_vs`, and then re-run `make test-it-postgres`, the binary 2.4 is editing. A mid-scan recreate surfaces as an ADBC connection error, which is the error family 2.4 asserts on, so the collision would read as the behavior under test and could be "fixed" by weakening the assertion.
- **Rationale:** File disjointness is necessary but not sufficient. Every task here drives one shared Compose stack, so a task that destroys or reseeds a container excludes every task that queries it, whatever files each edits. Only task 1.1 is destructive, so it is the only task that must run alone. Groups B and B' may overlap because 2.4's tier writes nothing to the fixture database and 2.2's `ALTER TABLE` touches only `refresh_target`, which that tier never reads; their shared `target/` lock serializes their builds without failing either.
- **Promotes to ADR:** no

### [8] Tag no task `[expert]`

- **Decision:** Every task runs on the standard implementer.
- **Alternatives:** Tag 2.2 `[expert]` for its shared-fixture ordering hazard — a panic between the `ADD COLUMN` and the `DROP COLUMN` poisons the virtual schema every later test in the binary reads. Rejected.
- **Rationale:** The hazard is real but the solution is already written down and commented in `e2e_mysql_test.rs::refresh_picks_up_new_column`: read all three column sets, then assert. Mirroring a documented pattern is explicitly not an expert task, and the constraint is stated in the task text where the implementer will read it.
- **Promotes to ADR:** no

### [9] Publish `Postgres` on `postgres_stack::Stack` rather than construct it per test

- **Decision:** Task 2.2 adds `pub postgres: Postgres` to `postgres_stack::Stack`, binds it in `bring_up()` as `let postgres` instead of `let _postgres`, returns it in the `Stack` literal, and rewrites `bring_up`'s doc comment, which currently justifies dropping both constructed values. `MySql` stays bound as `let _mysql`.
- **Alternatives:** Call `Postgres::from_stack()` inside `refresh_picks_up_a_new_postgresql_column`, leaving `Stack` untouched. Rejected — `from_stack` runs `await_queries` and `check_fixture`, so every call repeats the readiness wait and the seed check, and the container would end up with two construction sites that can disagree about whether the fixture was verified.
- **Rationale:** `common/stack.rs` already publishes `pub mysql: MySql` for exactly this reason, and `e2e_mysql_test.rs` reaches `stack.mysql.execute(...)` through it. The field publishes a value `bring_up` constructs either way, so it owns no new decision. Without it, `Postgres::execute` would be a method no test could reach and the implementer would have to invent the wiring.
- **Promotes to ADR:** no

## Review Findings

### [plan-review] Task 2.2 named no route from a test to a `Postgres` value

- **Finding:** `plan-reviewer` round 1, `[HIDDEN_DEPENDENCY]` BLOCKER. `postgres_stack::Stack` holds only `pub exasol`, and `bring_up()` drops the constructed `Postgres` as `let _postgres` under a doc comment justifying the drop. Task 2.2 named only `e2e_postgres_test.rs` and `common/postgres.rs`, so `Postgres::execute` would have had no caller and the implementer would have improvised the wiring. The Design section's claim that `Postgres::execute` is the only new interface contradicted either route.
- **Direction change:** Task 2.2 now names `crates/adbc-vs-adapter/tests/common/postgres_stack.rs` and states the route: add `pub postgres: Postgres` to `Stack`, bind it in `bring_up()`, rewrite that function's doc comment, and construct no second `Postgres::from_stack()` in the test. Decision `[9]` records the route and the rejected alternative. The Consequences table gained a row for it, and its closing paragraph now names both new interfaces.
- **Promotes to ADR:** no

### [plan-review] Group A paired task 2.4 with the task that recreates its container

- **Finding:** `plan-reviewer` round 1, `[TASK_GRANULARITY]` BLOCKER. The independence claim for Group A rested on file disjointness alone. Task 1.1 runs `docker compose rm -sfv postgres && make stack-up-postgres`, destroying the container task 2.4's tier queries, and re-runs `make test-it-postgres`, the binary 2.4 edits. The resulting mid-scan failure is an ADBC connection error — the error family 2.4 asserts on — so the collision would read as the behavior under test.
- **Direction change:** Task 1.1 now runs alone in Group A. Task 2.4 moved to its own group, `Group B'`, ordered after Group A and permitted to overlap Group B, with the reason stated: no task driving the PostgreSQL stack may overlap 1.1. Decision `[7]` was rewritten to record that the shared Compose stack, not only file disjointness, decides what may run in parallel.
- **Promotes to ADR:** no

### [plan-review] Task 2.2's coverage was anchored to no spec obligation

- **Finding:** `plan-reviewer` round 1, `[TRACEABILITY_GAP]` BLOCKER. § Features claimed "PostgreSQL coverage was therefore always required; only the tests were missing", and that claim was the sole justification for shipping zero spec deltas. It is false for the refresh gap: `Refresh and setProperties re-run metadata discovery` names no driver, unlike its three siblings, so `e2e_mysql_test.rs::refresh_picks_up_new_column` already satisfies it in full. Task 2.2's test would have implemented no outstanding obligation, and a later consolidation could have deleted it without violating anything. Decision `[1]`'s alternatives also presented a false dichotomy — widen the whole `GIVEN`, or do nothing — omitting the split.
- **Direction change:** Took the reviewer's option (a). Authored `specs/_plans/add-postgresql-test-coverage/vs-adapter/adapter-request-dispatch/spec.md`, splitting the scenario into `Refresh re-runs metadata discovery` (carrying "for each driver this release supports") and `setProperties re-runs metadata discovery` (without it). The Features table row for `vs-adapter/adapter-request-dispatch` changed from UNCHANGED to CHANGED and points at the delta; the false sentence is gone; task 2.2 names the scenario it satisfies; the Scenario Coverage table gained rows for both split scenarios' existing MySQL tests. Decision `[1]` was rewritten with all three options, including the residual-gap route, on the record.
- **Promotes to ADR:** no
