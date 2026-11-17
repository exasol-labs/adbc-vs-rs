# Plan Review Findings: add-pushdown-scalar-functions

## Summary
- Axes checked: 4/4
- Total findings: 12 (Blockers: 6, Findings: 6)
- Intent Fidelity blockers: 1

## Intent Fidelity

#### [SCOPE_REDUCTION] BLOCKER
- Location: plan.md § Summary, § Context; vs-adapter/pushdown-aggregation-delegation/spec.md § Scenario: Grouping by a scalar expression is delegated by every dialect
- Issue: Issue #21 names as its problem that "grouping by `YEAR()`/`MONTH()` fetched 1,000,000 rows (PG VS: 33)". The new scenario delegates the two-key query only "under the Trino dialect", because the MySQL and PostgreSQL dialects keep withholding `AGGREGATE_GROUP_BY_TUPLE` (ADR `group-by-tuple-caps-column-count`). The motivating PostgreSQL query therefore still reads every row, while plan.md § Summary states "so a query using them no longer reads the full table". The interview never asked about this, and no direction or `Follow-up plans` entry records it. The same ADR records that every dialect already renders N grouping columns, so advertising the capability costs no renderer work.
- Fix: Either add `AGGREGATE_GROUP_BY_TUPLE` to the MySQL and PostgreSQL sets in this plan, with an `ADR: yes` direction that supersedes `group-by-tuple-caps-column-count`, and change the scenario to require both `GROUP BY` keys in every dialect; or add a `Follow-up plans` entry in plan.md for the tuple widening, add a direction stating that the issue's two-key query stays undelegated for MySQL and PostgreSQL, and correct the § Summary sentence to the one-key case. Ask the user which.

Other intent checks: no objection. `CAST` is deferred by the interview with a named follow-up plan; `AGGREGATE_GROUP_BY_EXPRESSION`, dropping the `+ 0` route, MySQL delegation, and the `TIMESTAMP` difference error all trace to interview answers. No scope creep found: every feature in plan.md § Features traces to the eleven functions or the grouping capability.

## Feasibility

#### [UNSTATED_ASSUMPTION] FINDINGS
- Location: intent.md § Directions D1, D2; vs-adapter/mysql-pushdown/spec.md and vs-adapter/trino-pushdown/spec.md § Scalar functions render ...
- Issue: D2 renders exact operands "through the target's widest exact decimal (MySQL `DECIMAL(65,s)`, ... Trino `DECIMAL(38,s)`)", but D1 derives only a "result type class", not a precision or scale. The request carries no type for a nested node, so the scale `s` of a nested operand such as `("a" * "b") * "c"`, a literal, or a `SUM` operand expression is never defined. MySQL also caps `DECIMAL` scale at 30, while an Exasol product of two scale-18 operands has scale 36.
- Fix: Extend D1 so the query model derives a scale for every exact-numeric expression (Exasol's rule: `+` and `-` take the larger scale, `*` adds scales), state the MySQL cap of 30 and what happens past it (render the operand without the cast, or fail with a named error), and add one nested-product case to the exact-arithmetic unit tests in plan.md § Verification.

#### [UNSTATED_ASSUMPTION] FINDINGS
- Location: intent.md § Directions D7; plan.md § Impact ("`docs/pushdown.md` drops that advice and states no replacement")
- Issue: D7 asserts a floating-point `SUM` "keeps no documented Exasol-side route". This plan withholds `FN_CAST`, `FN_DIV`, and `FN_FLOAT_DIV`, so `SUM(CAST("price" AS DOUBLE))` or `SUM("price" / 1)` still makes Exasol compute the sum. The same holds for the MySQL pre-8.0.17 `AVG` advice. The assumption drives a user-facing loss of documented behavior and the floating-point half of the D7 ADR.
- Fix: Verify on the test Exasol that `SUM("price" / 1)` (or `CAST`) is not delegated, then change D7 and plan.md § Impact so `docs/pushdown.md` names that wrapper as the replacement route, and drop the floating-point `SUM` part from the D7 ADR if the route holds.

## Requirement Quality

#### [COMPLETENESS_GAP] BLOCKER
- Location: vs-adapter/pushdown-scalar-function/spec.md (all row-level scenarios); intent.md § Directions D1, D3
- Issue: plan.md § Context states "each advertised function must render for every operand type Exasol admits for it", and ADR `delegate-sum-avg-and-let-floating-point-sum-follow-source-order` forbids refusing one operand type. Exasol converts implicitly, so it admits `LENGTH`, `UPPER`, `SUBSTR` over a `DECIMAL`, `DATE`, `TIMESTAMP`, or `BOOLEAN` column, `YEAR` over a `VARCHAR` column holding a date, and `+` over a `BOOLEAN` column. No scenario or direction states how these render. PostgreSQL defines no `char_length(numeric)` and no `EXTRACT` over `text`, so the source fails; where it succeeds, the text form of a number or date differs from Exasol's (`LENGTH` of a `DECIMAL(18,2)` value, a date under `NLS_DATE_FORMAT`). D3 covers only a character operand of arithmetic.
- Fix: Add a direction that lists, per function, every declared operand type Exasol admits (measure it on the test Exasol, as the `SUM` operand scenario did) and the conversion each dialect renders for it; add a scenario in vs-adapter/pushdown-scalar-function/spec.md asserting mirror-equal rows for a string function over a `DECIMAL` and a `DATE` column, a date part over a `VARCHAR` column, and `+` over a `BOOLEAN` column; and add those arms to `every_advertised_capability_renders_per_dialect` in plan.md § Verification. Where a conversion cannot match Exasol, withhold the function rather than refuse the operand type.

#### [COMPLETENESS_GAP] BLOCKER
- Location: intent.md § Directions D6; vs-adapter/pushdown-scalar-function/spec.md § Scenario: String function results follow the source for padding, case mapping, and the empty string
- Issue: D6 covers the empty string only "in a filter". With `AGGREGATE_GROUP_BY_EXPRESSION` advertised, `GROUP BY TRIM("c")` over `'  '` and a null makes the source return two groups, `''` and null, which Exasol receives as two null-key rows where Exasol's own query returns one. `COUNT(DISTINCT TRIM("c"))` counts `''` as a value, and `SUBSTR("c", 10) IS NULL` matches at Exasol and not at the source. None is stated or documented. Mapping an empty character result to null at the source (for example `CASE WHEN char_length(r) = 0 THEN NULL ELSE r END`, or `NULLIF(r, '')` where no PAD SPACE collation applies) removes all of these differences in every position.
- Fix: Either change D6 so every dialect renders an empty character result of `TRIM`, `SUBSTR`, `UPPER`, and `LOWER` as null, and replace the empty-string clause of the scenario with a mirror-equal assertion for a grouping key, a `DISTINCT` operand, and an `IS NULL` filter; or keep D6 and add scenarios plus `docs/pushdown.md` entries for the grouping, `DISTINCT`, and `IS NULL` differences.

#### [COMPLETENESS_GAP] BLOCKER
- Location: vs-adapter/pushdown-scalar-function/spec.md § Scenario: A select-list TIMESTAMP difference fails with a named error; vs-adapter/mysql-pushdown/spec.md, postgresql-pushdown/spec.md, trino-pushdown/spec.md § Scalar functions render ...
- Issue: The scenario requires "a `WHERE` comparison between two such differences MUST render rather than fail", but no dialect scenario states how `TIMESTAMP - TIMESTAMP` renders, and no row-level scenario checks it. MySQL's `-` over two timestamps yields a `YYYYMMDDhhmmss` number difference, so a bare rendering compares wrong values. The plan also leaves out `DATE - TIMESTAMP`, `TIMESTAMP - DATE`, and `ADD` with an interval-typed operand such as `"ts" + ("t1" - "t2")`, all of which Exasol can delegate once `FN_ADD` and `FN_SUB` are advertised.
- Fix: Add to each `<technology>-pushdown` scenario the rendering of a `TIMESTAMP` difference (MySQL through `TIMESTAMPDIFF(MICROSECOND, ...)` or equivalent), state in vs-adapter/pushdown-scalar-function/spec.md the result for mixed `DATE`/`TIMESTAMP` subtraction and for an interval operand of `ADD`, and add a mirror-equal E2E assertion for a `WHERE` comparison of two differences to plan.md § Verification.

#### [COMPLETENESS_GAP] BLOCKER
- Location: intent.md § Directions D5; vs-adapter/mysql-pushdown/spec.md § Scenario: Scalar functions render with MySQL's character and day semantics
- Issue: MySQL's `REGEXP_REPLACE` without a `match_type` matches case-insensitively under a `_ci` collation, and the default `utf8mb4_0900_ai_ci` is one. `TRIM('xXax', 'x')` returns `Xa` in Exasol and `a` at MySQL. The interview's "follow the source" covers `CHAR` padding and case mapping, not set membership, and vs-adapter/pushdown-scalar-function/spec.md requires two-argument `TRIM` to "remove every leading and trailing character contained in its second argument, as Exasol does".
- Fix: State in D5 and the MySQL scenario that the `REGEXP_REPLACE` renders with `match_type` `'c'` (or an equivalent binary comparison), and add a mixed-case operand to the two-argument `TRIM` case of `string_functions_reach_<source>_and_match_the_mirror`.

#### [COMPLETENESS_GAP] FINDINGS
- Location: vs-adapter/pushdown-scalar-function/spec.md § Scenario: Date and timestamp arithmetic follows Exasol's day arithmetic, § Scenario: SUBSTR positions follow Exasol's rule
- Issue: The date scenario tests `"dt" + 1` but no fractional number added to a `DATE`. MySQL rounds `INTERVAL 1.5 DAY` to 2 days, and PostgreSQL defines no `date + numeric`, so each dialect needs a stated rule that matches Exasol's. Neither scenario has a null operand, while issue #21's acceptance is "rows match a local mirror table, including NULL and edge values" per function.
- Fix: Add `"dt" + 1.5` and `"dt" + "n"` with a fractional `"n"` to the date scenario, state Exasol's measured result for them in the per-dialect scenarios, and add a null row to the date-arithmetic and `SUBSTR` scenarios.

#### [AMBIGUOUS_REQUIREMENT] FINDINGS
- Location: plan.md § Verification § Scenario Coverage; vs-adapter/mysql-pushdown/spec.md
- Issue: Tests in `crates/adbc-vs-adapter/tests/e2e_*_test.rs` are labeled "Integration", but AGENTS.md defines integration tiers as ADBC-only without Exasol; these need `make test-e2e*`. `speq plan validate` warns that the MySQL scalar-function scenario has 4 `AND` steps.
- Fix: Relabel those rows as `E2E` in § Scenario Coverage, and split the MySQL scenario into a date-arithmetic scenario and a string-function scenario.

## Feasibility and Requirement Quality: other checks

No objection on the Trino entry count: `trino.toml` states 40 entries today, and 11 `FN_` entries plus `AGGREGATE_GROUP_BY_EXPRESSION` give the 52 the delta states. No objection on Background drift: the changed Backgrounds in trino-pushdown and pushdown-aggregation stay within 5 bullets, and each line feeds a scenario. Every existing spec line that withholds `AGGREGATE_GROUP_BY_EXPRESSION` or states that no dialect advertises a scalar function (mysql-pushdown, postgresql-pushdown, trino-pushdown, pushdown-dialect, pushdown-aggregation-delegation) has a matching CHANGED or REMOVED block.

## Design Depth

#### [ADR_CONFLICT] BLOCKER
- Location: intent.md § Directions D5, D7
- Issue: ADR `capability-negotiation-all-or-nothing` states: "an aggregation that cannot be safely and fully pushed down under a capability means the capability is not advertised, rather than advertised broadly and failed narrowly at render time for a subset of operand types." D7 fails `FN_SUB` for the `TIMESTAMP`-`TIMESTAMP` operand pair, and D5 fails `FN_TRIM` for a non-literal character set at MySQL. D7 supersedes only `delegate-sum-avg-and-let-floating-point-sum-follow-source-order`, and its text covers the `TIMESTAMP` difference but not the MySQL `TRIM` refusal. D5 has no `ADR: yes` and no supersede.
- Fix: Add `capability-negotiation-all-or-nothing` to D7's supersede list (or state that D7's ADR narrows its principle), and widen D7's decision to name the MySQL non-literal `TRIM` set as a second permitted render-time refusal, or withhold two-argument `TRIM` at MySQL instead.

#### [ADR_OVERPROMOTION] FINDINGS
- Location: intent.md § Directions D7
- Issue: D7 holds two decisions in one ADR: the select-list `TIMESTAMP` difference error, and the loss of the floating-point `SUM` route. ADR rule 5 allows one decision per ADR, and the second is a consequence of advertising `FN_ADD`. Superseding `delegate-sum-avg-and-let-floating-point-sum-follow-source-order` wholesale also retires its still-valid decision that every dialect advertises `FN_AGG_SUM`, `FN_AGG_SUM_DISTINCT`, `FN_AGG_AVG`, and `FN_AGG_AVG_DISTINCT`.
- Fix: Reduce the D7 ADR to the render-time refusal decision, move the floating-point `SUM` route into its Consequences (or drop it, per the Feasibility finding), and state in the new ADR's Decision which parts of the superseded ADR it carries forward.

#### [DESIGN_VIOLATION] FINDINGS
- Location: intent.md § Directions D1, D4; architecture.md § Components (pushdown_query, sql_dialect)
- Issue: Information hiding. Exasol's semantics of the overloaded `ADD` and `SUB` (day arithmetic for a date, days between two dates, exact decimal for numbers) and its `SUBSTR` position rule are one design decision, but the plan has each of the three dialects re-derive them from the result type class and render its own normalization. A fix to Exasol's rule then needs three edits that can drift apart.
- Fix: Have pushdown_query lower each overloaded node into a dialect-neutral operation (for example add-days, days-between, exact add, double add, and a `SUBSTR` with Exasol-normalized start and length), state that in D1 and D4 and in the pushdown_query line of architecture.md § Components, so each dialect renders only the source syntax of an already-resolved operation.

Architecture drift: no objection — axis checked. The delta's BASE hash matches `git hash-object specs/architecture.md` (`cbf2746e...`), both blocks are CHANGED on canonical sections, the Components block copies every current component and changes only pushdown_query, the Constraints block copies the full section and adds the delegated-function and named-error constraints, and D7 names `§ Constraints`, which the delta holds.
