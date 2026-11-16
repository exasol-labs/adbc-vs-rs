# Plan Review Findings: change-pushdown-limit

## Summary
- Axes checked: 6/6
- Total findings: 9 (Blockers: 0, Findings: 9)
- Intent Fidelity blockers: 0

The `OFFSET` removal is applied consistently to the capability counts, scenarios, tasks, coverage rows, architecture delta, and docs tasks. Five findings are new in this pass. Four findings are still open from the previous review and are marked "Open from the previous review".

## Intent Fidelity

No objection. Axis checked. Every consequence listed in `notes/pr-feedback.md` Feedback 1 is carried through:
- Task 2.5 removes `LIMIT_WITH_OFFSET` from the driver declarations.
- The counts become MySQL 40, PostgreSQL 41, Trino 41, and 40 shared. They are stated in the `trino-pushdown` delta (Background 41, exact set 41, shared 40) and in task 2.6.
- The Backgrounds of `pushdown-limit`, `result-ordering`, and `pushdown-dialect` and the architecture delta are updated.
- The three clause-form scenarios, the Trino override, the offset tests, and old task 3.6 with its IT tests are removed.
- The offset rejection is kept as a scenario and as task 2.1.
- decision-log [2] is replaced and states that the user reversed the interview answer on the PR.
- The Impact text is updated, and the gate keeps `LIMIT 3 OFFSET 1` as an observation.

Top-N remains the declared follow-up plan, which now also decides `LIMIT_WITH_OFFSET`.

## Feasibility

#### [UNSTATED_ASSUMPTION] FINDINGS
- Location: plan.md § Implementation Tasks, task 2.1, and the shape table row `SELECT "d" FROM t LIMIT 3`; decision-log.md § Design Decisions [2]
- Issue: Task 2.1 refuses "a `limit` that carries an `offset`, whatever its value", and task 2.2 tests offset 0. The reference parser in `virtual-schema-common-java` reads `"offset": 0` as no offset: `PushdownSqlParser.parseLimit` defaults the offset to 0, and `SqlLimit.hasOffset` returns `offset != 0`. For offset 0, `LIMIT n` is the exact answer. Suppose Exasol attaches `"offset": 0` to a plain limit. Gate row 1 ("request carries `limit` with `numElements` 3 and no `offset`") then fails. Task 1.3 stops the plan with the reason "a delegated limit would return wrong rows", which is false for that request. No decision-log entry states why offset 0 is refused.
- Fix: In decision-log.md [2] Consequences, add one sentence that states why the adapter refuses any `offset`, 0 included: Exasol sends the key only when `LIMIT_WITH_OFFSET` was advertised, so its presence means that the advertised set and the parser have drifted apart. In the plan.md shape table, change row 1's "On failure" to "stop: the plan delivers nothing. If the only difference is `"offset": 0`, report the request to the user and ask whether the parser reads offset 0 as no offset before group B starts."

#### [UNSTATED_ASSUMPTION] FINDINGS
- Location: plan.md § Implementation Tasks, task 1.1 and the shape table row "self-join of `t` on `"d"` with `LIMIT 3`"
- Issue: Open from the previous review. The probe has one table, `t`, so the gate measures a self-join. The join scenario and its task 3.4 test join two different tables (`items` with `item_notes`). Exasol can delegate one shape and not the other. If the self-join row fails, the "adjust" branch rewrites the join scenario from a shape that the scenario does not describe.
- Fix: In task 1.1, let the probe also answer `createVirtualSchema` with a second table `u` that has a column `d`. Change the gate row to "join of `t` and `u` on `"d"` with `LIMIT 3`".

#### [NFR_IGNORED] FINDINGS
- Location: plan.md § Impact, last bullet
- Issue: Open from the previous review. The bullet says "This plan does not verify whether Exasol asks an existing virtual schema for its capabilities again without a refresh." The answer decides upgrade and rollback behavior. Suppose Exasol keeps the old set. An upgraded virtual schema then gains nothing until `REFRESH`. Suppose instead that Exasol keeps the new set after a rollback. The previous release refuses `limit`, so every unordered `LIMIT` query then fails until `REFRESH`. No task resolves the question, and `docs/pushdown.md` does not tell users.
- Fix: Add a step to task 1.2. Create the probe virtual schema with the default set. Re-create the adapter script with the limit set. Run `SELECT "d" FROM t LIMIT 3` without `REFRESH`, and record whether the request carries `limit`. In task 4.2, add one sentence to `docs/pushdown.md` that states whether an existing virtual schema needs `ALTER VIRTUAL SCHEMA <name> REFRESH` after an upgrade or a rollback. Replace the Impact bullet's "does not verify" sentence with the measured answer.

Also checked, no objection: the probe set is `Driver::Postgres.capabilities()` with `LIMIT` added only when absent. That set is identical before and after task 2.5 (41 entries), and it never holds `LIMIT_WITH_OFFSET`. The probe captures the first `pushdown` request, which serves every "no request carries" row, because each such query involves one virtual table. Task 2.3 no longer needs a Trino override, because all three cited syntaxes accept `LIMIT n` as the last clause.

## Requirement Quality

#### [COMPLETENESS_GAP] FINDINGS
- Location: plan.md § Implementation Tasks, tasks 1.2, 1.3, 1.4, and the shape table row `SELECT "d" FROM t LIMIT 3 OFFSET 1`; vs-adapter/pushdown-limit/spec.md § Background bullet 5
- Issue: The observation row has three gaps.
  - (a) Task 1.2 makes the permanent gate test assert every row's "Required outcome". For this row, that outcome is "Exasol rejects the statement and sends no request". Task 1.3 calls the row an observation that "stops nothing", but gives no instruction for the test when Exasol accepts the statement. `make test-e2e-postgres` in task 3.6 then fails, and no task fixes it.
  - (b) The report trigger is "a request carries an `offset`". The probe never advertises `LIMIT_WITH_OFFSET`, so the trigger tests a case that decision [2] already handles with a named error.
  - (c) The harmful case is not a stop. Suppose Exasol accepts the statement, sends `limit` with `numElements` 3 and no `offset`, and applies `OFFSET 1` above the scan. The query then returns 2 rows instead of 3. Every other "stop" row guards against this class of wrong rows.

  If Exasol accepts the statement at all, three statements become false: the pushdown-limit Background bullet 5, the architecture constraint "because Exasol accepts an `OFFSET` only after an `ORDER BY`", and the decision-log [2] Rationale "no query reaches the offset form". Task 1.4 corrects only the Background.
- Fix: In the plan.md shape table, change this row's "Required outcome" to "Exasol rejects the statement, or the request carries no `limit`, or it carries `limit` with `numElements` 4 or more and no `offset`". Change its "On failure" to "stop: report the measured request to the user". In task 1.2, make the test accept any of the three outcomes. In task 1.3, replace the sentence that starts "The `OFFSET` row is an observation" with one that names this row as a stop row. In task 1.4, add: "If Exasol accepted the statement, also correct the `LIMIT ... OFFSET` reason in architecture.md § Constraints and the decision-log.md [2] Rationale, so that both state the measured behavior."

#### [IMPLEMENTATION_DETAIL] FINDINGS
- Location: vs-adapter/result-ordering/spec.md § Background, bullets 3 and 5
- Issue: Open from the previous review. No GIVEN, WHEN, or THEN step of `result-ordering`, after the merge, depends on two facts in the CHANGED Background. Bullet 3 says that "`postgresql-virtual-schema` advertises `ORDER_BY_COLUMN` over an `IMPORT` transport ... and this release does not inherit that decision". Bullet 5 says that an ordered query "costs a sort in Exasol" and calls the cost "the deliberate price of a stable result".
- Fix: Delete the `postgresql-virtual-schema` sentence from bullet 3, and delete bullet 5.

Also checked, no objection:
- `speq plan validate change-pushdown-limit` passes for all 8 deltas.
- The driver declarations hold 39, 40, and 40 entries today, so the new counts 40, 41, 41, and 40 shared are correct.
- After the merge, no recorded scenario or Background line still names `LIMIT` as withheld. The CHANGED blocks replace recorded mysql-pushdown line 21, postgresql-pushdown line 21, trino-pushdown lines 8, 9, 21, 90, and 99, result-ordering, pushdown-dialect line 11, pushdown-aggregation line 9, and pushdown-projection line 9.
- No recorded spec outside this plan pins a capability count.
- Every Background has 5 bullets or fewer.
- The new offset scenario has a testable GIVEN, WHEN, and THEN. The render scenario no longer mentions an offset.

## Task Breakdown

#### [TRACEABILITY_GAP] FINDINGS
- Location: plan.md § Implementation Tasks, task 2.5; § Dead Code Removal
- Issue: Three comments describe the old sets, and no task or Dead Code Removal row updates them:
  - `crates/adbc-vs-scan/drivers/trino.toml` lines 52-54 say "The set is the 40 entries `postgresql.toml` states ... That makes 40 entries". Both numbers become 41.
  - `crates/adbc-vs-adapter/tests/common/bench.rs` lines 194-197 say that "the ADBC side pushes neither". This becomes false for an unordered `LIMIT`.
  - The doc comment at `crates/adbc-vs-adapter/src/postgres_dialect_tests.rs` line 1032 says that "`orderBy` and `limit` stay in Exasol".

  The phrase "entry counts in doc comments" in task 2.6 covers only `capabilities_tests.rs`.
- Fix: In task 2.5, append "and change both counts in the set comment at `trino.toml` lines 52-54 from 40 to 41". Add two § Dead Code Removal rows. Row 1: `bench.rs` lines 194-197, reason "the ADBC side now pushes an unordered `LIMIT`; the benchmark query still carries no `LIMIT`". Row 2: the `postgres_dialect_tests.rs` line 1032 doc comment, reason "only `orderBy` is refused".

Also checked, no objection:
- The three clause-form coverage rows and their IT tests are removed.
- Group C no longer lists `crates/adbc-vs-scan/tests/`.
- The task numbers 3.1 to 3.6 match the group table.
- Every pushdown-limit scenario has a coverage row, including the new offset scenario.
- No Manual Testing row expects an offset at the source.
- Groups A to D share spec deltas or files, and group C runs after group A on `e2e_postgres_test.rs`.

## Design Depth

#### [INFORMATION_COUPLING] FINDINGS
- Location: plan.md § Implementation Tasks, task 2.3
- Issue: Open from the previous review, in a smaller form. Two functions assemble the clause order: `dialect_common::render_select` for PostgreSQL and Trino, and `MySqlDialect::render`. Task 2.3 appends the limit in both. The top-N follow-up must then insert its ordering clause between `HAVING` and `LIMIT` in both places, and only the per-dialect unit tests catch a miss.
- Fix: In task 2.3, add one function in `dialect_common.rs` that takes the rendered clauses and the limit and returns the statement in clause order. Call it from `render_select` and from `MySqlDialect::render`.

Also checked, no objection:
- No decision-log entry has `Promotes to ADR: yes`, so `[ADR_OVERPROMOTION]` does not apply.
- The ADR digest holds no ADR about ordering or limiting.
- The plan conforms to `capability-negotiation-all-or-nothing`. `LIMIT` renders every `limit` that Exasol sends when only `LIMIT` is advertised. `LIMIT_WITH_OFFSET` is withheld, not advertised and then refused for some requests.
- The plan conforms to the Trino-set ADR in `003-remove-pushdown-collation-and-like-escape.md`, because both sets gain only `LIMIT`.
- The architecture delta's BASE equals `git hash-object specs/architecture.md` (`cbf2746`). Its Constraints section differs from the base only in the two limit bullets.
- plan.md states no component, interface, or data-flow change that the delta lacks.

## Prose Quality

#### [PROSE_UNCLEAR] FINDINGS
- Location: architecture.md § Constraints, the bullet starting "Every dialect advertises `LIMIT`"; vs-adapter/trino-pushdown/spec.md § Scenario "Capability set withholds the fragments the Trino dialect does not delegate", last step; vs-adapter/pushdown-limit/spec.md § Background bullet 5
- Issue: Three artifacts give three different reasons for withholding `LIMIT_WITH_OFFSET`:
  - The architecture bullet reads "No dialect advertises `LIMIT ... OFFSET`, because Exasol accepts an `OFFSET` only after an `ORDER BY`, so a `limit` that carries an offset fails rather than render without it". The word "so" ties the failure to the `ORDER BY` rule, but the failure comes from the parser rule in task 2.1.
  - The Trino step says "because Exasol sends an offset only after an `ORDER BY`". This is a claim about the request that nothing measured, and this adapter never receives an `orderBy` element.
  - The pushdown-limit bullet says "because no query would reach an offset rendering".
- Fix: Use one reason in all three places: "Exasol accepts an `OFFSET` only together with an `ORDER BY`, and every `ORDER BY` stays in Exasol." In architecture.md, replace the sentence with two sentences: "No dialect advertises `LIMIT ... OFFSET`, because Exasol accepts an `OFFSET` only together with an `ORDER BY`, and every `ORDER BY` stays in Exasol. The adapter refuses a `limit` that carries an offset rather than render the count without it." In the Trino step, replace "because Exasol sends an offset only after an `ORDER BY`" with "because Exasol accepts an `OFFSET` only together with an `ORDER BY`, which stays in Exasol".

#### [PROSE_UNCLEAR] FINDINGS
- Location: vs-adapter/mysql-pushdown/spec.md § Background, bullet 3
- Issue: Open from the previous review. The bullet is one sentence with four ownership claims, and the delta adds the fourth: "`vs-adapter/pushdown-limit` owns the limiting capabilities". The phrase "the limiting capabilities" also differs from the name that the pushdown-dialect delta uses for the same two capabilities: "the advertised `LIMIT`, the withheld `LIMIT_WITH_OFFSET`".
- Fix: Split bullet 3 into one sentence per owning feature. End with "`vs-adapter/pushdown-limit` owns the advertised `LIMIT` and the withheld `LIMIT_WITH_OFFSET`."

Also checked, no objection: decision-log [2] states the reversal on the PR, as Feedback 1 asks. The Interview section stays unchanged, as the planning skill requires. plan.md § Summary and § Impact state that every `OFFSET` stays in Exasol, with no hedging.
