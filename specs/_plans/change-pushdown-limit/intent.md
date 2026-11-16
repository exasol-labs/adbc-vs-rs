# Intent: change-pushdown-limit

## Interview

- Q: Source of intent → A: issue exasol-labs/adbc-vs-rs#20, a preview `LIMIT 100` on a 5M-row table fetches all 5M rows
- Q: Scope → A: `LIMIT` without `ORDER BY` only; top-N is a later plan
- Q: `OFFSET` → A: include it, advertise `LIMIT_WITH_OFFSET` in all three dialects
- Q: If Exasol drops the outer `ORDER BY` once `ORDER BY` and `LIMIT` are advertised → A: no top-N; `ORDER BY` stays in Exasol, the ordering guarantee stays unconditional
- Q: Which `limit` shapes arrive with only `LIMIT` advertised → A: measure on the test Exasol, specs state it
- Q: `OFFSET` (PR #24 feedback, reverses the earlier answer) → A: advertise `LIMIT` only, every `OFFSET` stays in Exasol; a `limit` carrying an `offset` fails with a named error; the gate observes `LIMIT ... OFFSET` without `ORDER BY`
- Q: Offset 0 (review finding) → A: refuse it like any offset
- Q: Gate join shape (review finding) → A: probe gets a second table `u`, gate joins `t` and `u`
- Q: Capabilities after upgrade without `REFRESH` (review finding) → A: do not measure; docs tell operators to run `REFRESH`
- Q: Gate offset row (review finding) → A: a stop row with three accepted outcomes

## Directions

- [D1] A new framework feature `vs-adapter/pushdown-limit` owns limit delegation, and `result-ordering` keeps ordering; they change for different reasons, and framework features are named after the protocol element they own.
- [D2] The query-shape scenarios and the gate run against PostgreSQL only; Exasol picks the request shape from the query and the advertised capabilities, which are the same in all three dialects for every measured shape.
- [D3] A measurement gate runs before any product change, and its stop and adjust rules in `plan.md` § Requirements govern the plan; planning starts no Exasol container, and the gate test stays in the suite to catch a later Exasol change.
