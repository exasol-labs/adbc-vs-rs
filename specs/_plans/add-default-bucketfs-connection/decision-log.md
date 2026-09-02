# Decision Log: add-default-bucketfs-connection

## Interview

Headless run. No human was available, so the issue body stands in for the interview and every decision below was assumed and documented per `/speq:planning`'s Headless Mode. Nothing reached the escalation bar: no decision here is irreversible, none changes what an existing statement does, and none touches a credential boundary Exasol does not already enforce.

**Q:** What is the problem?
**A:** (issue #4) `createVirtualSchema` requires `BUCKETFS_CONNECTION` whenever `DRIVER_PATH` is absent. Every virtual schema must name its own BucketFS write credential explicitly, even though most virtual schemas on one cluster share the same BucketFS target.

**Q:** What behavior is proposed?
**A:** (issue #4) Four points. (1) A default `BUCKETFS_CONNECTION` exists — the adapter looks up a reserved connection name, naming to be picked by the plan, before failing on a missing property. (2) An explicit `BUCKETFS_CONNECTION` property overrides the default. (3) If both the default and the explicit property are absent, assume the driver is already placed in BucketFS and skip self-install entirely, the same operator-vouches-for-it contract `DRIVER_PATH` has, reached without requiring `DRIVER_PATH`. (4) Drivers get a default BucketFS location too; `DRIVER_PATH`, when set, still overrides it.

**Q:** What is in scope?
**A:** (issue #4) `driver-provisioning/driver-self-install`, `driver-provisioning/driver-path-override`, and `vs-adapter/create-virtual-schema`'s "Missing required property fails with a named error" scenario. The issue names the reserved default-connection name and the interaction between "look for the default" and the presence check's existing BucketFS read as the two questions the plan must answer.

**Q:** What must the plan not do?
**A:** (orchestrator) This is a framework-level change. Per `CLAUDE.md` § Spec governance, no `<technology>-*` feature is needed and no `e2e/`, `toolchain/`, `ci/`, or `licensing/` spec group may be created.

## Design Decisions

### [1] Reserve `ADBC_VS_BUCKETFS` as the default BucketFS connection name

- **Decision:** The adapter resolves the `CONNECTION` object named `ADBC_VS_BUCKETFS` when a statement carries no `BUCKETFS_CONNECTION` property. The name is compiled in as `create_schema::DEFAULT_BUCKETFS_CONNECTION` and is not configurable.
- **Alternatives:** `ADBC_VS_BUCKETFS_CONNECTION`, which restates the object's kind in its name. A `DEFAULT_BUCKETFS_CONNECTION` property naming the default, which is a default that is not one and adds the fifth property the feature exists to avoid. A bare `BUCKETFS`, which is unscoped and could collide with an operator's own connection.
- **Rationale:** The name is not an invention. `README.md`'s quick start, both end-to-end harnesses, and `tests/common/schema.rs` already use exactly `ADBC_VS_BUCKETFS` for this object, so the convention being reserved is the one the project already documents, and a cluster that followed the README gains the shorter statement with no change at all. It is a valid unquoted Exasol identifier, so `CREATE CONNECTION ADBC_VS_BUCKETFS ...` needs no quoting, and the `ADBC_VS_` prefix scopes it to this product.
- **Promotes to ADR:** yes

### [2] Give the pre-placed route its own version-free location, `adbc-drivers/<driver>/supplied/<library>`

- **Decision:** The default driver location is `<mount>/adbc-drivers/<driver>/supplied/<library>`. It is separate from the versioned install path, carries no version segment, is read by the adapter and never written by it, and is the location the last-resort route uses.
- **Alternatives:** Reuse the versioned install path `<mount>/adbc-drivers/<driver>/<version>/<library>`, which an air-gapped operator cannot pre-place at because the version comes from a registry read they cannot perform. A `default` segment, which reads as a version and produces `/buckets/bfsdefault/default/adbc-drivers/mysql/default/...`. A separate `adbc-drivers/supplied/<driver>/` subtree, which buys nothing over a per-driver segment and puts two layouts in the same directory's first level.
- **Rationale:** ADR 003 rejected keeping release 0.2's pre-load fallback on two grounds — the path was not a supported input, and it stopped being fixed once the install path carried a version. Both are answered here rather than waived: this location is stated by `driver-provisioning/driver-path-override`, is named by the error an operator gets when nothing is there, and is version-independent by construction. `supplied` names who owns the file, matches `provisioning::supplied_driver`'s vocabulary, and can never collide with an install path, because that path's third segment is always a parsed `semver::Version` and no version parses as `supplied`.
- **Promotes to ADR:** yes

### [3] Consult the default location last, never as a pre-registry short-circuit

- **Decision:** Self-install is unchanged: it reads the registry index first, resolves the version, and tests the versioned presence check. The default location is reached only when no BucketFS connection resolves at all. A library sitting there never satisfies the versioned presence check and never short-circuits an install.
- **Alternatives:** Check the default location first as a cheap stat before the index read. Fall back to it when the registry is unreachable.
- **Rationale:** `provisioning.rs` states why the install path carries a version: an unversioned path "would have made it answer for whichever version arrived first, so a range would resolve once per cluster and never again." Any location consulted before the index read reintroduces exactly that defect, and a stale pre-placed library would then shadow self-install forever. Falling back on an unreachable registry is a silent fallback, which `driver-path-override` already forbids because it turns an operator's typo into a slow first query.
- **Promotes to ADR:** yes

### [4] Fall through when the default connection fails to resolve; never when an explicit property does

- **Decision:** Any failure to resolve `ADBC_VS_BUCKETFS` moves to the default driver location, carrying the failure's cause into the eventual error. A `BUCKETFS_CONNECTION` the statement named, and a `DRIVER_PATH` it named, never fall back to anything.
- **Alternatives:** Fail the statement on any resolution failure, which makes point 3 of the issue unreachable. Distinguish "no such connection" from "no access" and fall through only for the former.
- **Rationale:** The distinction cannot be drawn: `exasol_udf_sdk::error::UdfError` reports every resolution failure as one opaque `ConnectBack(String)`, with no structured not-found variant. Falling through is nonetheless safe. Exasol enforces access to a `CONNECTION` object at resolution, so a session that cannot resolve `ADBC_VS_BUCKETFS` was never entitled to its credentials and no privilege is gained. And `create_schema::open` resolves the source `CONNECTION` before it reaches the driver route, so a connect-back transport that is broken has already failed the statement — a resolution failure at this point is a fact about the connection, not about the transport. Carrying the cause into the last-resort error keeps a permission misconfiguration diagnosable rather than reported as a missing file.
- **Promotes to ADR:** yes

### [5] Extend `driver-provisioning/driver-path-override` rather than add a fourth provisioning feature

- **Decision:** `driver-path-override` owns both operator-supplied routes. Its Feature-description line broadens; its name does not change.
- **Alternatives:** A new `driver-provisioning/driver-preplaced-default` feature. Renaming the feature to something route-neutral, such as `operator-supplied-driver`.
- **Rationale:** The two routes reach the same three obligations — use exactly this file, assert only readability, never fall back — and differ only in where the path string came from. A second feature would be two shallow owners of one mechanism, which is the classitis `/speq:design-philosophy` warns against, and would leave the no-fallback rule stated twice with nothing keeping the two statements in agreement. The name is kept because `CLAUDE.md`, `specs/_decision/003-add-postgresql-support.md`, and three other feature specs reference it, and `specs/_decision/` is append-only history that a rename would leave pointing at nothing.
- **Promotes to ADR:** no

### [6] Leave `driver-provisioning/driver-version-resolution` untouched

- **Decision:** The unreachable-registry error keeps naming `DRIVER_PATH` alone, and keeps its clause forbidding a fixed BucketFS path to pre-load and forbidding the adapter from loading an unrelated version found on the mount.
- **Alternatives:** Extend that error to also name the default driver location as a remedy.
- **Rationale:** Both clauses stay true on the self-install route, which is the only route that reaches that error: the versioned path is still unknowable without the index, and the default location is still never loaded there. Naming the default location would be advice an operator cannot act on, because reaching it requires a cluster with no BucketFS connection at all — on a cluster with a shared `ADBC_VS_BUCKETFS`, that is not a change one statement can make. `DRIVER_PATH` remains the one-statement remedy the error already names.
- **Promotes to ADR:** no

### [7] Keep route selection a pure function of the properties

- **Decision:** `create_schema::driver_source` gains a third variant, `DefaultBucketFs`, and stays a pure function of the property map. Resolving the reserved connection and testing the default location both happen in `open_driver_library`, where the UDF context already is.
- **Alternatives:** Pass `ctx` into `driver_source` and have it return a fully resolved route.
- **Rationale:** `create_schema`'s module comment states the property that ordering protects: every property is validated before any `CONNECTION` is resolved, so a statement that could never provision is refused without reaching anything outside the process. Threading `ctx` into route selection would put a connect-back round trip ahead of that validation and would make the pure route-selection unit tests need a fake context.
- **Promotes to ADR:** no

### [8] One shared definition of "a readable driver library"

- **Decision:** Extract `provisioning::readable_library(path) -> Result<(), String>` from `supplied_driver` and have both operator-supplied routes call it, each phrasing its own error over the shared cause. `supplied_driver` gains no parameter selecting which error to build.
- **Alternatives:** Have `create_schema` test the default location with a bare `Path::is_file`. Add a route descriptor parameter to `supplied_driver`.
- **Rationale:** Two callers deciding independently what counts as readable is back-door leakage — `supplied_driver` already refuses a directory as well as a missing file, and a second caller checking only `is_file` would accept what the first refuses. A route-selecting parameter is a decision the function declined to make, and the two messages differ in more than a word: they name different properties and different remedies.
- **Promotes to ADR:** no

### [9] Prove the connection probe is recoverable before building on it

- **Decision:** Task 1.1 is a live end-to-end assertion that a `BUCKETFS_CONNECTION` naming no existing `CONNECTION` object surfaces as the adapter's own error. Tasks 2.2 and 3.3 depend on its result, and the task states that a failure is a finding to report rather than a bug to work around.
- **Alternatives:** Assume recoverability and discover it during implementation. Design a second mechanism as a hedge.
- **Rationale:** `UdfContext::connection`'s body in `exasol-udf-sdk` 0.23.0 is a default returning `Unimplemented`; the real implementation lives in the script language container's host bridge, which is not in this workspace, and no test in this repository resolves a connection that does not exist. Point 3 of the issue is unreachable if the host aborts the statement instead of answering. Points 1, 2 and 4 do not depend on it, so the plan is still worth most of its value if the probe fails — which is why the dependency is stated rather than hedged with a second design.
- **Promotes to ADR:** no

### [10] `BUCKETFS_CONNECTION` becomes optional unconditionally, and its missing-property error is deleted

- **Decision:** `create-virtual-schema`'s "Missing required property fails with a named error" scenario keeps `DRIVER` and `CONNECTION_NAME` and loses every `BUCKETFS_CONNECTION` clause. The `required_property` call and the unit test asserting that error are removed.
- **Alternatives:** Keep the error as a fallback for a cluster with no reserved connection, raising it instead of the new last-resort error.
- **Rationale:** After this change the property is never required, so an error naming it as missing would be false. The information it carried is not lost: the new error names both `BUCKETFS_CONNECTION` and `DRIVER_PATH` as the explicit routes, alongside the reserved connection and the location the adapter actually looked at, which is strictly more than the old message said.
- **Promotes to ADR:** no

## Review Findings
