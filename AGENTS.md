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
- `make cross-udf-build` builds for the host's architecture. Build the aarch64 `.so` on an aarch64 host, never cross-compiled: `cargo exasol-udf validate` loads the `.so`.
- `make release-tarball` names the tarball after the host architecture (`adbc-vs.tar.gz` on x86_64, `adbc-vs-linux-arm64.tar.gz` on aarch64). CI packs each tarball on a runner of its architecture.
- `SLC_VERSION` in the `Makefile` must equal the `exasol-udf-sdk` version.
- Every check goes through a `make` target. CI calls `make <target>`, never inline shell, except runner preparation, artifact upload and download glue, and `release` glue.
- CI builds the `.so` once per architecture in `build-so`. Every E2E job downloads that artifact and touches it, so `make` treats `cross-udf-build` as up to date. A job that rebuilds the `.so` is a defect.
- When you change `Makefile`, `ci.yml`, or `deny.toml`, keep `tests/make_targets_test.rs` and `tests/policy_test.rs` passing.

## Testing

- `make test` runs host unit tests and needs no Docker. Other tiers sit behind default-off cargo features and `make` targets: `test-it`, `test-it-postgres`, `test-it-trino`, `test-artifact`, `test-e2e`, `test-e2e-postgres`, `test-e2e-trino`, `deny`.
- `make test` and the `test-it*` tiers pass on x86_64 and aarch64. Test code takes the platform the running binary requests from `adbc_vs_scan::driver::PLATFORM` and never spells it as a literal. The exception is `self_install_downloads_the_package_of_the_compile_targets_platform`, which spells `linux_amd64` and `linux_arm64` per `#[cfg(target_arch)]`, so that the mapping itself is under test. The other Linux platform, fixture indexes, and URLs that only label an error are data and may spell either platform.
- `EXASOL_TARGET` selects the E2E Exasol: `docker` (the Compose `exasol` service, tag from `EXASOL_IMAGE`) or `personal` (an Exasol Personal local deployment named `adbc-vs`, which needs Podman, `jq`, and the `exasol` launcher). No arm64 Docker Exasol image exists, so aarch64 E2E runs only against Exasol Personal. `make stack-down` under `personal` destroys only the `adbc-vs` deployment.
- Switching `EXASOL_IMAGE` reuses the `exasol-data` volume, so run `docker compose down -v` first.
- Exasol Personal local has no BucketFS HTTP endpoint. Under `personal`, the harness writes BucketFS objects as files in the deployment directory and loads drivers from the `supplied` location. Tests that need self-install are compiled out with `#[cfg(not(feature = "exasol-personal"))]`, which is not a run-time skip.
- The harness reads Exasol Personal's address and password from `exasol info --json` and the deployment's `secrets.json`. No Exasol credential goes into an env var, a make variable, a CI file, or a log.
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
- `make lint` runs clippy twice, because `exasol-personal` compiles out the Docker backend. The first run enables every feature except `exasol-personal`. The second enables `exasol-e2e` and `exasol-personal`. CI runs `make lint` on both architectures. A new cargo feature goes into the first run's list.
- A comment states a non-obvious why: an invariant, an external-system quirk, or a spec constraint. Keep it to 1 or 2 lines. Never restate the code, narrate history, or add banners.

## Security and policy

- Credentials never appear in errors. PostgreSQL's `CONNECTION` is a conninfo string with its password: redact it, but still name host and database.
- Self-install verifies no checksum or signature. TLS verification is the only integrity guarantee. Do not add a digest property without a plan.
- `deny.toml` is the license source of truth.

## Release

- Bump the version in the root `Cargo.toml` in every PR that should release. The CI `release` job tags and publishes only when the version is new.
- The release publishes both architecture tarballs. Each `package` row needs every build, lint, and test job, and `release` needs both rows.
