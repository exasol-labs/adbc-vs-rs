# Plan Review Findings: add-postgresql-test-coverage (round 2)

## Summary
- Axes checked: 6/6
- Total findings: 9 (Blockers: 0, Advisory: 9)
- Intent Fidelity blockers: 0

All three round-1 BLOCKERs are resolved against the real artifacts and the real code, not merely claimed. Three new ADVISORY findings arise from the fixes themselves. Six round-1 ADVISORY findings remain unaddressed and are carried forward.

## Round-1 Blocker Recheck

- **Resolved:** [HIDDEN_DEPENDENCY] *Task 2.2 named no route from a test to a `Postgres` value* — plan.md task 2.2 now names `crates/adbc-vs-adapter/tests/common/postgres_stack.rs` as a third file and states the exact route. Verified against the code the route touches: `postgres_stack.rs:50-52` is `pub struct Stack { pub exasol: Exasol }`, `bring_up()` binds `let _postgres = Postgres::from_stack();` under the doc comment ending "neither constructed value outlives this function", and returns `Stack { exasol }` — all three edits the task names are real and necessary. The precedent it cites is real: `common/stack.rs:31-34` is `pub struct Stack { pub exasol: Exasol, pub mysql: MySql }`. `Postgres` is `struct Postgres { container: String }` (`common/postgres.rs:52-54`), so it is `Send + Sync` and the `static OnceLock<Stack>` still compiles with the new field. Decision `[9]` records the route and the rejected per-test `from_stack()`; the Consequences table gained a row for it; the closing Design paragraph now names both new interfaces instead of claiming `Postgres::execute` is the only one. Every helper the task depends on exists: `declared_columns` at `e2e_postgres_test.rs:1194`, `column_names(schema, table)` at `:1082`, `table_names` at `:1069`, `generated_statement` at `:1117`.
- **Resolved:** [TASK_GRANULARITY] *Group A paired task 2.4 with the task that recreates its container* — § Parallelization now reads `Group A | 1.1 — alone` and `Group B' | 2.4 — concurrent with Group B`, with `Group A → Group B'` in Sequential dependencies and the reason stated in the plan's own words ("No task driving the PostgreSQL stack may overlap task 1.1"). Decision `[7]` was rewritten from a file-disjointness argument to a two-criteria one (file, plus the Compose stack the task drives) and records the rejected "run 2.4 beside 1.1" option. The remaining Group B ∥ Group B' overlap claim checks out: `postgres_it_test.rs:140` projects its 300,000-row result from `generate_series` and writes nothing, and no test in `postgres_it_test.rs` reads `refresh_target`.
- **Resolved:** [TRACEABILITY_GAP] *Task 2.2's coverage was anchored to no spec obligation* — the delta `specs/_plans/add-postgresql-test-coverage/vs-adapter/adapter-request-dispatch/spec.md` exists and is sound. Its `DELTA:REMOVED` block reproduces `specs/vs-adapter/adapter-request-dispatch/spec.md:36-43` verbatim, and its header, description line, and all six Background bullets match the recorded file byte-for-byte, so the merge drops nothing. The `refresh` half carries "for each driver this release supports" in its `GIVEN`, matching how its three sibling scenarios in other features phrase it, which is what anchors task 2.2. The split is faithful to the product: `create_schema.rs:338` documents `echo_requested_tables` as "Puts a `refresh` request's own `requestedTables` back on its response" and `set_properties` (`:139-143`) never calls it, so keeping that clause only in the refresh half removes no obligation. The `setProperties` half is genuinely covered, not nominally: `e2e_mysql_test.rs:337-371` creates `DISPATCHVS` over the seeded database, runs `SET CONNECTION_NAME = 'ADBC_VS_DISPATCH_EMPTY'`, and asserts the re-discovered table set is empty — which satisfies all three of its steps. The Features table row is `CHANGED` and points at the delta; the false "PostgreSQL coverage was therefore always required" sentence is gone; the Scenario Coverage table gained rows for both split scenarios. Feature scenario count goes 7 → 8, under `/speq:spec-merge`'s >10 signal as decision `[1]` claims. `speq plan validate add-postgresql-test-coverage` and `speq feature validate` both pass, and no code file, ADR, or `specs/mission.md` line references the removed scenario by name, so the rename orphans nothing.

## Intent Fidelity

[no objection — axis checked: the four gaps issue #2 names still map one-to-one to tasks 2.1, 2.2, 2.3, 2.4; the blocker-3 fix added a spec delta rather than new implementation work, so no scope grew; the delta's `setProperties` half deliberately carries no per-driver phrase, which keeps the plan from adding the `setProperties` test issue #2 does not raise; decision `[5]` still records the MySQL mixed-case gap as deliberately left open]

## Feasibility

#### [HIDDEN_DEPENDENCY] ADVISORY
- Location: plan.md § Parallelization, Sequential dependencies list; plan.md § Implementation Tasks, task 3.1 first bullet
- Issue: the blocker-2 fix broke an edge it did not replace. Task 3.1's first bullet is "Extend the `make test-it-postgres` \"Covers:\" bullet with the rejected-statement ADBC status" — that is task 2.4's test. In round 1, 2.4 sat in Group A, which precedes every other group, so Group C followed it implicitly. Now 2.4 sits in Group B', parallel with Group B, and Sequential dependencies lists only "Group B → Group C — 3.1 names the tests the earlier tasks add." The stated reason for that edge applies to 2.4 verbatim, so the edge is missing rather than deliberately absent. Nothing catches the consequence: `crates/adbc-vs-adapter/tests/make_targets_test.rs` and `tests/workflow_test.rs` assert against the `Makefile` and `ci.yml`, never against `CLAUDE.md`, so a `CLAUDE.md` line naming a test 2.4 renamed or never added would ship green.
- Fix: In plan.md § Parallelization, add "Group B' → Group C — 3.1's first bullet documents task 2.4's test" to the Sequential dependencies list, and extend decision `[7]`'s Decision line so it states that Group C follows both Group B and Group B'.

## Requirement Quality

#### [COMPLETENESS_GAP] ADVISORY
- Location: plan.md § Implementation Tasks, task 1.1 (`SEEDED_TABLES` step) and task 2.2 (`Postgres::execute` step)
- Issue: two doc comments become false and neither task says to fix them, while task 2.2 requires exactly that repair for a third. `common/postgres.rs:40-42` documents the constant as "The tables `docker/postgres/init.sql` seeds beyond the `orders` table the integration tier reads" — after task 1.1 the constant also holds `order_names`, which is a `VIEW`, not a table. `common/postgres.rs:8-12`, the module header, states the module "creates nothing and instead *checks* that the seed a run depends on actually ran" — after task 2.2 the module also mutates the source. Task 2.2 already requires rewriting `bring_up`'s doc comment for precisely this class of drift ("it currently justifies dropping both values with \"neither constructed value outlives this function\", which stops being true"), so the standard being missed is the plan's own.
- Fix: Add a bullet to plan.md task 1.1 requiring `SEEDED_TABLES`' doc comment to be rewritten to cover a view and a mutated table rather than "tables ... beyond the `orders` table". Add a bullet to task 2.2 requiring `common/postgres.rs`'s module header to be rewritten so its "creates nothing" claim states the one mutation `Postgres::execute` adds and why the read-only stance still holds for shape.

#### [COMPLETENESS_GAP] ADVISORY (carried forward from round 1, unaddressed)
- Location: plan.md § Impact; plan.md § Implementation Tasks, task 3.1 third bullet
- Issue: both places still name `docker compose rm -sfv postgres` as the recovery without recording the coupling that makes the trap easy to hit. `Makefile:179-181` defines `stack-down` as `$(COMPOSE) down` with no `--volumes`, and `docker-compose.yml`'s `postgres` service declares no named data volume, so `make stack-down && make stack-up` preserves the old seed. That is the path a reader will assume is safe.
- Fix: In plan.md § Impact and in task 3.1's third bullet, state that `make stack-down` (`docker compose down`, no `--volumes`) preserves the `postgres` anonymous volume, so a down-then-up cycle does not re-seed and only `docker compose rm -sfv postgres` does.

## Task Breakdown

[no objection — axis checked: every scenario in the Verification table has an implementing task and every task implements a named scenario or a documentation obligation; the Group B serialization of 2.1/2.2/2.3 is justified by their shared file `e2e_postgres_test.rs`; the Group B ∥ Group B' claim holds against the code (`postgres_it_test.rs` writes nothing and reads no object task 2.2 mutates); the only defect found on this axis is the missing Group B' → Group C edge, raised under Feasibility]

## Design Depth

[no objection — axis checked: `Postgres::execute` owns one decision (how a statement reaches `psql` in the container) and reuses `client_arguments`, so the credential-handling decision keeps its single owner at `common/postgres.rs:155`, and `--variable=ON_ERROR_STOP=1` already there is what makes the task's "panics on a rejected statement" requirement achievable; the `pub postgres: Postgres` field publishes a value `bring_up` constructs either way and replicates `common/stack.rs:31-34` exactly, so it introduces no new decision and no new dependency direction; the resulting field-set asymmetry between the two twin `Stack` structs — `{exasol, mysql}` versus `{exasol, postgres}` — is need-driven, since no PostgreSQL test mutates the MySQL source; no product module changes, so no boundary moves]

#### [TACTICAL_SHORTCUT] ADVISORY (carried forward from round 1, unaddressed)
- Location: decision-log `[2]`, Alternatives (a); plan.md § Patterns, "Single owner for fixture shape"; plan.md § Consequences, first row
- Issue: the rationale rejecting the Rust-created route is still "a reviewer diffing `postgres_mirror.rs` against the seed would be reading half the fixture", and task 1.1 still says "Add no `postgres_mirror` counterpart: none of the three objects is read differentially" — so the stated risk cannot materialize for any of the three new objects. Meanwhile `common/mysql.rs:76-97` already creates `order_names` and `refresh_target` from Rust for the MySQL tier, which is the precedent the decision does not address, and the chosen route imposes a destructive container recreate on every developer holding a stack.
- Fix: In decision-log `[2]`, replace the mirror-diff rationale with one that survives task 1.1's "no mirror counterpart" bullet, and state why `common/mysql.rs`'s Rust-created `order_names` and `refresh_target` are not the precedent to follow. If no such reason holds, adopt the MySQL route for `refresh_target` and `order_names` and drop the container-recreate obligation from plan.md § Impact and task 3.1 accordingly.

#### [UNSTATED_ASSUMPTION] ADVISORY (carried forward from round 1, unaddressed)
- Location: plan.md § Design → Non-Goals ("no product-code change"); plan.md § Impact; decision-log `[4]`
- Issue: decision `[4]` still states the mixed-case clause "has never been proven end to end for any driver" while task 2.1 asserts it, and the Non-Goals still forbid the only remedy if it fails. No contingency is stated.
- Fix: Add a bullet to plan.md § Design → Non-Goals stating that a failing mixed-case assertion in task 2.1 is a product defect to escalate as a separate plan, never an assertion to weaken, and name `crates/adbc-vs-adapter/src/postgres_dialect_tests.rs::every_identifier_is_quoted_whatever_its_case` in task 2.1 as the unit-level evidence that makes a failure surprising.

#### [UNSTATED_ASSUMPTION] ADVISORY (carried forward from round 1, unaddressed)
- Location: plan.md § Implementation Tasks, tasks 2.1 through 2.4
- Issue: only task 1.1 carries a red step. Tasks 2.1 to 2.4 add tests over shipped, unchanged product code, so each must pass on its first run — which inverts the failing-test-first cycle `/speq:code-guardrails` requires, and the plan still never says so.
- Fix: Add a sentence to plan.md § Design → Goals stating that tasks 2.1 to 2.4 are characterization tests over already-shipped behavior, that they have no red step by construction, and that a red first run is a product defect to escalate rather than a step to engineer.

## Prose Quality

#### [PROSE_UNCLEAR] ADVISORY
- Location: plan.md § Design → Non-Goals (line 16); plan.md § Consequences, the scenario-split row (line 50); plan.md § Features closing paragraph (line 67)
- Issue: plan.md misdescribes the delta the blocker-3 fix authored, in a direction that invites the fix's own reversal. Line 16 reads "The one spec delta splits an existing scenario in two and adds no obligation the library did not already carry", and decision `[1]`'s Rationale repeats it — but adding "for each driver this release supports" to the refresh half *is* a new obligation, and it is the entire reason task 2.2's test now anchors to something. The `setProperties` half also adds three normative steps the removed scenario never carried; that scenario's `WHEN` named only `refresh`. A recorder or later reviewer reading line 16 would conclude the per-driver phrase changes nothing and could strip it as redundant. Separately, line 50's Alternatives cell lists two options and line 67 says "the two alternatives rejected", while decision `[1]` now records three.
- Fix: In plan.md § Design → Non-Goals, replace "adds no obligation the library did not already carry" with what the delta does: it adds a per-driver obligation to the refresh half, satisfied by the existing MySQL test plus task 2.2, and adds no obligation any existing test fails to meet. Apply the same correction to decision-log `[1]`'s Rationale. Change line 50's Alternatives cell and line 67's "the two alternatives rejected" to name all three options decision `[1]` records.

#### [PROSE_BLOAT] ADVISORY (carried forward from round 1, unaddressed)
- Location: plan.md § Design → Context, lines 11-14
- Issue: the section still withholds its conclusion and still ends on a rhetorical question — "So the question is where the three new objects live, and whether `Postgres` gains the ability to change the source at all." The answer is one line later under `### Decision`. BLUF requires the conclusion first.
- Fix: In plan.md § Design → Context, delete the closing rhetorical question and open the section with the constraint it exists to establish: three fixture objects are missing, and the refresh scenario requires DDL no static seed can produce.

#### [PROSE_UNCLEAR] ADVISORY (carried forward from round 1, unaddressed)
- Location: plan.md § Design → Context, line 14 ("The refresh scenario forces half the answer.")
- Issue: "half the answer" still names no quantity and no artifact, and the sentence is interpretable only after the two that follow it.
- Fix: Replace "The refresh scenario forces half the answer." with the constraint it stands for: the refresh scenario requires a column added after schema creation, so mutation must run from Rust even though the static shape need not.
