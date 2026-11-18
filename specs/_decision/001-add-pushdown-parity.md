# Decisions: add-pushdown-parity

## ADR: Advertise SELECTLIST_EXPRESSIONS and admit BOOLEAN as a declarable EMITS type

**ID:** advertise-selectlist-expressions-with-boolean-emits
**Plan:** add-pushdown-parity
**Status:** Accepted

### Context

Exasol sends an aggregate select list only when `SELECTLIST_EXPRESSIONS` is advertised. Once advertised, Exasol also places a bare predicate in the select list, declared `BOOLEAN`.

### Decision

Advertise `SELECTLIST_EXPRESSIONS` with the aggregation and filter-predicate capabilities, and add `BOOLEAN` to the types an `EMITS` column may declare.

### Options Considered

| Option | Verdict |
|--------|---------|
| Advertise `SELECTLIST_EXPRESSIONS` and admit `BOOLEAN` in `EMITS` | ✓ Chosen: keeps filter and aggregation delegation working together |
| Fail the pushdown on a `BOOLEAN` select-list entry | ✗ Rejected: Exasol does not re-plan after an adapter error |
| Withhold `SELECTLIST_EXPRESSIONS` | ✗ Rejected: abandons aggregation delegation |

### Consequences

- The capability is session-wide, so an Exasol older than 8.6.0 fails every query whose select list carries an aggregate or a predicate.

## ADR: A GROUP BY key and a DISTINCT aggregate operand carry the comparison collation

**ID:** collate-group-by-key-and-distinct-operand
**Plan:** add-pushdown-parity
**Status:** Accepted

### Context

An uncollated `GROUP BY` or `COUNT(DISTINCT)` over a MySQL `utf8mb4_0900_ai_ci` column merges case and accent variants that Exasol keeps apart. Under `ONLY_FULL_GROUP_BY`, a collated key also needs its bare select-list projection collated.

### Decision

Render a character-typed `GROUP BY` key, its bare select-list occurrence, and a character-typed `DISTINCT` aggregate operand with `COLLATE utf8mb4_0900_as_cs`.

### Options Considered

| Option | Verdict |
|--------|---------|
| Collate the key, its projection, and the `DISTINCT` operand | ✓ Chosen: matches Exasol's group and distinct counts |
| Collate the `GROUP BY` clause alone | ✗ Rejected: MySQL error 1055 under `ONLY_FULL_GROUP_BY` |
| Clear `ONLY_FULL_GROUP_BY` in the session | ✗ Rejected: correctness would depend on a session variable |

## ADR: A select-list predicate reading its query's character GROUP BY key fails with a named error

**ID:** fail-select-list-predicate-over-grouped-character-key
**Plan:** add-pushdown-parity
**Status:** Accepted

### Context

MySQL matches a select-list expression against a collated `GROUP BY` key whole, so a predicate over the same bare column fails with error 1055.

### Decision

Fail the pushdown with a named error when a select-list predicate reads a character column that the same query's `groupBy` names, outside any aggregate. Render every other select-list predicate.

### Options Considered

| Option | Verdict |
|--------|---------|
| Fail the one composed shape with a named error | ✓ Chosen: bounds the hazard exactly |
| Rewrite the operand as `ANY_VALUE(g COLLATE ...)` | ✗ Rejected: picks one of two values the collation treats as equal |
| Drop the grouping collation when a select-list predicate is present | ✗ Rejected: brings back the wrong group count |

## ADR: Push-down capability negotiation is all-or-nothing at the capability level

**ID:** capability-negotiation-all-or-nothing
**Plan:** add-pushdown-parity
**Status:** Accepted

### Context

No rendering of `AVG` matches Exasol for every operand type: a `DOUBLE` cast corrupts `AVG(DISTINCT)` over wide integers, and floating-point summation order is not fixed.

### Decision

Withhold `FN_AGG_AVG`, `FN_AGG_AVG_DISTINCT`, `FN_AGG_SUM`, and `FN_AGG_SUM_DISTINCT`. A capability that cannot be rendered safely for every operand type it admits is not advertised, rather than advertised and refused at render time.

### Options Considered

| Option | Verdict |
|--------|---------|
| Withhold `AVG` and `SUM` | ✓ Chosen: every withheld query keeps its current result |
| `AVG(CAST(operand AS DOUBLE))` | ✗ Rejected: up to 47 % error under `DISTINCT` over a `DECIMAL(19,0)` column |
| `CAST(SUM(v) AS DOUBLE) / COUNT(v)` | ✗ Rejected: exact for exact-numeric operands, but the gain is throughput only |
| Advertise `SUM`, refuse a `DOUBLE` operand at render time | ✗ Rejected: turns a working query into a failure |
