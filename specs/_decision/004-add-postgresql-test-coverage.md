# Decisions: add-postgresql-test-coverage

## ADR: Static fixture shape stays in the Compose seed; only mutation reaches Rust

**ID:** postgresql-fixture-mutation-boundary
**Plan:** add-postgresql-test-coverage
**Status:** Accepted

### Context

Three new PostgreSQL end-to-end tests need fixture objects `docker/postgres/init.sql` does not seed: a table a refresh can grow a column onto, a view, and a mixed-case table. `crates/adbc-vs-adapter/tests/common/postgres.rs` is deliberately read-only — the fixture is authored in SQL and the module only checks that the Compose seed ran. The refresh scenario cannot be satisfied by a static seed, because it requires a source that has gained a column since creation, so some Rust code must run DDL mid-run.

### Decision

Add `refresh_target`, `order_names`, and `"MixedCase"` to `docker/postgres/init.sql`. Add exactly one mutation method, `Postgres::execute`, scoped to the `ALTER TABLE` the refresh scenario requires, with its boundary documented in its own doc comment.

### Options Considered

| Option | Verdict |
|--------|---------|
| Static shape in `init.sql`; one narrow `Postgres::execute` mutation method for the refresh `ALTER TABLE` | ✓ Chosen — keeps fixture shape decided in one place, extends the existing `SEEDED_TABLES` staleness check to the new objects at no cost, and adds the smallest interface that satisfies a requirement no static seed can meet |
| Create all three objects at run time from a new `Postgres` method, leaving `init.sql` untouched | ✗ Rejected — fixture shape would then be decided independently in two places, splitting what a reviewer must diff against the seed |
| Route the `ALTER TABLE` through the existing `query_scalar` | ✗ Rejected — it asserts a single-value result, so a DDL command tag could pass as a value and a rejected statement could read as success |
| Keep `Postgres` read-only and seed the grown column directly in `init.sql` | ✗ Rejected — impossible: the refresh scenario requires a source that has gained a column since creation |

### Consequences

Fixture shape and fixture mutation stay separate concerns: SQL owns what exists, one documented Rust method owns what changes mid-run. The `SEEDED_TABLES` staleness check now also covers the three new objects. Any future test needing a different mid-run mutation must justify a second method against this same narrow-boundary precedent rather than reusing `query_scalar` for DDL.
