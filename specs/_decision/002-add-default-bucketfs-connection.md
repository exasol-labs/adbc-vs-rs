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

An air-gapped cluster holds `ADBC_VS_BUCKETFS`, has no HTTPS egress, and has a pre-placed library but no `DRIVER_PATH`. A fallback that fires only when the connection does not resolve never reaches that cluster.

### Decision

The default driver location answers when `ADBC_VS_BUCKETFS` does not resolve, and when it resolves but the registry index read through it fails. Both cases record the pre-placed library in `adapterNotes` with no driver version.

### Options Considered

| Option | Verdict |
|--------|---------|
| Fall back on a failed resolution and on an unreachable registry | ✓ Chosen: reaches the air-gapped cluster |
| Fall back only when the connection does not resolve | ✗ Rejected: never reaches the air-gapped cluster |

### Consequences

- The default location is consulted only after a failed index read, never before it, so a pre-placed library never satisfies the versioned presence check and the next statement reads the index again.
- The notes record the pre-placed path and no version, the same signature `DRIVER_PATH` leaves.

## ADR: Only a registry index request reaching no host at all triggers the fallback

**ID:** fallback-trigger-narrowed-to-unreachable-index-request
**Plan:** add-default-bucketfs-connection
**Status:** Accepted

### Context

A fallback on a failed package download or an interrupted transfer would pin a cluster to an unversioned pre-placed library during a transient CDN outage.

### Decision

Only a connect-, DNS-, or TLS-level failure of the index request triggers the fallback. Every other registry failure fails the statement on every route, and the error type keeps that case distinct so that a new failure kind cannot trigger the fallback silently.

### Options Considered

| Option | Verdict |
|--------|---------|
| Fall back only when the index request reaches no host | ✓ Chosen: only a real air gap reaches the pre-placed library |
| Fall back on any provisioning error | ✗ Rejected: pins a cluster to a stale library on a transient failure |
