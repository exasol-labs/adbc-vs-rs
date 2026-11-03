# Plan Review Findings: change-type-mapping-coverage

## Summary
- Axes checked: 6/6
- Total findings: 7 (Blockers: 1, Findings: 6)
- Intent Fidelity blockers: 1

Scope of this pass: the PR feedback pass 2 changes (every delta Background shortened, its content moved into scenarios, decision [7] set to `Promotes to ADR: no`), compared against the permanent specs under `specs/` and against commit 7b6d8db. Decisions [1] and [6] still promote to ADR and need the user's confirmation; see Design Depth.

## How this plan may fail

1. Recording the plan deletes two framework rules that the code cites as spec rules: "no second capability list" (`vs-adapter/pushdown-dialect`) and "no literal can change the shape of the generated statement" (`vs-adapter/pushdown-filter`). A later change that breaks either then contradicts no recorded spec, and the Trino dialect loses its only statement of the literal guarantee. Routed to Intent Fidelity, `[SCOPE_REDUCTION]` BLOCKER.
2. Rationale and process statements moved out of the Backgrounds became MUST steps that no test can check. In one case, a PostgreSQL behavior the permanent spec records as only "expected" became a MUST step with no test. The verify phase then reports these scenarios as covered when some steps are unchecked. Routed to Requirement Quality, `[AMBIGUOUS_REQUIREMENT]`, and Feasibility, `[UNSTATED_ASSUMPTION]`.
3. The PR reviewer's "too long and verbose" objection now applies to the scenarios. Fourteen steps exceed 70 words, and several steps hold two or more requirements each. Routed to Prose Quality, `[PROSE_BLOAT]`.

## Intent Fidelity

No objection on the rest of this axis, checked: decision-log.md [7] now reads `Promotes to ADR: no` and its Rationale no longer names an ADR criterion (decision-log.md:65, :71). All 12 deltas that change their Background now have 3 to 5 one-line bullets. The other two deltas (`create-virtual-schema`, `postgresql-pushdown`) carry an unmarked copy that `speq record` ignores, so their Backgrounds do not change. For the other 10 changed Backgrounds, I compared every permanent bullet with the delta scenarios and the unchanged permanent scenarios: `type-mapping`, `pushdown-join`, `pushdown-aggregation`, `arrow-scan`, `postgresql-scan`, `mysql-collation`, `postgresql-collation`, and the PostgreSQL, MySQL, and Trino type mappings. Their content survives, apart from the case below and the provenance loss under Feasibility.

#### [SCOPE_REDUCTION] BLOCKER
- Location: `vs-adapter/pushdown-dialect` delta § Background; `vs-adapter/pushdown-filter` delta § Background and § Scenario "String, date, and timestamp literals render in a form no server setting can change"; `vs-adapter/pushdown-aggregation` delta § Scenarios "Aggregation is delegated only beside SELECTLIST_EXPRESSIONS" and "A multi-column GROUP BY is delegated only by the dialect advertising AGGREGATE_GROUP_BY_TUPLE"
- Issue: The PR comment asks to "Move requirements into scenarios", and the orchestrator's reading adds "with no loss of recorded content". Two requirements and two measured facts were dropped instead of moved:
  - The permanent `pushdown-dialect` Background says "Each dialect states its own set by hand, so no second capability list exists in the adapter's production modules." No delta scenario or Background keeps it. Code cites it as a rule of this feature: `crates/adbc-vs-scan/tests/driver_toml_test.rs:273-274` ("the second capability list `vs-adapter/pushdown-dialect` forbids"), `crates/adbc-vs-adapter/src/capabilities_tests.rs:287` and `:345`, and `crates/adbc-vs-scan/build.rs:22` and `:68`.
  - The permanent `pushdown-filter` Background says a string literal's form "is rendered unconditionally rather than only for a value that needs it, so no literal can change the shape of the generated statement." The new scenario keeps "for every string literal rather than only for one that needs it" and drops the shape guarantee. `mysql_dialect_tests.rs:334` and `postgres_dialect_tests.rs:440` cite it as "`vs-adapter/pushdown-filter` requires that no literal can change the shape of the generated statement". `mysql-pushdown` and `postgresql-pushdown` restate it, but no Trino feature does, so after recording no spec states the guarantee for the Trino dialect.
  - Measured facts in the permanent `pushdown-dialect` Background that no delta keeps: "adding `AGGREGATE_GROUP_BY_TUPLE` delegated both a two-column and a three-column `GROUP BY` in request order", and "`SELECTLIST_EXPRESSIONS` without the aggregate capabilities did not" delegate the aggregation, measured "with `AGGREGATE_SINGLE_GROUP`, `AGGREGATE_GROUP_BY_COLUMN`, and nine `FN_AGG_*` advertised".
- Fix:
  - In `vs-adapter/pushdown-dialect/spec.md`, add a DELTA:NEW scenario "Each dialect's capability set is stated once". GIVEN the adapter's and the scan crate's production modules; WHEN each dialect's advertised capability set is read; THEN it MUST come from that technology's driver declaration alone; AND no production module, the build script included, MUST hold a second capability list, such as a list of valid capability names. Add a Scenario Coverage row in plan.md that maps it to `advertised_set_matches_the_rendered_fragments` (`capabilities_tests.rs:174`) and `a_declaration_with_no_capability_list_names_the_file_and_the_field` (`crates/adbc-vs-scan/tests/driver_toml_test.rs:278`).
  - In `vs-adapter/pushdown-filter/spec.md`, add a third AND step to "String, date, and timestamp literals render in a form no server setting can change": "* *AND* no literal value MUST change the shape of the generated statement, in any dialect".
  - In `vs-adapter/pushdown-aggregation/spec.md`, add both measured facts to the measurement clauses of the two scenarios named in Location.
  - Re-check every permanent Background bullet of the 12 changed deltas once more. In plan.md § Verification, list every statement the revision drops on purpose.

## Feasibility

No objection on the rest of this axis, checked: this pass changes no task order, gate, or dependency. Tasks 2.1 and 2.2 still gate all later work. The test extensions the pass adds to tasks 2.9 and 3.6 target tests that exist at the cited lines: `e2e_postgres_test.rs:419`, `:456`, `:504`, `:543`; `e2e_mysql_test.rs:1405`, `:1823`; `mysql_dialect_tests.rs:64`. The removal targets in Dead Code Removal also exist at their cited lines: `mysql_source_tests.rs:90`, `:108`; `mysql_type_mapping_tests.rs:123`, `:137`; `postgres_source_tests.rs:176`.

#### [UNSTATED_ASSUMPTION] FINDINGS
- Location: `vs-adapter/pushdown-filter` delta § Scenario "LIKE renders its pattern and its escape character", AND step 1; plan.md § Scenario Coverage rows for that scenario
- Issue: The permanent Background records the backslash rule as "verified on the test Exasol and the test MySQL and expected of PostgreSQL by its documented default". The delta turns it into "a backslash MUST escape the next character when the pattern is delegated to MySQL or PostgreSQL". The coverage rows name only rendering unit tests (`like_renders_pattern_and_escape`, `like_renders_its_escape_clause`, `like_renders_its_pattern_with_no_escape_clause`). No E2E test in `tests/e2e_*_test.rs` runs a pattern that holds a backslash. The step therefore asserts unmeasured PostgreSQL behavior, and neither source is checked by a test.
- Fix: In tasks 3.6 and 2.9, extend `filter_matches_local_exasol_result` (`e2e_mysql_test.rs:1250`) and `filter_reaches_postgresql_and_matches_the_mirror` (`e2e_postgres_test.rs:176`). Each runs a `LIKE` whose pattern escapes `%` with a backslash, over a fixture value that holds a literal `%`, and compares the rows with the local Exasol result. Add both tests as coverage rows for this scenario. Alternatively, keep the permanent wording "expected of PostgreSQL by its documented default" and remove the MUST from the PostgreSQL half of the step.

## Requirement Quality

No objection on the rest of this axis, checked: `speq plan validate change-type-mapping-coverage` passes. Every scenario in all 14 deltas has one GIVEN, one WHEN, one THEN, and at most 3 AND steps. Every THEN and AND step carries an RFC 2119 keyword, and no GIVEN or WHEN step carries one. Every scenario sits inside a DELTA marker. Every CHANGED and REMOVED scenario name exists in its permanent spec, and no NEW scenario name does.

#### [AMBIGUOUS_REQUIREMENT] FINDINGS
- Location: `vs-adapter/type-mapping` delta § Scenario "Each technology states its own mapping table", AND steps 2 and 3; `vs-adapter/pushdown-aggregation` delta § Scenario "Aggregation is delegated only beside SELECTLIST_EXPRESSIONS", AND step 1; `scan-udf/arrow-scan` delta § Scenario "One scan instance streams the whole result as Arrow IPC bytes", AND step 2; plan.md § Verification bullet 2
- Issue: The revision moved design intent, process rules, and mechanism out of the Backgrounds and into MUST steps that have no pass or fail test:
  - "the PostgreSQL and MySQL mappings MUST start from the Exasol JDBC Virtual Schema's mapping for the same source": no test can check "start from".
  - In the same step, "a source type with no Exasol counterpart MUST map to a `VARCHAR` ... except the types the driver's `<technology>-type-mapping` feature leaves unmapped": any omission satisfies this.
  - AND step 3 requires each feature to state "every departure from the JDBC Virtual Schema's mapping, the read expressions, the known differences": a rule about spec documents, not software behavior.
  - "a further dialect MUST follow the same rule without a new measurement against its source": a development-process rule.
  - "the scan MUST serialize each batch to Arrow IPC bytes inside the UDF": a mechanism.

  plan.md § Verification bullet 2 lists some untested moved statements but omits the first four.
- Fix:
  - In the type-mapping scenario, delete "the PostgreSQL and MySQL mappings MUST start from the Exasol JDBC Virtual Schema's mapping for the same source, and". The architecture delta § Constraints already states this. Reduce AND step 3 to the permanent wording: "which source types map, and why the rest are omitted, MUST be stated by each `<technology>-type-mapping` feature rather than here".
  - In the pushdown-aggregation scenario, turn AND step 1 into a "because" clause of the THEN step, or delete it.
  - In the arrow-scan scenario, turn AND step 2 into a "because" clause of the THEN step.
  - Make plan.md § Verification bullet 2 list every remaining step that no test checks.

#### [COMPLETENESS_GAP] FINDINGS
- Location: `vs-adapter/mysql-type-mapping` delta § Scenario "Character types preserve the declared length and use UTF8"; plan.md task 3.2
- Issue: The rule "a declared length of 0 MUST map to size 1" now sits inside the `VARCHAR` THEN step. MySQL `CHAR` now maps to Exasol `CHAR`, and MySQL accepts `CHAR(0)`. No step says that `CHAR(0)` maps to `CHAR(1)`, and task 3.2 says "`CHAR(n)` to `CHAR(n)`". Exasol rejects a `CHAR(0)` declaration, so one such column would fail the whole schema. That breaks the `vs-adapter/type-mapping` rule that one unsupported column never blocks a schema.
- Fix: In the AND step for the `CHAR` column, add "and a `CHAR` column with declared length 0 MUST map to `CHAR` with size 1". In task 3.2, name the unit test `character_types_preserve_the_declared_length` as the test that covers the length-0 `CHAR` case.

## Task Breakdown

No objection, axis checked: the plan.md Scenario Coverage table and the NEW and CHANGED delta scenarios match one to one, with no row missing on either side (`comm` over both lists). There are 181 test rows. Each named test either exists today at its stated location (checked with `grep "fn <name>"`) or is created by a task. Tasks 1.1 to 1.7, 2.1, 2.2, 2.6, 3.1, 3.3, 3.6, 4.1, and 4.2 name such tests explicitly. Tasks 2.4, 2.9, 3.2, and 3.6 cover the rest through "named in the Scenario Coverage table". Groups A and C share the `pushdown-aggregation` and `mysql-collation` deltas in their Knowledge columns. C depends on A, so the overlap is sequenced. The BLOCKER fix above adds one scenario that needs a coverage row; its Fix line names the tests.

## Design Depth

No objection on the rest of this axis, checked:
- `[ARCHITECTURE_DRIFT]`: the delta's BASE equals `git hash-object specs/architecture.md` (`31874c94...`), and this pass does not change `architecture.md`. [1] (§ Interfaces) and [6] (§ Components, § Data Flow, § Interfaces, § Constraints) name sections the delta carries. [7]'s § Constraints line still matches a delta block and is harmless on a `no` entry.
- `[ADR_CONFLICT]`: no new conflict. One pre-existing ADR inconsistency exists outside this plan: `trino-set-is-postgresql-without-like-escape-plus-group-by-tuple` advertises `AGGREGATE_GROUP_BY_TUPLE` for Trino, while `group-by-tuple-caps-column-count` says "Keep `AGGREGATE_GROUP_BY_TUPLE` withheld", and no `Supersedes:` line connects them. The new pushdown-aggregation scenario restates what the permanent Background already records, which follows the newer ADR. Resolve this with `/speq:audit`, not in this plan.
- [1] must stay `yes`, because it supersedes `advertise-selectlist-expressions-with-boolean-emits` (rule 5).

#### [ADR_OVERPROMOTION] FINDINGS
- Location: decision-log.md § Design Decisions [6]
- Issue: `/speq:adr-rules` rule 3 excludes "a statement that already lives in a feature spec scenario". [6]'s Decision ("Each mapped column's own `adapterNotes` record its source type ... the dialect renders every reference to the column through it ... A column without a recorded source type renders bare") now restates two scenarios: `vs-adapter/type-mapping` "A mapped column records its source type in its adapterNotes", to which this pass added "the recorded source type MUST select the read expression each dialect renders", and `vs-adapter/pushdown-dialect` "A column reference renders through its read expression in every position". The part of [6] that a scenario does not state is the rejected alternatives: converting in the select list only, as the JDBC PostgreSQL dialect does, and keying on the declared type ([2]).
- Fix: Rewrite [6]'s Decision to the choice the scenarios leave out, for example: "A converted column is converted in every position it is referenced, keyed on its recorded source type, rather than in the select list only or keyed on its declared Exasol type." Keep its Rationale and Alternatives. If the user declines the ADR when confirming [1] and [6], set [6] to `Promotes to ADR: no`.

## Prose Quality

#### [PROSE_BLOAT] FINDINGS
- Location: `vs-adapter/pushdown-filter` delta § "A delegated predicate follows the source's comparison rules" AND step 3 (110 words); `vs-adapter/pushdown-dialect` § "Every advertised capability has a rendering path" AND step 3 (106); `vs-adapter/trino-type-mapping` § "No source-server precondition is checked for Trino" AND step 1 (105); `vs-adapter/type-mapping` § "Each technology states its own mapping table" AND step 1 (99); `vs-adapter/pushdown-join` § "Aliases derive from table position, never from the request" THEN (95); 9 more steps of 70 to 89 words
- Issue: The PR comment says the Background "is too long and verbose". The revision moved that text into scenario steps almost word for word, so the same length now sits in the scenarios. Fourteen steps exceed 70 words. Several steps join two or more requirements: pushdown-filter AND step 3 holds a measurement and two MUST outcomes, and postgresql-type-mapping "character maps to CHAR" AND step 1 joins two MUSTs with a semicolon. Joining requirements keeps the AND count at 3, but it breaks writing-guardrails rule 2 (one idea per sentence), and it hides which test checks which outcome.
- Fix: Give each step one MUST outcome and at most one short "because" clause. Where a scenario runs out of AND steps, split it. For example, move the `CHAR` comparison outcomes out of the pushdown-filter AND step 3 into a DELTA:NEW scenario "A CHAR column compares as the source compares fixed-width values", with the measured Exasol values in its GIVEN and one THEN or AND step per comparison kind. Split the two MUSTs of the postgresql-type-mapping step into two AND steps, and move the `> 2000000` case into the THEN step.

#### [PROSE_UNCLEAR] FINDINGS
- Location: `vs-adapter/pushdown-aggregation` delta § Background bullet 2; `vs-adapter/mysql-collation` delta § Scenario "A select-list predicate over the grouped character column is delegated", AND step 1
- Issue:
  - The pushdown-aggregation Background says "`vs-adapter/pushdown-dialect` owns `SELECTLIST_EXPRESSIONS`". After this pass, the requirement to advertise it beside the aggregate capabilities lives in this feature's own scenario "Aggregation is delegated only beside SELECTLIST_EXPRESSIONS". The pushdown-dialect delta mentions it only inside a "because" clause of "BOOLEAN is declarable in the EMITS clause".
  - The mysql-collation step says a converted key "renders in the select list as `ANY_VALUE(<read expression>)`, which MySQL exempts from that check because it matches a select-list expression against a grouping expression only whole". The "because" clause explains why a converted key needs `ANY_VALUE`, not why MySQL exempts `ANY_VALUE`. The scenario's GIVEN is a `VARCHAR` column, which always renders bare, so the `ANY_VALUE` clause cannot apply to it.
- Fix:
  - In the pushdown-aggregation Background bullet 2, replace "owns `SELECTLIST_EXPRESSIONS`, the `BOOLEAN` declared type, and the all-or-nothing negotiation rule" with "owns the `BOOLEAN` declared type and the all-or-nothing negotiation rule".
  - In the mysql-collation AND step 1, delete the clause from "while a key read through a conversion" to the end of the step. The `vs-adapter/mysql-type-mapping` scenario "A select-list predicate over a grouped converted column is delegated" already states the `ANY_VALUE` rule.
