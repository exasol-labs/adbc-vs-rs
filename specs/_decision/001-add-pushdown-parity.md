# Decisions: add-pushdown-parity

## ADR: Advertise SELECTLIST_EXPRESSIONS and admit BOOLEAN as a declarable EMITS type

**ID:** advertise-selectlist-expressions-with-boolean-emits
**Plan:** add-pushdown-parity
**Status:** Accepted

### Context

The aggregation tier assumed `AGGREGATE_SINGLE_GROUP`, `AGGREGATE_GROUP_BY_COLUMN`, and the `FN_AGG_*` set alone authorize an aggregate select-list entry, with `MainCapability.SELECTLIST_EXPRESSIONS` withheld to keep `BOOLEAN` out of the `EMITS` clause. A gate probe against the deployed Exasol 2025.2.1 stack falsified the assumption: with the aggregate capabilities alone advertised, Exasol sent a plain `column` select list and no `aggregationType`. Only adding `SELECTLIST_EXPRESSIONS` produced a real `aggregationType: group_by` request. The same probe showed that once advertised, Exasol also places a bare predicate in select-list position, declared `{"type":"BOOLEAN"}` — confirmed for comparison, `IS NULL`, `BETWEEN`, `LIKE`, `IN`, `AND`, and a predicate wrapping an aggregate.

### Decision

Advertise `SELECTLIST_EXPRESSIONS` alongside the aggregation and filter-predicate capabilities. Add `BOOLEAN` to the set of types an `EMITS` column may declare, on `exa_type::ExasolType` only — `type_mapping::MappedType`, which answers what a MySQL column may map to, gains no such variant, since no MySQL column produces one.

### Options Considered

| Option | Verdict |
|--------|---------|
| Advertise `SELECTLIST_EXPRESSIONS`; add `BOOLEAN` to the declarable `EMITS` type set | ✓ Chosen — the only option that keeps filter and aggregation delegation, both Core Capability 3 goals, working together |
| Fail the pushdown whenever a select-list entry's declared type is `BOOLEAN` | ✗ Rejected — breaks a query that succeeds today, since Exasol never re-plans after an adapter error |
| Drop the filter tier's `FN_PRED_*` set so no predicate can reach the select list | ✗ Rejected — abandons filter delegation, a Core Capability 3 goal |
| Withhold `SELECTLIST_EXPRESSIONS` and abandon aggregation delegation | ✗ Rejected — abandons aggregation delegation, a Core Capability 3 goal |
| Widen `type_mapping::MappedType` with a `Boolean` variant instead of splitting the type | ✗ Rejected — no MySQL column maps to it, so it would be a representable state no input can produce |

### Consequences

Because the capability is session-wide rather than per-query, an Exasol older than 8.6.0 now fails every query whose select list carries an aggregate or a predicate, not only an aggregate query. `exasol-udf-sdk` already carries `ExaType::Boolean`, `Value::Bool`, and an Arrow `DataType::Boolean` coercion path, so the type addition costs one enum variant and its three serialization arms.

## ADR: A GROUP BY key and a DISTINCT aggregate operand carry the comparison collation

**ID:** collate-group-by-key-and-distinct-operand
**Plan:** add-pushdown-parity
**Status:** Accepted

### Context

Task 3.3 collated `MIN`/`MAX` operands but left `GROUP BY` keys and `DISTINCT` aggregate operands uncollated, since no artifact stated a rule for them. Measured on MySQL 8.4 and Exasol 2025.2.1 over four case/accent-variant values in a `utf8mb4_0900_ai_ci` column, an uncollated `GROUP BY v` and `COUNT(DISTINCT v)` both returned 2 where Exasol returns 4 — half the groups, silently. MySQL also runs under `ONLY_FULL_GROUP_BY` by default, so collating a `GROUP BY` key without collating its bare select-list projection fails with error 1055: MySQL does not see the two as functionally dependent once their collations differ.

### Decision

Render a character-typed `GROUP BY` key with `COLLATE utf8mb4_0900_as_cs`, and render the same suffix on that column's bare select-list occurrence in the same statement. Render a character-typed `DISTINCT` aggregate operand with the same suffix. `MIN`/`MAX` operands keep `utf8mb4_0900_bin`.

### Options Considered

| Option | Verdict |
|--------|---------|
| Collate both the `GROUP BY` key and its select-list projection with the comparison collation | ✓ Chosen — grouping and `DISTINCT` are equality questions, matching Exasol's group and distinct counts in every measured case |
| Leave a `GROUP BY` key and a `DISTINCT` operand uncollated | ✗ Rejected — this is the defect being fixed: returns half the groups Exasol asked for |
| Reuse `utf8mb4_0900_bin`, the `MIN`/`MAX` collation, for grouping | ✗ Rejected — its equality behavior was never measured; only `utf8mb4_0900_as_cs` was |
| Collate the `GROUP BY` clause alone and leave the select-list projection uncollated | ✗ Rejected — fails with MySQL error 1055 under `ONLY_FULL_GROUP_BY` |
| Set `sql_mode` on the session to clear `ONLY_FULL_GROUP_BY` | ✗ Rejected — makes correctness depend on a session variable the adapter sets, breaking outside the adapter's own session |

### Consequences

A delegated `GROUP BY` and `COUNT(DISTINCT)` over a character column now match Exasol's group count. The rule is stated once and applied at two rendering sites rather than as two independent rules.

## ADR: Multi-column GROUP BY is unreachable this release, because AGGREGATE_GROUP_BY_TUPLE gates the column count

**ID:** group-by-tuple-caps-column-count
**Plan:** add-pushdown-parity
**Status:** Accepted

### Context

A probe against deployed Exasol 2025.2.1 showed `AGGREGATE_GROUP_BY_TUPLE` gates any `GROUP BY` naming more than one column, not only a tuple-valued grouping expression as its name and the earlier decision withholding it assumed. With only `AGGREGATE_GROUP_BY_COLUMN` advertised, `GROUP BY "d", "f"` arrived stripped of its `aggregationType` and `groupBy` entirely — Exasol computed the aggregation itself rather than sending a partial request.

### Decision

Keep `AGGREGATE_GROUP_BY_TUPLE` withheld. Record that withholding it caps a delegated `GROUP BY` at exactly one column. Keep the renderer's existing N-column code path, verified by a unit test over a constructed request, and state the cap as a tested requirement — an `EXPLAIN VIRTUAL` assertion that a two-column `GROUP BY` produces a generated statement with no `GROUP BY` clause — rather than an unstated omission.

### Options Considered

| Option | Verdict |
|--------|---------|
| Keep `AGGREGATE_GROUP_BY_TUPLE` withheld and record the one-column cap as tested | ✓ Chosen — this revision's mandate is fixing two defects in a tier already under implementation, not widening the advertised set again |
| Advertise `AGGREGATE_GROUP_BY_TUPLE` and delegate multi-column grouping this release | ✗ Rejected (deferred, not dismissed) — cheapest change available, left to a follow-up plan on this measurement |
| Narrow `Aggregation::GroupBy` to a single `ColumnRef`, matching what Exasol can send | ✗ Rejected — would delete a rendering path already written and tested for no correctness gain |
| Leave the multi-column `GROUP BY` scenario as written and test it end to end | ✗ Rejected — claims a behavior this release cannot reach; the test would either fail or be a unit test presented as an end-to-end one |

### Consequences

A multi-column `GROUP BY` withdraws to Exasol's own computation at no correctness cost, matching today's behavior. A follow-up plan can advertise `AGGREGATE_GROUP_BY_TUPLE` on this measurement without repeating the probe.

## ADR: A select-list predicate reading its query's character GROUP BY key fails with a named error

**ID:** fail-select-list-predicate-over-grouped-character-key
**Plan:** add-pushdown-parity
**Status:** Accepted

### Context

The grouping-collation decision and select-list predicate rendering are each correct alone and jointly unrenderable for one shape. MySQL matches a select-list expression against a `GROUP BY` expression whole and credits no sub-expression, so once the grouping key carries `COLLATE utf8mb4_0900_as_cs`, any predicate over that same bare column fails with error 1055 regardless of where the collation is placed on the predicate's operand. A probe against deployed Exasol 2025.2.1 confirmed every predicate kind over a grouped character column arrives in select-list position, declared `BOOLEAN`, with `groupBy` naming that column — the shape is reachable under the tier's own advertised capabilities.

### Decision

Fail the pushdown with a named error when a select-list predicate's operand tree reaches a bare column reference that is character-typed and named by the same query's `groupBy`, outside any aggregate. Render every other select-list predicate: over literals, over an aggregate, over a numeric grouping key's column, and in a query whose aggregation is `single_group` or absent.

### Options Considered

| Option | Verdict |
|--------|---------|
| Fail the one composed shape with a named error | ✓ Chosen — the hazard is bounded exactly; every other shape was measured safe |
| Rewrite the operand as `ANY_VALUE(g COLLATE utf8mb4_0900_as_cs)`, the one rewrite measured to run | ✗ Rejected — correctness is unresolved: an NFC/NFD normalization-variant pair can compare equal under the collation, and `ANY_VALUE` would pick one arbitrarily where Exasol evaluates the predicate per row |
| Drop the grouping collation for any query carrying a select-list predicate | ✗ Rejected — reintroduces the silent wrong group count the grouping-collation decision exists to fix |
| Withhold `SELECTLIST_EXPRESSIONS` again and abandon aggregation delegation | ✗ Rejected — abandons a Core Capability 3 goal to protect one query shape |
| Render the statement anyway and let MySQL reject it | ✗ Rejected — forwards a statement the operator never wrote and surfaces MySQL's raw error code |
| Fail on any `BOOLEAN` declared select-list type | ✗ Rejected — fails every projected predicate where this decision fails one shape |

### Consequences

The failure is a hard, self-diagnosing error rather than silent wrong rows, the harmless direction of the two possible failure modes. A query mixing a select-list predicate with its own grouped character key, working against release 0.1, now fails once `SELECTLIST_EXPRESSIONS` is advertised. Whether the same NFC/NFD hazard reaches the already-shipped grouping collation is recorded as an open question for a follow-up plan to measure.

## ADR: Push-down capability negotiation is all-or-nothing at the capability level

**ID:** capability-negotiation-all-or-nothing
**Plan:** add-pushdown-parity
**Status:** Accepted

### Context

A differential end-to-end test measured a delegated `AVG` over a `DECIMAL(9,0)` column returning `5.3333` where Exasol's own `AVG` over the same rows returns `5.333333333333333`. The proposed fix, casting the operand to `DOUBLE`, was measured and falsified: it fixes the reported case and introduces a worse one, an unbounded-relative-error corruption of `AVG(DISTINCT)` over a wide `DECIMAL` or `BIGINT`-mapped column, up to 47 percent wrong on a common identifier-column shape. No rendering of `AVG` reaches parity: even a plain `DOUBLE` column, involving no cast and no type-rule difference, matched Exasol on only 18 of 40 measured groups, because IEEE 754 summation order is not fixed and the adapter controls no physical row order. A render-time guard restricting `SUM` to safe operand types was drafted and then rejected as a regression in kind: it would turn `SELECT SUM(double_column) FROM vs.t`, which works against release 0.1, into a hard failure, trading availability for exactness on one operand type rather than removing the trade.

### Decision

Withhold `FN_AGG_AVG`, `FN_AGG_AVG_DISTINCT`, `FN_AGG_SUM`, and `FN_AGG_SUM_DISTINCT` entirely from the advertised capability set. No render-time guard is added to `render_aggregate`. Stated as a general principle for this plan: an aggregation that cannot be safely and fully pushed down under a capability means the capability is not advertised, rather than advertised broadly and failed narrowly at render time for a subset of operand types.

### Options Considered

| Option | Verdict |
|--------|---------|
| Withhold `AVG` and `SUM` (and their `DISTINCT` forms) entirely | ✓ Chosen — the only option exactly right by construction; every withheld query behaves exactly as it does against release 0.1 |
| `AVG(CAST(operand AS DOUBLE))` | ✗ Rejected — falsified by measurement: catastrophic under `DISTINCT`, up to 47% relative error on a `DECIMAL(19,0)`/`BIGINT` column |
| `CAST(SUM(v) AS DOUBLE) / COUNT(v)` | ✗ Rejected, not falsified — exact and order-independent for every measured exact-numeric case, but not adopted: the user chose the simpler withholding, since the gain is throughput rather than correctness given Exasol's own `AVG` is already exact; left available to a follow-up plan on this evidence |
| Advertise `SUM` broadly, fail narrowly at render time for a `DOUBLE` operand | ✗ Rejected (reconsidered) — turns a query that works against release 0.1 into a hard failure for one operand type, trading availability for exactness rather than removing the trade |

### Consequences

Every `AVG` and `SUM` query, over any operand type, now withdraws to Exasol's own computation, matching release 0.1 behavior at the cost of throughput alone — no answer any operator reads changes. The principle governs every future dialect and aggregate: withhold a capability the dialect cannot render safely for every operand type it admits. Decision `[19]`'s select-list-predicate guard uses the opposite pattern — advertise broadly, fail one composed shape at render time — and was deliberately left unrevisited: its failure is a composition of two capabilities rather than one capability over one operand type, and MySQL rejects that shape's rendering outright with error 1055, where the refused `SUM` would have executed and returned a number. This inconsistency is recorded, not resolved.
