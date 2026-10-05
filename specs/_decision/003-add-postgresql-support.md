# Decisions: add-postgresql-support

## ADR: A closed driver-identity enum owns every per-driver fact

**ID:** closed-driver-identity-enum
**Plan:** add-postgresql-support
**Status:** Accepted

### Context

Release 0.2 scattered "there is one driver" across six places: `adbc-vs-scan::driver`'s module constants, `connection.rs`'s entry-point constant, and `create_schema.rs`'s supported-driver constant. Adding PostgreSQL as a second driver means every one of those six either threads a parameter or gains one owner.

### Decision

Introduce `Driver` as a closed enum in `adbc-vs-scan::driver`. Three tables key on it: the driver package (version range, library filename, registry path, ADBC entry-point symbol) in `adbc-vs-scan`, and the dialect and the metadata source in `adbc-vs-adapter`. No call site branches on a driver name.

### Options Considered

| Option | Verdict |
|--------|---------|
| Closed enum with three tables keyed on it | ✓ Chosen — an exhaustive `match` fails the build at every table missing a row |
| Thread a driver string through the six modules holding single-driver constants | ✗ Rejected — nothing fails when a seventh place needs updating |
| Bundle all per-driver behavior behind one trait object | ✗ Rejected — the package data is read by `adbc-vs-scan`, which cannot depend on the adapter crate where the dialect lives, so one bundle would invert the crate dependency |

### Consequences

"Which drivers exist" gets exactly one owner instead of five. Adding a third driver becomes a compiler-enforced task: the build fails at every table missing a row rather than silently keeping a stale answer.

## ADR: Both getCapabilities and pushdown resolve the dialect from adapterNotes.driver

**ID:** resolve-dialect-from-adapternotes-driver
**Plan:** add-postgresql-support
**Status:** Accepted

### Context

`pushdown` already reads `schemaMetadataInfo.adapterNotes.driver`, written by `createVirtualSchema` but never read elsewhere. `getCapabilities` needed its own source for the same identity, and a second field risked disagreeing with the first.

### Decision

Resolve the driver identity from `schemaMetadataInfo.adapterNotes` in both the `getCapabilities` and `pushdown` dispatch arms. A request carrying no notes errors naming the missing field rather than defaulting to a driver.

### Options Considered

| Option | Verdict |
|--------|---------|
| Resolve both arms from `adapterNotes.driver` | ✓ Chosen — one field cannot drift from itself |
| Resolve from `schemaMetadataInfo.properties.DRIVER` | Kept as the fallback only if gate 2.1 found `adapterNotes` absent from `getCapabilities` |
| Default to MySQL when the field is missing | ✗ Rejected — advertising one dialect's set for another's source returns wrong rows for every capability the two do not share |

### Consequences

The advertised capability set and the renderer are the same dialect by construction, preserving release 0.2's stated property while letting the answer differ per virtual schema.

## ADR: Each dialect states its own capability set, pinned by a type-enumerated invariant test

**ID:** per-dialect-capability-set-enumerated-by-type
**Plan:** add-postgresql-support
**Status:** Accepted

### Context

`SqlDialect::capabilities` was a per-implementation hand-stated list validated by an invariant test written against one dialect. A second dialect needed the same guarantee without letting one dialect's withholding govern the other's set.

### Decision

Keep `SqlDialect::capabilities` as a per-implementation hand-stated list. Generalize the invariant test that renders every advertised capability to run once per `Driver` variant, enumerated from the type rather than from a list in the test. Add a test asserting the two sets differ in exactly `JOIN_TYPE_FULL_OUTER`.

### Options Considered

| Option | Verdict |
|--------|---------|
| Per-dialect hand-stated set, invariant test enumerated from the `Driver` type | ✓ Chosen — a third dialect inherits the invariant with no test edit |
| Derive one release-wide set as the intersection of both dialects | ✗ Rejected — makes one target's limitation govern the other's delegation and discards PostgreSQL's full outer join |
| Derive each set from the other by addition or removal | ✗ Rejected — a derived set still lets one dialect's withholding silently govern the other's |

### Consequences

A third dialect's capability set is checked by the same test with no code change to the test itself, only a new `Driver` variant.

## ADR: PostgreSQL credentials travel inside the driver's single connection option

**ID:** postgresql-credentials-in-single-connection-option
**Plan:** add-postgresql-support
**Status:** Accepted

### Context

`arrow-scan` requires the user and password to pass as separate ADBC options so no credential travels in a connection string. The ADBC PostgreSQL driver's `SetOption` accepts exactly `uri` and rejects `username` and `password` as unknown options, handing `uri`'s value verbatim to libpq's connect call. No design preserves the separate-option invariant for this driver.

### Decision

Build a libpq keyword/value connection string in memory at connect time from the freshly resolved `CONNECTION` object — host, port, and `dbname` from the address, every address query parameter carried across as a keyword of the same name, `user` and `password` from the object, every value single-quoted with an embedded quote or backslash escaped — and pass it as the driver's only accepted option. Strip the resolved password from every surfaced error, including text the driver or libpq produced, at every caller of the shared open path, not the scan alone.

### Options Considered

| Option | Verdict |
|--------|---------|
| Escaped libpq conninfo string built at connect time | ✓ Chosen — conninfo escaping is total, and the driver accepts no other option |
| Inject the credential into the `postgresql://` URI | ✗ Rejected — needs percent-encoding, and a password containing `&`, `#`, `@`, or `/` changes the parse rather than the value |
| Pass the credential through libpq environment variables | ✗ Rejected — untested inside a UDF sandbox, process-visible, and `set_var` is `unsafe` in edition 2024 |

### Consequences

This is the plan's one deliberate narrowing of a security property, forced by the driver's API rather than chosen between equal designs. Every invariant about persistence and disclosure still holds and is asserted: nothing reaches the scan specification, `adapterNotes`, SQL text, `EXPLAIN VIRTUAL`, or any log line, and redaction is applied by the scan rather than relied on from the driver. A PostgreSQL `CONNECTION` deserves a least-privilege, read-only role as a result.

## ADR: Per-technology TOML files hold the capability list and the driver package row

**ID:** per-technology-toml-driver-tables
**Plan:** add-postgresql-support
**Status:** Accepted
**Supersedes:** driver-provisioning-in-adapter

### Context

A capability list and a package row are values a maintainer edits, with a shape that does not differ between technologies, while a SQL renderer and a type mapping are behavior no data format expresses. Release 0.2 wrote both as Rust, which made adding a technology read as a code change even where it was only a data change.

### Decision

Add one TOML file per technology — `crates/adbc-vs-scan/drivers/mysql.toml` and `drivers/postgresql.toml` — checked into the repo, each holding that technology's advertised capability list and driver package row. A `build.rs` in `adbc-vs-scan` reads them at build time and generates the `&'static` Rust tables `Driver::package()` and the dialect capability lists return, so the data compiles into the `.so` rather than being parsed at UDF run time.

### Options Considered

| Option | Verdict |
|--------|---------|
| TOML source files generated into Rust by `build.rs` | ✓ Chosen — keeps exhaustive `match` and `&'static` signatures, adds no run-time dependency, turns a malformed table into a build failure |
| Parse embedded TOML at run time via `include_str!` plus `LazyLock` | ✗ Rejected — adds `serde`/`toml` to the sandbox crate, breaks every `&'static` signature, turns a malformed table into a query-time panic |
| Read the TOML from BucketFS at run time | ✗ Rejected — loses compile-time exhaustiveness for nothing, since the behavior stays code regardless |
| Keep Rust constants and test them against a TOML file | ✗ Rejected — keeps the duplication the TOML exists to remove |

### Consequences

Adding a technology's values becomes a data edit the build checks; MySQL migrates onto the same mechanism in this plan rather than leaving two mechanisms for one kind of fact. This supersedes release 0.2's "provision the driver in the adapter" ADR's premise that per-driver package facts are Rust constants; provisioning still runs in the adapter, but the facts it reads are now generated from checked-in data.

## ADR: Adding a technology stays a release-time change; the Driver enum stays closed to run-time extension

**ID:** driver-enum-closed-to-runtime-extension
**Plan:** add-postgresql-support
**Status:** Accepted

### Context

The per-technology TOML files make run-time pluggability look available, but a technology still needs a dialect and a metadata source, which are Rust code no TOML file can supply.

### Decision

A third technology requires a new adbc-vs release carrying a new `Driver` variant, a new `SqlDialect` and `MetadataSource` implementation, and a new TOML file. No run-time driver-registration, plugin, or discovery mechanism is added. "Dynamic" means per virtual schema, which `adapterNotes.driver` already delivers.

### Options Considered

| Option | Verdict |
|--------|---------|
| Closed enum, release-time extension only | ✓ Chosen — states the boundary the TOML files could otherwise obscure |
| Discover technologies by scanning a directory of TOML files at run time | ✗ Rejected — cannot work: a TOML file naming an unknown technology could only fail, trading the compiler's exhaustiveness for a run-time error saying the same thing later |

### Consequences

A later reader is kept from building a loader that could never load anything. The data moved to TOML is exactly the data with no code behind it; every technology-specific behavior stays code.

## ADR: DRIVER_PATH names an operator-supplied driver and skips self-install

**ID:** driver-path-property-for-air-gapped-install
**Plan:** add-postgresql-support
**Status:** Accepted

### Context

Release 0.2's only air-gapped route was an informal fallback: an unreachable registry named a fixed compiled-in path an operator was expected to have pre-loaded by hand, discoverable only from an error message and no longer a single fixed location once the install path carries a version.

### Decision

Add a fourth `CREATE VIRTUAL SCHEMA` property, `DRIVER_PATH`, optional and applying to every driver. When present, the adapter uses exactly that BucketFS path and performs no registry index read, no download, no integrity check, and no BucketFS write. A path holding no readable file fails with a named error and never falls back to the registry. `BUCKETFS_CONNECTION` becomes optional when `DRIVER_PATH` is present.

### Options Considered

| Option | Verdict |
|--------|---------|
| `DRIVER_PATH` property, failing hard on a missing file | ✓ Chosen — turns an undocumented convention into a stated interface the operator chooses |
| Keep the informal "place the library at a fixed compiled-in path" fallback | ✗ Rejected — not a supported input, and no longer a fixed location once the path carries a version |
| Make it a compiled-in override rather than a property | ✗ Rejected — defeats the point of choosing it per virtual schema without a rebuild |
| Fall back to the registry when the path is missing | ✗ Rejected — would defeat the air gap the property exists to serve |

### Consequences

An air-gapped operator has a documented, chosen route to a fixed driver artifact. This supersedes the "no fourth property is added" clause of release 0.2's public-schema-only decision, and the fixed-path language in `driver-provisioning/driver-self-install`.

## ADR: The driver version is a range resolved against the registry, not a pinned string

**ID:** driver-version-range-resolved-against-registry
**Plan:** add-postgresql-support
**Status:** Accepted
**Supersedes:** pinned-driver-version-with-sha256-verification

### Context

Release 0.2 pinned one exact version per driver and matched it against the registry index by raw string equality. The index spells MySQL's versions with a leading `v` and PostgreSQL's without one, so no single normalization rule finds both, and a pinned version cannot track a driver's own compatible releases without an adbc-vs release.

### Decision

Each driver's TOML states a `version_range` in `semver` `VersionReq` syntax — `>=0.6.0, <0.7.0` for MySQL, `>=1.12.0, <2.0.0` for PostgreSQL. Self-install parses every published `version` scalar, normalizes one optional leading `v`, and selects the highest version satisfying the range, then reads that block's `linux_amd64` `url`. `semver` becomes a direct dependency of `adbc-vs-adapter`.

### Options Considered

| Option | Verdict |
|--------|---------|
| Range resolved to the highest published match | ✓ Chosen — asked for in the interview; semantic comparison removes the `v`-spelling hazard exact matching created |
| Keep one pinned exact version per driver | ✗ Rejected by the interview |
| Pin a version and let a property override it | ✗ Rejected — re-pins the common case and duplicates `version_range`'s purpose |

### Consequences

A MySQL cluster can now install a different driver version than it installed before, with no adbc-vs release, version bump, or operator action — a behavior change to already-released behavior, stated in the plan's Impact section. The two ranges are deliberately not derivable from one rule: MySQL sits below 1.0, where a minor bump is breaking, so it stops at `<0.7.0`; PostgreSQL at 1.x admits `<2.0.0`. This supersedes the release-0.2 ADR that paired a pinned version with a compiled-in SHA-256 digest — the pin is gone; decision `tls-only-integrity-for-self-installed-driver` in this fragment covers what replaces the digest half.

## ADR: TLS is the only integrity guarantee for a self-installed driver package

**ID:** tls-only-integrity-for-self-installed-driver
**Plan:** add-postgresql-support
**Status:** Accepted

### Context

A pinned version and a compiled-in SHA-256 digest were one mechanism: release 0.2 fetched one exact, maintainer-reviewed version and checked it against a digest of that exact artifact. A version range breaks the pairing, because no maintainer has reviewed a version the registry has not yet published. Measurement, not assumption, showed the registry publishes no checksum of any kind, and the one Ed25519 key signing both packages has no public half obtainable independently of the CDN that serves them.

### Decision

Self-install verifies no checksum and no signature of a downloaded driver package. The download's TLS connection is the sole integrity guarantee, applied uniformly to both drivers with no MySQL carve-out. The compiled-in SHA-256 digest is removed rather than replaced, and no `integrity` field or operator-supplied digest property is added. Decided by a human in round 3.

### Options Considered

| Option | Verdict |
|--------|---------|
| TLS only, uniformly for both drivers | ✓ Chosen by the human — the registry offers nothing to verify against, and the one available key is anchored in the host it would be protecting |
| Pin the registry's signing key and verify each package's embedded OpenPGP signature | ✗ Rejected on key provenance — the public half is not found on any public keyserver, the ASF Arrow `KEYS` file, or a `.well-known/openpgpkey` on the CDN's own domain |
| TLS by default plus an optional operator-supplied expected digest | ✗ Rejected as premature — a real middle ground, but adds a property and a documented contract for a use case nobody has asked for yet |
| Keep pinned exact versions so the compiled-in digest survives | ✗ Rejected — undoes the version range the interview asked for |

### Consequences

A narrowing of a released security property, stated rather than left to imply the digest guarantee survived: a MySQL deployment upgrading from 0.2 loses maintainer review of the exact artifact it installs. An operator who needs a fixed, vouched-for artifact uses `DRIVER_PATH` instead. Adding checksum or signature verification is named future work, to be planned separately, unblocked by either the registry publishing per-version checksums or the signing key's public half becoming independently obtainable.

## ADR: The BucketFS install path gains driver and version segments

**ID:** versioned-bucketfs-install-path
**Plan:** add-postgresql-support
**Status:** Accepted

### Context

A version range resolves to different versions over a cluster's lifetime, but `driver_install_path` was `<mount>/adbc-drivers/<library>` and `ensure_driver` returned early whenever that fixed path already held a file — before any network call. Under a range, that presence check would answer for whichever version was installed first and never test whether a newer one now satisfies the range.

### Decision

Change the installed driver's BucketFS path to `<mount>/adbc-drivers/<driver>/<version>/<library>`, and move the presence check to after range resolution so it tests the version actually resolved, not merely any version.

### Options Considered

| Option | Verdict |
|--------|---------|
| Versioned path, presence check after resolution | ✓ Chosen — the range resolves on every call rather than once per cluster, ever |
| Keep the unversioned path | ✗ Rejected — silently defeats the range: a cluster that installed one in-range version would never fetch a newer one |
| Overwrite the same path when the resolved version differs | ✗ Rejected — a running query may have the old library mapped, and overwriting a shared object under it is not safe |

### Consequences

`createVirtualSchema` and `refresh` now reach the driver registry on every call unless `DRIVER_PATH` is set, where release 0.2 could answer entirely from the mount — the single most likely upgrade surprise this release creates, with `DRIVER_PATH` as the supported answer. An older installed version is left in place rather than deleted, which is harmless once the path is versioned.
