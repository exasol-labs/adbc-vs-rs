# AGENTS.md

@specs/mission.md
@specs/architecture.md

## Specs

- `specs/` holds product behavior only. E2E, integration testing, toolchain, build, CI, and licensing never get a spec. Their rules live in this file.
- Feature specs never pin dependency, toolchain, or image versions. Those live in `Cargo.toml`, `rust-toolchain.toml`, and CI.
- A feature is a framework feature (protocol vocabulary, e.g. `pushdown-filter`) or a technology feature (`<technology>-<concern>`, e.g. `mysql-pushdown`). Both sit flat in a domain, never nested: `speq` drops nested features silently.
- `vs-adapter` exceeds the speq >8-features-per-domain guideline on purpose. Do not re-consolidate features to fix it.
- A new technology adds `<technology>-pushdown`, `-collation`, `-type-mapping` (`vs-adapter`), `<technology>-scan` (`scan-udf`), and a `crates/adbc-vs-scan/drivers/<technology>.toml`. It adds no `driver-provisioning` feature.

## Build

- `make cross-udf-build` is the only way to build the UDF `.so`. Never run host `cargo build --release`: the glibc fingerprint must match the SLC or the `.so` is rejected at load time.
- `SLC_VERSION` in the `Makefile` must equal the `exasol-udf-sdk` version.
- Every check goes through a `make` target. CI calls `make <target>`, never inline shell, except runner preparation and `release` glue.
- When you change `Makefile`, `ci.yml`, or `deny.toml`, keep `tests/make_targets_test.rs`, `tests/workflow_test.rs`, and `tests/policy_test.rs` passing.

## Testing

- `make test` runs host unit tests and needs no Docker. Other tiers sit behind default-off cargo features and `make` targets: `test-it`, `test-it-postgres`, `test-it-trino`, `test-artifact`, `test-e2e`, `test-e2e-postgres`, `test-e2e-trino`, `deny`.
- Integration tiers (`test-it*`) cover the ADBC path only and must not need Exasol or `cross-udf-build`.
- E2E tiers need `cross-udf-build` plus `stack-up-exasol` and only the sources their binary queries. Start containers yourself via `make stack-up-*`, do not ask the user. Run with `--test-threads=1`: one shared setup installs the SLC and creates the virtual schemas.
- Tests fail, never skip, when a container is unavailable. Bounded waits panic and name the subject and deadline. Never use a fixed `sleep`.
- Containers are addressed by static IP (MySQL `172.28.5.10`, PostgreSQL `172.28.5.11`, Trino `172.28.5.12`), because a Compose service name does not resolve inside a UDF sandbox.
- `exapump` has no `wait` subcommand. This overrides the workspace-level rule; readiness is a bounded `exapump sql ... "SELECT 1"` poll.
- BucketFS writes are asynchronous. Wait until the object is readable to a UDF. Retry `423 Locked` after a delete.
- Verify each delegated pushdown fragment twice: `EXPLAIN VIRTUAL` shows it reached the source statement, and the returned rows show it is correct. Prefer differential assertions against a local Exasol mirror table.
- Editing `docker/postgres/init.sql` needs `docker compose rm -sfv postgres && make stack-up-postgres`, since Postgres skips it on a reused volume.
- Benchmarks (`make bench*`) run one source at a time, with unrelated containers stopped. They run in no CI job.

## Code quality

- `make fmt-check` and `make lint` pass with zero warnings before committing.
- A comment states a non-obvious why: an invariant, an external-system quirk, or a spec constraint. Keep it to 1 or 2 lines. Never restate the code, narrate history, or add banners.

## Security and policy

- Credentials never appear in errors. PostgreSQL's `CONNECTION` is a conninfo string with its password: redact it, but still name host and database.
- Self-install verifies no checksum or signature. TLS verification is the only integrity guarantee. Do not add a digest property without a plan.
- `deny.toml` is the license source of truth.

## Release

- Bump the version in the root `Cargo.toml` in every PR that should release. The CI `release` job tags and publishes only when the version is new.
