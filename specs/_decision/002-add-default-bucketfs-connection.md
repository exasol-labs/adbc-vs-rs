# Decisions: add-default-bucketfs-connection

## ADR: Reserve ADBC_VS_BUCKETFS as the default BucketFS connection name

**ID:** reserved-default-bucketfs-connection-name
**Plan:** add-default-bucketfs-connection
**Status:** Accepted

### Context

`createVirtualSchema` required `BUCKETFS_CONNECTION` on every statement, even though most virtual schemas on one cluster share the same BucketFS write target. `README.md`'s quick start, both E2E harnesses, and `tests/common/schema.rs` already used the name `ADBC_VS_BUCKETFS` for that object.

### Decision

The adapter resolves the `CONNECTION` object named `ADBC_VS_BUCKETFS` when a statement carries no `BUCKETFS_CONNECTION` property. The name is compiled in as `create_schema::DEFAULT_BUCKETFS_CONNECTION` and is not configurable.

### Options Considered

| Option | Verdict |
|--------|---------|
| Reserve `ADBC_VS_BUCKETFS`, the name already documented | ✓ Chosen — a cluster that followed the README gains the shorter statement with no change at all |
| `ADBC_VS_BUCKETFS_CONNECTION` | ✗ Rejected — restates the object's kind in its name |
| A `DEFAULT_BUCKETFS_CONNECTION` property naming the default | ✗ Rejected — a default that is not one, and a fifth property the feature exists to avoid |
| A bare `BUCKETFS` | ✗ Rejected — unscoped, could collide with an operator's own connection |

### Consequences

A `CREATE VIRTUAL SCHEMA` on a cluster holding `ADBC_VS_BUCKETFS` needs only `DRIVER` and `CONNECTION_NAME`. Every statement that already names `BUCKETFS_CONNECTION` explicitly keeps working unchanged.

## ADR: Give the pre-placed route its own version-free BucketFS location

**ID:** version-free-supplied-driver-location
**Plan:** add-default-bucketfs-connection
**Status:** Accepted

### Context

ADR `driver-path-property-for-air-gapped-install` rejected release 0.2's fixed compiled-in fallback path on two grounds: the path was not a supported input, and it stopped being fixed once the install path carried a resolved version. An air-gapped operator with no `DRIVER_PATH` set and no way to resolve a version needed a location to pre-place a driver library at.

### Decision

The default driver location is `<mount>/adbc-drivers/<driver>/supplied/<library>`: separate from the versioned install path, carrying no version segment, read by the adapter and never written by it.

### Options Considered

| Option | Verdict |
|--------|---------|
| A per-driver `supplied` segment | ✓ Chosen — the install path's third segment is always a parsed `semver::Version`, and no version parses as `supplied`, so the two trees are provably disjoint |
| Reuse the versioned install path | ✗ Rejected — an air-gapped operator cannot resolve a version to pre-place at |
| A `default` segment | ✗ Rejected — reads as a version, and collides in spirit with the mount's own `bfsdefault/default` segment |
| A separate `adbc-drivers/supplied/<driver>/` subtree | ✗ Rejected — buys nothing over a per-driver segment and puts two layouts at the same directory level |

### Consequences

An air-gapped operator can pre-place a driver library without setting `DRIVER_PATH`. `supplied_driver` and the new location share one definition of "readable", so a second reader cannot silently accept what the first refuses. A registry index request that reaches no host, or a registry that answers nothing, on the reserved-connection route falls through to this location, and no provisioning message denies that the location exists.

## ADR: Consult the default driver location only after a failed index read, never before

**ID:** default-location-consulted-after-failed-index-read
**Plan:** add-default-bucketfs-connection
**Status:** Accepted

### Context

The versioned install path exists specifically so the presence check binds to a resolved version rather than to whichever version happened to be pre-placed first. Any pre-placed location consulted before the registry index read would reintroduce that defect.

### Decision

Self-install is unchanged: it reads the registry index first, resolves the version, and tests the versioned presence check. The default driver location is reached only after self-install through the reserved connection could not run at all, never as a cheap pre-registry short-circuit. A library sitting there never satisfies the versioned presence check.

### Options Considered

| Option | Verdict |
|--------|---------|
| Consult the default location only after a failed attempt to provision through the reserved connection | ✓ Chosen — nothing resolved and nothing installed, so the next statement reads the index again |
| Check the default location first as a cheap stat before the index read | ✗ Rejected — reintroduces "answers for whichever version arrived first" |
| Fall back to it on any registry-unreachable condition, checked before self-install | ✗ Rejected — same defect, differently phrased |

### Consequences

A pre-placed library shadows self-install for exactly as long as the registry stays unreachable, and not one statement longer. The versioned presence check keeps binding to the resolved version alone.

## ADR: Fall through on the reserved-connection route; never on an explicit property

**ID:** fallback-scoped-to-reserved-connection-route
**Plan:** add-default-bucketfs-connection
**Status:** Accepted

### Context

`exasol_udf_sdk::error::UdfError` reports every `CONNECTION`-resolution failure as one opaque error, with no structured not-found variant, so "no such connection" cannot be told from "no access". The plan needed a rule for which routes may fall back to the default driver location.

### Decision

Any failure to resolve `ADBC_VS_BUCKETFS` moves to the default driver location, carrying the failure's cause into the eventual error. A `BUCKETFS_CONNECTION` or `DRIVER_PATH` the statement named explicitly never falls back to anything, on a failed resolution or an unreachable registry alike.

### Options Considered

| Option | Verdict |
|--------|---------|
| Fall through only on the reserved-connection route | ✓ Chosen — an explicit property is an instruction, not a preference |
| Fail the statement on any resolution failure | ✗ Rejected — makes the issue's soft-landing point unreachable |
| Distinguish "no such connection" from "no access" and fall through only for the former | ✗ Rejected — the UDF SDK reports both as one opaque error |

### Consequences

Falling through is safe on the derived route because Exasol enforces `CONNECTION` access at resolution, and the source `CONNECTION` is resolved first, so a broken connect-back transport has already failed the statement. An explicit property's failures always surface unchanged.

## ADR: Reach the default driver location when the registry answers nothing through the reserved connection

**ID:** widen-fallback-to-unreachable-registry
**Plan:** add-default-bucketfs-connection
**Status:** Accepted

### Context

The headless-planned trigger fired only when `ADBC_VS_BUCKETFS` itself did not resolve. That served only a cluster that had deleted the connection its own README told it to create — never the air-gapped cluster the issue's own "Why" describes: reserved connection present, no HTTPS egress, library pre-placed, `DRIVER_PATH` forgotten. Round-1 plan review raised this as an Intent Fidelity blocker; the maintainer answered it directly.

### Decision

The last-resort route has two triggers, not one. The default driver location answers when `ADBC_VS_BUCKETFS` does not resolve, and when it resolves but self-install through it reaches a registry that answered nothing at all. Both triggers produce the same result: the pre-placed library, recorded in `adapterNotes` with no driver version.

### Options Considered

| Option | Verdict |
|--------|---------|
| Widen the trigger to cover both a failed resolution and an unreachable registry | ✓ Chosen (human-directed) — delivers the soft landing to the cluster that actually needs it |
| Keep the no-connection-only trigger | ✗ Rejected — never reaches the air-gapped cluster holding the reserved connection |
| Fall back on every provisioning failure | ✗ Rejected — see the narrowed-trigger ADR below |

### Consequences

The version-first defect does not return: the fallback fires after a failed index read rather than before a successful one, so nothing resolved and nothing installed, and the next statement reads the index again. The fallback is not silent — the notes record the pre-placed path and no version, the same signature `DRIVER_PATH` leaves.

## ADR: Only a registry index request reaching no host at all triggers the fallback

**ID:** fallback-trigger-narrowed-to-unreachable-index-request
**Plan:** add-default-bucketfs-connection
**Status:** Accepted

### Context

`ProvisioningError::RegistryUnreachable` covered four failures before this plan: the index request reaching no host, the package request reaching no host, and either request's body stopping mid-arrival after its status did. Firing the fallback on any of the last three would pin a cluster to an unversioned pre-placed library on a transient CDN outage or a reset download, after a version had already been resolved and the versioned presence check had already run.

### Decision

`ProvisioningError` is split into four cases along two axes — the leg (index or package) and the point of failure (no host answered, or a body that stopped arriving after its status did). `registry_unreachable()` answers `Some((index URL, transport cause))` for a connect-, DNS-, or TLS-level failure of the index request alone, and `None` for every other variant, including the three that used to share one variant with it. The accessor is a `match` with no wildcard arm, so a variant added later fails the build rather than silently answering `None`.

### Options Considered

| Option | Verdict |
|--------|---------|
| Four-case split, accessor with no wildcard arm | ✓ Chosen (human-directed) — a variant added later is forced back into this decision |
| Fall back on any `ProvisioningError` | ✗ Rejected — pins a cluster to a stale library on a transient failure |
| Keep one variant and document the boundary in prose | ✗ Rejected — found unenforceable in round-2 review |
| Export the variant and let `create_schema` match its fields directly | ✗ Rejected — puts the error taxonomy's shape in a second module |

### Consequences

A registry that answered any HTTP status, an index that will not parse, a resolved version with no package, a package request reaching no host, and either leg's interrupted transfer all fail the statement on every route. Only a genuine air gap — no host answered at all, on the index leg — reaches the pre-placed library.
