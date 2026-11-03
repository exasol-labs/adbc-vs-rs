# Plan Review Findings: change-type-mapping-coverage

## Summary
- Axes checked: 6/6
- Total findings: 13 (Blockers: 2, Findings: 11)
- Intent Fidelity blockers: 1

## How this plan may fail

1. Application databases built with Django or Hibernate still lose their `timestamptz` columns, which issue #17 names as one of its two headline types. Routed to Intent Fidelity, `[SCOPE_REDUCTION]` BLOCKER.
2. The new architecture invariant "a source type maps only when the source answers every delegated comparison as Exasol does" is false the day it is recorded, because the retained timestamp mappings violate it. Routed to Requirement Quality, `[REQUIREMENT_CONFLICT]` BLOCKER.
3. The MySQL driver returns `FLOAT`, `BIGINT UNSIGNED`, or `TINYINT(1)` in a form the mapping did not assume. The E2E tests fail late, and the plan gives no planned response. Routed to Feasibility, `[UNSTATED_ASSUMPTION]`.

## Intent Fidelity

#### [SCOPE_REDUCTION] BLOCKER
- Location: decision-log.md § Design Decisions [4]; `vs-adapter/postgresql-type-mapping` delta § Scenario "timestamp with time zone is unmapped because no session time zone is aligned"; `vs-adapter/mysql-type-mapping` delta § Scenario "TIMESTAMP, JSON, TIME, and YEAR are unmapped for how MySQL compares them"; plan.md § Impact bullet 6
- Issue: The interview maps the issue's types and MySQL `TIMESTAMP`. Its note allows omission only for "a type whose delegated comparison cannot be made safe", decided "with measured evidence". Issue #17 states "`text` and `timestamptz` are the defaults of Rails, Django, Prisma, and Hibernate, so many application databases lose most of their columns". Decision [4] omits PostgreSQL `timestamptz` and MySQL `TIMESTAMP`. Its own Alternatives line names a route that makes both safe: "Mapping both types to TIMESTAMP in UTC, with the source session pinned to UTC". [4] rejects that route because it "needs new per-technology session initialization in `connection`" (a cost argument, not the interview's safety criterion) and because it "departs from the interview answer". Full omission departs further from the interview than a different target type does. For MySQL `TIMESTAMP` the UTC route does not depart at all, because the interview named the source type and no target type. With the session pinned to UTC, the daylight-saving ambiguity the MySQL scenario cites does not occur. PostgreSQL then compares a `timestamptz` against a `timestamp` or `date` in UTC, the same zone as the emitted wall-clock value. The Rationale sentence "Making either type safe needs a mechanism that carries Exasol's session time zone to the source session" holds only for the `TIMESTAMP WITH LOCAL TIME ZONE` target. The delta also states as fact "the wall-clock value PostgreSQL returns depends on that same session `TimeZone`". The ADBC PostgreSQL driver reads results through binary `COPY`, where a `timestamptz` is a UTC instant, and `notes/planning.md` records no measurement of the returned value. plan.md Context says "planning starts no containers", although AGENTS.md says to start containers with `make stack-up-*`. As a result, the interview's "measured evidence" was not gathered for these two omissions. Keeping `TIMESTAMP WITH LOCAL TIME ZONE` out of the mapped set is justified. Omitting the two source types is not.
- Fix: Map PostgreSQL `timestamp with time zone` and MySQL `TIMESTAMP` to Exasol `TIMESTAMP` holding the UTC wall-clock value, and pin the source session time zone to UTC when the connection opens. For PostgreSQL, use the conninfo keyword `options` with `-c TimeZone=UTC`. For MySQL, use `time_zone='+00:00'`, applied by the connection before the statement runs, and verify that the ADBC MySQL driver accepts it. Then make these changes:
  - Rewrite decision-log [4] so that it keeps only the `TIMESTAMP WITH LOCAL TIME ZONE` target omission.
  - Add a decision-log entry for the UTC mapping that states it departs from the interview in the target type only.
  - Replace the two omission scenarios with mapping scenarios. Their THEN steps cover a comparison with a `TIMESTAMP` literal and a `DATE` literal, `MIN` and `MAX`, `GROUP BY`, and an equi-join against a `timestamp without time zone` or `DATETIME` column.
  - Add tasks for the session pin in `crates/adbc-vs-scan/src/connection.rs`, both mappings, fixture rows on both sides of a daylight-saving transition, and differential E2E tests.
  - Add the session pin to the architecture delta under § Data Flow or § Constraints.
  - Add both types to docs task 5.1.

  If a measurement on the running stack (`make stack-up-postgres`, `make stack-up-mysql`, `make stack-up-exasol`) shows that a pinned-UTC comparison diverges from Exasol, keep the omission and replace the cost argument in [4] with that measured result.

#### [SCOPE_REDUCTION] FINDINGS
- Location: plan.md § Summary and § Impact bullet 6; decision-log.md [4] Consequences
- Issue: The omission of MySQL `JSON`, `TIME`, and `YEAR` is justified. Each type needs a conversion keyed on the source type. The MySQL dialect cannot convert a `VARCHAR` or `DECIMAL` reference without changing the column's collation or losing index use on every integer column. Only a per-column mechanism would make these types safe, and the interview excluded one. However, plan.md never says that types the interview requested are not delivered. The Summary says only "A type whose comparison at the source differs from Exasol's stays omitted with a reason that names the difference", and Impact bullet 6 lists the types without saying that the interview requested them. Only decision-log [4] says "This departs from the interview answer". plan.md Context bullet 4 shows that the evidence for these omissions is documentation, while the interview asked for measured evidence.
- Fix: Add one sentence to plan.md § Summary that names every type the interview requested that stays omitted after the blocker above is fixed (MySQL `JSON`, `TIME`, `YEAR`, and the `TIMESTAMP WITH LOCAL TIME ZONE` target type). The sentence states that this departs from the interview answer and points to decision-log [4]. Add a bullet to plan.md § Context stating that the evidence for these omissions is the MySQL 8.4 reference and not a measurement.

## Feasibility

#### [UNSTATED_ASSUMPTION] FINDINGS
- Location: plan.md § Context bullet "The MySQL driver's Arrow types for `FLOAT` and `BIGINT UNSIGNED` are not documented"; plan.md task 3.4; `scan-udf/arrow-scan` delta § Background bullet 5
- Issue: plan.md says the E2E tests in task 3.4 "detect a `FLOAT` returned as a double parsed from text and an unsigned value returned as a signed one", but the plan gives no response to either result. Three failure cases follow:
  - If the driver parses `FLOAT` from text, a stored `0.1` is emitted as `0.1`, while MySQL compares `0.100000001490116…`. That breaks the `mysql-type-mapping` FLOAT step and the new mapping-safety rule.
  - A `BIGINT UNSIGNED` returned as `Int64` arrives as `-1`, and the checked Arrow cast to `Decimal128(20,0)` keeps that value without an error.
  - A `TINYINT(1)` returned as a boolean emits `5` as `1`.

  The arrow-scan delta also states as fact "an integer arrives at the width and signedness of its source type", which plan.md says is not documented.
- Fix: Add to task 3.4: "If an E2E test shows that the value the driver returns for `FLOAT`, an unsigned integer, or `TINYINT(1)` differs from the value MySQL compares, unmap that type with a reason that names the difference, and update the `mysql-type-mapping` delta and docs task 5.1." Delete "and an integer arrives at the width and signedness of its source type" from the arrow-scan delta Background.

#### [NFR_IGNORED] FINDINGS
- Location: plan.md § Impact bullet 3; decision-log.md [2] Consequences; plan.md task 5.1
- Issue: Impact says "A delegated filter on a `real`, unconstrained `numeric`, `uuid`, `json`, or `jsonb` column cannot use a source index on that column". Task 2.2 applies the same conversion in every position, so join keys, `GROUP BY` keys, and `MIN` or `MAX` operands lose index use too. `uuid` is a common primary-key and foreign-key type. A point lookup by `uuid` becomes a sequential scan, and a delegated join on `uuid` keys cannot use an index nested-loop join. The plan names no remedy for an operator who sees the slowdown.
- Fix: In plan.md Impact bullet 3 and decision-log [2] Consequences, name join keys and grouping keys beside filters, and name `uuid` primary-key lookups as the main cost case. In task 5.1, have the docs page name the remedy: a PostgreSQL expression index on the rendered form, for example `CREATE INDEX ON t ((CAST(uid AS text)))`. Confirm with one `EXPLAIN` on the test stack that PostgreSQL uses that index before documenting it.

#### [HIDDEN_DEPENDENCY] FINDINGS
- Location: plan.md § Parallelization rows B and C
- Issue: Groups B and C have no dependency edge between them, so they can run at the same time. Both end in E2E runs (tasks 2.6, 2.7, and 3.4). Each run needs `make cross-udf-build` and uses the single Exasol stack. Per AGENTS.md, one shared setup installs the SLC and creates the virtual schemas, BucketFS writes are asynchronous, and a write after a delete can return `423 Locked`. Two concurrent runs would each rebuild and upload the `.so` from one working tree that holds the other group's unfinished edits.
- Fix: Add a note to plan.md § Parallelization: groups B and C may write code in parallel, but tasks 2.6, 2.7, and 3.4 run their E2E suites one at a time, after the code of both groups compiles.

## Requirement Quality

#### [REQUIREMENT_CONFLICT] BLOCKER
- Location: architecture.md delta § Constraints, new bullet "A source type maps only when …"; `vs-adapter/type-mapping` delta § Background bullet 3 and NEW Scenario "A source type whose delegated comparison differs from Exasol's is unmapped"; recorded `vs-adapter/postgresql-type-mapping` step "`timestamp without time zone` MUST map to Exasol `TIMESTAMP`" (kept in the delta); recorded `vs-adapter/trino-type-mapping` step "a `timestamp(p)` of any other precision Trino admits MUST map to Exasol `TIMESTAMP` as well"; recorded MySQL `DATETIME` mapping
- Issue: The new rule names one exception, the source's collation. Every mapped timestamp is declared as a bare `TIMESTAMP`, which Exasol treats as precision 3 (`exa_type.rs` `from_json` doc comment: "Exasol's own default of 3 applies"). PostgreSQL `timestamp`, MySQL `DATETIME(6)`, and Trino `timestamp(6)` to `timestamp(12)` hold sub-millisecond digits, and the source compares them at full precision. Two examples show the difference:
  - A delegated `ts = TIMESTAMP '2024-01-01 12:00:00.123'` selects no row stored as `.123456`.
  - `GROUP BY ts` and `COUNT(DISTINCT ts)` treat `.123456` and `.123789` as two values, while Exasol holds both rows as `.123`.

  The new scenario's GIVEN requires these types to be unmapped, and the retained scenarios require them to map, so both cannot pass. The GIVEN wording "under any declared type in the set" can mean either "some" or "every" declared type.
- Fix: Add sub-millisecond timestamp precision as a second stated exception to the new bullet in the architecture delta's § Constraints, to `vs-adapter/type-mapping` delta Background bullet 3, and to the new type-mapping scenario. The exception states that a delegated comparison over a source timestamp compares at the source's fractional precision, while Exasol holds the value at millisecond precision. In that scenario's GIVEN, replace "under any declared type in the set" with "under every declared type in the set". Add the precision behavior to docs task 5.1.

#### [COMPLETENESS_GAP] FINDINGS
- Location: `vs-adapter/postgresql-type-mapping` delta § Scenario "numeric without a declared precision maps to DOUBLE"; `scan-udf/arrow-scan` delta § Scenario "Non-finite DOUBLE value fails with a named error"; plan.md § Impact bullet 4
- Issue: The named error fires only when a non-finite value reaches the scan. A delegated filter or aggregate reads the value without emitting it. PostgreSQL orders `NaN` above every number and treats `NaN = NaN` as true. As a result, `SELECT COUNT(*) FROM t WHERE x > 0` counts the `NaN` and `Infinity` rows and returns without an error, and `COUNT(DISTINCT x)` counts `NaN` as one value. The arrow-scan scenario acknowledges this ("a delegated filter has already compared the original value"), but no scenario or Impact line states the outcome. A separate case is also unstated: an unconstrained `numeric` value outside the double range makes `CAST(... AS DOUBLE PRECISION)` fail at PostgreSQL with an out-of-range error.
- Fix: Add an AND step to the "numeric without a declared precision maps to DOUBLE" scenario. The step states that a delegated filter or count over a non-finite value the query does not project follows PostgreSQL's ordering and succeeds, and that a value outside the double range fails the query with PostgreSQL's out-of-range error. Add both cases to plan.md § Impact and docs task 5.1. Extend task 2.7 with a filter-only query over `numeric_specials.unbounded_amount`.

#### [COMPLETENESS_GAP] FINDINGS
- Location: `vs-adapter/pushdown-filter` delta § Scenario "BOOLEAN column is a predicate on its own"; plan.md task 1.3
- Issue: Before this plan no column was `BOOLEAN`, so a comparison between a column and a predicate could not arise. Once `boolean` is mapped, `WHERE flag = (amount > 5)` arrives as a `predicate_equal` whose operand is a `predicate_*` node. `read_expression` (`pushdown_query.rs:851-880`) rejects predicate nodes with "filter expression of type `…` has no rendering path in this release". Exasol does not compute a delegated fragment itself after an adapter error, so such a query would fail. The plan does not measure whether Exasol sends this shape.
- Fix: Add to task 1.3: measure on the test Exasol whether `SELECT ID FROM PGVS.TYPE_COVERAGE WHERE FLAG = (ID > 5)` arrives with a predicate operand. If it does, accept a predicate node as a comparison operand in `read_expression`, render it parenthesized, add a unit test, and add an AND step to the pushdown-filter scenario. If it does not, record the measured request shape in plan.md § Context.

#### [AMBIGUOUS_REQUIREMENT] FINDINGS
- Location: `vs-adapter/pushdown-filter` delta § Scenario "BOOLEAN column is a predicate on its own"; plan.md § Manual Testing row `pushdown-filter`
- Issue: The scenario says "WHEN each dialect parses and renders each request", and then "AND the rows the source returns MUST equal the rows Exasol returns". MySQL and Trino map no column to `BOOLEAN`, so the row step has no possible test for two of the three dialects. The Scenario Coverage table tests rows on PostgreSQL only. The manual test `WHERE FLAG = OTHER_FLAG` exercises a comparison of two columns, not a column standing alone as a predicate.
- Fix: Scope the row-equality AND step to "a PostgreSQL `boolean` column". Change the manual-test command to `EXPLAIN VIRTUAL SELECT ID FROM PGVS.TYPE_COVERAGE WHERE FLAG AND NOT OTHER_FLAG`, with the expected output "a `WHERE` clause carrying the bare `flag` column and `NOT` over the bare `other_flag` column, with no comparison added".

## Task Breakdown

Checked with no objection: every spec delta has an implementing task (type-mapping 1.1, 2.1, 3.1; pushdown-filter 1.3, 1.4; pushdown-dialect 1.2, 2.3; postgresql-pushdown 2.2, 2.3; trino-type-mapping 1.5; arrow-scan 4.1, 4.2; create-virtual-schema and postgresql-collation 2.6). The test names and line numbers the tasks cite exist at the stated lines.

#### [TRACEABILITY_GAP] FINDINGS
- Location: plan.md task 2.4; `crates/adbc-vs-adapter/tests/common/postgres.rs:48-58` (`SEEDED_OBJECTS`)
- Issue: The task leaves out two things the tests depend on:
  - `SEEDED_OBJECTS` lists every object `docker/postgres/init.sql` seeds, so that a stack running an older seed fails fast. Task 2.4 adds `type_coverage` and a separate long-text table but does not add them to that list. A stack seeded before the change then fails later with a missing-table error.
  - The boolean scenario needs "its equality with a second `boolean` column", and the manual tests read `OTHER_FLAG`, but task 2.4 lists only one `boolean` column.
- Fix: In task 2.4, add both new table names to `SEEDED_OBJECTS`, and add a second `boolean` column `other_flag` to `type_coverage` whose value differs from `flag` in at least one row.

## Design Depth

Checked with no objection:
- `[ARCHITECTURE_DRIFT]`: the delta's BASE equals `git hash-object specs/architecture.md` (`31874c94…`), and its § Constraints block is the current section plus two bullets. The entry for [2], `Architecture: § Constraints`, matches the delta. The entry for [1], `no change`, gives a reason that is correct: architecture.md names no mapped-type set. The fix for the intent blocker above adds a session pin, which needs an architecture delta entry.
- `[ADR_CONFLICT]`: [1] supersedes `advertise-selectlist-expressions-with-boolean-emits` explicitly. [2] is consistent with `character-comparisons-follow-source-collation`, because a `CAST` renders no `COLLATE` and the conversion to `text` keeps the column's collation.
- Coupling: the conversion is chosen by declared type and is applied in the select list too, so PostgreSQL compares the same expression the scan emits. No decision is shared between two modules.

#### [ADR_OVERPROMOTION] FINDINGS
- Location: decision-log.md [2] Decision
- Issue: The Decision reads: "renders a column reference declared `VARCHAR` as `CAST(<column> AS TEXT)` and one declared `DOUBLE` as `CAST(<column> AS DOUBLE PRECISION)`, in every position". This is the rendered SQL that the `postgresql-pushdown` scenario "Character and floating-point column references render as their declared type" already states (adr-rules rule 3 and rule 6). The durable decision is the policy, not the SQL text.
- Fix: Rewrite the Decision of [2] as: "The PostgreSQL dialect renders a column reference converted to the PostgreSQL type that holds its declared Exasol type's value, so the source compares the value the scan emits. The declared type selects the conversion, and a declared type whose comparison already agrees with Exasol's renders the column without one." Keep Rationale, Alternatives, and Consequences.

## Prose Quality

#### [PROSE_UNCLEAR] FINDINGS
- Location: plan.md task 3.1
- Issue: "the four text types with their catalog length capped at `MAX_VARCHAR_SIZE` under the `utf8mb4` rule" uses "the `utf8mb4` rule" without defining it. It is unclear whether the size comes from `CHARACTER_MAXIMUM_LENGTH` or from each type's fixed byte limit, which the `mysql-type-mapping` Background names as the bound ("A text type declares a storage limit in bytes … so that limit bounds the length too").
- Fix: Rewrite the clause as "the four text types, each mapped to `VARCHAR` of its `CHARACTER_MAXIMUM_LENGTH` capped at `MAX_VARCHAR_SIZE`, and unmapped unless its character set is `utf8mb4`". If the fixed byte limit is the intended input, name it instead.

#### [PROSE_BLOAT] FINDINGS
- Location: plan.md § Summary
- Issue: "so far fewer columns are omitted" is an unmeasured intensifier (writing-guardrails rule 6).
- Fix: Replace it with the counts the deltas state: "PostgreSQL maps 14 `data_type` spellings instead of 7, and MySQL maps 16 types instead of 8".
