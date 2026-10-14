# Decision Log: remove-pushdown-collation-and-like-escape

## Interview

- Q: Mental model → A: push-down is first choice (fastest); keep the push-down layer simple; where Exasol and the source differ with no straightforward push-down, do not push down and let the user write the SQL; not auto-magic, but fast
- Q: Collation when Exasol and source compare strings differently → A: plain passthrough, document it (delete all COLLATE rewriting for MySQL and PostgreSQL; source collation applies; user writes COLLATE for Exasol semantics)
- Q: Trino LIKE ... ESCAPE emulation → A: withdraw FN_PRED_LIKE_ESCAPE, keep plain LIKE
- Q: Merge triplicated per-dialect code → A: yes where common design patterns apply; push-down logic abstracted per dialect so the architecture is pluggable (separate plan, phase 2)
- Q: Widen push-down vs JDBC VS → A: yes, add SUM/AVG and a small scalar-function allow-list (separate plan, phase 3); ORDER BY and LIMIT stay out
- Q: Exasol rejects COLLATE in a query, so "user writes COLLATE" is impossible; which remedies does the documentation give for code-point string semantics → A: option A, document both: (a) declare utf8mb4_0900_bin (MySQL) or "C" (PostgreSQL) on the source column or in a source view; (b) wrap the operand in an expression no dialect advertises (e.g. "name" || '') so Exasol evaluates the predicate itself, noting the speed cost and that plan 3 may change which wrappers work. Restoring collation rendering rejected.
- Split: three plans in order. (1) this plan: remove collation + LIKE_ESCAPE emulation. (2) refactor to pluggable per-dialect push-down. (3) add SUM/AVG + scalar allow-list. Phases 2 and 3 name this plan in their Existing Context.

## Evidence

Comparison report: `specs/_plans/remove-pushdown-collation-and-like-escape/pushdown-comparison-report.md`.

## Design Decisions

### [1] Character comparisons follow the source's collation

- **Decision:** No dialect renders a collation. A delegated character comparison, `LIKE`, `IN`, grouping key, `DISTINCT` operand, `MIN` or `MAX` operand, and join key is evaluated under the collation the source applies to the column. A user who needs Exasol's code-point comparison has two remedies: declare a code-point collation at the source, on the column or in a view the virtual schema exposes, or wrap the operand in an expression no dialect advertises, so that Exasol evaluates the predicate itself.
- **Rationale:** Criterion 2: the rule binds every current dialect and every dialect the pluggable refactor (plan 2) adds. Criterion 4: it rejects per-dialect emulation of Exasol's comparison. Exasol accepts no `COLLATE` clause in a query: measured on the local Exasol 2025.2.1 container, `SELECT CASE WHEN 'a' COLLATE UTF8_BIN = 'A' THEN 1 ELSE 0 END` fails with a syntax error at `COLLATE`. A source-side collation keeps the fragment delegated but needs control of the source schema. A wrapper such as `"name" || ''` needs no source change. No dialect advertises a scalar-function capability: every entry in `drivers/*.toml` is a main capability or an `FN_PRED_`, `FN_AGG_`, or `LITERAL_` entry. Measured on Exasol 2025.2.1 with a throwaway Lua adapter advertising MySQL's set, Exasol delegated none of `C || '' = 'widget'`, `C || '' < 'B'`, `GROUP BY C || ''`, and `MIN(C || '')`, and from `C || '' = 'widget' AND ID < 5` it delegated `ID < 5` alone. Exasol keeps `'abc' || ''` equal to `'abc'`. The wrapper costs speed, because Exasol fetches the rows the wrapped predicate would have filtered at the source. A wrapper keeps a predicate in Exasol only while no dialect advertises the function it uses, so advertising scalar functions changes which wrappers work. ADR digest: supersedes `collate-group-by-key-and-distinct-operand`. Conforms to `trino-no-collation-suffix`, whose decision now holds for every dialect, and to `trino-connector-pushdown-condition-unchecked`.
- **Supersedes:** collate-group-by-key-and-distinct-operand
- **Architecture:** § Components, § Constraints
- **Alternatives:** Keep the per-dialect collation rendering. The requester rejected it in the interview. It costs about 400 lines across the MySQL and PostgreSQL dialects and the MySQL source check, and it still leaves the NFC/NFD normalization case open.
- **Consequences:** A delegated query over a MySQL `utf8mb4_0900_ai_ci` column matches case and accent variants, groups them together, and orders by collation weight. A delegated ordering comparison and `MIN`/`MAX` over a PostgreSQL column follow the database locale. The MySQL source check stops querying collations and the server version, so MySQL 8.0.17 is no longer a checked floor. Decision [6] states what happens to the two type-mapping guards.
- **Promotes to ADR:** yes

### [2] A select-list predicate over the grouped character key is delegated

- **Decision:** The MySQL dialect renders a select-list predicate that reads its own query's character `GROUP BY` key like every other select-list predicate.
- **Rationale:** The named error existed because a collated grouping key is an expression that MySQL matches whole under `ONLY_FULL_GROUP_BY`. With no collation, the key and the predicate's operand are the same bare column, the shape MySQL 8.4 already executes for a numeric grouping key. Criterion 4: it rejects keeping the error. ADR digest: the accepted ADR `fail-select-list-predicate-over-grouped-character-key` forbids this rendering, so this entry supersedes it. No other ADR covers the shape.
- **Supersedes:** fail-select-list-predicate-over-grouped-character-key
- **Architecture:** no change: one rendering rule inside the MySQL dialect
- **Promotes to ADR:** yes

### [3] Trino withholds FN_PRED_LIKE_ESCAPE and renders LIKE with no ESCAPE clause

- **Decision:** The Trino dialect does not advertise `FN_PRED_LIKE_ESCAPE` and renders `predicate_like` as a plain `LIKE` with no `ESCAPE` clause. A backslash in a delegated pattern is an ordinary character at Trino. A query that needs an escape character writes an explicit `ESCAPE` clause, and Exasol evaluates that predicate itself.
- **Rationale:** Trino defines no default escape character and rejects an escape character followed by anything other than `%`, `_`, or itself. Reproducing Exasol's escape handling therefore needed a default clause on every pattern and a pattern validator in the dialect. The requester chose withdrawal in the interview. Criterion 4: it rejects the default-clause rendering. ADR digest: supersedes `trino-like-explicit-escape-clause`.
- **Supersedes:** trino-like-explicit-escape-clause
- **Architecture:** no change: the capability list is data in `drivers/trino.toml`, and the rendering stays inside the Trino dialect
- **Alternatives:** Render `ESCAPE '\'` on every plain `LIKE` and let Trino reject an inadmissible sequence with its own error. That returns Exasol's rows or a Trino error, where the chosen rendering returns Trino's rows for a pattern that holds a backslash. It was rejected because it keeps emulating Exasol's default escape rule, which the interview answer withdraws.
- **Consequences:** A plain `LIKE` whose pattern holds a backslash matches different rows at Trino than Exasol computes locally. The README states this. A Trino request that carries an `escapeChar` anyway fails with a named error under the framework rule that an unrepresentable fragment fails.
- **Promotes to ADR:** yes

### [4] Trino's set is PostgreSQL's without FN_PRED_LIKE_ESCAPE, plus AGGREGATE_GROUP_BY_TUPLE

- **Decision:** The Trino dialect's capability set is the PostgreSQL dialect's set without `FN_PRED_LIKE_ESCAPE`, plus `AGGREGATE_GROUP_BY_TUPLE`.
- **Rationale:** Decision [3] withholds `FN_PRED_LIKE_ESCAPE` from Trino alone. The MySQL and PostgreSQL dialects already render the clause only when a request carries one, with no default clause and no validation, so keeping it there costs no code. For an escape character followed by `%`, `_`, or itself, both targets match what Exasol matches. Any other sequence follows the source's own rule, like every other passthrough fragment. Criterion 4: it rejects a uniform withdrawal. ADR digest: supersedes `trino-exceeds-parity-with-group-by-tuple`, whose "PostgreSQL's entries plus `AGGREGATE_GROUP_BY_TUPLE`, and nothing else" no longer holds.
- **Supersedes:** trino-exceeds-parity-with-group-by-tuple
- **Architecture:** no change: each capability list is data in `drivers/<technology>.toml`
- **Alternatives:** Withdraw `FN_PRED_LIKE_ESCAPE` from every dialect, so all three sets follow one rule. Rejected: MySQL and PostgreSQL keep the capability and render a request's `escapeChar` as a verbatim `ESCAPE` clause, because withdrawing it would move a working fragment back into Exasol.
- **Promotes to ADR:** yes

### [5] The three technology collation features stay, rewritten

- **Decision:** `mysql-collation`, `postgresql-collation`, and `trino-collation` stay. Each states that its dialect renders no collation, what its source's default collation does to a delegated comparison, and which source collation reproduces Exasol's comparison.
- **Rationale:** What a source's default collation does is a fact about that technology. CLAUDE.md's spec governance names `<technology>-collation` as one of the three `vs-adapter` features each technology adds. `speq` has no marker that retires a feature: on a scratch copy of `specs/`, recording a delta that removes every scenario left a spec that `speq feature validate` rejects with "No scenarios defined".
- **Alternatives:** Fold the rule into one framework feature and delete the three technology features. That needs a manual directory deletion outside `speq record` and a CLAUDE.md governance change, and it leaves MySQL's case-insensitive default, PostgreSQL's locale order, and Trino's connector condition with no technology feature to state them.
- **Promotes to ADR:** no

### [6] Type-mapping guards after the collation removal

- **Decision:** The PostgreSQL mapping maps a `character varying` column of a nondeterministic collation like any other column and stops reading collation determinism. The MySQL mapping keeps refusing a character column whose character set is not `utf8mb4`, and its stated reason changes.
- **Rationale:** The PostgreSQL guard compensated for the missing equality collation. Decision [1] accepts the same divergence for MySQL's default `utf8mb4_0900_ai_ci`, so keeping the guard would refuse a PostgreSQL column for the reason a MySQL column is now exposed. The MySQL guard is about character sets, not collation: the dialect renders every string literal as `utf8mb4`, the mapping declares every character column `UTF8`, and this plan does not measure how MySQL converts a literal for a column of another character set.
- **Alternatives:** Keep the PostgreSQL guard unchanged. Rejected for the inconsistency above. It is the one change in this plan that the brief's scope notes do not list.
- **Promotes to ADR:** no

## Review Findings

### [plan-review] Remedy for Exasol's code-point comparison

- **Finding:** The interview answer named a remedy, "user writes COLLATE", that Exasol rejects in a query. The plan replaced it with a source-side collation without asking the requester, and it left out a remedy that works in a query today.
- **Direction change:** The requester chose to document two remedies. The first declares `utf8mb4_0900_bin` (MySQL) or `"C"` (PostgreSQL) on the source column or in a source view. The second wraps the operand in an expression no dialect advertises, such as `"name" || ''`, so Exasol evaluates the predicate itself, at the cost of fetching the rows the source would have filtered. Decision [1], plan.md § Summary, § Context, § Impact, task 4.2, and the `pushdown-filter`, `mysql-collation`, and `postgresql-collation` deltas now state both. Two new E2E scenarios, one in each collation feature, verify the wrapper against a live source.
- **Promotes to ADR:** no
