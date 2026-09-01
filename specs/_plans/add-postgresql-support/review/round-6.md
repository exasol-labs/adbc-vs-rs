# Plan Review Findings: add-postgresql-support (round 6)

## Summary
- Axes checked: targeted recheck of the round-5 BLOCKER fix (Requirement Quality, Task Breakdown, Prose); not a fresh full pass
- Total findings: 2 (Blockers: 1, Advisory: 1)
- Intent Fidelity blockers: 0

Scope: confirmation pass on the single round-5 BLOCKER (`[REQUIREMENT_CONFLICT]`, three untouched `pushdown-filter` scenarios stating MySQL-only rules as framework `MUST`s) plus a screen for defects the fix itself introduces. Round 5's seven advisories are not re-verified and stand as written.

## Round-5 BLOCKER verdict

**`[REQUIREMENT_CONFLICT]` — NOT RESOLVED.** Every step the round-5 fix prescribed was executed correctly and verified below. The conflict survives anyway, with its polarity inverted, because the fix's premise — that `pushdown-filter`'s rewritten Background declares the feature framework — never reaches the merged library. `/speq:spec-merge` merges scenarios only; the Background rewrite is silently discarded at record time. See the BLOCKER below.

The scenario half of the fix is sound and needs no rework. What is missing is a delivery mechanism for the Background half, which no delta marker provides.

## Mechanical verification performed

Reported because the brief asked for it and the results are load-bearing for what is *not* flagged.

- **All three `DELTA:REMOVED` blocks match the shipped library byte for byte.** Compared heading and body by script against `specs/vs-adapter/pushdown-filter/spec.md:62`, `:78`, and `:94`. Zero mismatches, no duplicate removal blocks.
- **All three `DELTA:NEW` replacements are present and correctly stripped.** "LIKE renders its pattern and its escape character" keeps the two `ESCAPE`-clause rules and defers pattern form and collation to the four dialect features by name. "BETWEEN and IN keep their operand order" keeps both operand-order rules and makes the suffix conditional while preserving the operand-not-list placement rule for a dialect that renders one. "Predicate in select-list position renders as a projected boolean" replaces the unconditional suffix clause with "whatever collation suffix that dialect's own feature requires in `WHERE` position", and its null step is generalized from "MySQL and Exasol" to "Exasol and both targets" — standard three-valued logic, true of PostgreSQL, no measurement needed.
- **The MySQL halves landed as specified.** `vs-adapter/mysql-collation` gained one scenario, "A LIKE operand, an IN comparison, and a select-list predicate carry the comparison collation", covering all three sites, with Background bullet 9 grounding it. The file lands at **exactly 10** scenarios — at the guideline, not over it, so § Library organization's over-10 table correctly omits it.
- **The hex-literal step was absorbed, not duplicated.** `vs-adapter/mysql-pushdown`'s existing "String literal is rendered as a hex literal" scenario gained the `LIKE`-pattern clause in place; its `GIVEN` and `WHEN` were widened rather than a new scenario opened. The file stays at 10 scenarios. Background bullet 13 gained the grounding sentence.
- **The `escapeChar` clause the fix added beyond the prescription is shipped behavior, not a new unverified `MUST`.** `mysql-pushdown` now requires the `escapeChar` to render in hex too. `crates/adbc-vs-adapter/src/mysql_dialect.rs:646` renders it through `render_expression`, and `mysql_dialect_tests.rs:261` asserts the exact string `(\`note\` COLLATE utf8mb4_0900_as_cs LIKE _utf8mb4 X'57696425' ESCAPE _utf8mb4 X'5C')`. Documents reality; no finding.
- **The PostgreSQL halves are genuinely already covered and no redundant scenario was added.** `postgresql-pushdown`'s "LIKE, IS NULL, BETWEEN, and IN render in their standard forms" states the escape-string pattern, the `ESCAPE` clause exactly when an `escapeChar` is present, and both operand orders. `postgresql-collation`'s "An ordering comparison is collated and equality is left bare" states no suffix on `predicate_equal`, `predicate_in_constlist`, or `predicate_like`, and suffix-on-operand-not-list. Counts unchanged at 9 and 6.
- **§ Scenario Coverage is exact.** Four rows added (three under `vs-adapter/pushdown-filter` in § Framework, one under `vs-adapter/mysql-collation` in § Technology), one amended (`mysql-pushdown`'s hex row gained `like_renders_pattern_and_escape`). Cross-checked every non-`REMOVED` scenario in all 22 deltas against the coverage tables by script: exactly one row each, no orphans, no duplicates. The only mismatch across the whole plan is `driver-self-install`'s five deliberately carried-forward rows — round 5's advisory 3, untouched by this fix and not re-raised.
- **Task ownership is accurate under the current numbering.** Task 4.2 exists and now states the "nothing else" rule is load-bearing "at exactly three sites where MySQL does render a suffix", each of which "MUST be asserted rather than left implied". Task 4.5 exists and names the three PostgreSQL tests (`like_renders_its_escape_clause`, `between_and_in_keep_operand_order`, `select_list_predicate_renders_as_projected_boolean`) and the three MySQL tests that must keep asserting the MySQL arms. All six names match the coverage rows exactly. All three named MySQL tests exist today at `mysql_dialect_tests.rs:251`, `:298`, and `:1056`.
- **Both validators pass, re-run rather than trusted.** `speq plan validate add-postgresql-support` → "validation passed", 22 deltas, warnings only. `speq feature validate` → 0 errors across all 11 library features, 1 pre-existing AND-step warning.
- **The merge was simulated, not reasoned about.** Copied the real library file and the real delta into a throwaway project and ran `speq record`: `pushdown-filter` merges at **6 scenarios**, matching round 5's verified count. The `DELTA:REMOVED` is applied before the same-heading `DELTA:NEW`, so the replacement lands exactly once — the planner's claim in the `[plan-review]` entry "Three surviving pushdown-filter scenarios stated MySQL's collation and literal form as framework `MUST`s" is true. No scenario-level residue of the MySQL collation or hex rules survives in the merged file.

## Requirement Quality

#### [REQUIREMENT_CONFLICT] BLOCKER

- Location: `specs/_plans/add-postgresql-support/vs-adapter/pushdown-filter/spec.md:3` and `:7-20` (the `DELTA:CHANGED` Background), against `specs/vs-adapter/pushdown-filter/spec.md:3` and `:7-18`; and the same pattern in the ten other deltas listed below. Root cause: `/speq:spec-merge`'s marker table at `skills/spec-merge/SKILL.md:29-33`.

- Issue: the merge applies scenarios only. `/speq:spec-merge`'s table defines exactly three actions — `DELTA:NEW` append scenario, `DELTA:CHANGED` replace scenario with same name, `DELTA:REMOVED` delete scenario with same name — and names neither the Background nor the feature-description line. Confirmed empirically: a minimal throwaway project whose delta wrapped a rewritten Background in `DELTA:CHANGED` and changed the description line recorded with **both discarded**, no warning, and `speq feature validate` passing. Re-confirmed on the real files: after `speq record`, `specs/vs-adapter/pushdown-filter/spec.md` keeps the library's MySQL-only description line and all 12 MySQL-only Background bullets.

  The merged `pushdown-filter` therefore reads:

  - Description line: "Delegates a query's `WHERE` clause **to MySQL**, rendering each predicate so **MySQL** selects exactly the rows Exasol would have selected itself."
  - Background bullet 4: "A character-typed operand is therefore rendered with an explicit `COLLATE utf8mb4_0900_as_cs` suffix."
  - Background bullet 5: "`utf8mb4_0900_as_cs` is the collation of **every equality question this release delegates**."
  - Background bullet 6: "A string literal is rendered as `_utf8mb4 X'<hex>'`."
  - Scenarios: dialect-neutral, deferring collation placement and literal form to `vs-adapter/mysql-collation`, `postgresql-collation`, `mysql-pushdown`, and `postgresql-pushdown`.

  This is round 5's BLOCKER with the halves swapped. Round 5 found MySQL rules in the scenarios under a framework Background; the fix corrected the scenarios, and the Background stays MySQL-only. `vs-adapter/postgresql-collation` requires no suffix on `predicate_equal`, `predicate_in_constlist`, or `predicate_like`, so a framework Background asserting `COLLATE utf8mb4_0900_as_cs` for every delegated equality question contradicts it exactly as the scenarios did. An implementer who reads the Background — the part of a spec file that states what the feature owns — writes the renderer round 5 warned about.

  The scope is not one file. Eleven of the twenty-two deltas target a feature that already exists in the library, and **every one of them has its Background rewrite dropped**; eight also lose a rewritten description line:

  | Feature | Background bullets library → delta | Description line |
  |---|---|---|
  | `vs-adapter/pushdown-filter` | 12 → 12 | rewritten, dropped |
  | `vs-adapter/pushdown-aggregation` | 23 → 19 | rewritten, dropped |
  | `vs-adapter/pushdown-dialect` | 17 → 21 | unchanged |
  | `vs-adapter/pushdown-join` | 8 → 10 | rewritten, dropped |
  | `vs-adapter/pushdown-projection` | 8 → 9 | rewritten, dropped |
  | `vs-adapter/type-mapping` | 8 → 8 | rewritten, dropped |
  | `vs-adapter/create-virtual-schema` | 10 → 11 | rewritten, dropped |
  | `vs-adapter/adapter-request-dispatch` | 5 → 6 | unchanged |
  | `vs-adapter/result-ordering` | 7 → 9 | unchanged |
  | `scan-udf/arrow-scan` | 9 → 12 | rewritten, dropped |
  | `driver-provisioning/driver-self-install` | 11 → 15 | rewritten, dropped |

  The eleven new features are unaffected — no library file exists, so `/speq:spec-merge` copies the delta whole and their Backgrounds land intact. That asymmetry is what makes the defect invisible: the PostgreSQL and MySQL technology features read correctly after recording, and only the framework features they defer to keep saying MySQL.

  This voids a load-bearing claim in round 5's own Intent Fidelity certification — "every framework Background rewritten to name what it no longer owns" — and in decision `[25]`. The Background rewrite *is* the restructuring's mechanism for stating what each framework feature no longer owns. None of it merges. Neither `/speq:spec-merge`, nor `recorder-agent.md`, nor `/speq:record` mentions the `speq record` CLI or any step that carries a Background across, so delivery is discarded on the CLI path and undefined on the agent path.

- Fix: Give the Background and description rewrites an explicit delivery step, since no delta marker provides one.
  (a) Add a `### Recording notes` subsection to plan.md § Verification stating that `/speq:spec-merge`'s marker table merges scenarios only, and instructing `/speq:record` that for the eleven features tabulated above the recorder MUST replace `specs/<domain>/<feature>/spec.md`'s feature-description line and its entire `## Background` block with the delta's before stripping markers. Enumerate the eleven features by path so the step is checkable rather than general advice.
  (b) Add a checklist item to plan.md § Verification § Checklist asserting the post-record state directly: `specs/vs-adapter/pushdown-filter/spec.md` contains no occurrence of `utf8mb4`, `_utf8mb4 X'`, or the word `MySQL` in its description line or Background, and the same for the other ten. A grep is enough and it fails loudly if the recorder skips step (a).
  (c) Record the mechanism in `decision-log.md` as a `[plan-review]` entry so a later plan that restructures spec files does not rediscover it — the marker vocabulary has no Background verb, and any plan that rewrites one needs this same explicit step.
  (d) Re-verify by simulation rather than by reading: copy the eleven library files and their deltas into a throwaway project, apply the recording notes, and confirm the merged Backgrounds carry the delta text. The scenario-level merge is already verified correct and needs no rework.

## Task Breakdown

No objection — axis checked. Tasks 4.2 and 4.5 both exist under the current numbering, both own the work the `[plan-review]` entry "Three surviving pushdown-filter scenarios stated MySQL's collation and literal form as framework `MUST`s" assigns them, and the six test names they enumerate match the five coverage rows exactly. Task 4.5's instruction that the three MySQL tests "MUST NOT be renamed" is the right guard: they are what keeps the removed `pushdown-filter` rules asserted, and a rename would silently orphan three coverage rows. No new task is needed for the MySQL arms — all three tests ship today and already assert every step the two MySQL features now state, which I confirmed against `mysql_dialect_tests.rs`. The fix added no task and removed none; § Parallelization Group E (`4.1 → 4.2 → 4.3 → 4.4 → 4.5`) is unaffected.

## Prose Quality

#### [PROSE_BLOAT] ADVISORY

- Location: `specs/_plans/add-postgresql-support/vs-adapter/mysql-pushdown/spec.md:70`, the step added to "String literal is rendered as a hex literal".

- Issue: absorbing the pattern rule into that scenario pushed it from three `AND` steps to four, adding a validator warning that did not exist in round 5. `speq plan validate` reported 9 AND-step warnings in round 5 and reports 10 now; the new one is `vs-adapter/mysql-pushdown/spec.md: Scenario 'String literal is rendered as a hex literal' has 4 AND steps (recommended: 3 or fewer)`. The step itself carries two rules — the pattern renders in hex, and so does the `escapeChar` — joined by "and so MUST", which is what makes it long enough to notice. Absorption was the right call over opening a new scenario, since a new one would have put `mysql-pushdown` at 11 and added a sixth over-threshold file; the cost is this warning. Advisory, not a blocker: the guideline is a recommendation, the validator passes, and ten of these warnings are already accepted across the plan.

- Fix: Split the step in two — keep "the `LIKE` pattern MUST be rendered in that same hex form, because a pattern is a string literal and no rendering position exempts one" as the fourth step, and move the `escapeChar` clause into the `mysql-collation` scenario's sibling position or drop it, since `mysql_dialect_tests.rs:261` already asserts it and `mysql-pushdown`'s Background bullet 13 already states the general rule it follows from. Alternatively accept the warning and note it beside the other nine.

Otherwise no objection — axis checked. The three replacement scenarios and the new `mysql-collation` scenario each lead with the rule and follow with the reason, and each names the feature that answers the question it declines to answer rather than leaving the reader to search. That entry's Finding bullet states the conflict before the fix and quotes the contradicting requirements, which is the BLUF shape the guardrails ask for.

**Carried forward, not re-raised.** That entry's "Direction change" bullet is a single ~350-word paragraph containing several sentences over the 25-word cap. Every entry in this decision log is written at that density and prior rounds accepted them, so flagging this one alone would be arbitrary; not counted in this round's totals. The round-1 `[PROSE_BLOAT]` on plan.md § Summary also stands unchanged and uncounted, as in rounds 2 through 5.
