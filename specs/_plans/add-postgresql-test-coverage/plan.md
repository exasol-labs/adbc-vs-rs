# Plan: add-postgresql-test-coverage

## Summary

Close the four PostgreSQL test-coverage gaps issue #2 names — identifier case, view exposure, refresh success, and rejected-statement ADBC status — by adding one test per gap plus the fixture objects three of them read. One spec delta splits `vs-adapter/adapter-request-dispatch`'s refresh scenario from its `setProperties` half, so the refresh half carries the per-driver obligation task 2.2 satisfies.

## Design

### Context

Three of the four new tests need a PostgreSQL fixture object that does not exist: a table a refresh can grow a column onto, a view, and a table whose name and columns carry mixed case. MySQL's fixture module creates its own objects from Rust; PostgreSQL's does not. `crates/adbc-vs-adapter/tests/common/postgres.rs` states that difference as deliberate — `docker/postgres/init.sql` owns the fixture's shape, and the module only checks that the Compose seed ran. So the question is where the three new objects live, and whether `Postgres` gains the ability to change the source at all.

The refresh scenario forces half the answer. It requires "a virtual schema whose source has gained a column since creation", which no static seed can produce. Some Rust code must run DDL mid-run.

- **Goals** — one home for fixture shape; the staleness check covering the new objects for free; a mutation path narrow enough that no other test reaches for it.
- **Non-Goals** — no product-code change, no new test tier, no new Make target, no CI job change. The one spec delta splits an existing scenario in two and adds no obligation the library did not already carry. This plan also does not close the mixed-case gap on the MySQL side, which issue #2 does not raise.

### Decision

Static shape stays in the Compose seed. Mutation gets one method, used by one test.

#### Architecture

```
docker/postgres/init.sql  ──── Compose seed, fresh volume only ────▶ postgres container
   owns fixture SHAPE                                                      │  ▲
   (+ refresh_target, order_names, "MixedCase")                            │  │
                                                                           │  │
crates/adbc-vs-adapter/tests/common/postgres.rs                            │  │
   SEEDED_TABLES     ── proves the seed a run depends on actually ran ──────┘  │
   Postgres::execute ── the one mid-run MUTATION the refresh scenario needs ───┘
                        (ALTER TABLE ... ADD/DROP COLUMN, nothing else)
```

#### Patterns

| Pattern | Where | Why |
|---------|-------|-----|
| Single owner for fixture shape | `docker/postgres/init.sql` | Splitting shape across SQL and Rust would let two places decide it independently, and a reviewer diffing the mirror against the seed would read half the fixture |
| Fail-loud staleness check | `SEEDED_TABLES` in `common/postgres.rs` | A container that outlived a seed change presents an older fixture; without the new names there, the three new tests fail as mysteries instead of naming the recreate command |
| Compute-then-assert | `refresh_picks_up_a_new_postgresql_column` | The refresh runs against the virtual schema every other test in the binary reads; a panic between the `ADD COLUMN` and the `DROP COLUMN` would poison it for whichever test runs next |
| Verify each delegation twice | the three end-to-end tests | `EXPLAIN VIRTUAL` proves the identifier reached the generated statement; the returned rows prove the source accepted it |

### Consequences

| Decision | Alternatives Considered | Rationale |
|----------|------------------------|-----------|
| Add the three static objects to `docker/postgres/init.sql` | Create all three from a new `Postgres::create_fixture`, leaving the seed untouched | The seed is the stated home for PostgreSQL fixture shape, and `SEEDED_TABLES` then guards the new objects with no extra code |
| Add `Postgres::execute`, scoped by its doc comment to mid-run mutation | Route the `ALTER` through `query_scalar`, which returns `psql`'s command tag | `query_scalar` asserts a single-value result; passing DDL through it would make a rejected statement read as a value |
| Split `Refresh and setProperties re-run metadata discovery` into a `refresh` scenario carrying "for each driver this release supports" and a `setProperties` scenario without it | (a) Widen the whole scenario's `GIVEN` with that phrase; (b) author no delta at all | Widening the whole scenario obliges a per-driver `setProperties` success test issue #2 does not raise; authoring nothing leaves task 2.2's test implementing no spec obligation, so a later consolidation of the feature could delete it without violating anything |
| Reach `Postgres` from a test through a new `pub postgres` field on `postgres_stack::Stack` | Construct `Postgres::from_stack()` inside `refresh_picks_up_a_new_postgresql_column` | A per-test construction re-runs the readiness wait and the seed check on every call, and leaves two construction sites for one container; the field matches `common/stack.rs`'s `pub mysql: MySql`, which is how the MySQL tier already reaches its source |
| Prove mixed case with a quoted `"MixedCase"` table | Assert only that lower-case names stay lower case, as the MySQL test does | PostgreSQL folds unquoted identifiers to lower case, so a lower-case-only test cannot tell case preservation from case folding; the scenario's mixed-case `THEN` clause needs a mixed-case source |
| Read rows and `EXPLAIN VIRTUAL` from one `ORDER BY` query per test | Separate `COUNT(*)` and `EXPLAIN` queries | `ORDER BY` is never delegated, so the query yields exactly one generated statement and `generated_statement` stays usable |

Reviewed against `/speq:design-philosophy`'s Quick Diagnostic. Two interfaces are new: `Postgres::execute`, and a `pub postgres: Postgres` field on `postgres_stack::Stack` that gives one test a route to it. `Postgres::execute` owns one decision — how a statement reaches `psql` inside the container — and shares `client_arguments` with `query_scalar`, so the credential-handling decision keeps its single owner. Changing how the statement is delivered forces no edit outside the module. Its doc comment states the boundary rather than restating the name. The `Stack` field adds no decision of its own: it publishes a value `bring_up` already constructs, exactly as `common/stack.rs` publishes `pub mysql: MySql`.

## Features

One feature changes. The other two already carry the scenarios this plan implements for the PostgreSQL driver.

| Feature | Status | Spec |
|---------|--------|------|
| `vs-adapter/create-virtual-schema` | UNCHANGED | `specs/vs-adapter/create-virtual-schema/spec.md` |
| `vs-adapter/adapter-request-dispatch` | CHANGED | `specs/_plans/add-postgresql-test-coverage/vs-adapter/adapter-request-dispatch/spec.md` |
| `scan-udf/arrow-scan` | UNCHANGED | `specs/scan-udf/arrow-scan/spec.md` |

Three of the four scenarios issue #2 names already read "for each driver this release supports", so PostgreSQL coverage was always required there and only the tests were missing. The fourth did not: `Refresh and setProperties re-run metadata discovery` names no driver, so one driver's test satisfies it and `e2e_mysql_test.rs::refresh_picks_up_new_column` already does. The delta splits that scenario into a `refresh` half carrying the per-driver phrase and a `setProperties` half without it, which anchors task 2.2's test in the library without obliging the per-driver `setProperties` success test issue #2 does not raise. Decision `[1]` in `decision-log.md` records the split and the two alternatives rejected.

## Impact

Test-only. No product code changes, so no adapter, scan, or provisioning behavior changes for a user or an operator.

One local-workflow consequence: `docker/postgres/init.sql` runs only against a fresh volume, so a developer holding a `postgres` container from before this change must recreate it with `docker compose rm -sfv postgres && make stack-up-postgres`. Skipping that step fails `Postgres::check_fixture`, which already names the recovery. CI starts every stack fresh and is unaffected.

## Dependencies

None. No new crate, no new registry driver, no new Make target, no new CI job.

## Implementation Tasks

- [ ] 1.1 Extend the PostgreSQL Compose seed with the three fixture objects the end-to-end tests need, guarded by the staleness check.
  - Red first: add `refresh_target`, `order_names`, and `MixedCase` to `SEEDED_TABLES` in `crates/adbc-vs-adapter/tests/common/postgres.rs`, then run `make test-e2e-postgres` — `Postgres::check_fixture` must fail, naming the missing objects and the recreate command.
  - Green: add to `docker/postgres/init.sql`, following the file's existing comment convention (each object states why it exists and which test reads it):
    - `refresh_target (id INTEGER NOT NULL, name VARCHAR(30) NOT NULL, PRIMARY KEY (id))` with one seeded row. Dedicated to the refresh test so its `ALTER` disturbs no table another test reads.
    - `order_names` as `CREATE VIEW order_names AS SELECT id, name FROM orders` — both source columns are mapped types, so the view exposes two columns and adds no `omitted` entry.
    - `"MixedCase" ("ItemId" INTEGER NOT NULL, "Label" VARCHAR(20) NOT NULL, PRIMARY KEY ("ItemId"))` with two seeded rows. Every identifier is quoted, or PostgreSQL folds it to lower case and the object cannot prove case preservation.
  - Recreate the service (`docker compose rm -sfv postgres && make stack-up-postgres`) and confirm `make test-e2e-postgres` and `make test-it-postgres` still pass unchanged.
  - Add no `postgres_mirror` counterpart: none of the three objects is read differentially, and a mirror column compared against nothing is dead fixture.

- [ ] 2.1 Prove PostgreSQL identifier case survives discovery and rendering. Add `identifiers_keep_their_case_through_discovery_and_rendering` to `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs`.
  - `table_names(VIRTUAL_SCHEMA)` MUST contain `items`, `orders`, and `MixedCase`, and MUST contain neither `ITEMS` nor `MIXEDCASE`.
  - `column_names(VIRTUAL_SCHEMA, "MixedCase")` MUST equal `["ItemId", "Label"]`.
  - `generated_statement` for `SELECT "ItemId", "Label" FROM PGVS."MixedCase" ORDER BY "ItemId"` MUST carry `"public"."MixedCase"` and `"ItemId"`.
  - The same query MUST return the two seeded rows, which is what separates a correctly rendered identifier from one the source rejected.

- [ ] 2.2 Prove a normal PostgreSQL `REFRESH` re-runs metadata discovery, satisfying the delta's `Refresh re-runs metadata discovery` scenario for the PostgreSQL driver. Add `refresh_picks_up_a_new_postgresql_column` to `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs`, `Postgres::execute` to `crates/adbc-vs-adapter/tests/common/postgres.rs`, and the handle that reaches it to `crates/adbc-vs-adapter/tests/common/postgres_stack.rs`.
  - `Postgres::execute` runs a statement through the existing `client_arguments` path and panics on a rejected statement, so a DDL no-op cannot read as a pass. Its doc comment MUST state the boundary: `docker/postgres/init.sql` owns the fixture's shape, and this method exists for the one test that must change the source mid-run.
  - In `postgres_stack.rs`, add `pub postgres: Postgres` to `Stack`, bind it in `bring_up()` as `let postgres = Postgres::from_stack();` instead of `let _postgres`, and return it in the `Stack` literal — mirroring `common/stack.rs`'s `pub mysql: MySql`, which is how `e2e_mysql_test.rs` reaches `stack.mysql.execute(...)`. Rewrite `bring_up`'s doc comment: it currently justifies dropping both values with "neither constructed value outlives this function", which stops being true for `postgres`. Leave `MySql` bound as `let _mysql` — no PostgreSQL test mutates the MySQL source. Do NOT construct a second `Postgres::from_stack()` inside the test: it re-runs `await_queries` and `check_fixture` on every call and leaves one container with two construction sites.
  - The test adds `extra NUMERIC(9,3)` to `refresh_target`, runs `ALTER VIRTUAL SCHEMA PGVS REFRESH`, drops the column, and refreshes again.
  - Follow `e2e_mysql_test.rs::refresh_picks_up_new_column`: read all three column sets first and assert only after the second refresh has restored the source. A panic between the `ADD COLUMN` and the `DROP COLUMN` leaves the shared virtual schema declaring a column the source no longer has, which breaks whichever test runs next rather than this one.
  - Assert on `declared_columns`, not `column_names`: the new column MUST arrive as `("extra", "DECIMAL(9,3)")`. The declared Exasol type is what proves discovery ran through PostgreSQL's own `numeric` mapping rather than returning a cached shape.
  - Assert the restored column set equals the set read before the `ADD COLUMN`.

- [ ] 2.3 Prove a PostgreSQL view is exposed as a virtual table. Add `views_are_exposed_alongside_base_tables` to `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs`.
  - `table_names(VIRTUAL_SCHEMA)` MUST contain `order_names` beside the base tables.
  - `SELECT "id", "name" FROM PGVS."order_names" ORDER BY "id"` MUST return the three rows `orders` seeds.
  - `generated_statement` for that query MUST name `"public"."order_names"`, which proves the view was queried through the pushdown path as a table rather than resolved by Exasol.

- [ ] 2.4 Prove a rejected PostgreSQL statement surfaces the ADBC status and the driver's own diagnosis. Add `source_failure_reports_adbc_status` to `crates/adbc-vs-scan/tests/postgres_it_test.rs`, mirroring `crates/adbc-vs-scan/tests/mysql_it_test.rs::source_failure_reports_adbc_status`.
  - Scan `SELECT "c1", "c2" FROM "no_such_table"` with two `DeclaredType::Text` columns built through the module's existing `declared` helper.
  - Take the error from `execute_scan` when it fails there, and otherwise from the first failing batch of the returned stream — the MySQL test's own shape, because the driver may defer the failure to the first batch.
  - The message MUST contain `ADBC status`, MUST contain `no_such_table`, and MUST NOT contain `POSTGRES_PASSWORD`.
  - Take no serialization guard. `mysql_it_test.rs::serialized` exists for its resident-set measurement, which this tier has no counterpart for.

- [ ] 3.1 Update `CLAUDE.md` for the coverage this plan adds.
  - Extend the `make test-it-postgres` "Covers:" bullet with the rejected-statement ADBC status.
  - Extend the `make test-e2e-postgres` bullet with identifier case, view exposure, and refresh success.
  - Record that `docker/postgres/init.sql` seeds the PostgreSQL fixture only against a fresh volume, so a change to it requires `docker compose rm -sfv postgres && make stack-up-postgres`, and that `SEEDED_TABLES` in `crates/adbc-vs-adapter/tests/common/postgres.rs` is what turns a stale container into a named failure.

## Parallelization

| Parallel Group | Tasks |
|----------------|-------|
| Group A | 1.1 — alone |
| Group B | 2.1, then 2.2, then 2.3 — sequential, one file |
| Group B' | 2.4 — concurrent with Group B |
| Group C | 3.1 |

Sequential dependencies:
- Group A → Group B — 2.1, 2.2, and 2.3 all read fixture objects task 1.1 seeds.
- Group A → Group B' — task 1.1 runs `docker compose rm -sfv postgres && make stack-up-postgres`, destroying and recreating the container task 2.4's tier queries at `postgresql://127.0.0.1:5432/adbc_vs`, and then re-runs `make test-it-postgres`, the binary 2.4 edits. No task driving the PostgreSQL stack may overlap task 1.1: a mid-scan container recreate surfaces as an ADBC connection error, which is the exact error family 2.4 asserts on, so the collision would read as the behavior under test.
- Group B → Group C — 3.1 names the tests the earlier tasks add.

Groups B and B' may overlap. Task 2.4's tier writes nothing to the fixture database (`postgres_it_test.rs` projects its wide result from `generate_series`), and task 2.2's `ALTER TABLE` touches only `refresh_target`, which the integration tier never reads. Both invoke `cargo test` in one workspace, so they contend on the `target/` lock — a serialized build, not a failure. Tasks 2.1, 2.2, and 2.3 all edit `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` and run one after another rather than concurrently.

## Dead Code Removal

None. This plan adds tests and fixture objects; it replaces no function, test, or module. No existing test becomes redundant — each new test covers a driver its MySQL counterpart does not reach.

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| `vs-adapter/create-virtual-schema` — Source identifiers keep their original case, PostgreSQL driver | Integration | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `identifiers_keep_their_case_through_discovery_and_rendering` |
| `vs-adapter/create-virtual-schema` — Views are exposed alongside base tables, PostgreSQL driver | Integration | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `views_are_exposed_alongside_base_tables` |
| `vs-adapter/adapter-request-dispatch` — Refresh re-runs metadata discovery, PostgreSQL driver | Integration | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `refresh_picks_up_a_new_postgresql_column` |
| `vs-adapter/adapter-request-dispatch` — Refresh re-runs metadata discovery, MySQL driver | Integration | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` | `refresh_picks_up_new_column` — exists, unchanged |
| `vs-adapter/adapter-request-dispatch` — setProperties re-runs metadata discovery | Integration | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` | `all_request_types_return_matching_type` — exists, unchanged |
| `scan-udf/arrow-scan` — Source failure surfaces the driver's own diagnosis, PostgreSQL driver, statement-rejection half | Integration | `crates/adbc-vs-scan/tests/postgres_it_test.rs` | `source_failure_reports_adbc_status` |

The two scenarios the delta splits out of `Refresh and setProperties re-run metadata discovery` keep every test they already had. `refresh_picks_up_new_column` covers the refresh scenario's MySQL driver, and task 2.2 adds its PostgreSQL driver — the half the per-driver phrase now obliges. `all_request_types_return_matching_type` covers the `setProperties` scenario by pointing a virtual schema at an empty database and asserting the re-discovered table set is empty; that scenario carries no per-driver phrase, so this plan adds no test for it. The connection-refusal half of the source-failure scenario stays covered by `e2e_postgres_test.rs::unreachable_host_never_leaks_the_password_across_all_three_entry_points`. Neither is a gap issue #2 raises.

### Manual Testing

Run `make stack-up` first, and recreate the `postgres` service once so the new seed applies:

```bash
docker compose rm -sfv postgres && make stack-up
```

`DSN` below is `exasol://sys:exasol@localhost:8563?validateservercertificate=0`.

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| `vs-adapter/create-virtual-schema` — identifier case | `exapump sql --dsn "$DSN" 'SELECT "ItemId", "Label" FROM PGVS."MixedCase" ORDER BY "ItemId"'` | Two rows under the column headers `ItemId` and `Label`, in the source's case |
| `vs-adapter/create-virtual-schema` — identifier case | `exapump sql --dsn "$DSN" "SELECT TABLE_NAME FROM SYS.EXA_ALL_TABLES WHERE TABLE_SCHEMA = 'PGVS' ORDER BY TABLE_NAME"` | Includes `MixedCase`, `items`, `orders`; includes no upper-cased spelling of any of them |
| `vs-adapter/create-virtual-schema` — view exposure | `exapump sql --dsn "$DSN" 'SELECT COUNT(*) FROM PGVS."order_names"'` | `3` |
| `vs-adapter/adapter-request-dispatch` — refresh | `docker compose exec postgres psql -U adbc_vs -d adbc_vs -c 'ALTER TABLE refresh_target ADD COLUMN extra NUMERIC(9,3)'`, then `exapump sql --dsn "$DSN" 'ALTER VIRTUAL SCHEMA PGVS REFRESH'`, then `exapump sql --dsn "$DSN" "SELECT COLUMN_NAME, COLUMN_TYPE FROM SYS.EXA_ALL_COLUMNS WHERE COLUMN_SCHEMA = 'PGVS' AND COLUMN_TABLE = 'refresh_target' ORDER BY COLUMN_ORDINAL_POSITION"` | `id`, `name`, then `extra` as `DECIMAL(9,3)`. Restore with the matching `DROP COLUMN` and a second `REFRESH` |
| `scan-udf/arrow-scan` — source failure | `docker compose exec postgres psql -U adbc_vs -d adbc_vs -c 'SELECT * FROM no_such_table'` | `ERROR: relation "no_such_table" does not exist` — the diagnosis the ADBC error must carry through |

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Recreate the seeded PostgreSQL service | `docker compose rm -sfv postgres && make stack-up-postgres` | Exit 0 |
| Build | `make cross-udf-build` | Exit 0 |
| Format | `make fmt-check` | No changes |
| Lint | `make lint` | 0 warnings |
| Unit tests | `make test` | 0 failures |
| Integration, MySQL | `make test-it` | 0 failures |
| Integration, PostgreSQL | `make test-it-postgres` | 0 failures, including `source_failure_reports_adbc_status` |
| E2E, PostgreSQL | `make test-e2e-postgres` | 0 failures, including the three new tests |
| E2E, MySQL | `make test-e2e` | 0 failures — the shared stack change must not regress the MySQL tier |
| Dependency policy | `make deny` | 0 findings |
