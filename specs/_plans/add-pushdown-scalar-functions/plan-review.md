# Plan Review Findings: add-pushdown-scalar-functions

## Summary
- Axes checked: 4/4
- Total findings: 7 (Blockers: 1, Findings: 6)
- Intent Fidelity blockers: 0

## Intent Fidelity

No objection, axis checked. The latest user revision ("compute it in Exasol around the scan, no breaking change, no ADR") is carried by D7, the scenario "A select-list TIMESTAMP difference is converted to an interval in Exasol", the CHANGED pushdown-declared-types scenario, and the Components, Data Flow, and Constraints blocks of the delta. No direction is `ADR: yes`, and the former render-time refusal is gone from every artifact. The issue's two-key `GROUP BY YEAR(), MONTH()` gap is recorded in D6 and in `Follow-up plans` (`change-pushdown-group-by-tuple`), and `CAST` in `add-pushdown-cast`, both agreed in the interview. The interview line "TIMESTAMP minus TIMESTAMP ... fail the select-list case with a named error, new ADR" stays in `## Interview`, but the later line "Revisit ... → Compute it in Exasol around the scan" overrides it, and no other artifact follows the earlier answer. No scope creep: every feature in plan.md § Features traces to the eleven functions, grouping by an expression, or the interval transport.

## Feasibility

#### [UNSTATED_ASSUMPTION] FINDINGS
- Location: intent.md § Directions D7; plan.md § Manual Testing (pushdown-dialect row)
- Issue: D7 rests on two Exasol behaviors no artifact states as measured: that Exasol accepts a `pushdown` response whose `sql` wraps the scan call in an outer `SELECT ... FROM (SELECT ADBC_VS.ADBC_SCAN(...) EMITS (...) FROM DUAL)`, today it receives only the bare call (`crates/adbc-vs-adapter/src/pushdown.rs:80`), and that `NUMTODSINTERVAL(<seconds>, 'SECOND')` keeps the fractional seconds the scenario's "a fraction of a second apart" row needs before the cast to the declared `INTERVAL DAY(p) TO SECOND(f)`. If either fails, the select-list difference breaks, which the user revision forbids.
- Fix: Add to D7 that the implementation measures both on the test Exasol before the dialect work: an outer-`SELECT` response in a single-table query and in a join, and `NUMTODSINTERVAL` over a seconds value with six fractional digits cast to the declared type. State the fallback if the outer `SELECT` is refused (withhold `FN_SUB` is not acceptable under the revision, so name the alternative, such as `NUMTODSINTERVAL` over a second scan column).

#### [UNSTATED_ASSUMPTION] FINDINGS
- Location: vs-adapter/pushdown-aggregation-delegation/spec.md § Scenario: Grouping by a scalar expression is delegated by every dialect; plan.md § Dead Code Removal (`any_value_keys` row)
- Issue: The scenario requires only that "the select-list occurrence of a grouped expression MUST render as the same text as its `GROUP BY` key". Exasol can send a select-list entry that wraps a grouped key, such as `YEAR("d") + 1 ... GROUP BY YEAR("d")`, and under D7 a grouped interval key whose select-list occurrence the source wraps in a seconds conversion. MySQL's `ONLY_FULL_GROUP_BY` matches a select-list expression to a `GROUP BY` expression by structure, and whether it accepts an expression that contains the key is not stated or measured. The `any_value_keys` logic (`mysql_dialect.rs:238`) matches column read expressions only.
- Fix: Add to the scenario a `SELECT YEAR("d") + 1, COUNT(*) ... GROUP BY YEAR("d")` query and a grouped `TIMESTAMP` difference under the MySQL dialect, each mirror-equal, and state in D1 or a new direction how the MySQL dialect renders an expression over a grouped key if MySQL refuses it.

No other feasibility objection: no new dependency, the Trino count holds (`trino.toml` and `postgresql.toml` list 40 entries today, plus 11 `FN_` entries and `AGGREGATE_GROUP_BY_EXPRESSION` gives the 52 the trino-pushdown delta states), and D8 carries the measurement that `/ 1` stays undelegated.

## Requirement Quality

#### [COMPLETENESS_GAP] BLOCKER
- Location: intent.md § Directions D5, D7; vs-adapter/postgresql-pushdown/spec.md and vs-adapter/trino-pushdown/spec.md § Scalar functions render ...; vs-adapter/mysql-pushdown/spec.md § Arithmetic renders with MySQL's day and decimal semantics; vs-adapter/pushdown-scalar-function/spec.md § A select-list TIMESTAMP difference is converted to an interval in Exasol
- Issue: The scan must emit "a `DECIMAL` count of seconds the source computes", but every dialect scenario renders a `TIMESTAMP` difference as an interval value: "PostgreSQL's `interval` subtraction", "Trino's `interval day to second` subtraction", and MySQL "`TIMESTAMPDIFF(MICROSECOND, ...)`", a microsecond count. No artifact states the source expression that turns the top-level select-list value of `"t1" - "t2"` or `MIN("t1" - "t2")` into seconds (for example `EXTRACT(EPOCH FROM ...)` at PostgreSQL, a microsecond count divided by 1000000 at MySQL, a millisecond-based form at Trino). The scan cannot do it: `coerce.rs` converts through Arrow `cast_with_options`, which has no cast from an interval type to `Decimal128`, so a PostgreSQL or Trino interval reaching the scan fails the coercion. The `DECIMAL(p,s)` the `EMITS` clause declares for the seconds is also unstated, which decides whether a fractional second survives (`s`) and whether a difference of "more than a year" fits (`p`), and no component owns that encoding.
- Fix: Add a direction stating that a select-list entry declared `INTERVAL DAY TO SECOND` renders, outermost, as the source's seconds count of the interval expression, and that its `EMITS` type is `DECIMAL(p,s)` derived from the declared interval's day precision and fraction (state the formula). Add the per-dialect seconds form to each `<technology>-pushdown` scalar-function scenario. Name in architecture.md § Components the one component that owns the interval-to-seconds encoding and its inverse (pushdown, with sql_dialect rendering only the source syntax), and add a unit test row in plan.md § Verification asserting each dialect's rendered seconds form and the declared `DECIMAL(p,s)`.

#### [COMPLETENESS_GAP] FINDINGS
- Location: intent.md § Directions D5; vs-adapter/pushdown-scalar-function/spec.md § A select-list TIMESTAMP difference is converted to an interval in Exasol
- Issue: Once `FN_ADD`, `FN_SUB`, and `FN_MULT` are advertised, Exasol may delegate every interval operand combination it admits, and ADR `delegate-sum-avg-and-let-floating-point-sum-follow-source-order` forbids refusing one. D5 covers "an interval operand of `ADD` or `SUB` adds that interval" to a timestamp, but not interval plus or minus interval, nor an interval times a number, if Exasol admits them. MySQL holds no interval value, so each needs a stated microsecond-count rendering.
- Fix: Measure on the test Exasol which `INTERVAL DAY TO SECOND` operand combinations `ADD`, `SUB`, and `MULT` admit, extend D5 with the rendering of each admitted one per dialect, and add those queries to the scenario's `WHEN` step.

#### [REQUIREMENT_CONFLICT] FINDINGS
- Location: architecture.md § Constraints ("A delegated `SUM` or `AVG` over a floating-point operand follows the source's accumulation order, and a source `NaN` makes it null"); intent.md § Directions D8
- Issue: The delta drops the clause of the current constraint "so a user who needs Exasol's value wraps the operand in an expression no dialect advertises, so that Exasol evaluates it". D8 and plan.md § Impact keep that route with `SUM("price" / 1)`, so the recorded architecture would lose a rule the plan still holds.
- Fix: Restore the dropped clause in the architecture.md § Constraints line, unchanged.

#### [AMBIGUOUS_REQUIREMENT] FINDINGS
- Location: vs-adapter/pushdown-declared-types/spec.md § Scenario: Declared type outside the declarable set fails with a named error
- Issue: `speq plan validate` warns that the scenario has 4 `AND` steps. The new `INTERVAL DAY TO SECOND` step is a separate behavior from the failure the scenario title names.
- Fix: Move the `INTERVAL DAY TO SECOND` step into a new scenario in the same delta, "An INTERVAL DAY TO SECOND select-list entry is accepted", and point the exa_type_tests row in plan.md § Verification at it.

Background drift: no objection, axis checked. The changed Backgrounds of pushdown-aggregation, pushdown-projection, and trino-pushdown stay within their existing bullets, and the new pushdown-projection line on the outer `SELECT` feeds the interval scenario. Every existing line that withholds `AGGREGATE_GROUP_BY_EXPRESSION` or advertises no scalar function has a matching CHANGED or REMOVED block.

## Design Depth

#### [DESIGN_VIOLATION] FINDINGS
- Location: intent.md § Directions D1, D4, D5; architecture.md § Components (pushdown_query, sql_dialect)
- Issue: Information hiding. Exasol's meaning of the overloaded `ADD` and `SUB` (day arithmetic for a date, days between two dates, a `TIMESTAMP` difference, exact decimal for numbers) and its `SUBSTR` position rule is one decision, but D1 gives pushdown_query only a "result type class" and leaves each of the three dialects to re-derive the operation and normalize `SUBSTR` itself. A fix to Exasol's rule then needs three edits that can drift apart.
- Fix: State in D1 and D4, and in the pushdown_query line of architecture.md § Components, that pushdown_query resolves each overloaded node into a dialect-neutral operation (add days, days between, timestamp difference, exact add, double add, `SUBSTR` with Exasol-normalized start and length), so each dialect renders only the source syntax of an already-resolved operation.

ADR overpromotion: no objection, axis checked. No direction carries `ADR: yes`.

ADR conflict: no objection, axis checked. With the render-time refusal removed and `FN_TRIM` withheld at MySQL, no renderer refuses an operand type of an advertised capability (`capability-negotiation-all-or-nothing`, `delegate-sum-avg-and-let-floating-point-sum-follow-source-order`), subject to the interval-operand finding above. The `EMITS` clause still declares only the seven types of `one-exasol-type-set-serves-mapping-and-emits`, since an interval entry declares a `DECIMAL`. `group-by-tuple-caps-column-count` stays in force per D6. `character-comparisons-follow-source-collation` holds: no string function renders a collation.

Architecture drift: no objection, axis checked. The BASE hash matches `git hash-object specs/architecture.md` (`cbf2746e...`). The Components block changes pushdown_query and pushdown, the Data Flow block adds the outer `SELECT`, and the Constraints block adds the delegated-function and interval-transport constraints, each matching a plan.md § Context or § Impact statement. All three blocks are CHANGED on canonical sections and copy the unchanged lines.
