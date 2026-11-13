# Plan Review Findings: change-pushdown-limit

## Summary
- Axes checked: 6/6
- Total findings: 12 (Blockers: 1, Findings: 11)
- Intent Fidelity blockers: 0

## Intent Fidelity

#### [INTENT_DRIFT] FINDINGS
- Location: plan.md § Implementation Tasks, group 1; decision-log.md § Interview, last bullet
- Issue: The interview reads "Open for the planner: measure on the test Exasol which `limit` shapes arrive with only LIMIT advertised (with ORDER BY present, over joins, over aggregation), and make the specs state it". The plan moves that measurement into implementation task group A. Its "stop" and "adjust" branches can rewrite `vs-adapter/pushdown-limit` after the plan is reviewed. No decision-log entry says why the planner did not measure. The PR reviewer therefore approves query-shape scenarios that state required outcomes, not measured ones. The project has a precedent for implementation gates (`exasol_sends_the_measured_boolean_and_timestamp_filter_shapes`), so the deferral can be acceptable, but the plan does not say so.
- Fix: Do one of two things. Either run the probe measurement now and write the measured request shapes into the `vs-adapter/pushdown-limit` scenarios, then delete the "adjust" branch of task 1.3. The probe in `crates/adbc-vs-adapter/tests/common/probe.rs` needs only a different capability string. Or add a `decision-log.md` § Design Decisions entry "[4] Measure the limit shapes in an implementation gate, not at planning time" with `Promotes to ADR: no`. Its Rationale states why the measurement was not run at planning time, and that a gate can still rewrite the query-shape steps after review. Cite it from a plan.md § Context bullet.

Also checked, no objection: the plan delegates `LIMIT` without `ORDER BY` and advertises `LIMIT_WITH_OFFSET`, as the interview decided. Top-N is a declared follow-up plan. The issue's acceptance "`SELECT ... LIMIT 100` fetches at most 100 rows" maps to the three `LIMIT pushdown reaches <technology>` scenarios. The condensed Backgrounds of `result-ordering` and `pushdown-projection` drop only lines that no scenario depends on, which is not scope creep.

## Feasibility

#### [UNSTATED_ASSUMPTION] FINDINGS
- Location: plan.md § Implementation Tasks, task 1.2
- Issue: Task 1.2 builds the probe set as "`Driver::Postgres.capabilities()` plus `LIMIT` and `LIMIT_WITH_OFFSET`". Task 2.5 later adds both entries to `drivers/postgresql.toml`. From group B on, the gate test therefore advertises each entry twice. The gate test is permanent and runs again in task 3.7. The plan does not state that Exasol accepts a `getCapabilities` response with duplicate entries.
- Fix: In task 1.2, replace "plus `LIMIT` and `LIMIT_WITH_OFFSET`" with "with `LIMIT` and `LIMIT_WITH_OFFSET` added only when absent", so the probe set is the same before and after task 2.5.

#### [UNSTATED_ASSUMPTION] FINDINGS
- Location: plan.md § Implementation Tasks, shape table row "self-join of `t` on `"d"` with `LIMIT 3`"; task 3.4 `limit_over_a_delegated_join_applies_to_the_joined_rows`
- Issue: The probe has one table, `t`, so the gate measures a self-join. The scenario and its E2E test use a join of two different tables (`items` with `item_notes`). Exasol can delegate one and not the other. If the self-join row fails, the "adjust" branch rewrites the join scenario from a shape that the scenario does not describe.
- Fix: In task 1.1, let the probe also answer `createVirtualSchema` with a second table, for example `u` with a column `d`. Change the gate row to "join of `t` and `u` on `"d"` with `LIMIT 3`". If that is not done, add to task 1.3: "the self-join is a proxy; before adjusting the join scenario, run the join of `PGVS."items"` and `PGVS."item_notes"` with `LIMIT 2` under `EXPLAIN VIRTUAL` and adjust only if its generated statement carries no limit."

#### [NFR_IGNORED] FINDINGS
- Location: plan.md § Impact, last bullet
- Issue: "This plan does not verify whether Exasol asks an existing virtual schema for its capabilities again without a refresh." The answer decides upgrade and rollback behavior. If Exasol keeps the old set, an upgraded virtual schema gains nothing until `REFRESH`, and the user docs do not say so. If Exasol keeps the new set after a rollback to a release that refuses `limit`, every unordered `LIMIT` query fails until `REFRESH`. No task resolves the question.
- Fix: Add a step to task 1.2: create the probe virtual schema with the default set, re-create the adapter script with the limit set, run `SELECT "d" FROM t LIMIT 3` without `REFRESH`, and record whether the request carries `limit`. In task 4.2, add one sentence to `docs/pushdown.md` that states whether an existing virtual schema needs `ALTER VIRTUAL SCHEMA <name> REFRESH` after an upgrade or a rollback. Replace the Impact bullet's "does not verify" sentence with the measured answer.

Also checked, no objection: the clause forms match the cited MySQL, PostgreSQL, and Trino syntax. The Exasol SELECT usage notes confirm "The optional offset can only be used in combination with `ORDER BY`". `PushdownSqlParser.parseLimit` in `virtual-schema-common-java` confirms the `numElements` and optional `offset` shape. About 15 `Query` literals need `limit: None`. The limit renders from parsed integers, so the change opens no injection path.

## Requirement Quality

#### [COMPLETENESS_GAP] BLOCKER
- Location: specs/_plans/change-pushdown-limit/vs-adapter/pushdown-limit/spec.md § Scenario "A LIMIT above a fragment Exasol evaluates itself stays in Exasol"; plan.md § Implementation Tasks, task 1 shape table and task 3.4
- Issue: The architecture delta says a delegated limit is correct only with "no ordering, filter, join, or aggregation of its own between the scan and the limit". The interview asked to measure shapes "over aggregation". The scenarios and the gate cover a filter Exasol evaluates (`"d" + 0 > 1`) and a join Exasol evaluates. They cover no aggregation that Exasol evaluates directly below the `LIMIT`, and no analytic function. In these shapes a limit applied at the source returns wrong values, not only fewer rows. `GROUP BY "d" + 0 ... LIMIT 3` would group only 3 source rows. `COUNT(*) OVER () ... LIMIT 3` would return 3 instead of the table's row count. The derived-table `COUNT(*)` scenario puts the limit below the aggregate, which is a different shape.
- Fix: (1) In `vs-adapter/pushdown-limit/spec.md`, extend the scenario "A LIMIT above a fragment Exasol evaluates itself stays in Exasol" with two GIVEN shapes: a `GROUP BY` over an expression no dialect delegates, such as `GROUP BY "price" + 0`, with a `LIMIT` and no `ORDER BY`; and a select list with an analytic function, such as `COUNT(*) OVER ()`, with a `LIMIT` and no `ORDER BY`. Add the THEN step "every analytic value MUST equal the value Exasol computes over the full local mirror". (2) In the plan.md task 1 shape table, add two rows with the outcome "request carries no `limit`" and "stop" on failure: `SELECT "d" + 0, COUNT(*) FROM t GROUP BY "d" + 0 LIMIT 3` and `SELECT "d", COUNT(*) OVER () FROM t LIMIT 3`. (3) In task 3.4, add both queries to `limit_above_a_fragment_exasol_evaluates_stays_in_exasol`. Exasol evaluates both shapes itself for every dialect, because every dialect withholds `AGGREGATE_GROUP_BY_EXPRESSION` and no capability delegates an analytic function. Decision-log [3]'s PostgreSQL-only rationale therefore still holds.

#### [AMBIGUOUS_REQUIREMENT] FINDINGS
- Location: specs/_plans/change-pushdown-limit/architecture.md § Constraints, the bullet starting "Every dialect advertises `LIMIT`"; vs-adapter/pushdown-limit/spec.md § Background bullet 3
- Issue: The architecture constraint names the Exasol operators that make a delegated limit wrong as "ordering, filter, join, or aggregation". The spec Background says "nothing it applies itself". The architecture list is incomplete: an analytic function, a `DISTINCT` that Exasol evaluates, and a `UNION ALL` operand also sit above the scan. A reader can take the four names as the complete rule.
- Fix: In architecture.md § Constraints, replace "with no ordering, filter, join, or aggregation of its own between the scan and the limit" with "with nothing Exasol evaluates itself between the scan and the limit, such as an ordering, a filter, a join, an aggregation, or an analytic function".

#### [AMBIGUOUS_REQUIREMENT] FINDINGS
- Location: vs-adapter/pushdown-limit/spec.md § Scenario "A limit renders as the last clause of the statement"
- Issue: The GIVEN carries "a `limit` with `numElements` 5" and no offset. The second THEN step says "the count and an offset MUST render as the decimal integers the request carries". A test written from this scenario cannot check an offset that the request does not carry.
- Fix: Either change the GIVEN to "a `limit` with `numElements` 5 and `offset` 2" and the first THEN step to "the statement MUST end with the dialect's limit clause for the count 5 and the offset 2", or change the second THEN step to "the count MUST render as the decimal integer the request carries" and leave the offset to the three `<technology>-pushdown` clause-form scenarios.

#### [IMPLEMENTATION_DETAIL] FINDINGS
- Location: vs-adapter/result-ordering/spec.md § Background, bullets 3 and 5
- Issue: No GIVEN, WHEN, or THEN step of `result-ordering` depends on two facts in the Background. Bullet 3 says "`postgresql-virtual-schema` advertises `ORDER_BY_COLUMN` over an `IMPORT` transport ... and this release does not inherit that decision". Bullet 5 says an ordered query "costs a sort in Exasol" and calls that cost "the deliberate price of a stable result".
- Fix: Delete the `postgresql-virtual-schema` sentence from bullet 3, and delete bullet 5. `docs/pushdown.md` already tells users that a part Exasol runs makes Exasol fetch more rows.

Also checked, no objection: `speq plan validate change-pushdown-limit` passes. Each CHANGED Background keeps the recorded lines, except the intended edits and the condensations named above. The capability counts match the driver declarations: MySQL 39 to 41, PostgreSQL 40 to 42, Trino 40 to 42, and 39 to 41 shared by PostgreSQL and Trino. No recorded scenario outside the deltas still names `LIMIT` as withheld.

## Task Breakdown

#### [TRACEABILITY_GAP] FINDINGS
- Location: plan.md § Implementation Tasks, task 1.3, the sentence starting "If a row marked "adjust" fails"
- Issue: The "adjust" branch changes a scenario's delegation step to "no generated statement MUST carry a limit". No task changes the tests that pin the old step. The gate test `exasol_sends_the_measured_limit_shapes` still asserts the row's "Required outcome". The task 3.4 test for that scenario, for example `limit_over_a_delegated_filter_reaches_postgresql_with_the_filter`, keeps a name and an `EXPLAIN VIRTUAL` assertion that say the limit reaches the source. `make test-e2e-postgres` in task 3.7 then fails on a measured fact.
- Fix: Append to that sentence of task 1.3: "change that row's assertion in `exasol_sends_the_measured_limit_shapes` to 'request carries no `limit`', and in task 3.4 rename that scenario's test to `limit_over_<shape>_stays_in_exasol` and assert that no generated statement carries a limit." Update the matching row of § Verification, Scenario Coverage.

Also checked, no objection: every new and changed scenario has a row in Scenario Coverage. Groups A to D share spec deltas or files, and group C is sequenced after group A on their shared file `e2e_postgres_test.rs`. Task 3.6 is needed, because no end-to-end query reaches the offset form.

## Design Depth

#### [INFORMATION_COUPLING] FINDINGS
- Location: plan.md § Implementation Tasks, task 2.3
- Issue: Two functions assemble the clause order `WHERE`, `GROUP BY`, `HAVING`: `dialect_common::render_select` for PostgreSQL and Trino, and `MySqlDialect::render`. Task 2.3 appends the limit in both. The top-N follow-up must add its ordering clause in both places again, and only the per-dialect unit tests catch a miss.
- Fix: In task 2.3, add one function in `dialect_common.rs` that appends `render_limit` to an assembled statement body. Call it from `render_select` and from `MySqlDialect::render`, so that one place decides the position of the limit.

Also checked, no objection: no entry is `Promotes to ADR: yes`, so `[ADR_OVERPROMOTION]` does not apply. The plan does not contradict the accepted ADRs `trino-set-is-postgresql-without-like-escape-plus-group-by-tuple`, because both sets gain the same two entries, or `delegate-sum-avg-and-let-floating-point-sum-follow-source-order`, because every dialect renders `LIMIT_WITH_OFFSET` for every offset. The architecture delta matches BASE `cbf2746` except the two intended Constraints lines. plan.md states no component, interface, or data-flow change that the delta lacks.

## Prose Quality

#### [PROSE_UNCLEAR] FINDINGS
- Location: decision-log.md § Design Decisions [2] Alternatives, and [3] Decision
- Issue: [2] ends "The interview predates this documentation fact, so the choice is open to revisit before implementation." It names no one who revisits the choice and no trigger, so the decision reads as unsettled. [3] says the gate measurement runs "against PostgreSQL". The gate runs against the Lua probe adapter, which advertises the PostgreSQL set and has no PostgreSQL connection.
- Fix: In [2], delete the "open to revisit" sentence, or replace it with "The PR description asks the user to confirm this choice before implementation." In [3] Decision, replace "and the gate measurement run against PostgreSQL" with "and the gate measurement runs against the probe adapter with the PostgreSQL capability set".

#### [PROSE_UNCLEAR] FINDINGS
- Location: vs-adapter/mysql-pushdown/spec.md § Background bullet 3; vs-adapter/pushdown-limit/spec.md § Background bullet 1
- Issue: The mysql-pushdown bullet is one sentence with four ownership claims, and the delta adds the fourth. It breaks the one-idea-per-sentence rule. The pushdown-limit bullet says `result-ordering` keeps "every `ORDER BY`, and the limit under it", where "under" can mean before or after the `ORDER BY`.
- Fix: Split the mysql-pushdown bullet into one sentence per owning feature. In pushdown-limit bullet 1, replace "and the limit under it" with "and every `LIMIT` or `OFFSET` that follows one".
