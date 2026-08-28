# Decisions: add-mysql-vs-mvp

## ADR: Discover MySQL metadata through information_schema rather than ADBC catalog calls

**ID:** mysql-metadata-via-information-schema
**Plan:** add-mysql-vs-mvp
**Status:** Accepted

### Context

Exasol requires an exact column type at `CREATE VIRTUAL SCHEMA` time. ADBC's `get_table_schema` maps `VARCHAR` to Arrow `Utf8`, which carries no length, forcing the adapter to invent one. Whether `adbc-drivers/mysql` populates the `xdbc_*` fields that `get_objects` exposes is undocumented, making them an unverified basis for a declared type.

### Decision

`createVirtualSchema` discovers tables and columns by executing `information_schema` queries over the ADBC connection, behind an interface the adapter defines in its own vocabulary.

### Options Considered

| Option | Verdict |
|--------|---------|
| `information_schema` queries | ✓ Chosen — reports exact character length, numeric precision, and scale; verified to exist |
| ADBC `get_table_schema` | ✗ Rejected — Arrow `Utf8` carries no length |
| ADBC `get_objects` with `xdbc_*` fields | ✗ Rejected — driver support for these fields is undocumented |

### Consequences

The declared type is correct by construction, at the cost of one extra query path. The consumer-defined interface lets a later backend supply its own metadata source without changing the adapter.

## ADR: Declare types from the catalog and coerce Arrow batches at scan time

**ID:** catalog-declared-types-with-scan-coercion
**Plan:** add-mysql-vs-mvp
**Status:** Accepted

### Context

The MySQL ADBC driver documents that `DATETIME` returns as `timestamp[s]`, `timestamp[ms]`, or `timestamp[us]` depending on how a query is written, and its tested matrix omits `DECIMAL` entirely. A declared Exasol type derived from that Arrow type would change without warning.

### Decision

The declared Exasol type comes from MySQL catalog metadata. The scan UDF coerces every Arrow batch the driver returns into the representation that declared type requires.

### Options Considered

| Option | Verdict |
|--------|---------|
| Catalog-declared type, scan-time coercion | ✓ Chosen — moves the instability into one owned place |
| Derive Exasol type from the returned Arrow type | ✗ Rejected — the Arrow type is not stable across query forms |

### Consequences

The declared type stays fixed for the life of the virtual schema. The scan UDF carries the coercion cost on every batch.

## ADR: Pass credentials by connection name, never by value

**ID:** credentials-by-connection-reference
**Plan:** add-mysql-vs-mvp
**Status:** Accepted

### Context

The pushdown response's SQL text is stored and visible through Exasol's audit and profiling tables. A credential embedded in that text leaks to anyone who can read those tables.

### Decision

The adapter and the scan UDF each receive the name of an Exasol `CONNECTION` object and resolve it through `ctx.connection()`. The MySQL user and password pass as separate ADBC options, not embedded in the connection URI.

### Options Considered

| Option | Verdict |
|--------|---------|
| Resolve credentials by name at point of use | ✓ Chosen — no credential ever exists in persisted text |
| Resolve once in the adapter, embed in the scan specification | ✗ Rejected — the sibling repository's shape leaks credentials into stored SQL text |

### Consequences

Both components pay one extra connection lookup per scan. No credential appears in audit tables, logs, or `adapterNotes`.

## ADR: Preserve MySQL identifier case verbatim

**ID:** preserve-mysql-identifier-case
**Plan:** add-mysql-vs-mvp
**Status:** Accepted

### Context

MySQL table names are case-sensitive on Linux under the default `lower_case_table_names=0`. Exasol's own convention folds unquoted identifiers to upper case. A folded name cannot be used in generated SQL without a reverse map, and only schema-level `adapterNotes` is verified to round-trip through Exasol 2025.2.1.

### Decision

Table and column names are reported to Exasol exactly as MySQL spells them. 0.1 performs no case folding and exposes no identifier-mapping property.

### Options Considered

| Option | Verdict |
|--------|---------|
| Preserve case verbatim | ✓ Chosen — deletes the round-trip state and its failure mode |
| Fold to upper case with a reverse name map in `adapterNotes` | ✗ Rejected — only schema-level `adapterNotes` round-trips; a table/column-level map does not |

### Consequences

Users write quoted identifiers to reference mixed-case names. A later release can add case folding as a property, matching `postgresql-virtual-schema`.

## ADR: Pin one driver version and verify a compiled-in SHA-256

**ID:** pinned-driver-version-with-sha256-verification
**Plan:** add-mysql-vs-mvp
**Status:** Accepted

### Context

The `dbc` registry index publishes no checksum field; the `dbc` CLI computes one locally only for drift detection. Verifying the driver archive's detached OpenPGP signature would need an OpenPGP implementation and Columnar's public key. `sequoia` is LGPL and barred by the license policy, leaving `rpgp` as a substantial new dependency for an MVP.

### Decision

Release 0.1 installs exactly one pinned `mysql` driver version, verified against a SHA-256 digest compiled into adbc-vs. No property lets an operator select a different version.

### Options Considered

| Option | Verdict |
|--------|---------|
| Pinned version, compiled-in SHA-256 | ✓ Chosen — real integrity, deterministic E2E, upgrade becomes a reviewed code change |
| Trust the registry over TLS with no integrity check | ✗ Rejected — no protection against a tampered or corrupted download |
| Verify the in-archive OpenPGP signature | ✗ Rejected — needs a new, license-blocked or heavyweight dependency |

### Consequences

A driver upgrade requires a code change and a new recorded digest. The E2E test is deterministic against a known artifact.

## ADR: Provision the driver in the adapter, not in the scan UDF

**ID:** driver-provisioning-in-adapter
**Plan:** add-mysql-vs-mvp
**Status:** Accepted

### Context

Scan UDFs run concurrently on every cluster node. Provisioning lazily on first scan would have every instance race to write the same BucketFS object, requiring distributed coordination to stay correct.

### Decision

`ensure_driver` runs during `createVirtualSchema`. The scan UDF only loads an already-installed library.

### Options Considered

| Option | Verdict |
|--------|---------|
| Provision in the adapter | ✓ Chosen — the adapter runs once per statement, making the write single-writer |
| Provision lazily on first scan | ✗ Rejected — concurrent scan instances would race on the same BucketFS write |

### Consequences

A virtual schema cannot be created against an unreachable registry, trading lazy availability for a race-free write path.

## ADR: Omit unmappable columns instead of failing the schema

**ID:** omit-unmappable-columns
**Plan:** add-mysql-vs-mvp
**Status:** Accepted

### Context

0.1 maps only eight MySQL types. A real MySQL database will almost always contain a column of an unmapped type.

### Decision

A column whose MySQL type this release cannot map is left out of the virtual table, and the omission — table, column, MySQL type, reason — is recorded in `adapterNotes`. A table with no mappable column is omitted entirely.

### Options Considered

| Option | Verdict |
|--------|---------|
| Omit and record | ✓ Chosen — keeps 0.1 usable against real databases; omissions stay visible via `SYS.EXA_ALL_VIRTUAL_SCHEMAS.ADAPTER_NOTES` |
| Fail `CREATE VIRTUAL SCHEMA` on any unsupported type | ✗ Rejected — would make 0.1 unusable outside its own fixture |

### Consequences

Silent omission becomes the accepted hazard, mitigated by recording it where an operator can read it back.

## ADR: Scope the license policy to Rust crates only

**ID:** license-policy-scoped-to-rust-crates
**Plan:** add-mysql-vs-mvp
**Status:** Accepted

### Context

adbc-vs never bundles a driver binary; it fetches the Apache-2.0-licensed ADBC MySQL driver into BucketFS at run time. Because adbc-vs redistributes no driver, no driver license attaches to its artifact.

### Decision

`cargo-deny` governs the Rust dependency tree only. The fetched ADBC driver is outside the policy.

### Options Considered

| Option | Verdict |
|--------|---------|
| Scope to Rust crates only | ✓ Chosen — matches the mission constraint that adbc-vs never bundles a driver binary |
| Extend the policy to cover fetched runtime binaries | ✗ Rejected — no such binary is redistributed, so no license attaches |

### Consequences

The boundary is stated in CLAUDE.md's "Licensing & dependency policy" section so it is not re-litigated in a later plan.

## ADR: Split the registry client and the BucketFS client

**ID:** split-registry-and-bucketfs-http-clients
**Plan:** add-mysql-vs-mvp
**Status:** Accepted

### Context

Plan review found that a single shared `reqwest` client for the registry fetch and the in-cluster BucketFS `PUT` would need certificate verification relaxed to pass the end-to-end test against the E2E Exasol container's self-signed certificate. That relaxation would silently extend to the public-internet registry fetch, leaving the shipped product trusting any host that answers for `dbc-cdn.columnar.tech`.

### Decision

Provisioning uses two HTTP clients. The registry client fetches the index and the driver package with certificate verification permanently on, with no property, feature, or environment variable able to disable it. The BucketFS client performs the in-cluster `PUT` and accepts the cluster's own self-signed certificate.

### Options Considered

| Option | Verdict |
|--------|---------|
| Two clients, verification scoped per endpoint | ✓ Chosen — confines the relaxation to the in-cluster endpoint, where the certificate is the cluster's own and the host is not attacker-selectable |
| One shared client for all three calls | ✗ Rejected — would force relaxing verification on the public registry fetch |

### Consequences

Provisioning code owns two client instances instead of one. The registry fetch keeps full certificate verification under all configurations.
