# Decisions: remove-pushdown-collation-and-like-escape

## ADR: Character comparisons follow the source's collation

**ID:** character-comparisons-follow-source-collation
**Plan:** remove-pushdown-collation-and-like-escape
**Status:** Accepted
**Supersedes:** collate-group-by-key-and-distinct-operand

### Context

Exasol accepts no `COLLATE` clause in a query, so a dialect cannot hand the user a way to request code-point comparison. Per-dialect collation rendering emulates Exasol's comparison and still leaves the NFC/NFD normalization case open.

### Decision

No dialect renders a collation. A delegated character comparison, `LIKE`, `IN`, grouping key, `DISTINCT` operand, `MIN` or `MAX` operand, and join key is evaluated under the collation the source applies to the column. A user who needs Exasol's code-point comparison declares a code-point collation at the source, or wraps the operand in an expression no dialect advertises so that Exasol evaluates the predicate itself.

### Options Considered

| Option | Verdict |
|--------|---------|
| Keep per-dialect collation rendering | Rejected: about 400 lines across the MySQL and PostgreSQL dialects and the MySQL source check, with the normalization case still open |

### Consequences

- A delegated query over a MySQL `utf8mb4_0900_ai_ci` column matches case and accent variants, groups them together, and orders by collation weight.
- A delegated ordering comparison and `MIN`/`MAX` over a PostgreSQL column follow the database locale.
- The MySQL source check stops querying collations and the server version, so MySQL 8.0.17 is no longer a checked floor.
- A wrapper keeps a predicate in Exasol only while no dialect advertises the function it uses.

## ADR: A select-list predicate over the grouped character key is delegated

**ID:** delegate-select-list-predicate-over-grouped-character-key
**Plan:** remove-pushdown-collation-and-like-escape
**Status:** Accepted
**Supersedes:** fail-select-list-predicate-over-grouped-character-key

### Context

The named error existed because a collated grouping key is an expression that MySQL matches whole under `ONLY_FULL_GROUP_BY`. With no collation, the key and the predicate's operand are the same bare column.

### Decision

The MySQL dialect renders a select-list predicate that reads its own query's character `GROUP BY` key like every other select-list predicate.

## ADR: Trino withholds FN_PRED_LIKE_ESCAPE and renders LIKE with no ESCAPE clause

**ID:** trino-withholds-like-escape
**Plan:** remove-pushdown-collation-and-like-escape
**Status:** Accepted
**Supersedes:** trino-like-explicit-escape-clause

### Context

Trino defines no default escape character and rejects an escape character followed by anything other than `%`, `_`, or itself. Reproducing Exasol's escape handling needs a default clause on every pattern and a pattern validator in the dialect.

### Decision

The Trino dialect does not advertise `FN_PRED_LIKE_ESCAPE` and renders `predicate_like` as a plain `LIKE` with no `ESCAPE` clause. A backslash in a delegated pattern is an ordinary character at Trino, and a query that needs an escape character writes an explicit `ESCAPE` clause, which Exasol evaluates itself.

### Options Considered

| Option | Verdict |
|--------|---------|
| Render `ESCAPE '\'` on every plain `LIKE` and let Trino reject an inadmissible sequence | Rejected: keeps emulating Exasol's default escape rule |

### Consequences

- A plain `LIKE` whose pattern holds a backslash matches different rows at Trino than Exasol computes locally. The README states this.
- A Trino request that carries an `escapeChar` fails with a named error.

## ADR: Trino's set is PostgreSQL's without FN_PRED_LIKE_ESCAPE, plus AGGREGATE_GROUP_BY_TUPLE

**ID:** trino-set-is-postgresql-without-like-escape-plus-group-by-tuple
**Plan:** remove-pushdown-collation-and-like-escape
**Status:** Accepted
**Supersedes:** trino-exceeds-parity-with-group-by-tuple

### Context

Trino alone withholds `FN_PRED_LIKE_ESCAPE`. The MySQL and PostgreSQL dialects render the clause only when a request carries one, with no default clause and no validation, and match Exasol for an escape character followed by `%`, `_`, or itself.

### Decision

The Trino dialect's capability set is the PostgreSQL dialect's set without `FN_PRED_LIKE_ESCAPE`, plus `AGGREGATE_GROUP_BY_TUPLE`.

### Options Considered

| Option | Verdict |
|--------|---------|
| Withdraw `FN_PRED_LIKE_ESCAPE` from every dialect | Rejected: moves a working MySQL and PostgreSQL fragment back into Exasol |
