# Decision Log: change-pushdown-limit

## Interview

- Source: GitHub issue exasol-labs/adbc-vs-rs#20 "Push down LIMIT and top-N"; a preview `LIMIT 100` on a 5M-row table fetches all 5M rows today → A: issue is the intent
- Q: Scope of this plan → A: LIMIT without ORDER BY only; top-N is a later phase-2 plan
- Q: OFFSET → A: include it; advertise LIMIT_WITH_OFFSET too, rendered in all three dialects
- Q: If Exasol drops the outer ORDER BY once ORDER BY and LIMIT are advertised → A: do not ship top-N; ORDER BY stays in Exasol and the result-ordering guarantee stays unconditional
- Open for the planner: measure on the test Exasol which `limit` shapes arrive with only LIMIT advertised (with ORDER BY present, over joins, over aggregation), and make the specs state it

## Design Decisions

### [1] A new framework feature owns limit delegation

- **Decision:** `vs-adapter/pushdown-limit` owns the `LIMIT` and `LIMIT_WITH_OFFSET` capabilities, the `limit` element, and the query shapes a delegated limit must answer correctly. `vs-adapter/result-ordering` keeps the ordering capabilities and the rule that a limit after an `ORDER BY` stays in Exasol.
- **Rationale:** The two change for different reasons. Ordering is withheld because the emitting-UDF transport carries no ordering guarantee. A limit with no ordering is a row-count fragment, like a filter, and the existing framework features are named after the protocol element they own (`pushdown-filter`, `pushdown-join`, `pushdown-aggregation`).
- **Alternatives:** Add the limit scenarios to `result-ordering`. Rejected: its name and description would state an ordering decision while most of its scenarios state a delegation.
- **Promotes to ADR:** no

### [2] Advertise LIMIT_WITH_OFFSET although Exasol sends an offset only with ORDER BY

- **Decision:** Every dialect advertises and renders `LIMIT_WITH_OFFSET`, as the interview decided.
- **Rationale:** Exasol's SQL reference accepts an `OFFSET` only together with `ORDER BY`, and ordering stays in Exasol, so Exasol applies every offset itself in this release. Advertising the capability costs one rendering arm per dialect and changes no query result, and add-pushdown-top-n needs the rendering.
- **Alternatives:** Withhold `LIMIT_WITH_OFFSET` until add-pushdown-top-n. Not taken, because the interview chose to include it.
- **Consequences:** No end-to-end query reaches the offset form. Unit tests pin each dialect's rendering, integration tests run each form at its source, and gate task 1.3 records whether Exasol ever sends an offset.
- **Promotes to ADR:** no

### [3] Measure the query shapes against PostgreSQL only

- **Decision:** The query-shape scenarios of `vs-adapter/pushdown-limit` run against PostgreSQL, and the gate measurement runs against the probe adapter with the PostgreSQL capability set. The capability sets, the clause forms, the basic `LIMIT` reach, and the ordered-limit scenario run for every driver.
- **Rationale:** Exasol's planner decides which request carries `limit` from the query and the advertised capabilities, not from the source. The three dialects advertise the same capabilities for every shape measured: filters, a one-column `GROUP BY`, `HAVING`, an inner equi-join, and both limiting capabilities. No dialect advertises `FN_ADD` or `AGGREGATE_GROUP_BY_EXPRESSION`, and no capability delegates an analytic function, so Exasol evaluates the shapes that keep their limit in Exasol itself for every dialect. Each dialect's clause placement is covered by its own unit tests.
- **Alternatives:** Repeat every shape per driver. Rejected: it triples the end-to-end run time without exercising a code path that differs between dialects.
- **Promotes to ADR:** no

### [4] Measure the limit shapes in an implementation gate, not at planning time

- **Decision:** Task group A measures which `limit` shapes Exasol sends before any product change, and the `vs-adapter/pushdown-limit` query-shape scenarios state the required outcome for each shape until then.
- **Rationale:** The planning process reads code and documentation only and starts no Exasol container, so it cannot run the probe adapter. The project already gates a product change on a probe measurement in the same way (`exasol_sends_the_measured_boolean_and_timestamp_filter_shapes`). The gate test stays in the suite, so a later Exasol release that sends a different shape fails it.
- **Alternatives:** Run the probe measurement during planning and write the measured request shapes into the scenarios. Not taken, because planning starts no container.
- **Consequences:** A failed "stop" row halts the implementation before any product change. A failed "adjust" row rewrites that scenario's delegation step after the plan review, and task 1.3 names the test and coverage changes that follow.
- **Promotes to ADR:** no

## Review Findings
