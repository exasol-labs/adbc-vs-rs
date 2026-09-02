# Open Questions: add-exasol-technology

speq-plan-pr could not complete this plan without human input. What's done so far is committed on this branch. Reply inline on the PR, or resume with `/speq:plan add-exasol-technology` locally, or re-run `/speq:plan-pr add-exasol-technology` after commenting.

- [ ] **The stated throughput goal is refuted. Ship anyway on a different value proposition, or hold for the benchmark?** Both the built-in Lua adapter and adbc-vs route Exasol-to-Exasol transfer through `IMPORT ... FROM EXA`, which Exasol documents as "always parallelized" across cluster nodes. adbc-vs would instead move rows through one UDF over one connection, and that driver buffers the whole result before the scan reads a batch, so it cannot bound peak memory. There is no credible bulk-throughput win. What remains: `ORDER BY`/`LIMIT` pushdown fidelity (the built-in adapters withhold both and prescribe an `ORDER BY FALSE` workaround) and correct `HASHTYPE`-in-join handling (the built-in route maps it to `VARCHAR` and fails the join) — both real, but both have documented workarounds today, and a result exceeding the UDF's memory limit fails outright where the bulk loader handles any volume. Pick one: (a) ship as a correctness-and-uniformity feature, no performance claim, or (b) hold this plan until the follow-up benchmark plan (see plan.md § Dependencies) has run and can state the actual number.
- [ ] **Is cross-cluster federation (two separate Exasol instances) in scope for this release, or deferred?** The plan's Non-Goals exclude it, but the only cited reason is CI disk budget ("runners already exhaust disk with three containers"), not a technical limitation — nothing in the adapter distinguishes a remote Exasol source from the same instance. Confirm whether untested-but-expected-to-work is acceptable for this release, or whether a two-instance test tier is required before merge.

## Other review findings (non-blocking on this decision, but need fixing before merge)

Full detail: `specs/_plans/add-exasol-technology/review/round-1.md`. Six additional BLOCKERs surfaced, independent of the two questions above:
- Recorded specs `create-virtual-schema` and `pushdown-dialect` are left self-contradictory by this plan (views-exposed and two-dialects invariants both break for Exasol with no delta correcting them).
- `driver-path-override`, `adapter-request-dispatch`, and several other "for each driver/dialect this release supports" scenarios get no delta, no task, and no test row for the new Exasol technology.
- The certificate-handling claim ("the SLC ships no CA bundle") is attributed to CLAUDE.md, which does not say that; it needs its own measurement, from inside the UDF sandbox, not the host-based integration tier currently scheduled.
- The `scan-udf/exasol-scan` memory-limit scenario is asserted only against the recorded plan.md § Verification test whose tier never runs inside the UDF sandbox.
- § Parallelization groups tasks that write the same not-yet-existing file (e.g. Group B's 2.2/2.4/2.5 all assume 2.3 already created `exasol_it_test.rs`) — dispatching them concurrently under `/speq:implement` will collide.

15 ADVISORY findings also surfaced (task granularity, a few ambiguous MUST clauses, prose consistency). None block merge; see the round-1 file for the full list.
