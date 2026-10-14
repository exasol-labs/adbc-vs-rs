# Push-down: adbc-vs vs Exasol's JDBC virtual schemas

Date: 2026-10-14. Sources: `exasol/virtual-schema-common-jdbc`, `exasol/mysql-virtual-schema`, `exasol/postgresql-virtual-schema` (public; checked out under `_public/` and `exa-postgresql-virtual-schema/`), and this repo. Found by code reading only; no integration tests were run.

## Summary

adbc-vs pushes down less than the JDBC schemas, but its push-down layer is far larger. Most of the extra size is collation compensation that the JDBC schemas do not have.

| | JDBC VS (MySQL / PostgreSQL) | adbc-vs |
|---|---|---|
| Dialect code | MySQL 353 lines, PostgreSQL 825 lines, one shared visitor of 706 lines | about 4,200 lines of Rust (about 40% doc comments); MySQL 1,036, PostgreSQL 902, Trino 834, shared `pushdown_query.rs` 967 |
| Tests | n/a here | 236 unit tests (about 6,680 lines), 90 E2E tests, 102 scenarios in 12 push-down specs |
| Collation | none; the source's collation applies | MySQL about 350 lines, PostgreSQL about 50, Trino none |
| `SUM`, `AVG` | pushed | withdrawn |
| `ORDER BY`, `LIMIT` | pushed | withdrawn (the scan UDF gives no ordering guarantee) |
| Scalar functions | all enabled, minus a blocklist | none |
| Capability model | static per dialect; `EXCLUDED_CAPABILITIES` subtracts | static per dialect, from `drivers/*.toml`; no per-query decision |
| Data path | adapter returns SQL; ExaLoader runs `IMPORT FROM JDBC` | adapter returns SQL; the scan UDF reads over ADBC |
| Rewrites | mostly passthrough; PostgreSQL has date-function and cast rewrites and identifier case folding | COLLATE and ESCAPE rewriting; named errors where a rewrite is unsafe |

## JDBC VS

- Flow: `RequestDispatcher`, then `JDBCAdapter`, then `SqlGenerationVisitor` builds the remote SQL. `ImportIntoTemporaryTableQueryRewriter` wraps it as `IMPORT INTO (c1 type, ...) FROM JDBC ...` using the types Exasol computes (`selectListDataTypes`).
- Capabilities: explicit allow-lists for main, literal, predicate and aggregate. Scalar functions are all enabled minus a blocklist, so a new Exasol function is pushed by default.
- Not pushed on PostgreSQL, with reasons in code: `ROUND` (0.5 rounds down), `CONCAT` (fails on boolean), `SECOND` (precision), `POSIX_TIME` (session timezone), `*_BETWEEN` (no equivalent).
- MySQL declares no `LIKE_ESCAPE`, `IN_CONSTLIST` or `FULL OUTER JOIN`. PostgreSQL declares `LIKE_ESCAPE`, `REGEXP_LIKE` and `IN_CONSTLIST`.
- Identifier handling: PostgreSQL folds case (`POSTGRESQL_IDENTIFIER_MAPPING`); MySQL does not.
- No collation handling anywhere: `=`, `LIKE` and `ORDER BY` on strings use the source's collation.
- Unverified suspects: generic `REGEXP_LIKE` rendering for PostgreSQL; MySQL declaring `NULLS_SORTED_AT_END`.

## adbc-vs

- Pipeline: capabilities (TOML, compiled in) → `pushdown_query::parse` → `SqlDialect::render` → `ScanSpec` → `SELECT ADBC_VS.ADBC_SCAN('<json>') EMITS (...)`.
- Capability sets: 34 names (MySQL), 35 (PostgreSQL adds `JOIN_TYPE_FULL_OUTER`), 36 (Trino adds `AGGREGATE_GROUP_BY_TUPLE`, `JOIN_TYPE_FULL_OUTER`). Withdrawn everywhere: `SUM`/`AVG` variants, `ORDER BY`, `LIMIT`, `HAVING`, `GROUP BY` expressions, `LITERAL_BOOL`, all scalar functions.
- Nothing ties the advertised set to what `render()` can render, except `capabilities_tests.rs`.

### Where the complexity is

| # | What | Cost | Verdict |
|---|---|---|---|
| 1 | MySQL equality `COLLATE utf8mb4_0900_as_cs` on every character comparison (also loses source indexes) | about 100 lines | remove (this plan) |
| 2 | MySQL `utf8mb4_0900_bin` on `MIN`/`MAX` | about 45 lines | remove (this plan) |
| 3 | MySQL `GROUP BY` key collation plus `ONLY_FULL_GROUP_BY` matching | about 90 lines | remove (this plan) |
| 4 | `COUNT(DISTINCT)` collation | in `render_aggregate` | remove (this plan) |
| 5 | Named error for a select-list predicate over a collated grouping key | about 85 lines | remove (this plan) |
| 6 | MySQL source preconditions (collations exist, server 8.0.17 or newer) | about 100 lines in `mysql_source.rs` | remove (this plan) |
| 7 | PostgreSQL `COLLATE "C"` on ordering comparisons and `MIN`/`MAX` | about 50 lines | remove (this plan) |
| 10 | Trino `LIKE` explicit `ESCAPE` plus error path | about 70 lines | withdraw `FN_PRED_LIKE_ESCAPE`, keep plain `LIKE` (this plan) |
| 19 | Join aliasing, triplicated per dialect | about 120 lines each | pluggable refactor (plan 2) |
| 12 | Literal rendering and the character whitelist | about 70 lines each | keep; the injection guard is security-relevant; dedupe in plan 2 |
| 13 | Result types read from Exasol's `selectListDataTypes`; `SUM`/`AVG` withdrawn | `emit_type.rs` 69 lines | revisit in plan 3 |

Already well judged: no collation in Trino, no `ORDER BY`/`LIMIT`, named errors over silent rewrites.

## Direction (from the interview)

1. This plan: collation passthrough for MySQL and PostgreSQL; Trino `LIKE_ESCAPE` withdrawn. Exasol accepts no `COLLATE` in a query, so users who need Exasol string semantics either declare a code-point collation at the source or wrap the operand in an expression no dialect advertises; both are documented.
2. Plan 2: abstract push-down per dialect so the architecture is pluggable; dedupe the shared code.
3. Plan 3: add `SUM`/`AVG` and a small scalar-function allow-list, as plain passthrough with result types from Exasol.
