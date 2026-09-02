# Plan Review Findings: add-postgresql-test-coverage (round 1)

## Summary
- Axes checked: 6/6
- Total findings: 9 (Blockers: 3, Advisory: 6)
- Intent Fidelity blockers: 0

## Premortem

Four failure stories, each routed to a finding below.

1. **The stale-fixture trap.** Every developer's `make test-e2e-postgres` fails with "database `adbc_vs` holds 5 of the 8 seeded tables". `make stack-down` runs `docker compose down` without `--volumes`, and the `postgres` service uses the image's anonymous volume, so down-then-up never re-seeds. The named recovery exists but is reachable only after a full Exasol readiness wait. → A2, A4.
2. **The missing handle.** The implementer reaches task 2.2, finds no route from a test to a `Postgres` value, and improvises. `Postgres::from_stack()` inside the test re-runs the readiness wait and the fixture check on every invocation; a later contributor adds `pub postgres: Postgres` to `Stack` as well, and two constructions coexist. → B1.
3. **The self-inflicted source failure.** Group A runs 1.1 and 2.4 concurrently. Task 1.1's `docker compose rm -sfv postgres` kills task 2.4's scan mid-stream. The failure reads as an ADBC connection error, which is exactly the error family 2.4 asserts on, and 2.4 is "fixed" by loosening its assertion. → B2.
4. **The unanchored test.** A later consolidation of `vs-adapter/adapter-request-dispatch` deletes `refresh_picks_up_a_new_postgresql_column`, because no scenario in the library names PostgreSQL refresh and the deletion violates nothing. → B3.

## Intent Fidelity

[no objection — axis checked: all four gaps issue #2 names have an implementing task (2.1 identifier case, 2.3 view exposure, 2.2 refresh success, 2.4 source-failure status); the Non-Goals add no unrequested work; decision `[5]` records the MySQL-side mixed-case gap as deliberately left open rather than silently dropped; task 3.1's CLAUDE.md edit follows the repo's own tier-documentation practice]

## Feasibility

#### [HIDDEN_DEPENDENCY] BLOCKER
- Location: plan.md § Implementation Tasks, task 2.2; plan.md § Consequences, closing paragraph ("`Postgres::execute` is the only new interface")
- Issue: no test can reach a `Postgres` value, and task 2.2 names no file that would change that. `crates/adbc-vs-adapter/tests/common/postgres_stack.rs:50-52` defines `pub struct Stack { pub exasol: Exasol }` — no `postgres` field — and `bring_up()` binds the value as `let _postgres = Postgres::from_stack();` under a doc comment that justifies the drop: "neither constructed value outlives this function." The MySQL twin does the opposite: `common/stack.rs:31-34` holds `pub mysql: MySql`, which is how `e2e_mysql_test.rs:384` reaches `stack.mysql.execute(...)`. Task 2.2 lists only `e2e_postgres_test.rs` and `common/postgres.rs`, so an implementer following the plan literally has a method with no caller and must invent the wiring — either a third file edit plus a doc-comment rewrite, or a second `Postgres::from_stack()` construction inside the test that re-runs `await_queries` and `check_fixture`. The plan's design paragraph also asserts `Postgres::execute` is the only new interface, which the first route contradicts (a new public field).
- Fix: In plan.md § Implementation Tasks task 2.2, add `crates/adbc-vs-adapter/tests/common/postgres_stack.rs` to the named files and state the chosen route: add `pub postgres: Postgres` to `postgres_stack::Stack`, bind it in `bring_up()` instead of `let _postgres`, and rewrite that function's doc comment so it no longer claims the value does not outlive the call — matching `common/stack.rs`'s `pub mysql: MySql`. Add a decision-log entry recording that route and why a per-test `Postgres::from_stack()` was rejected. Correct plan.md § Consequences' closing paragraph to name both new interfaces.

#### [UNSTATED_ASSUMPTION] ADVISORY
- Location: plan.md § Design → Non-Goals ("no product-code change"); plan.md § Impact ("Test-only. No product code changes"); decision-log `[4]`
- Issue: decision `[4]` states the mixed-case clause "has never been proven end to end for any driver", then task 2.1 asserts it. If the assertion fails, the cause is a product defect, and the plan's own Non-Goals forbid the only remedy. No contingency is stated. Existing evidence lowers but does not remove the risk: `crates/adbc-vs-adapter/src/postgres_dialect_tests.rs::every_identifier_is_quoted_whatever_its_case` proves the rendering half at unit level for a mixed-case column, and `pushdown_tests.rs` shows the `EMITS` clause carries positional names rather than source names, so only Exasol's own handling of a mixed-case virtual table name is untested.
- Fix: Add a bullet to plan.md § Design → Non-Goals stating that a failing mixed-case assertion in task 2.1 is a product defect to escalate as a separate plan, never an assertion to weaken, and name `postgres_dialect_tests.rs::every_identifier_is_quoted_whatever_its_case` in task 2.1 as the unit-level evidence that makes a failure surprising.

#### [UNSTATED_ASSUMPTION] ADVISORY
- Location: plan.md § Implementation Tasks, tasks 2.1 through 2.4
- Issue: only task 1.1 carries a red step ("Red first: add ... to `SEEDED_TABLES` ... `Postgres::check_fixture` must fail"). Tasks 2.1 to 2.4 add tests over shipped, unchanged product code, so each must pass on its first run. That inverts the failing-test-first cycle `/speq:code-guardrails` requires of the implementer, and the plan never says so, leaving the implementer to either stall or manufacture a red step by temporarily breaking product code.
- Fix: Add a sentence to plan.md § Design → Goals stating that tasks 2.1 to 2.4 are characterization tests over already-shipped behavior, that they have no red step by construction, and that a red first run is a product defect to escalate rather than a step to engineer.

## Requirement Quality

#### [COMPLETENESS_GAP] ADVISORY
- Location: plan.md § Impact; plan.md § Implementation Tasks, task 3.1, third bullet
- Issue: both places record that `docker/postgres/init.sql` applies only to a fresh volume and name `docker compose rm -sfv postgres` as the recovery, but neither records the coupling that makes the trap easy to fall into. `Makefile:179-181` defines `stack-down` as `$(COMPOSE) down` with no `--volumes`, and `docker-compose.yml`'s `postgres` service declares no named volume for the data directory, so the image's anonymous volume survives `make stack-down`. A developer who runs `make stack-down && make stack-up` therefore still holds the old seed, which is the least obvious way to reach the failure and the one a reader will assume is safe.
- Fix: In plan.md § Impact and in task 3.1's third bullet, state that `make stack-down` (`docker compose down`, no `--volumes`) preserves the `postgres` anonymous volume, so a down-then-up cycle does not re-seed and only `docker compose rm -sfv postgres` does.

## Task Breakdown

#### [TASK_GRANULARITY] BLOCKER
- Location: plan.md § Parallelization, Group A and the sentence "Task 2.4 needs no fixture object and edits a different file from every other task, so it runs beside 1.1"
- Issue: the independence claim rests on file disjointness alone and ignores the shared Compose stack. Task 1.1's own steps include `docker compose rm -sfv postgres && make stack-up-postgres` — destroying and recreating the container that task 2.4's tier queries at `postgresql://127.0.0.1:5432/adbc_vs` (`crates/adbc-vs-scan/tests/postgres_it_test.rs:38`). Task 2.4's tier also streams a 300,000-row result (`WIDE_RESULT_ROWS`), so the window is wide. Two further collisions: task 1.1's verification step runs `make test-it-postgres`, the same binary task 2.4 is editing, and both tasks invoke `cargo test -p adbc-vs-scan`, contending on one `target/` lock. Decision `[7]` reasons only about concurrent edits to one file and never about the stack.
- Fix: In plan.md § Parallelization, make task 2.4 sequential after task 1.1 — move 2.4 into Group B or give it a Group of its own ordered after Group A — and replace the "runs beside 1.1" sentence with the reason: task 1.1 recreates the `postgres` container and re-runs `make test-it-postgres`, so no task driving the PostgreSQL stack may overlap it. Extend decision `[7]` to record that the shared Compose stack, not only file disjointness, decides what may run in parallel here.

#### [TRACEABILITY_GAP] BLOCKER
- Location: plan.md § Features, closing paragraph ("PostgreSQL coverage was therefore always required; only the tests were missing"); decision-log `[1]`
- Issue: the claim is false for one of the four gaps, and that claim is the sole justification for shipping zero spec deltas. `vs-adapter/adapter-request-dispatch/Refresh and setProperties re-run metadata discovery` reads `Given a virtual schema whose source has gained a column since creation` and names no driver, unlike its three siblings, which all carry "for each driver this release supports". A scenario naming no driver is satisfied by one driver's test, and `e2e_mysql_test.rs::refresh_picks_up_new_column` already satisfies it. The clause the plan cites instead — "the discovery MUST run through the metadata source the request's recorded driver resolves to" — constrains how one discovery runs; it does not multiply the scenario across drivers. Consequence: task 2.2's test implements no outstanding obligation, and nothing in the spec library will require it to survive a future consolidation. Decision `[1]`'s alternatives also present a false dichotomy — widen the whole `GIVEN`, or do nothing — and omit splitting the scenario into a `refresh` half carrying the per-driver phrase and a `setProperties` half that keeps its single-driver obligation, which would anchor task 2.2 without obliging the per-driver `setProperties` test the decision rightly refuses.
- Fix: Choose one and record it. Either (a) author a spec delta splitting `vs-adapter/adapter-request-dispatch/Refresh and setProperties re-run metadata discovery` into a `refresh` scenario carrying "for each driver this release supports" and a `setProperties` scenario without it, change the Features table row for `vs-adapter/adapter-request-dispatch` from UNCHANGED to the delta, and point task 2.2 at the new scenario; or (b) delete the sentence "PostgreSQL coverage was therefore always required; only the tests were missing" from plan.md § Features, state that the refresh scenario names no driver so task 2.2's coverage is issue-driven rather than spec-required, and add a decision-log entry recording the unanchored coverage as a residual gap in the form decision `[5]` already uses for the MySQL mixed-case gap. Rewrite decision `[1]`'s Alternatives either way, so the scenario-split option is on the record as considered.

## Design Depth

#### [TACTICAL_SHORTCUT] ADVISORY
- Location: decision-log `[2]`, Alternatives (a); plan.md § Patterns, "Single owner for fixture shape"; plan.md § Consequences, first row
- Issue: the rejected alternative is already this repository's established route for these exact three object kinds, and the stated rationale against it cannot apply to them. `crates/adbc-vs-adapter/tests/common/mysql.rs:76-97` creates `customers`, the `order_names` VIEW (`CREATE VIEW order_names AS SELECT id, name FROM orders`), `refresh_target (id INT NOT NULL, name VARCHAR(30) NOT NULL)`, `items`, and `item_notes` from Rust behind `DROP ... IF EXISTS`, while `docker/mysql/init.sql` creates only `orders` — so MySQL already splits fixture shape across SQL and Rust, and `common/postgres.rs`'s own header calls itself a "Structural twin" of that module. The rationale offered against the Rust route is that "a reviewer diffing the mirror against the seed would be reading half the fixture", but task 1.1 states "Add no `postgres_mirror` counterpart: none of the three objects is read differentially", and `postgres_mirror.rs` mirrors only `items` and `item_notes`. The risk the decision defends against cannot materialize for any of the three new objects, while the chosen route imposes a real cost the alternative avoids: a destructive container recreate for every developer holding a stack, plus a full Exasol readiness wait before `check_fixture` names the problem (`postgres_stack.rs:70-72` constructs `Exasol::from_stack()` before `Postgres::from_stack()`).
- Fix: In decision-log `[2]`, replace the mirror-diff rationale with one that survives task 1.1's "no mirror counterpart" bullet, and state explicitly why `common/mysql.rs`'s Rust-created `order_names` and `refresh_target` are not the precedent to follow here. If no such reason holds, adopt the MySQL route for `refresh_target` and `order_names` — leaving only `"MixedCase"` in the seed, or nothing — and drop the container-recreate obligation from plan.md § Impact and task 3.1 accordingly.

## Prose Quality

#### [PROSE_BLOAT] ADVISORY
- Location: plan.md § Design → Context, lines 11-14
- Issue: the section withholds its conclusion for four sentences and ends on a rhetorical question — "So the question is where the three new objects live, and whether `Postgres` gains the ability to change the source at all." The answer is one line later, under `### Decision`. BLUF requires the conclusion first; a question is not a statement heading's payload. The following `### Decision` line ("Static shape stays in the Compose seed. Mutation gets one method, used by one test.") is the model the Context section should have followed.
- Fix: In plan.md § Design → Context, delete the closing rhetorical question and open the section with the constraint it exists to establish: three fixture objects are missing, and the refresh scenario requires DDL that no static seed can produce.

#### [PROSE_UNCLEAR] ADVISORY
- Location: plan.md § Design → Context, line 14 ("The refresh scenario forces half the answer.")
- Issue: "half the answer" names no quantity and no artifact, and the sentence is only interpretable after reading the two that follow it.
- Fix: Replace "The refresh scenario forces half the answer." in plan.md § Design → Context with the constraint it stands for: the refresh scenario requires a column added after schema creation, so mutation must run from Rust even though the static shape need not.
