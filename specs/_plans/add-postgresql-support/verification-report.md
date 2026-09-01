# Verification Report: add-postgresql-support

## Verdict

| Result | Details |
|--------|---------|
| **PASS, with one outstanding pre-record item** | All automated checklist tiers pass (0 failures across 781 tests + 9 policy tests). Scenario coverage traces to a passing test for effectively every plan.md scenario; 4 minor PostgreSQL-side gaps found and documented (framework scenarios tested only against MySQL — see Notes). Manual verification executed live against both stacks with captured, real output. Record-time check (task 10.7) confirms the pre-record state exactly as plan.md predicts. **Task 2.3 is not actually complete**: two spec-delta Background bullets it identified as needing rewording "before record" (decision-log.md divergence 3, and the `scan-udf/postgresql-scan` NaN/Infinity bullet) are still unreworded in the delta files as of this verification — see "Outstanding item: task 2.3" below. `tasks.md` still shows `2.3` as `[~]`, deliberately not flipped to `[x]`, because it is not done. |
| Code review | 37 findings — standard: 26, expert: 11 — all fixed |

| Check | Status |
|-------|--------|
| Build (`make cross-udf-build`) | ✓ |
| Tests (unit + artifact + integration + e2e, both drivers) | ✓ |
| Lint (`make lint`) | ✓ |
| Format (`make fmt-check`) | ✓ |
| Deny (`make deny`) | ✓ |
| Scenario Coverage | ✓ (4 documented gaps, non-blocking — see Notes) |
| Manual Tests | ✓ |

## Test Evidence

### Coverage

No line/branch coverage tool is part of this repo's toolchain (`CLAUDE.md` defines no `make coverage` target); coverage is reasoned about through the Scenario Coverage cross-reference below instead.

### Test Results

| Type | Run | Passed | Failed |
|------|-----|--------|--------|
| Unit (`make test`, `cargo test --workspace`) | 346 | 346 | 0 |
| Artifact (`make test-artifact`) | 3 | 3 | 0 |
| Integration — MySQL (`make test-it`) | 6 | 6 | 0 |
| Integration — PostgreSQL (`make test-it-postgres`) | 3 | 3 | 0 |
| E2E — MySQL (`make test-e2e`) | 215 | 215 | 0 |
| E2E — PostgreSQL (`make test-e2e-postgres`) | 199 | 199 | 0 |
| Policy (`make deny`'s `policy_test`) | 9 | 9 | 0 |
| **Total** | **781** | **781** | **0** |

`make lint` and `make fmt-check` both exit 0. Full raw output for every tier: `target/speq:checklist.log` (not reproduced here; trusted as current per the task brief — working tree unchanged since).

### Manual Tests

Executed live against the running Exasol (`exasol/docker-db:2025.2.1`), MySQL (`mysql:8.4`), and PostgreSQL (`postgres:18.6-trixie`) containers. `PG_CONN`, `MY_CONN`, `BFS_CONN`, and the `pgvs`/`pgvs2`/`pgvs3`/`pgvs_badpw`/`myvs` schemas were created for this run and dropped afterward (verified via `SYS.EXA_ALL_VIRTUAL_SCHEMAS` / `SYS.EXA_ALL_CONNECTIONS` post-cleanup — none of the objects this run created remain).

Two adaptations from plan.md's literal table, both noted inline below:
- The table's MySQL-side `EXPLAIN VIRTUAL` examples wrap the pushed-down-to identifiers in backtick (`` ` ``) quoting. Exasol's own SQL grammar has no backtick quoting at all — `EXPLAIN VIRTUAL` is Exasol SQL, sent to Exasol, regardless of which dialect it will delegate to — so the literal commands are invalid syntax. Every command below uses Exasol's own double-quote identifier syntax instead; the *rendered* pushdown SQL — the thing under test — still shows MySQL's backtick quoting and PostgreSQL's double-quote quoting, correctly, in the `PUSHDOWN_SQL` payload.
- The table's example schema uses `orders`/`customers` with a `cid`/`id` join. PostgreSQL's real fixture (`docker/postgres/init.sql`) has no `customers` table and `orders` has no `cid` column; MySQL's `orders`/`customers` pair exists but not with those exact columns either. The real cross-dialect join fixture both drivers share is `items`/`item_notes` (join key `items.id = item_notes.item_id`), so every join-shaped command below uses that pair instead.

| Feature | Command | Result |
|---------|---------|--------|
| driver-selection | `CREATE VIRTUAL SCHEMA pgvs ... DRIVER = 'snowflake' ...` | ✓ Failed: `DRIVER 'snowflake' names no driver adbc-vs supports; the supported drivers are 'mysql', 'postgresql'` |
| create-virtual-schema | `CREATE VIRTUAL SCHEMA pgvs ... DRIVER = 'postgresql' ...` | ✓ Succeeded; `EXA_ALL_VIRTUAL_TABLES` for `PGVS` lists `item_notes`, `items`, `numeric_specials`, `orders` (4 tables; `all_unmapped` correctly absent) |
| driver-self-install | `exapump bucketfs ls --recursive adbc-drivers ...` (exapump 0.12.0 spells this subcommand `bucketfs`, not `bfs`, superseding plan.md's `exapump bfs ls`) | ✓ Lists `postgresql/1.12.0/libadbc_driver_postgresql.so` and `mysql/0.6.0/libadbc_driver_mysql.so`, each under its own technology/version segments |
| driver-version-resolution | `SELECT ADAPTER_NOTES FROM SYS.EXA_ALL_VIRTUAL_SCHEMAS WHERE SCHEMA_NAME='PGVS'` | ✓ `driverVersion":"1.12.0"`, `driverPath":".../adbc-drivers/postgresql/1.12.0/libadbc_driver_postgresql.so"`; no credential present anywhere in the JSON |
| driver-path-override (success) | `CREATE VIRTUAL SCHEMA pgvs2 ... DRIVER_PATH = '<the resolved path above>'` (no `BUCKETFS_CONNECTION`) | ✓ Succeeded with no registry/BucketFS-write property; `SELECT "id" FROM pgvs2."orders"` returned the 3 seeded rows through the supplied library |
| driver-path-override (missing file) | `CREATE VIRTUAL SCHEMA pgvs3 ... DRIVER_PATH = '/buckets/bfsdefault/default/custom/pg-driver.so'` | ✓ Failed: `the DRIVER_PATH property names '/buckets/bfsdefault/default/custom/pg-driver.so' as the 'postgresql' driver shared library, but no readable file is there ... A supplied driver path is used exactly as given and is never fallen back on, so no driver was fetched from the registry instead` |
| pushdown-dialect | `EXPLAIN VIRTUAL SELECT * FROM pgvs."items" o FULL OUTER JOIN pgvs."item_notes" c ON o."id"=c."item_id"`, then the same shape on `myvs` | ✓ PostgreSQL's `PUSHDOWN_SQL` carries one statement with `"public"."items" AS "t1" FULL OUTER JOIN "public"."item_notes" AS "t2"`; MySQL's `EXPLAIN VIRTUAL` returns **two** separate pushed-down statements (`items` alone, `item_notes` alone), no join in either — Exasol withdrew and performed the join itself |
| postgresql-pushdown | `EXPLAIN VIRTUAL SELECT "id","tag" FROM pgvs."items"` | ✓ `PUSHDOWN_SQL` projects exactly `"id", "tag"`, both double-quoted |
| postgresql-collation | `... WHERE "sort_key" < 'm'` | ✓ Renders `("sort_key" COLLATE "C" < E'm' COLLATE "C")` — both operands collated |
| postgresql-collation | `... WHERE "category" = 'widget'` | ✓ Renders `("category" = E'widget')` — no collation suffix |
| postgresql-collation | `SELECT "category", COUNT(*) FROM pgvs."items" GROUP BY "category"` | ✓ Returns 4 groups (`Widget`:2, `cafe`:1, `café`:1, `widget`:2 — case/accent-sensitive, matching Exasol's own comparison); generated `GROUP BY "category"` carries no collation suffix |
| postgresql-pushdown | Same FULL OUTER JOIN command as the pushdown-dialect row | ✓ (see pushdown-dialect row above — one delegated statement) |
| mysql-collation | `EXPLAIN VIRTUAL SELECT "id" FROM myvs."items" WHERE "category" = 'Widget'` | ✓ Renders `` (`category` COLLATE utf8mb4_0900_as_cs = _utf8mb4 X'576964676574' COLLATE utf8mb4_0900_as_cs) `` — MySQL carries the suffix where the PostgreSQL bare-equality row above carries none |
| result-ordering | `EXPLAIN VIRTUAL SELECT "id" FROM pgvs."items" ORDER BY "id" LIMIT 5` | ✓ Generated statement is exactly `SELECT "id" FROM "public"."items"` — no `ORDER BY`, `LIMIT`, or `OFFSET` |
| postgresql-type-mapping | `SELECT ADAPTER_NOTES ...` (same query as driver-version-resolution) | ✓ `omitted` lists `boolean`, `text`, `character`, `jsonb`, and unconstrained-`numeric` columns (among others), each under `sourceType` with a stated reason |
| postgresql-scan | `SELECT "id","amount" FROM pgvs."orders"` | ✓ Returns `1234.5600`, `NULL`, `99999999.9999` — scale intact, arrived from the driver as strings |
| postgresql-scan | `CREATE VIRTUAL SCHEMA` with a wrong password, then query | ✓ Failed naming host `172.28.5.11:5432` and database `adbc_vs`; the wrong password string appears nowhere in the error |
| adapter-request-dispatch | `ALTER VIRTUAL SCHEMA pgvs REFRESH` | ✓ Succeeded (operational smoke test only — see Notes for the automated-coverage gap this does not close) |

## Tool Evidence

### Linter

```
cargo clippy --workspace --all-targets --all-features -- -D warnings
EXIT:0
```

### Formatter

```
cargo fmt --all -- --check
EXIT:0
```

## Scenario Coverage

Plan.md's own `## Verification > ### Scenario Coverage` tables (lines 437-708) already enumerate every scenario against a domain/feature/test-type/test-location/test-name row — roughly 150 scenario rows across 22 features (16 `vs-adapter`, 3 `scan-udf`, 3 `driver-provisioning`). Rather than reproduce that table verbatim a second time, this audit cross-referenced every listed test name against the actual test suite (`grep -rn "fn <name>"` across `crates/`) and reports the outcome by category.

**Result: 100/157 literal test names matched an identically-named function. Of the 57 that did not, 53 were traced to a passing, behaviorally-equivalent test under a different name or location, and 4 are genuine coverage gaps** (below). None of the 57 indicate a scenario that silently regressed — the checklist's 781/781 pass rate holds regardless — but the literal test names plan.md records have drifted from what implementation and code review actually produced, which is worth recording so a future scenario audit does not have to re-derive this mapping.

The drift falls into three explainable buckets, verified by reading the actual test/comment content rather than assumed from name similarity alone:

1. **Explicit review-driven consolidation.** Standard-fix 12.13 deleted 7 capability-membership tests (`filter_capabilities_match_per_dialect`, `aggregation_capabilities_match_per_dialect`, `mysql_join_capabilities_are_exact`, `postgres_join_capabilities_add_full_outer`, `dialect_sets_differ_only_in_full_outer_join`, `every_dialect_withholds_ordering`, `avg_and_sum_are_withheld_per_dialect`) and folded their rationale into code comments on `mysql_capability_set_is_exact` / `postgres_capability_set_is_exact`'s capability-list entries in `capabilities_tests.rs` — confirmed by reading those comments (e.g. `JOIN_TYPE_FULL_OUTER is the whole of the difference between this set and RELEASE_0_2_MYSQL_CAPABILITIES`).
2. **Refactor-driven relocation.** Expert fixes 11.1/11.3 split MySQL- and PostgreSQL-specific source logic out of `create_schema_tests.rs` into `mysql_source_tests.rs` / `postgres_source_tests.rs`. Scenarios plan.md lists at `create_schema_tests.rs` (e.g. "source server without required collations is rejected," "database whose encoding is not UTF8 is rejected") now live as `a_source_missing_a_required_collation_is_refused_naming_it_and_the_version` and `a_database_of_another_encoding_is_refused_naming_it_and_utf8` in those new files.
3. **Ordinary TDD naming refinement.** The large majority of the remaining 46 are the same scenario under a more descriptive name chosen during implementation (e.g. `full_outer_join_matches_local_exasol_result` → `full_outer_join_is_delegated_and_returns_both_unmatched_rows`; `decimal_width_variation_is_normalized` → `decimal_widths_normalize`), or a scenario plan.md listed twice under two feature headings (framework + technology) that implementation correctly satisfies with one shared test (e.g. `unsupported_driver_names_every_supported_value` and `unsupported_driver_value_fails_named` are the same `dispatch_test.rs` test serving both the `driver-selection` and `create-virtual-schema` feature tables).

### Genuine gaps (4)

All four are the same shape: a **framework** scenario plan.md's own convention says should get "one test per dialect," where the pre-existing MySQL test (release 0.1) is real but no PostgreSQL-side equivalent was added by this plan. None represents a known defect — the shared code path is exercised by the MySQL test and by the green e2e/integration runs — but the PostgreSQL side of these four scenarios has no dedicated regression test:

| Feature | Scenario | MySQL test (exists) | PostgreSQL test (absent) |
|---------|----------|----------------------|---------------------------|
| `vs-adapter/create-virtual-schema` | Source identifiers keep their original case | `identifiers_keep_original_case` (`e2e_mysql_test.rs`) | None. `docker/postgres/init.sql`'s identifiers are all lower-case, so case-preservation is untested for PostgreSQL even implicitly |
| `vs-adapter/create-virtual-schema` | Views are exposed alongside base tables | `views_are_exposed` (`e2e_mysql_test.rs`) | None. `docker/postgres/init.sql` defines no view at all |
| `vs-adapter/adapter-request-dispatch` | Refresh and setProperties re-run metadata discovery | `refresh_picks_up_new_column` (`e2e_mysql_test.rs`, actually adds a column and asserts refresh discovers it) | `e2e_postgres_test.rs`'s only `REFRESH` usage (`credential_failures`) asserts that a *failing* refresh does not leak a password; no test asserts a *successful* PostgreSQL refresh re-discovers new source state |
| `scan-udf/arrow-scan` | Source failure surfaces the driver's own diagnosis | `source_failure_reports_adbc_status` (`mysql_it_test.rs`) | None in `postgres_it_test.rs` |

Recommendation: not blocking for this plan (the checklist is green and the underlying dispatch/refresh/error-surfacing code is dialect-agnostic and exercised via MySQL), but worth a follow-up task if a future plan touches any of these four code paths.

One additional item is weaker evidence rather than a gap: `vs-adapter/create-virtual-schema`'s "Creating a virtual schema exposes the source tables" has no dedicated PostgreSQL test under any name, but is proven implicitly by `mirror_declares_the_same_columns_and_types_as_the_virtual_table` (which would fail if `items`/`item_notes` were not exposed) and by `every_supported_driver_creates_a_virtual_schema`'s `!table_names(&schema_name).is_empty()` assertion.

## Record-time check (task 10.7)

Ran `specs/_plans/add-postgresql-support/plan.md`'s § Recording notes post-record check verbatim against the current, unmerged library:

```
FAIL  vs-adapter/pushdown-dialect
FAIL  vs-adapter/pushdown-projection
FAIL  vs-adapter/pushdown-filter
FAIL  vs-adapter/pushdown-aggregation
FAIL  vs-adapter/pushdown-join
FAIL  vs-adapter/type-mapping
FAIL  vs-adapter/create-virtual-schema
FAIL  vs-adapter/result-ordering
FAIL  vs-adapter/adapter-request-dispatch
FAIL  scan-udf/arrow-scan
FAIL  driver-provisioning/driver-self-install
exit_code=1
```

`FAIL` on all eleven, exit 1 — exactly the pre-record state plan.md predicts. No drift: the library has not merged this plan's Background/description rewrites, so `/speq:record` still owes the manual head-replacement step this section documents (§ Recording notes: apply the Background and description rewrites by hand, plan.md lines 363-436) for these eleven features: `vs-adapter/pushdown-dialect`, `vs-adapter/pushdown-projection`, `vs-adapter/pushdown-filter`, `vs-adapter/pushdown-aggregation`, `vs-adapter/pushdown-join`, `vs-adapter/type-mapping`, `vs-adapter/create-virtual-schema`, `vs-adapter/result-ordering`, `vs-adapter/adapter-request-dispatch`, `scan-udf/arrow-scan`, `driver-provisioning/driver-self-install`. This obligation must reach whoever runs `/speq:record` next, not sit only in plan.md.

## Outstanding item: task 2.3

Task 2.3 (`Gate: measure PostgreSQL dialect semantic assumptions vs postgres:18.6-trixie + Exasol`) is marked `[~]` in `tasks.md`, not `[x]`, and this verification leaves it that way rather than completing it — it falls in Phase 2/Group C, outside this verification pass's assigned scope (10.5-10.8), and its remaining obligation is spec-delta prose, which a verification pass does not author.

Task 2.3's own text states three Background contradictions gate 2.3 measured, and says explicitly: "Re-plan `scan-udf/postgresql-scan`'s special-value Background bullet plus tasks 7.1 and 8.1 before Phase 7, and reword `vs-adapter/postgresql-type-mapping`'s nondeterministic-collation bullet before record." Checking both:

- **Tasks 7.1 and 8.1: done correctly.** `docker/postgres/init.sql`'s `numeric_specials` table seeds `NaN` into the constrained `fixed_amount NUMERIC(12,4)` column and `Infinity`/`-Infinity` only into the unconstrained `unbounded_amount NUMERIC` column — exactly what decision-log.md's divergence 1 consequence (ii) calls for. `crates/adbc-vs-scan/src/coerce.rs` coerces every declared type, including string-form `Decimal128`, through Arrow's own `cast_with_options` rather than a bespoke permissive parse, so it never accepts the driver's actual `nan`/`inf`/`-inf` spellings as valid decimal digits — divergence 2's hazard (a permissive `f64::from_str` silently accepting non-finite spellings) does not arise, and `coerce_tests.rs`'s `non_finite_string_form_numeric_fails_naming_the_value_and_declared_type` passes. The "re-plan before Phase 7" half of task 2.3 was satisfied, in effect, by the actual implementation.
- **The Background rewording: not done.** `specs/_plans/add-postgresql-support/scan-udf/postgresql-scan/spec.md` line 14 still reads "PostgreSQL `numeric` legitimately holds `NaN`, `Infinity`, and `-Infinity`" and its scenario at line 55 still describes a `numeric(12,4)` column "holding the values `NaN`, `Infinity`, and `-Infinity`" — the exact claim decision-log.md's divergence 1 says is "false ... as applied to a mapped column," since `Infinity`/`-Infinity` cannot be stored in a constrained `numeric(p,s)` column at all. `specs/_plans/add-postgresql-support/vs-adapter/postgresql-type-mapping/spec.md` line 12 still reads "a nondeterministic collation ... makes `LIKE` fail outright" — the exact claim decision-log.md's divergence 3 says is "false on 18.6" and needs rewording to "returns rows Exasol does not, in every comparison path including `LIKE`."

Both bullets carry a **self-acknowledged, dated measurement in decision-log.md contradicting the current delta text**, and decision-log.md itself says the second one "needs rewording before record." If `/speq:record` merges these two deltas unreworded, the permanent spec library will assert something the plan's own gate evidence already disproved. This should be fixed — by whoever owns Phase 2/task 2.3, or as a `/speq:record`-time correction alongside the eleven-feature manual head-replacement task 10.7 already found outstanding — before this plan is recorded.

## Notes

- All three stacks (Exasol, MySQL, PostgreSQL) were up and healthy for this verification; no test or manual step was skipped.
- Manual testing required two corrections to plan.md's literal command table (Exasol has no backtick identifier syntax; the real join fixture is `items`/`item_notes`, not `orders`/`customers`) — see the Manual Tests section preamble. These are documentation-quality issues in plan.md, not implementation defects.
- `exapump` in this environment is 0.12.0 (CLAUDE.md documents 0.9.2-era behavior for this repo, e.g. "exposes NO `wait` subcommand"); 0.12.0 has both `wait` and `bucketfs` (spelled without the plan.md table's `bfs` abbreviation) subcommands. Out of scope for this plan to fix, noted for whoever next touches the E2E harness or CLAUDE.md's exapump bullets.
- Scenario coverage: 4 documented PostgreSQL-side gaps in framework scenarios that MySQL already covers (see table above) — recommended as a follow-up, not a blocker.
- Record-time obligation confirmed live and outstanding: `/speq:record` must perform the eleven-feature manual Background/description rewrite plan.md's § Recording notes documents before stripping DELTA markers, or the merge silently reverts to MySQL-only language on nine `vs-adapter` framework files plus `scan-udf/arrow-scan` and `driver-provisioning/driver-self-install`.
- All Exasol objects this verification run created (`PGVS`, `PGVS2`, `PGVS3`, `PGVS_BADPW`, `MYVS` virtual schemas; `PG_CONN`, `PG_CONN_BAD`, `MY_CONN`, `BFS_CONN` connections) were dropped after use; confirmed absent from `SYS.EXA_ALL_VIRTUAL_SCHEMAS` / `SYS.EXA_ALL_CONNECTIONS` post-cleanup. Pre-existing objects from other sessions (`MYSQLVS*`, `ADBC_VS_*`, etc.) were left untouched.
