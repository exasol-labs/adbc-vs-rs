# Plan: change-pushdown-limit

## Summary

Delegates a `LIMIT` to MySQL, PostgreSQL, and Trino whenever no `ORDER BY` precedes it, so `SELECT ... LIMIT n` over a virtual table reads at most n rows at the source. `ORDER BY`, every `OFFSET`, and every `LIMIT` that follows an `ORDER BY` stay in Exasol.

## Context

- Issue exasol-labs/adbc-vs-rs#20: a preview `SELECT ... LIMIT 100` over a 5M-row virtual table fetches all 5M rows, because every dialect withholds `LIMIT` and `LIMIT_WITH_OFFSET`.
- `vs-adapter/result-ordering` withholds `ORDER_BY_COLUMN`, `ORDER_BY_EXPRESSION`, `LIMIT`, and `LIMIT_WITH_OFFSET` from every dialect, and `pushdown_query::parse` refuses a request carrying `orderBy` or `limit` (`UNDELEGATED_ELEMENTS` in `crates/adbc-vs-adapter/src/pushdown_query.rs`).
- The emitting-UDF transport carries no ordering guarantee, so `ORDER BY` has to stay in Exasol. A `LIMIT` with no `ORDER BY` selects an arbitrary set of rows, so emission order does not affect its correctness.
- Exasol re-applies no delegated fragment, and the adapter cannot see from a request what Exasol applies above the scan. A delegated limit is therefore correct only if Exasol sends `limit` with nothing of its own between the scan and the limit. The limit-shape gate in § Requirements measures that on the test Exasol before any product change (intent.md D3).
- Exasol sends the element as `"limit": {"numElements": n, "offset": m}`, with `offset` optional (`virtual-schema-common-java`: `PushdownSqlParser.parseLimit` and `doc/development/api/virtual_schema_api.md`).
- Exasol's SQL reference (SELECT usage notes) accepts an `OFFSET` only together with `ORDER BY`, and `ORDER BY` stays in Exasol. No dialect advertises `LIMIT_WITH_OFFSET`, so Exasol applies every offset itself, and a `limit` that carries an `offset` fails with a named error.
- Clause form: MySQL (SELECT syntax `LIMIT {[offset,] row_count | row_count OFFSET offset}`), PostgreSQL (SELECT, LIMIT clause), and Trino (SELECT reference, LIMIT clause) each accept `LIMIT n` as the last clause of a `SELECT`, so every dialect renders the same clause.
- PostgreSQL and Trino assemble their statement in `dialect_common::render_select` through the `plain_dialect!` macro. MySQL assembles its own statement in `MySqlDialect::render`.
- Architecture: the Constraints line about ordering and limiting changes. See `architecture.md` in this plan directory.
- `specs/mission.md` capability 3 says "`ORDER BY` and `LIMIT`/`OFFSET` stay in Exasol", and `docs/pushdown.md` lists `LIMIT` and `OFFSET` under "What runs in Exasol". § Requirements updates both.
- Follow-up plans:
  - add-pushdown-top-n: investigate whether Exasol keeps the outer `ORDER BY` and re-applies the limit when `ORDER BY` and `LIMIT` are both advertised, and decide `LIMIT_WITH_OFFSET` together with it.

## Features

| Feature | Status | Spec |
|---------|--------|------|
| pushdown-limit | NEW | `vs-adapter/pushdown-limit/spec.md` |
| result-ordering | CHANGED | `vs-adapter/result-ordering/spec.md` |
| mysql-pushdown | CHANGED | `vs-adapter/mysql-pushdown/spec.md` |
| postgresql-pushdown | CHANGED | `vs-adapter/postgresql-pushdown/spec.md` |
| trino-pushdown | CHANGED | `vs-adapter/trino-pushdown/spec.md` |
| pushdown-dialect | CHANGED (Background only) | `vs-adapter/pushdown-dialect/spec.md` |
| pushdown-aggregation | CHANGED (Background only) | `vs-adapter/pushdown-aggregation/spec.md` |
| pushdown-projection | CHANGED (Background only) | `vs-adapter/pushdown-projection/spec.md` |

## Impact

- A `SELECT ... LIMIT n` with no `ORDER BY` over a virtual table reads at most n rows at the source and returns them. Before, the scan read the whole table and Exasol cut it.
- Behavior change: which n rows an unordered `LIMIT` returns is chosen by the source. They can differ from the rows an earlier release returned, and they can differ between runs. SQL promises no particular set, and `docs/pushdown.md` states how to get stable rows (add an `ORDER BY`).
- A query with an `ORDER BY` returns the same rows in the same order as before, with or without a `LIMIT` or `OFFSET`. Every `OFFSET` stays in Exasol.
- The `getCapabilities` response gains `LIMIT` for every driver, and `LIMIT_WITH_OFFSET` stays withheld. The adapter does not store the set in `adapterNotes`. After an upgrade or a rollback, an operator runs `ALTER VIRTUAL SCHEMA <name> REFRESH` on each existing virtual schema, so that Exasol asks for the new capability set.

## Requirements

| Requirement | Details |
|-------------|---------|
| Limit-shape gate | Before any product change, the probe adapter also answers `createVirtualSchema` with a second table `u` that has a column `d`, and `exasol_sends_the_measured_limit_shapes` in `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` creates the probe adapter (`crates/adbc-vs-adapter/tests/common/probe.rs`) with `Driver::Postgres.capabilities()` plus `LIMIT` when absent, never `LIMIT_WITH_OFFSET`, and asserts each row of the table below. |
| Gate: stop row fails | Stop the implementation and report the measured request to the user: a delegated limit would return wrong rows, and the interview rules out trading correctness for throughput. |
| Gate: adjust row fails | Change that scenario's delegation step in `vs-adapter/pushdown-limit/spec.md` to "no generated statement MUST carry a limit", keep its row-count steps, change the gate row's assertion to "request carries no `limit`", rename the scenario's test to `limit_over_<shape>_stays_in_exasol`, and update its § Scenario Coverage row. |
| Gate: offset row | A stop row. The test accepts any of its three outcomes. Any other outcome stops the implementation and is reported to the user. |
| Gate: record | Append "measured on the test Exasol" with the observed request to the reason of each query-shape scenario in `vs-adapter/pushdown-limit/spec.md`, in the form `vs-adapter/pushdown-dialect` uses, and make its Background's offset bullet state the measured outcome. If Exasol accepted `LIMIT 3 OFFSET 1` with no `ORDER BY`, also correct the `LIMIT ... OFFSET` reason in `architecture.md` § Constraints and in intent.md, so that both state the measured behavior. |
| Limit parsing | `numElements` is a required non-negative integer. A `limit` with an `offset`, whatever its value, fails naming `limit` and `LIMIT_WITH_OFFSET`. Offset 0 is refused too: Exasol sends the key only when `LIMIT_WITH_OFFSET` is advertised, so its presence means that the advertised set and the parser drifted apart. `orderBy` stays refused before any element is read, also beside a `limit`. |
| Limit rendering | One shared function in `dialect_common.rs` appends ` LIMIT n` after the `HAVING` clause, called from `render_select` and `MySqlDialect::render`. No dialect overrides it. |
| Mission | `specs/mission.md` capability 3: add "a `LIMIT` that no `ORDER BY` precedes" to the delegated fragments, and end with "`ORDER BY`, and every `LIMIT` or `OFFSET` that follows one, stay in Exasol." |
| User docs | `docs/pushdown.md`: a "`LIMIT` without `ORDER BY`" row with a check for all three databases under "What runs at the source"; under "What runs in Exasol", "every `OFFSET`, and a `LIMIT` after an `ORDER BY`" (`ORDER BY "price" LIMIT 10 OFFSET 5`) and a `LIMIT` above a part Exasol runs (`WHERE "price" + 0 > 10 LIMIT 5`); a section stating that an unordered `LIMIT` returns any n rows, which can change between runs, and that an `ORDER BY` makes them stable; a sentence telling operators to run `ALTER VIRTUAL SCHEMA <name> REFRESH` after an upgrade or a rollback. Every row matches the measured shapes. |

| Shape over probe table `t` | Required outcome | On failure |
|----------------------------|------------------|------------|
| `SELECT "d" FROM t LIMIT 3` | request carries `limit` with `numElements` 3 and no `offset` | stop: the plan delivers nothing |
| `... WHERE "d" > 1 LIMIT 3` | request carries `filter` and `limit` | adjust |
| `... GROUP BY "g" HAVING COUNT(*) > 0 LIMIT 3` | request carries `aggregationType`, `having`, and `limit` | adjust |
| join of `t` and `u` on `"d"` with `LIMIT 3` | request carries the join and `limit` | adjust |
| `... ORDER BY "d" LIMIT 3`, and with `OFFSET 1` | request carries no `limit` | stop |
| `... WHERE "d" + 0 > 1 LIMIT 3` | request carries no `limit` | stop |
| join of `t` with a local Exasol table, `LIMIT 3` | no request carries `limit` | stop |
| `SELECT "d" + 0, COUNT(*) FROM t GROUP BY "d" + 0 LIMIT 3` | request carries no `limit` | stop |
| `SELECT "d", COUNT(*) OVER () FROM t LIMIT 3` | request carries no `limit` | stop |
| `SELECT COUNT(*) FROM (SELECT "d" FROM t LIMIT 3)` | no request carries both an aggregate and `limit` | stop |
| `SELECT DISTINCT "g" FROM t LIMIT 3` | a request carrying `limit` also carries the grouping | stop |
| `SELECT "d" FROM t LIMIT 3 OFFSET 1` | Exasol rejects the statement, or the request carries no `limit`, or it carries `limit` with `numElements` 4 or more and no `offset` | stop: report the measured request to the user |

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| Constant entry | `crates/adbc-vs-adapter/src/pushdown_query.rs` `UNDELEGATED_ELEMENTS` (`limit` entry and its doc comment) | `limit` is parsed, so only `orderBy` is refused; a one-entry list becomes a single check |
| Test | `crates/adbc-vs-adapter/src/pushdown_query_tests.rs` `a_limit_element_is_refused` | `limit` is no longer refused |
| Test case | `crates/adbc-vs-adapter/src/postgres_dialect_tests.rs` `order_by_and_limit_elements_fail_named` (`limit` case) | Replaced by `order_by_element_fails_named` |
| Test | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` `generated_statement_carries_no_order_or_limit` | Replaced by `ordered_query_keeps_its_limit_and_offset_in_exasol`; an unordered `LIMIT` is now delegated |
| Test | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` and `e2e_trino_test.rs` `no_generated_statement_carries_order_limit_or_offset` | Same replacement |
| Comment | `crates/adbc-vs-adapter/src/capabilities_tests.rs` lines 282-285, `crates/adbc-vs-scan/drivers/trino.toml` lines 81-83 | They name `LIMIT` as withheld; only the ordering capabilities and `LIMIT_WITH_OFFSET` stay withheld |

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| pushdown-limit: Every dialect advertises LIMIT and withholds LIMIT_WITH_OFFSET | Unit | `crates/adbc-vs-adapter/src/capabilities_tests.rs` | `every_dialect_advertises_limit_without_offset` |
| pushdown-limit: A limit renders as the last clause of the statement | Unit | `crates/adbc-vs-adapter/src/{mysql,postgres,trino}_dialect_tests.rs` | `a_limit_renders_after_the_having_clause` |
| pushdown-limit: A limit element the adapter cannot read fails rather than being dropped | Unit | `crates/adbc-vs-adapter/src/pushdown_query_tests.rs` | `an_unreadable_limit_element_fails_naming_it` |
| pushdown-limit: A limit carrying an offset fails rather than dropping the offset | Unit | `crates/adbc-vs-adapter/src/pushdown_query_tests.rs` | `a_limit_with_an_offset_fails_naming_the_withheld_capability` |
| pushdown-limit: A LIMIT over a delegated filter reaches the source with the filter | Integration (E2E) | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `limit_over_a_delegated_filter_reaches_postgresql_with_the_filter` |
| pushdown-limit: A LIMIT over a delegated aggregation applies to the groups | Integration (E2E) | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `limit_over_a_delegated_aggregation_applies_to_the_groups` |
| pushdown-limit: A LIMIT over a delegated join applies to the joined rows | Integration (E2E) | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `limit_over_a_delegated_join_applies_to_the_joined_rows` |
| pushdown-limit: A LIMIT above a fragment Exasol evaluates itself stays in Exasol | Integration (E2E) | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `limit_above_a_fragment_exasol_evaluates_stays_in_exasol` |
| pushdown-limit: An aggregate over a limited derived table aggregates only the limited rows | Integration (E2E) | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `count_over_a_limited_derived_table_counts_only_the_limited_rows` |
| pushdown-limit: A LIMIT over DISTINCT returns that many distinct values | Integration (E2E) | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `limit_over_distinct_returns_that_many_distinct_values` |
| pushdown-limit: query-shape scenarios, request shapes | Integration (E2E, gate) | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `exasol_sends_the_measured_limit_shapes` |
| result-ordering: Capability set withholds ordering | Unit | `crates/adbc-vs-adapter/src/capabilities_tests.rs` | `no_dialect_advertises_an_ordering_capability` |
| result-ordering: An ordered query keeps its LIMIT and OFFSET in Exasol | Integration (E2E) | `crates/adbc-vs-adapter/tests/e2e_{mysql,postgres,trino}_test.rs` | `ordered_query_keeps_its_limit_and_offset_in_exasol` |
| result-ordering: Request carrying an order-by element fails rather than dropping it | Unit | `crates/adbc-vs-adapter/src/pushdown_query_tests.rs`, `postgres_dialect_tests.rs` | `an_order_by_element_is_refused`, `an_order_by_element_beside_a_limit_is_refused`, `order_by_element_fails_named` |
| result-ordering: the three stability scenarios (unchanged) | Integration (E2E) | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` | `ordered_query_is_stable_across_runs`, `ordered_query_with_ties_is_stable`, `ordering_survives_a_local_join` |
| mysql-pushdown: Capability set advertises the fragments this dialect renders | Unit | `crates/adbc-vs-adapter/src/capabilities_tests.rs` | `mysql_capability_set_is_exact` |
| mysql-pushdown: LIMIT pushdown reaches MySQL rather than Exasol | Integration (E2E) | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` | `limit_reaches_mysql_and_returns_rows_of_the_table` |
| postgresql-pushdown: Capability set advertises the fragments this dialect renders | Unit | `crates/adbc-vs-adapter/src/capabilities_tests.rs` | `postgres_capability_set_is_exact` |
| postgresql-pushdown: LIMIT pushdown reaches PostgreSQL rather than Exasol | Integration (E2E) | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `limit_reaches_postgresql_and_returns_rows_of_the_table` |
| trino-pushdown: Capability set advertises the fragments this dialect renders | Unit | `crates/adbc-vs-adapter/src/capabilities_tests.rs` | `trino_capability_set_is_exact` |
| trino-pushdown: Capability set withholds the fragments the Trino dialect does not delegate | Unit | `crates/adbc-vs-adapter/src/capabilities_tests.rs` | `trino_capability_set_is_exact` |
| trino-pushdown: The Trino set differs from the PostgreSQL set by two named entries, and each is stated on its own | Unit | `crates/adbc-vs-adapter/src/capabilities_tests.rs` | `trino_and_postgresql_sets_differ_by_two_named_entries` |
| trino-pushdown: LIMIT pushdown reaches Trino rather than Exasol | Integration (E2E) | `crates/adbc-vs-adapter/tests/e2e_trino_test.rs` | `limit_reaches_trino_and_returns_rows_of_the_table` |
| Every advertised capability has a rendering path (pushdown-dialect, now covering `LIMIT`) | Unit | `crates/adbc-vs-adapter/src/capabilities_tests.rs` | `every_advertised_capability_renders_per_dialect` |

The Background-only deltas of pushdown-dialect, pushdown-aggregation, and pushdown-projection change no scenario. Their existing tests stay as they are.

### Manual Testing

Run after `make test-e2e`, `make test-e2e-postgres`, and `make test-e2e-trino`, which leave the stack and the virtual schemas `MYSQLVS`, `PGVS`, and `TRVS` in place. `<dsn>` is the E2E Exasol connection string the test harness builds.

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| pushdown-limit | `exapump sql --dsn <dsn> 'SELECT PUSHDOWN_SQL FROM (EXPLAIN VIRTUAL SELECT "id" FROM PGVS."items" WHERE "id" > 1 LIMIT 2)'` | One `PUSHDOWN_SQL` row whose embedded statement ends with `WHERE ... LIMIT 2` |
| pushdown-limit | `exapump sql --dsn <dsn> 'SELECT "id" FROM PGVS."items" WHERE "id" > 1 LIMIT 2'` | Two rows, each with an `id` greater than 1 |
| result-ordering | `exapump sql --dsn <dsn> 'SELECT PUSHDOWN_SQL FROM (EXPLAIN VIRTUAL SELECT "id" FROM PGVS."items" ORDER BY "id" LIMIT 2 OFFSET 1)'` | The embedded statement carries no `ORDER BY`, `LIMIT`, or `OFFSET` |
| result-ordering | `exapump sql --dsn <dsn> 'SELECT "id" FROM PGVS."items" ORDER BY "id" LIMIT 2 OFFSET 1'` | The second and third `id` in ascending order |
| mysql-pushdown | `exapump sql --dsn <dsn> 'SELECT PUSHDOWN_SQL FROM (EXPLAIN VIRTUAL SELECT "id" FROM MYSQLVS."items" LIMIT 3)'` | The embedded statement ends with `LIMIT 3` |
| postgresql-pushdown | `exapump sql --dsn <dsn> 'SELECT "id" FROM PGVS."items" LIMIT 3'` | Three rows |
| trino-pushdown | `exapump sql --dsn <dsn> 'SELECT PUSHDOWN_SQL FROM (EXPLAIN VIRTUAL SELECT "id" FROM TRVS."items" LIMIT 3)'` | The embedded statement ends with `LIMIT 3` |
| pushdown-dialect, pushdown-aggregation, pushdown-projection | `make test` | 0 failures; Background-only change |

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Build | `make cross-udf-build` | Exit 0 |
| Unit tests | `make test` | 0 failures |
| Integration tests | `make test-it`, `make test-it-postgres`, `make test-it-trino` | 0 failures |
| E2E tests | `make test-e2e`, `make test-e2e-postgres`, `make test-e2e-trino` | 0 failures |
| Lint | `make lint` | 0 warnings |
| Format | `make fmt-check` | No changes |
