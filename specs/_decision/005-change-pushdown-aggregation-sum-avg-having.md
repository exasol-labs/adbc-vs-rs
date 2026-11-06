# Decisions: change-pushdown-aggregation-sum-avg-having

## ADR: Delegate SUM and AVG, and let a floating-point sum follow the source's order

**ID:** delegate-sum-avg-and-let-floating-point-sum-follow-source-order
**Plan:** change-pushdown-aggregation-sum-avg-having
**Status:** Accepted
**Supersedes:** capability-negotiation-all-or-nothing

### Context

Withholding `SUM` and `AVG` makes every such query fetch every source row, 1,000,000 rows for a five-group `SUM` in the evaluation. Over an exact-numeric operand the delegated value equals Exasol's, so only floating-point operands differ.

### Decision

Every dialect advertises `FN_AGG_SUM`, `FN_AGG_SUM_DISTINCT`, `FN_AGG_AVG`, and `FN_AGG_AVG_DISTINCT`. A delegated sum over a floating-point operand is the source's double-precision sum in the order the source chooses, and the user documentation states that the result can differ from Exasol's. A capability stays advertised only when its dialect renders it for every operand type it admits: no renderer refuses one operand type of an advertised capability.

### Options Considered

| Option | Verdict |
|--------|---------|
| Keep the four capabilities withheld | Rejected: every `SUM` and `AVG` keeps fetching every row |
| Delegate only an exact-numeric operand | Rejected: Exasol decides delegation by capability alone, so the adapter would refuse a `DOUBLE` operand at render time, and Exasol does not compute a fragment itself after the adapter errors |

### Consequences

- `AVG` renders as a quotient of `SUM` and `COUNT`, and two dialects widen the `SUM` operand.
- A `SUM` or `AVG` over a `DOUBLE` column can return another value than before, and two runs can differ.
- A `NaN` in a PostgreSQL or Trino floating-point column makes the delegated `SUM` and `AVG` null.
- Each dialect converts a `BOOLEAN` or character `SUM` and `AVG` operand as Exasol converts it.
