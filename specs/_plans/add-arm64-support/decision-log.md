# Decision Log: add-arm64-support

## Interview

- Q: Where do ARM integration and E2E tests run? → A: GitHub-hosted `ubuntu-24.04-arm` runner
- Q: How are Exasol instances managed by tests? → A: Docker-based Exasol 2025-latest (x86), Docker-based Exasol 2026-latest, and Exasol Personal-latest on ARM and x86, as a CI test matrix
- Q: Which source tiers run on ARM? → A: All three E2E tiers (MySQL, PostgreSQL, Trino)
- Q: Release artifact and gating? → A: separate `adbc-vs-linux-arm64.tar.gz`, existing x86 asset name unchanged, release gated on ARM build and validate
- Q: Feature specs for integration or E2E tests? → A: None. Rules for them go in AGENTS.md if required
- Q: Source issue? → A: https://github.com/exasol-labs/adbc-vs-rs/issues/13 (check ARM SLC for pinned SDK, aarch64 `.so` + `cargo exasol-udf validate`, registry platform from host arch, ARM CI job and artifact, ARM E2E, update docs/install.md and docs/limits.md)
- Q: Known facts to design against? → A: Exasol Personal local runs Exasol in a VM with Podman, ships without an SLC (install via `exasol slc install`), and its own CI marks Linux ARM64 as deferred, so ARM E2E on a hosted runner needs a feasibility spike
- Q: Which Docker Exasol tags does the matrix use (asked 2026-10-23, after a Docker Hub check)? → A: The moving tags `latest-2025.2` and `latest-2026.1`. Measured on Docker Hub: `latest-2026.1` was last pushed 2026-05-22 (2026.1.0), and 2026.1.2 (2026-09-14) is newer. `latest-2025.1` (2025.1.16) is newer than `latest-2025.2` (2025.2.1, 2026-03-20). `exasol/docker-db` is amd64 only. The moving tags can drift without a repository change
- Q: Which Exasol runs the aarch64 E2E tiers? → A: Exasol Personal only. Exasol Personal pulls its own pinned `exasol/nano` tag (`nano.7` on 2026-10-23, multi-arch amd64 and arm64), and the plan does not name a nano tag
- Q: Is fallback (a) confirmed? → A: Yes, for CI runners only. The non-loopback `SOURCE_BIND_ADDRESS` bind applies only when `GITHUB_ACTIONS` is set. Local runs stay on `127.0.0.1`
- Q: Is fallback (b) confirmed? → A: Yes. If Exasol Personal does not start on `ubuntu-24.04-arm`, aarch64 E2E runs manually only. CI keeps the aarch64 build, validate, lint, unit, and integration tiers. A make target runs aarch64 E2E on Apple silicon or another aarch64 host. The release still gates on the aarch64 build and validate
- Q: How does a local Personal run reach the sources, given that `127.0.0.1` inside Exasol Personal is the runtime and the Apple silicon VM cannot reach Docker Desktop container IPs? → A: `SOURCE_BIND_ADDRESS=<address>` passed on the make command line is a per-run opt-in for local runs, so `make test-e2e-arm64` works on Apple silicon. The Makefile computes the address itself only when `EXASOL_TARGET=personal` and `GITHUB_ACTIONS` are both set

## Design Decisions

### [1] The registry platform is fixed by the compile target's architecture

- **Decision:** `adbc_vs_scan::driver::PLATFORM` is chosen at compile time from `target_arch`: `linux_amd64` for `x86_64` and `linux_arm64` for `aarch64`. Any other target fails to compile.
- **Rationale:** The `.so` loads only on the architecture it was compiled for, so the compile target is already the cluster's architecture. A second source for that fact can disagree with the first and then install a driver the library cannot load. All callers keep reading the one constant, so no call site changes.
- **Alternatives:** A virtual schema property that names the platform was rejected, because it asks the operator for a fact the library already knows. A probe of the cluster at run time was rejected, because it can only return the compile target again.
- **Promotes to ADR:** no

### [2] On Exasol Personal, the E2E tiers prove the `supplied` driver route

- **Decision:** With `EXASOL_TARGET=personal`, the shared E2E setup places each driver at the `supplied` default location and creates no `ADBC_VS_BUCKETFS` connection. Tests that need self-install compile only for the Docker target, through `#[cfg(not(feature = "exasol-personal"))]`.
- **Rationale:** Exasol Personal local deployments have no BucketFS HTTP endpoint (exasol-personal user docs, `local-deployment.md:43` and `limitations.md:15`), so self-install cannot write a driver there. AGENTS.md forbids run-time skips. A compile-time selection keeps each run's test list fixed and visible in the `cargo test` output.
- **Alternatives:** An early return on Personal was rejected, because it reports a test that did not run as passed. A smoke subset on Personal was rejected, because the interview asks for all three E2E tiers on ARM.
- **Consequences:** On aarch64, self-install is proven by the stub-registry tests in `make test` and by the integration tiers, which download and load the `linux_arm64` driver from the live registry. No E2E proves it against a real BucketFS write endpoint on aarch64, because no aarch64 Exasol with that endpoint is available.
- **Promotes to ADR:** no

### [3] ARM E2E feasibility is checked first, with fallbacks defined in advance

- **Decision:** Group B starts with a local x86_64 spike of Exasol Personal under Podman (task 2.1). The first CI run of the `personal-arm64` matrix rows is the aarch64 check. Two fallbacks apply:
  - (a) If a UDF inside Exasol Personal cannot reach the Compose static IPs, each Compose source port binds to a make-controlled address, `${SOURCE_BIND_ADDRESS:-127.0.0.1}:<port>:<port>`. The `Makefile` computes `SOURCE_BIND_ADDRESS` as the host's primary IPv4 address only when `EXASOL_TARGET=personal` and `GITHUB_ACTIONS` is set. A local run opts in per run with `SOURCE_BIND_ADDRESS=<address>` on the make command line, and a command-line value wins over the computed one. When the variable is set, the harness addresses each source by that address and the source's published port. Without it, local runs keep the source ports on `127.0.0.1`. The AGENTS.md static-IP rule names the bind address as the exception. Consequence: in a CI run under `personal`, and in a local run with the opt-in, the test databases and their fixed passwords are reachable by every machine that can reach that interface. An agent never sets the opt-in without an address the user names for that run.
  - (b) If Exasol Personal cannot start on `ubuntu-24.04-arm`, for example because Podman or the arm64 nano image fails, the `personal-arm64` rows leave the CI matrix. CI keeps the aarch64 build with `cargo exasol-udf validate`, lint, the unit tests, and the integration tiers, and the release stays gated on them. `make test-e2e-arm64` runs the three E2E tiers against Exasol Personal on an aarch64 host, such as Apple silicon or Linux aarch64, and AGENTS.md requires it before each release. (b) adds the source bind address if (a) did not, because the Exasol Personal VM on Apple silicon reaches neither the Docker Desktop container IPs nor the Mac's `127.0.0.1`. On Apple silicon, the run needs the opt-in. The Apple silicon path is unverified until its E2E test binaries have been built there.
  - The user confirmed both fallbacks, so neither stops the implementation for a confirmation.
- **Rationale:** Exasol Personal documents Linux ARM64 local support, but its own CI defers Linux ARM64 coverage (exasol-personal `doc/ci.md:170`), so no hosted-runner run of it is known. Exasol Personal on Linux runs directly in Podman on the host, without a VM (exasol-personal user docs `local-deployment.md:7`), so the missing nested KVM on hosted ARM runners (actions/runner-images#14062) does not block it.
- **Promotes to ADR:** no

## Review Findings

### [plan-review] Fallback (a) binds the source ports to a make-controlled address

- **Finding:** Fallback (a) addressed each source by the host's primary IPv4 address, but `docker-compose.yml:33,54,79-80` publishes every source port on `127.0.0.1` only, so a UDF inside Exasol Personal would reach no listener in exactly the case that triggers the fallback.
- **Direction change:** Under `EXASOL_TARGET=personal` only, the source ports bind to `SOURCE_BIND_ADDRESS`, the host's primary IPv4 address. That publishes the test databases and their fixed passwords on a non-loopback interface, so (a) now needs the user's confirmation, as (b) already did.
- **Promotes to ADR:** no

### [plan-review] One harness function owns how a driver becomes loadable

- **Finding:** The plan spread the per-backend driver route across `schema.rs` and four stack files, each with its own `cfg`, and the Personal path needed a second package downloader beside `crates/adbc-vs-scan/tests/common/driver_fetch.rs`, whose doc comment requires that it be the only one.
- **Direction change:** `schema::provide_drivers` is the one function that decides how a driver becomes loadable. The Docker backend creates `ADBC_VS_BUCKETFS`, and the Personal backend copies the library `driver_fetch::fetch_driver_library` returns to the `supplied` location. Every stack bring-up calls it. The adapter harness includes the scan crate's `driver_fetch.rs` through `#[path]` instead of copying it. The function lives in `schema.rs`, not on the `Exasol` type, because the reserved connection's SQL is built in `schema.rs` and `schema.rs` already depends on `exasol.rs`. A method on `Exasol` would make the two modules depend on each other.
- **Promotes to ADR:** no

### [user] Fallback (a) binds a non-loopback address on CI runners only

- **Finding:** The user confirmed fallback (a) for CI runners only and confirmed fallback (b) (Interview). The plan had stopped the implementation for a confirmation before either fallback.
- **Direction change:** The `Makefile` computes `SOURCE_BIND_ADDRESS` only when `EXASOL_TARGET=personal` and `GITHUB_ACTIONS` is set. A local run opts in per run with `SOURCE_BIND_ADDRESS=<address>` on the make command line, and otherwise keeps the source ports on `127.0.0.1`. Without `SOURCE_BIND_ADDRESS`, the Personal backend keeps the static IPs. Fallback (b) adds the manual `make test-e2e-arm64` target, adds the source bind address if (a) did not, and removes the `personal-arm64` CI rows. Neither fallback stops the implementation for a confirmation.
- **Promotes to ADR:** no
