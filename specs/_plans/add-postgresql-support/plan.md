# Plan: add-postgresql-support

## Summary

Adds PostgreSQL as a second ADBC source behind a driver identity that keys every per-driver fact — dialect, driver package, metadata source — to one closed enum, so `getCapabilities` and `pushdown` resolve the same dialect per virtual schema and each dialect advertises the capability set its own SQL can render. Each technology's capability list and package row move into one checked-in TOML file, MySQL's included, compiled in at build time; the driver version becomes a range resolved against the registry, and a new `DRIVER_PATH` property lets an air-gapped operator skip self-install.

**Self-install's integrity guarantee narrows to TLS alone, for both drivers.** A range resolves to versions no maintainer reviewed and the registry publishes no checksum, so the compiled-in SHA-256 digest is removed rather than replaced; decision `[22]` records why, and § Impact states the cost.

**The spec library splits framework from technology in the same plan.** A framework feature states what a pushdown fragment, a type mapping, or a scan is, whatever backs the schema; a `<technology>-<concern>` feature states one target's own capabilities, rendering, collation placement, mapping, and connection mechanism. MySQL's shipped spec content migrates rather than staying behind, on decision `[19]`'s principle. Decision `[25]` records the structure, the measurement showing `speq` cannot represent a nested path, and the `DELTA` sequence that expresses a move the merge has no marker for. Decision `[26]` closes the two organization questions `[25]` escalated: `driver-provisioning` splits three ways along framework-internal lines, and `vs-adapter`'s 16 features are accepted as the standing shape.

## Design

### Context

mission.md names PostgreSQL first in its v1 driver set and uses `DRIVER = 'postgresql'` as the persona example, and `pushdown-dialect`'s design already states that the dialect owns its advertised capability set. What does not exist is any way to have two of them. Six forces shape the design.

**The dialect boundary is right but has no selection point.** `SqlDialect` already exposes exactly `capabilities()` and `render()`, and `capabilities.rs` derives the response from the dialect rather than from a list of its own. The single-driver assumption lives one layer up: `dispatch.rs` names `MySqlDialect` at both call sites, and its module doc justifies that as the property that keeps the advertised set and the renderer in agreement. Two drivers must preserve that property while letting the answer differ per virtual schema, which means both arms resolve the dialect from one field rather than neither naming one.

**Per-driver facts are scattered as module constants, not keyed on anything.** `adbc-vs-scan::driver` holds one `DRIVER_VERSION`, one `DRIVER_LIBRARY`, one `PACKAGE_SHA256`, one `REGISTRY_DRIVER_PATH`; `connection.rs` holds one `DRIVER_ENTRY_POINT`; `create_schema.rs` holds one `SUPPORTED_DRIVER`. Six places restate "there is one driver". A second driver either threads a parameter through all six or gives the decision one owner. The owner is a closed enum, because a Rust `match` over it is exhaustive: adding a variant fails the build at every table missing a row, which is what makes "a new driver adds an implementation rather than editing a renderer" a compiler guarantee rather than a convention.

**Two kinds of per-driver fact are mixed, and only one of them is code.** A capability list and a package row are *values*: a maintainer edits them, and nothing about their shape differs between technologies. A SQL renderer and a type mapping are *behavior*: no data format expresses them. Release 0.2 wrote both as Rust, which is why adding a technology reads as a code change even where it is only a data change. The values move into one TOML file per technology, generated into the same `&'static` tables at build time, so the compiler still names every missing row and the sandbox still parses nothing at run time; the behavior stays Rust. What this does *not* buy is run-time pluggability, and the plan says so rather than letting the file format imply it: a technology with no dialect and no metadata source behind it could only fail, so the `Driver` enum stays closed and a third technology still ships in a release. Decisions `[17]` and `[18]` record both halves.

**A pinned version and a compiled-in digest are one mechanism, and the range breaks it.** Release 0.2 fetches one exact version and checks it against one digest a maintainer reviewed. Fetching the newest version in a range keeps the fetch and removes the reviewable constant, because no maintainer has seen a version the registry has not yet published. The registry publishes no checksum to verify against instead — measured, not assumed, in decision `[22]` — and the one signing key both packages carry is obtainable only from the CDN that serves them, so verifying it would add ceremony rather than independent trust. The digest is therefore removed and nothing replaces it: TLS is the only integrity guarantee, uniformly for both drivers, and adding checksum or signature verification is named future work rather than a gap. Three further consequences follow and are settled here: the install path must carry the resolved version, or the presence check answers for a version nobody asked for and the range never resolves twice (decision `[23]`); the version an operator can read back must become the resolved one (decision `[24]`); and an operator who needs a fixed, vouched-for artifact supplies it through `DRIVER_PATH` (decision `[20]`).

**`getCapabilities` may not know which source it is being asked about.** `pushdown` reads `schemaMetadataInfo.adapterNotes`, whose `driver` field release 0.2 writes and never reads. Whether `getCapabilities` carries the same field is unverified: `virtual-schema-common-java` models `schemaMetadataInfo` on every adapter request, which is why the plan expects it, but the adapter reads the wire rather than the framework. The answer decides whether a per-dialect capability set is legal at all — without it, an adapter can advertise only the intersection of every dialect's set. Gate 2.1 measures it before any table is built.

**PostgreSQL needs less collation machinery than MySQL, not a translation of it.** Roughly a third of `mysql_dialect.rs` exists to undo MySQL's default collation, which answers both of Exasol's character questions differently: `utf8mb4_0900_ai_ci` matches `widget` to `Widget` and orders by collation weight. PostgreSQL's built-in collations are all deterministic, and a deterministic collation equates two values only when their bytes are identical — Exasol's own equality. So PostgreSQL needs no collation on an equality operand, a `GROUP BY` key, or a `DISTINCT` operand. It needs one on an ordering operand: the official `postgres` image sets `LANG=en_US.utf8`, so `MIN` over `B`, `a`, `C` returns `a` where Exasol returns `B`, and `COLLATE "C"` sorts by byte value, which for UTF-8 is code-point order. One collation where MySQL needs two, and rendered at fewer sites.

That difference cascades. MySQL's grouping collation turns the `GROUP BY` key into an expression, which forces the same suffix onto every bare select-list occurrence of that column and creates the one composed shape MySQL rejects outright — a select-list predicate reading its query's character grouping key, which fails with error 1055 at every collation placement. PostgreSQL renders a bare grouping column, so the key and its select-list occurrence are the same expression and the shape is expected to delegate. Gate 2.3 measures that rather than reasoning it away, because the failing direction is a hard source error.

**PostgreSQL's driver accepts one connection option, which breaks a stated invariant.** `arrow-scan` requires that the user and password pass as separate ADBC options so no credential travels in a connection string. The ADBC PostgreSQL driver's `SetOption` accepts exactly `uri` and rejects `username` and `password` as unknown options, handing `uri`'s value verbatim to libpq's connect call. There is no design that preserves the invariant for PostgreSQL; the only choice is where the credential goes instead. It goes into a libpq keyword/value connection string built in memory at connect time, because that form escapes any byte by single-quoting and backslash-escaping, where URI form would need percent-encoding and a password containing `&`, `#`, `@`, or `/` would change the parse. Every invariant about persistence and disclosure survives and is asserted: nothing reaches the scan specification, `adapterNotes`, SQL text, `EXPLAIN VIRTUAL`, or any log line, and the scan strips the password from libpq's own diagnostics rather than trusting the driver to omit it. This is the plan's one deliberate narrowing of a security property; decision-log `[6]` records it.

**The two drivers disagree in small, load-bearing details.** The registry spells MySQL's versions `v0.6.0` and PostgreSQL's `1.12.0`, and release 0.2's lookup matches the scalar by exact string equality, so no shared "add a `v`" rule finds both. Range resolution dissolves that particular trap by comparing versions semantically after normalizing one optional leading `v`, but it does not make the two drivers uniform: MySQL sits below 1.0, where a minor bump is breaking, so its range stops at `<0.7.0`, while PostgreSQL at 1.x admits minor bumps to `<2.0.0`. The range is per-technology data for the same reason the spelling was. The PostgreSQL driver returns every `numeric`, constrained or not, as a UTF-8 string, so a column declared `DECIMAL(p,s)` is parsed from text rather than cast from a decimal, and `NaN`/`±Infinity` are legal source values no Exasol `DECIMAL` can hold. The PostgreSQL library is C++ and links `libstdc++` and `libgcc_s`, where the Go-built MySQL driver links neither, so a provisioned library is not yet a loadable one — and if it does not load, the fix lands in `language-container-rs`, outside this repo. Gates 2.2a and 2.2b test that first, because late discovery is the most expensive outcome in the plan: the fix returns through the release chain rather than through this repo.

- **Goals** — `DRIVER = 'postgresql'` reaching PostgreSQL through the same self-install, pushdown, and scan path MySQL uses; one closed driver identity owning every per-driver fact, so a third driver adds rows and an implementation; per-technology TOML files holding each technology's capability list and package row, with MySQL migrated onto the same mechanism in this plan; a version range resolved against the registry instead of a pinned version; a `DRIVER_PATH` property that lets an air-gapped operator supply the driver and skip self-install entirely; a per-dialect advertised capability set, resolved from the same field the renderer is resolved from, proven by an invariant test that enumerates dialects from the type; PostgreSQL delegation matching MySQL's scope plus `FULL OUTER JOIN`; every delegated PostgreSQL fragment verified against Exasol's own answer for the same query.
- **Non-Goals** — any driver beyond MySQL and PostgreSQL; run-time driver pluggability of any kind — no plugin loader, no TOML directory scan, no way for an operator to add a technology without a new adbc-vs release, per decision `[18]`; a property that selects a driver version, which `version_range` supersedes; a PostgreSQL schema other than `public`, and the property that would name one; mapping PostgreSQL `boolean`, `character(n)`, `text`, `uuid`, `jsonb`, `timestamptz`, or an unconstrained `numeric`; `AVG` and `SUM` delegation for either dialect, withheld for PostgreSQL by the same all-or-nothing rule that withholds them for MySQL despite PostgreSQL's exact-numeric `sum` being exact; `AGGREGATE_HAVING`, grouping by an expression, grouping by more than one column, scalar-function delegation, `REGEXP_LIKE`, `JOIN_CONDITION_ALL`; `ORDER BY` and `LIMIT`/`OFFSET`, withheld for every dialect because the emitting-script transport rather than the source is what makes them unsound; a benchmark against `postgresql-virtual-schema`; and preserving the separate-credential-option invariant for PostgreSQL, which its driver makes impossible.

### Decision

#### Architecture

```
  drivers/mysql.toml      drivers/postgresql.toml     checked in, one per technology
   [package] registry_path · library · entry_point · version_range   (no integrity field)
   [capabilities] the list that technology's dialect advertises
                          │
                          │  build.rs in adbc-vs-scan — generates &'static tables.
                          │  Compiled into the .so; nothing is parsed at UDF run time.
                          ▼
                    schemaMetadataInfo.adapterNotes.driver
                                    │
                                    ▼
                        ┌───────────────────────┐
                        │   Driver  (closed)    │   adbc-vs-scan::driver
                        │   MySql | Postgres    │   parse() · as_str()
                        └───┬────────┬──────┬───┘
              ┌─────────────┘        │      └──────────────┐
              ▼                      ▼                     ▼
   ┌────────────────────┐  ┌──────────────────┐  ┌──────────────────────┐
   │  dialect_for()     │  │ metadata_        │  │  package()           │
   │  &dyn SqlDialect   │  │ source_for()     │  │  &DriverPackage      │
   ├────────────────────┤  ├──────────────────┤  ├──────────────────────┤
   │ MySqlDialect       │  │ MySqlSource      │  │ registry_path        │
   │ PostgresDialect    │  │ PostgresSource   │  │ library·entry_point  │
   └─────┬──────────┬───┘  └──────────────────┘  │ version_range        │
         │          │       catalog SQL +        └──────────┬───────────┘
         │          │       its own type mapping            │
         ▼          ▼                                       ▼
  capabilities()  render(&Query)                  ┌──────────────────────┐
    (from TOML)    (Rust, per dialect)            │ resolve range vs.    │
         │              │                        │ registry index →     │
         │              │                        │ HIGHEST match, then  │
         │              │                        │ install under        │
         │              │                        │ <driver>/<version>/  │
         │              │                        └──────────┬───────────┘
         │              │                          DRIVER_PATH set? skip
         │              │                          all of it, use that path
         ▼              ▼                                   ▼
   getCapabilities   pushdown → scan spec (carries Driver) → scan UDF
```

The TOML files hold values, never behavior. `capabilities()` reads its own generated list; `render()` and the type mapping stay Rust, because no data format expresses them — which is also why the enum stays closed and a third technology ships in a release rather than in a file (decision `[18]`).

`dispatch` names no dialect. Both the `getCapabilities` arm and the `pushdown` arm resolve `Driver` from `adapterNotes`, so the advertised set and the renderer are the same dialect by construction — the property release 0.2 got from naming `MySqlDialect` twice, preserved by resolving it twice from one field. A request carrying no notes errors rather than defaulting, because advertising one dialect's set for another's source returns wrong rows for every capability the two do not share.

Three tables key on the identity, and they sit in two crates because their consumers do. `DriverPackage` lives with the enum in `adbc-vs-scan`, which the scan UDF and the adapter's provisioning both already read. `dialect_for` and `metadata_source_for` live in `adbc-vs-adapter`, which `adbc-vs-scan` does not depend on, so the dependency still points one way.

Type mapping moves behind `MetadataSource`. The catalog row is the source's own shape — MySQL needs the unsigned flag and the column character set, PostgreSQL needs `udt_name` and the collation's determinism — and no shared struct survives reduction without losing whichever fields one source has and the other does not. `MetadataSource::discover_tables` therefore returns already-mapped columns and recorded omissions, and `create_schema` branches on no driver at all. The same trait carries each dialect's source precondition check, stated by the dialect that renders the requirement and executed by the module that speaks the source's SQL: MySQL's two collations and its 8.0.17 floor, PostgreSQL's UTF-8 database encoding.

#### Patterns

| Pattern | Where | Why |
|---------|-------|-----|
| Closed enum as the single identity | `Driver` in `adbc-vs-scan::driver` | An exhaustive `match` makes the compiler name every table a new driver must extend; a string would let a table silently lack a row |
| Open/closed over drivers | Three tables keyed on `Driver`; no call site branches on a driver name | Adding a driver adds rows and an implementation rather than editing a dispatch |
| One resolution field for two answers | `getCapabilities` and `pushdown` both read `adapterNotes.driver` | An advertised set and a renderer resolved from different fields can disagree, which returns wrong rows rather than an error |
| Capability set stated per dialect, pinned by an invariant test enumerating dialects from the type | `SqlDialect::capabilities`, tested over every `Driver` variant | A third dialect inherits the invariant without editing the test; neither dialect's withholding governs the other's set |
| Collation rendered where the target's default answers Exasol's question differently, not uniformly | MySQL: equality and ordering; PostgreSQL: ordering only | PostgreSQL's deterministic equality already is Exasol's, so a suffix there would cost index use and buy nothing — and collating its grouping key would create MySQL's composed-shape exclusion for no reason |
| Source-specific knowledge behind one trait per driver | `MetadataSource` owns catalog SQL, type mapping, and the precondition check | The catalog row shape is per source; a lowest-common-denominator row would leak each source's gaps into the caller |
| Credential built at the moment of use, never stored | libpq conninfo assembled in memory from the freshly resolved `CONNECTION` object | The driver forces the credential into the connection string; persistence and disclosure invariants are kept where they still can be, and asserted |
| Redact at the boundary the adapter controls | Scan strips the password from surfaced errors, including driver text | libpq's diagnostics can echo connection parameters, and the driver's output is not the adapter's to change |
| Values as data, behavior as code | Capability list and package row in `drivers/<tech>.toml`; renderer and type mapping in Rust | Adding a technology is a data edit where the shape is uniform and a code change only where it genuinely differs |
| Data compiled in at build time, not parsed at run time | `build.rs` generates the `&'static` tables from the TOML | Keeps exhaustive `match` and `&'static` signatures, adds no `serde`/`toml` to the sandbox crate, and turns a malformed table into a build failure rather than a query-time panic |
| Closed to run-time extension, open to release-time extension | `Driver` stays a closed enum; the TOML adds no loader | A technology needs a dialect and a metadata source, which are code; a file naming an unknown technology could only fail |
| Version resolved by range, never pinned | `version_range` per technology, highest published match wins | A new compatible driver release reaches a cluster with no adbc-vs release; semantic comparison also removes the `v`-spelling trap that exact matching created |
| Resolved version in the install path | `<mount>/adbc-drivers/<driver>/<version>/<library>` | An unversioned path makes the presence check answer for a version nobody asked for, so the range would resolve once and never again |
| Explicit operator override beats an implicit fallback | `DRIVER_PATH` skips self-install; a missing file fails and never falls back | An air-gapped path the operator chooses is an interface; one discoverable only from an error message is not |
| Measure before advertising | Gates 2.1, 2.2b, 2.3 precede the capability sets they justify | An advertised capability the source refuses converts a query Exasol would answer into a hard failure |
| Differential verification against a local Exasol mirror | PostgreSQL E2E tier | Catches collation, type, and aggregate gaps generically instead of predicting them one at a time |
| Specs split framework from technology, mirroring the code's own seam | Framework features keep the protocol's vocabulary; `<technology>-<concern>` features hold one target's facts | The code already gives each per-driver decision one owner keyed on `Driver`; a spec library that mixes both concerns in one file describes a design the code does not have |
| Technology axis in the feature name, never in directory depth | `mysql-collation`, `postgresql-type-mapping`, `scan-udf/postgresql-scan` | `speq` represents exactly two path levels and drops a nested feature from `feature list` silently rather than rejecting it, so nesting would make a spec invisible |
| A concern splits where the technologies disagree, not along the protocol's taxonomy | Collation is its own feature per technology; fragment kind is not | Collation is the one divergence whose *reason* differs, and cutting there keeps every file under the scenario threshold that a per-fragment cut would blow |

### Consequences

| Decision | Alternatives Considered | Rationale |
|----------|------------------------|-----------|
| Closed `Driver` enum owning three tables | A string threaded to each call site; a trait object bundling all per-driver behavior | The enum makes a missing table row a build failure. A bundled trait would put `DriverPackage` data behind a method in the adapter crate, which the scan crate cannot reach |
| One TOML file per technology, generated into Rust by `build.rs` | Parse `include_str!` TOML at run time behind `LazyLock`; read the TOML from BucketFS; keep Rust constants and test them against the TOML | Run-time parsing adds `serde` and `toml` to the sandbox crate, which has neither, breaks every `&'static` signature, and moves a malformed-table failure from build to query time. Reading from BucketFS loses compile-time exhaustiveness for nothing, since the behavior is code regardless. Testing constants against TOML keeps the duplication |
| MySQL migrates onto the TOML mechanism now | Leave MySQL on Rust constants, TOML for PostgreSQL only | Two mechanisms for one kind of fact is the scattering the identity exists to end, and the migration's correctness is checkable — MySQL's generated set and row must equal today's constants, which existing tests already assert |
| `version_range` resolved to the highest published match | Pin one exact version per driver; pin a version with a property to override it | Asked for in the interview. Semantic comparison also removes the `v`-spelling hazard exact matching created, and the two ranges are deliberately not derivable from one rule — MySQL below 1.0 stops at `<0.7.0`, PostgreSQL at 1.x admits `<2.0.0` |
| TLS is the only integrity guarantee; the compiled-in digest is removed, not replaced | Pin the registry's Ed25519 signing key and verify the in-archive signature; add an optional operator-supplied expected digest; keep pinned versions so a digest stays reviewable | The registry publishes no checksum, and the one key signing both packages has no public half obtainable independently of the CDN that serves them, so verification would be anchored in the host it protects. An optional digest property is a reasonable later step but adds a permanent interface for a use case nobody has asked for. Applied uniformly to both drivers; `DRIVER_PATH` is the stronger guarantee for an operator who needs one. Decision `[22]` |
| Install path carries driver and resolved version | Keep the unversioned path; overwrite in place when the resolved version changes | An unversioned path short-circuits before any network call, so a cluster would never resolve the range a second time. Overwriting a shared object a running query has mapped is unsafe |
| `DRIVER_PATH` property, failing hard when the file is absent | Keep the informal "place the library at a fixed compiled-in path" fallback; a compiled-in override; fall back to the registry on a missing file | The old fallback's path is no longer fixed once the path carries a version, was never an operator's to choose, and was discoverable only from an error message. A silent registry fallback would defeat the air gap the property exists to serve |
| Resolve the dialect from `adapterNotes.driver` | Resolve from `schemaMetadataInfo.properties.DRIVER` | `pushdown` already reads `adapterNotes`; using one field for both answers is what keeps the advertised set and the renderer from diverging. Gate 2.1 confirms the field arrives on `getCapabilities` |
| Advertise `JOIN_TYPE_FULL_OUTER` for PostgreSQL only | Withhold it from both, keeping the sets identical | It is the one capability the two targets genuinely disagree on, so it is what makes the per-dialect set observable rather than merely architectural. Gated on measuring that PostgreSQL accepts an equi-conditioned full join |
| PostgreSQL collates ordering operands only | Collate every character operand uniformly, mirroring MySQL | Uniform collation would cost equality index use, and collating the grouping key would import MySQL's composed-shape exclusion into a dialect that has no need of it. The cost is a dependency on deterministic collations, paid by unmapping a column that declares a nondeterministic one |
| `E'<body>'` string literals | Bare `'...'` plus a `standard_conforming_strings` precondition check; hex plus `convert_from` | `E'...'` is independent of `standard_conforming_strings` and, with a doubled quote, of `backslash_quote`, needing no server check. A decoded-hex literal is typed `text`, which PostgreSQL will not implicitly compare against a numeric or temporal column |
| PostgreSQL exposes `public` only | A `SCHEMA_NAME` property; auto-detection from `search_path` | A property is a decision declined, and one added now cannot be narrowed later. Auto-detection would make the exposed table set depend on invisible server state that persisted metadata cannot track. A virtual schema per source schema needs no new property, and adding the property later stays compatible with `public` as its default |
| PostgreSQL `boolean` unmapped | Map it to Exasol `BOOLEAN` | Mapping it widens the set of Exasol types a source column produces, which `pushdown-dialect`'s closure argument is built on, and needs `LITERAL_BOOL`, which neither dialect advertises. Declarable-but-unmappable stays the model |
| PostgreSQL `character(n)` unmapped | Map to Exasol `VARCHAR`; map to Exasol `CHAR` | PostgreSQL pads the returned value and compares ignoring trailing spaces, so `VARCHAR` matches neither the value nor the comparison. Exasol `CHAR` matches both, but four features state the invariant that every delegated character operand is `VARCHAR`, and reopening it is a larger change than this scope buys |
| `FN_AGG_SUM` withheld for PostgreSQL too | Advertise it, refusing a floating-point operand at render time | PostgreSQL's exact-numeric `sum` is exact, but a capability is advertised once per session and admits `double precision`, whose sum depends on accumulation order and, under a parallel plan, is not stable between runs. Refusing narrowly at render time turns a working query into a hard failure |
| Credential inside PostgreSQL's `uri` option, redacted everywhere | libpq environment variables (`PGPASSWORD`); percent-encoded URI credential | The driver accepts no other option, so the only choice is the form. Environment variables are untested in a UDF sandbox, are process-visible, and `set_var` is `unsafe` in edition 2024. Conninfo escaping is total where URI percent-encoding breaks on a password containing `&`, `#`, `@`, or `/` |
| Type mapping moves behind `MetadataSource` | A separate `TypeMapper` trait keyed on `Driver` | A `TypeMapper` needs a common source-column struct, which loses the fields one source has and the other does not. Discovery already knows the catalog's column names, so mapping belongs with it |
| Keep the default `en_US.utf8` locale in the test container | Force `C.UTF-8` at initdb | The locale default is what makes a missing `COLLATE "C"` fail a test rather than pass silently, and it is what a production cluster is likelier to have |
| Spec library splits into framework and `<technology>-<concern>` features | A `postgresql/` or `framework/` top-level domain; directory nesting under `vs-adapter/mysql/`; parameterizing the MySQL scenarios by dialect; leaving PostgreSQL's scenarios as `DELTA:NEW` blocks in the shared files | A new domain is forbidden by this repo's spec governance. Nesting is measured to be silently ignored by `speq`, so a nested spec would be invisible rather than rejected. Parameterizing would erase the measured MySQL specifics — collation names, error 1055, backtick escaping, hex literals. Leaving the blocks in place is what the instruction rejected, and `pushdown-join` already showed the failure shape: a `DELTA:REMOVED` reproducing a MySQL-named scenario beside its PostgreSQL-flavored replacement in one file. Decision `[25]` |
| MySQL's shipped spec content migrates in this plan | Leave MySQL's scenarios in the framework files and give only PostgreSQL technology features | Two structures for one kind of fact is the scattering the split exists to end, and it would leave the third technology's author copying the wrong one. The same argument decision `[19]` made for the capability TOML, applied to spec text |
| `driver-provisioning` gets no technology split | A `mysql-provisioning` / `postgresql-provisioning` pair | Its technology-dependence is entirely *data* — registry path, library, entry point, version range — already living in `drivers/<technology>.toml` under decisions `[17]` and `[21]`. No provisioning scenario states a behavior one technology contradicts, so a technology feature would duplicate the TOML rather than separate a concern |
| `driver-provisioning` splits three ways along framework-internal lines | Keep one `driver-self-install` feature at 17 scenarios; split it by execution stage (fetch, then verify, then upload) | 17 scenarios in one file mixes three questions with three different answers: how a package is installed, which version is wanted, and how an operator opts out of both. A stage split would organize by execution order, which is the temporal leakage `/speq:design-philosophy` names — every stage would still carry the version decision. Decision `[26]` |
| `scan-udf` does get the split | Keep both credential mechanisms in `arrow-scan` | PostgreSQL's credential path is behavior, not data: its driver accepts only `uri`, so the credential goes into an escaped libpq conninfo string and is stripped from every surfaced error, where MySQL passes separate options. Leaving MySQL's mechanism in the framework file while PostgreSQL's moved out would give one decision two owners |

## Features

This plan restructures the spec library as well as extending it. Every feature is now either **framework** — true whatever technology backs the virtual schema — or **technology** — one target's own capabilities, rendering, mapping, or connection mechanism. Decision `[25]` records the split, the measurement behind the naming, and the migration mechanics. Technology features are flat, prefixed feature names inside the three existing domains, because `speq` represents exactly two path levels and silently ignores a nested directory.

| Feature | Kind | Status | Spec |
|---------|------|--------|------|
| vs-adapter/pushdown-dialect | framework | CHANGED | `vs-adapter/pushdown-dialect/spec.md` |
| vs-adapter/pushdown-projection | framework | CHANGED | `vs-adapter/pushdown-projection/spec.md` |
| vs-adapter/pushdown-filter | framework | CHANGED | `vs-adapter/pushdown-filter/spec.md` |
| vs-adapter/pushdown-aggregation | framework | CHANGED | `vs-adapter/pushdown-aggregation/spec.md` |
| vs-adapter/pushdown-join | framework | CHANGED | `vs-adapter/pushdown-join/spec.md` |
| vs-adapter/type-mapping | framework | CHANGED | `vs-adapter/type-mapping/spec.md` |
| vs-adapter/create-virtual-schema | framework | CHANGED | `vs-adapter/create-virtual-schema/spec.md` |
| vs-adapter/result-ordering | framework | CHANGED | `vs-adapter/result-ordering/spec.md` |
| vs-adapter/adapter-request-dispatch | framework | CHANGED | `vs-adapter/adapter-request-dispatch/spec.md` |
| vs-adapter/driver-selection | framework | NEW | `vs-adapter/driver-selection/spec.md` |
| vs-adapter/mysql-pushdown | technology | NEW | `vs-adapter/mysql-pushdown/spec.md` |
| vs-adapter/mysql-collation | technology | NEW | `vs-adapter/mysql-collation/spec.md` |
| vs-adapter/mysql-type-mapping | technology | NEW | `vs-adapter/mysql-type-mapping/spec.md` |
| vs-adapter/postgresql-pushdown | technology | NEW | `vs-adapter/postgresql-pushdown/spec.md` |
| vs-adapter/postgresql-collation | technology | NEW | `vs-adapter/postgresql-collation/spec.md` |
| vs-adapter/postgresql-type-mapping | technology | NEW | `vs-adapter/postgresql-type-mapping/spec.md` |
| scan-udf/arrow-scan | framework | CHANGED | `scan-udf/arrow-scan/spec.md` |
| scan-udf/mysql-scan | technology | NEW | `scan-udf/mysql-scan/spec.md` |
| scan-udf/postgresql-scan | technology | NEW | `scan-udf/postgresql-scan/spec.md` |
| driver-provisioning/driver-self-install | framework | CHANGED | `driver-provisioning/driver-self-install/spec.md` |
| driver-provisioning/driver-version-resolution | framework | NEW | `driver-provisioning/driver-version-resolution/spec.md` |
| driver-provisioning/driver-path-override | framework | NEW | `driver-provisioning/driver-path-override/spec.md` |

Six NEW technology features carry content that already existed: MySQL's from the permanent library, PostgreSQL's from this plan's own earlier deltas. Each migration is one `DELTA:REMOVED` block naming the library's heading byte for byte, paired with the scenario reproduced in the new feature's full spec — the merge has no marker for moving a scenario, so a move is always that pair. No feature file empties, so none is retired and no manual deletion step exists; every source file keeps a genuine framework remainder.

Three `pushdown-filter` scenarios take a second shape, because only part of each moves. Their MySQL-only steps go to `mysql-collation` and `mysql-pushdown`, and a dialect-neutral remainder stays. Each is therefore a `DELTA:REMOVED` block paired with a `DELTA:NEW` block of the same heading, carrying only the framework steps. `speq record` applies the removal before the addition, so the merged file holds the replacement once — verified against the CLI rather than assumed.

`driver-provisioning` splits three ways, and the axis is framework-internal rather than per-technology. Decision `[26]` records it. `driver-self-install` keeps the install mechanism — download, manifest check, upload, visibility wait, transport policy. `driver-version-resolution` takes everything read out of the registry index: which version a range resolves to, which platform package serves it, and whether the mount already holds that version. `driver-path-override` takes the `DRIVER_PATH` air-gap mechanism. None of the three is a technology feature: provisioning's technology-dependence is already TOML *data*, not spec scenarios, which is what decision `[25]` settled and this split does not disturb. Three of the moves leave the permanent library, so each is the same `DELTA:REMOVED`-plus-full-scenario pair every other migration uses.

## Impact

`CREATE VIRTUAL SCHEMA ... WITH DRIVER = 'postgresql'` works, exposing the `public` schema of the addressed PostgreSQL database, with the driver self-installed from the same registry MySQL uses. A PostgreSQL-backed schema delegates projection, filter, aggregation, and equi/outer joins on MySQL's terms, and additionally delegates `FULL OUTER JOIN`.

Existing MySQL virtual schemas keep working without recreation. Seven visible changes reach an operator, and the first four reach **existing MySQL deployments**, not only new PostgreSQL ones. MySQL driver provisioning shipped in release 0.2 — tag `v0.2.0` exists and the workspace version is 0.2.0 — so these are changes to released behavior.

- **A MySQL cluster can install a different driver version than it installed before.** Release 0.2 pins `v0.6.0` and installs exactly it. This release states `>=0.6.0, <0.7.0` and installs the highest version the registry publishes in that range, so a `0.6.1` published tomorrow is what a later `CREATE VIRTUAL SCHEMA` or `ALTER VIRTUAL SCHEMA REFRESH` installs and loads — with no adbc-vs release, no version bump, and no change an operator made. The driver a virtual schema runs is no longer a property of the adbc-vs version alone. `adapterNotes.driverVersion` records which version was resolved, and is the only place an operator can read it back. An operator who needs a fixed version pins it with `DRIVER_PATH` and supplies the library themselves.
- **`createVirtualSchema` and `refresh` now reach the driver registry even when the driver is already installed.** Release 0.2 could answer entirely from the BucketFS mount, because the install path was fixed and a present file ended the work. Resolving a range requires reading the index first, so a cluster with no outbound HTTPS egress that worked before will now fail unless `DRIVER_PATH` is set. This is the single most likely upgrade surprise in this release, and `DRIVER_PATH` is the supported answer to it.
- **The installed driver's BucketFS path changes**, from `<mount>/adbc-drivers/<library>` to `<mount>/adbc-drivers/<driver>/<version>/<library>`. A library an operator pre-loaded by hand at the old path is no longer found, and a script or monitoring check reading the old path breaks. Existing virtual schemas keep working: each records its own `driverPath` in `adapterNotes` and keeps using the path it recorded.
- **A self-installed driver is verified against nothing but its TLS connection, for both drivers.** Release 0.2 checks every downloaded MySQL package against a SHA-256 digest compiled into adbc-vs, produced and reviewed by a maintainer for one exact version. This release removes that digest and adds no checksum and no signature check in its place, for MySQL as well as PostgreSQL. The guarantee moves from "a maintainer reviewed this exact artifact" to "this artifact came over a connection whose server certificate the registry client verified" — a real reduction in a security property of released MySQL behavior, chosen deliberately rather than lost. The reasons are in decision `[22]`: the registry publishes no checksum, and the key signing both packages is obtainable only from the CDN that serves them. Adding checksum or signature verification is named future work, planned as its own change rather than assumed here. An operator who needs a fixed, vouched-for artifact today supplies it with `DRIVER_PATH`, which adbc-vs also verifies nothing about beyond its readability — but there the operator chose the file.
- **`DRIVER_PATH` is a new optional property**, applying to every driver. `WITH DRIVER = 'postgresql' CONNECTION_NAME = '...' DRIVER_PATH = '/buckets/bfsdefault/default/custom/pg-driver.so'` uses exactly that file and performs no index read, no download, and no BucketFS write. A path holding no readable file fails with a named error and never falls back to the registry. `BUCKETFS_CONNECTION` becomes optional when `DRIVER_PATH` is present, because its only purpose is the write credential self-install needs.
- **The unsupported-driver error text changes.** It now lists every supported value instead of naming `mysql` as the only one. A script matching that message breaks.
- **PostgreSQL credentials travel inside the connection string handed to the driver, in process memory.** The ADBC PostgreSQL driver accepts no separate credential option. No credential is written into the scan specification, `adapterNotes`, generated SQL, `EXPLAIN VIRTUAL` output, or any log line or error message, and the scan strips the password from libpq's own diagnostics. MySQL's separate-option path is unchanged. This is a narrowing of the previous posture, not an equivalent, and it is the reason a PostgreSQL `CONNECTION` object deserves a least-privilege, read-only role.
- **PostgreSQL columns of unmapped types are omitted, and the omitted set is wider than MySQL's.** `boolean`, `text`, `character(n)`, an unconstrained `numeric`, `timestamptz`, `uuid`, `jsonb`, and `bytea` are all omitted with a recorded reason. A `character(n)` column omission is the one likely to surprise, because MySQL's `CHAR` maps.

`adapterNotes` records an omitted column's source type under `sourceType` rather than `mysqlType`, and records the resolved driver version and versioned driver path; notes written by an earlier release still parse. Running the full test stack now needs a third container, and the CI `e2e` job's disk cleanup carries a PostgreSQL image on top of Exasol and MySQL.

## Dependencies

| Dependency | Detail |
|------------|--------|
| ADBC PostgreSQL driver | Registry path `postgresql`, library `libadbc_driver_postgresql.so`, entry point `AdbcDriverPostgresqlInit`, Apache-2.0. Fetched at run time, never bundled, so outside `cargo deny`'s scope. The index publishes `1.8.0` through `1.12.0` as bare scalars, and `>=1.12.0, <2.0.0` is the range `drivers/postgresql.toml` states. No exact version is compiled in, so no spelling is load-bearing — resolution parses the scalar and normalizes one optional leading `v` |
| `semver` (range resolution) | New **direct** dependency of `adbc-vs-adapter`, already in `Cargo.lock` at 1.0.28 transitively via `rustc_version`. `MIT OR Apache-2.0`, so `deny.toml`'s allow list and `about.toml`'s `accepted` list are unchanged and `make deny` needs no policy edit. `adbc-vs-scan` does not need it: ranges are resolved in the adapter, and the scan only loads a path |
| `build.rs` in `adbc-vs-scan` | First build script in the workspace. Generates the driver tables from `drivers/*.toml` into `OUT_DIR`. Runs inside the `rust:1.94-trixie` container build like any other codegen; adds no run-time dependency and does not affect the SLC fingerprint `cargo exasol-udf validate` checks |
| `postgres:18.6-trixie` | Test container. Within the driver's documented tested range of PostgreSQL 14–18; Debian trixie matches the `rust:1.94-trixie` builder |
| `language-container-rs` SLC | Must supply `libstdc++.so.6` and `libgcc_s.so.1` for the C++ driver to load. Gate 2.2b verifies, recording the SLC's actual `ldd` output. A failure is fixed there, not here, and blocks this plan |

## Migration

| Current | New |
|---------|-----|
| `adapterNotes` omission field `mysqlType` | `sourceType`, with `mysqlType` accepted on deserialization so notes Exasol persisted before this release still parse |
| `ScanSpec` without a driver field | Carries the driver identity. No migration: the specification is generated fresh on every pushdown and never persisted |
| Existing MySQL virtual schemas | No action to keep working. `adapterNotes.driver` already records `mysql`, which is the field resolution now reads, and each schema keeps using the `driverPath` it recorded |
| MySQL driver pinned at `v0.6.0` (released behavior) | Range `>=0.6.0, <0.7.0`, resolved to the highest published match on every `createVirtualSchema` and `refresh`. **A behavior change to a released feature**: an existing deployment can acquire a `0.6.x` it never had. No operator action makes it happen and none prevents it, short of setting `DRIVER_PATH`. Called out in § Impact rather than treated as an implementation detail |
| Driver install path `<mount>/adbc-drivers/<library>` | `<mount>/adbc-drivers/<driver>/<version>/<library>`. Old files are left in place, not migrated or deleted: they are harmless once the path is versioned, and deleting a library a running query may have mapped is unsafe. An operator who pre-loaded the old path by hand must move the file or switch to `DRIVER_PATH` |
| Provisioning answerable from the mount alone | Requires the registry index unless `DRIVER_PATH` is set. An air-gapped cluster that worked in 0.2 needs `DRIVER_PATH` after upgrading |
| `adapterNotes.driverVersion` holding the pinned version | Holds the resolved version. Notes written before this release parse unchanged |
| Compiled-in SHA-256 digest verified against every downloaded package (released behavior) | Removed with nothing in its place, for **both** drivers. Self-install performs no checksum and no signature check; the download's TLS connection, whose server certificate the registry client verifies unconditionally, is the only integrity guarantee. **A narrowing of a released security property**, not an implementation detail: a MySQL deployment upgrading from 0.2 loses maintainer review of the exact artifact it installs. No operator action restores it; `DRIVER_PATH` replaces it for an operator who supplies the artifact themselves. Adding checksum or signature verification is deliberate future work — see decision `[22]`. Called out in § Impact |

## Implementation Tasks

### Phase 1: PostgreSQL test stack

- [ ] 1.1 Add the `postgres` service to `docker-compose.yml`: image `postgres:18.6-trixie`, static IP `172.28.5.11` on the existing `adbc-vs` network, host port `127.0.0.1:5432`, `pg_isready` healthcheck matching the MySQL service's interval and retries, and `./docker/postgres/init.sql` mounted into `/docker-entrypoint-initdb.d/`. Keep the image's default `LANG=en_US.utf8` cluster locale and set no `POSTGRES_INITDB_ARGS`: locale ordering is what makes a missing `COLLATE "C"` fail a test instead of passing silently. Seed a minimal fixture for now; task 8.1 grows it.
- [ ] 1.2 Add `stack-up-postgres` to the `Makefile` (`compose up -d --wait postgres`), make `stack-up` depend on it alongside `stack-up-mysql`, and add `test-it-postgres` with prereq `stack-up-postgres`. Extend `tests/make_targets_test.rs` for the new targets and the new prereq edges.

### Phase 2: Gates

- [ ] 2.1 Gate [expert]: measure whether a `getCapabilities` request on Exasol 2025.2.1 carries `schemaMetadataInfo.adapterNotes`, and whether the `driver` field inside it is the value `createVirtualSchema` recorded. Use the throwaway Lua adapter probe in `tests/common/probe.rs`, or log the raw request body from the Rust adapter — capture the request verbatim rather than inferring from a successful response. Also record whether `schemaMetadataInfo.properties` arrives, as the fallback source. **If the identity does not arrive, stop and re-plan**: per-dialect capability sets become illegal, every dialect must advertise the intersection, `JOIN_TYPE_FULL_OUTER` is withheld from both, and `driver-selection`'s and `pushdown-dialect`'s scenarios change. Record the measurement in decision-log under Review Findings.
- [ ] 2.2a Gate: resolve the PostgreSQL package from the registry index by hand — read the `postgresql` entry, record the **full list** of published `version` scalars and the exact relative `url` of the highest one's `linux_amd64` package — then fetch and extract `libadbc_driver_postgresql.so` and upload it to BucketFS using `BUCKETFS_CONNECTION`'s credentials. The version list is what task 6.1's range resolution is tested against, so record every scalar rather than only the one resolved. Record the resolved absolute URL for tasks 2.4 and 6.4. Prerequisites: outbound HTTPS egress and a reachable BucketFS write endpoint. Planning already measured this once — the index published `1.8.0`, `1.9.0`, `1.10.0`, `1.11.0`, `1.12.0` for `postgresql` and `v0.1.0` through `v0.6.0` for `mysql`, and the `1.12.0` block's `linux_amd64` url was `postgresql/v1.12.0/postgresql_linux_amd64_v1.12.0.tar.gz` — so this gate confirms a measurement rather than making one, and a divergence means the registry moved.
- [ ] 2.2b Gate: load the library task 2.2a uploaded from inside the Exasol UDF sandbox by the entry point `AdbcDriverPostgresqlInit`, calling the driver manager directly rather than through the scan path, from a throwaway raw driver-load entry point compiled into the crates. Prerequisites: task 2.2a, `make cross-udf-build`, `make stack-up`, and the SLC installed. Record the SLC's resolved shared-library set as measured by `ldd` inside the sandbox, so the gate reports what the container actually supplies rather than what the plan predicted. Record the exact loader error if it fails. **A failure blocks this plan** and is fixed in `language-container-rs`, then flows back through the release chain — escalate rather than working around it here.
- [ ] 2.3 Gate [expert]: measure the PostgreSQL dialect's semantic assumptions against `postgres:18.6-trixie` and Exasol 2025.2.1, using `psql` and `exapump` directly rather than the adapter, so the gate does not wait on Phase 4. Measure and record each: (a) `FULL OUTER JOIN` with an equality condition executes rather than being rejected over the merge-joinable restriction; (b) a select-list predicate reading a bare character `GROUP BY` key executes, i.e. there is no analogue of MySQL's error 1055 for this shape; (c) an `E'<body>'` literal carries its exact value under `standard_conforming_strings` both `on` and `off` and under `backslash_quote` set to `off`, for a value containing a backslash, a single quote, and a multi-byte character; (d) `MIN`/`MAX` over `B`, `a`, `C` returns Exasol's answer with `COLLATE "C"` and a different answer without it; (e) an uncollated `GROUP BY` and `COUNT(DISTINCT)` over values differing only by case and only by accent return the four groups Exasol returns; (f) `LIKE` matches the same rows with and without a collation suffix; (g) the driver returns `numeric(12,4)` as an Arrow string, and `NaN`/`Infinity` are storable in such a column; (h) the `selectListDataTypes` Exasol sends for aggregates over a PostgreSQL-backed schema match the values already measured against MySQL. Any result contradicting a spec Background bullet re-plans that bullet and its scenarios before the rendering task starts.
- [ ] 2.4 Gate: read the `MANIFEST` out of the PostgreSQL package task 2.2a fetched and record its `[Files] driver` value, which is expected to be `libadbc_driver_postgresql.so`. That value is what task 6.4's `library` field states and what the manifest check in provisioning compares against, so a divergence changes the TOML row rather than the code. Record it verbatim. Nothing else about the package is verified: decision `[22]` settles that self-install checks no checksum and no signature, so this gate measures the one archive fact provisioning still reads. Prerequisite: task 2.2a.

### Phase 3: Driver identity foundation

- [ ] 3.1 [expert] Introduce `Driver` in `adbc-vs-scan::driver` as a closed enum with `parse` (case-insensitive), `as_str` (canonical lower case), and `package() -> &'static DriverPackage`, and introduce the per-technology TOML mechanism that feeds it. Add `crates/adbc-vs-scan/drivers/mysql.toml` holding a `[package]` table with `registry_path`, `library`, `entry_point`, and `version_range`, and add `crates/adbc-vs-scan/build.rs` generating the `&'static DriverPackage` table from every file in `drivers/` into `OUT_DIR`, included by `driver.rs`. The build script MUST fail the build, naming the file and the field, on a malformed or incomplete table — that failure mode is the reason codegen was chosen over a run-time parse, per decision `[17]`, and it MUST be asserted by a test that builds a deliberately broken fixture. `adbc-vs-scan` MUST gain no run-time dependency: `toml` and `serde` belong to the build script's `[build-dependencies]` only. Set MySQL's `version_range` to `>=0.6.0, <0.7.0` and carry no exact version and no digest in the row; task 6.1 makes the range resolvable and until it lands MySQL resolves to `v0.6.0` as the only in-range published version, so this task stays behavior-preserving. `PACKAGE_SHA256` stays a bare constant that `Registry::pinned()` still reads, so digest verification keeps working unchanged through this task; task 6.6 removes it, which is why the row has no integrity field to hold it. Replace `Registry::pinned()`'s reads of the bare constants and `provisioning.rs`'s `DRIVER_LIBRARY`/`DRIVER_VERSION` re-export with per-driver reads threaded from the identity, and make `driver_install_path` take it. Add no PostgreSQL row yet — task 6.4 does. Every existing provisioning test MUST pass unchanged.
- [ ] 3.2 [expert] Route dialect resolution through the identity. Add `dialect_for(Driver) -> &'static dyn SqlDialect` beside the `SqlDialect` trait, resolve `Driver` from `schemaMetadataInfo.adapterNotes` in both `dispatch`'s `getCapabilities` and `pushdown` arms, and remove `dispatch`'s import of `MySqlDialect` so no call site names a concrete dialect. A `getCapabilities` request carrying no notes MUST error naming the missing field rather than defaulting to a driver. Replace `create_schema::SUPPORTED_DRIVER` and `validate_driver` with `Driver::parse`, whose error lists every supported value generated from the identities. Add the connection-address scheme check against the resolved driver. Update `dispatch_test.rs` and `capabilities_tests.rs` for the notes-bearing request shape.
- [ ] 3.3 Add the driver identity to `ScanSpec` and to the scan's driver load: rename `MysqlConnection` to `SourceConnection` and `open_mysql_connection` to an open taking the identity, reading the entry-point symbol from `Driver::package()` instead of the module-level `DRIVER_ENTRY_POINT`. Update `scan_entry.rs`, `scan_spec.rs`, their tests, and `mysql_it_test.rs`'s call sites. MySQL's rendered scan specification gains one field and nothing else changes. Split `connection_tests.rs`'s credential assertions along the same seam the specs now use: `credentials_are_resolved_per_open_and_never_persisted` asserts the framework invariant `scan-udf/arrow-scan` owns — the credential is read from the resolved `CONNECTION` at open and reaches no persisted structure — while `mysql_passes_credentials_as_separate_options` and `mysql_connection_string_carries_no_credential` assert the MySQL mechanism `scan-udf/mysql-scan` owns. Task 5.4 adds PostgreSQL's counterpart beside them.
- [ ] 3.4 Make the capability invariant test enumerate dialects from the `Driver` type rather than naming one: run the every-advertised-capability-renders arm and the byte-identical-re-render arm once per variant. Add the test that every identity yields a dialect, a package, and a metadata source with no fallback, and the test asserting the installed library filenames are pairwise distinct across identities. The set-difference test lands with task 4.4, once a second set exists.
- [ ] 3.5 [expert] Move type mapping behind `MetadataSource`. `discover_tables` returns columns already mapped to `MappedType` with omissions recorded, so `create_schema` no longer calls `type_mapping::map_column` and branches on no driver; add the dialect-stated source precondition check to the trait, with MySQL's collation-and-version check as its first implementation and `discover_collations` no longer an inherent method. Rename `Omission.mysql_type` to `source_type` serialized as `sourceType`, with `#[serde(alias = "mysqlType")]` so notes Exasol persisted before this release still parse. Relocate the mapping assertions that move out of `create_schema_tests.rs`. Rename `crates/adbc-vs-adapter/src/type_mapping_tests.rs` to `mysql_type_mapping_tests.rs`, so the test module names the technology its assertions belong to and sits beside the `postgres_type_mapping_tests.rs` task 5.2 adds. The rename is mechanical and behavior-preserving — every assertion moves unchanged — and it is what keeps the test layout readable against the spec layout decision `[25]` settles, where `vs-adapter/type-mapping` is framework and each technology's table lives in its own feature. Add `crates/adbc-vs-adapter/src/metadata_source_tests.rs` for the two framework rules that bind every source rather than one: `each_source_maps_its_own_catalog_row`, asserting each implementation returns already-mapped columns from its own catalog shape with no shared source-column struct, and `every_mapped_type_is_in_the_declarable_set`, asserting that no source maps a column to an Exasol type outside the closed set `vs-adapter/pushdown-dialect`'s closure argument rests on.
- [ ] 3.6 Generalize the integration tier's driver-fetch helper over the driver identity, and stop it reconstructing the package URL by convention. `mysql_it_test.rs`'s `package_url()` builds the whole URL from `REGISTRY_BASE_URL`, `REGISTRY_DRIVER_PATH`, `DRIVER_VERSION`, and `PLATFORM` without reading `index.yaml` at all, and `install_driver` caches the extracted library under `PACKAGE_SHA256`; the range removes both constants, so neither expression survives. Replace them with a shared per-driver helper taking a `Driver` that resolves the range through the index exactly as the product does and caches under the **resolved version**. This closes a divergence that predates the range: today the tier and the product disagree about where a package lives, and `extract_library` matches the filename directly where the product reads `MANIFEST`, so a registry layout change would break them differently. Have the helper read the manifest too. `mysql_it_test.rs` MUST still fetch, extract, and cache, and MUST still fail rather than skip. This is the only route the repo has to a driver library outside BucketFS, and it is a prerequisite of task 7.2. Depends on task 6.1 for range resolution, so it runs in Group G after task 6.1 rather than in Group D.
- [ ] 3.7 [expert] Move MySQL's advertised capability list out of `MySqlDialect::capabilities()` and into `drivers/mysql.toml` as a `[capabilities]` list, generated by task 3.1's build script into the same `&'static` shape the trait method returns. `SqlDialect::capabilities()` stays a trait method — only the values move — so the per-dialect ownership decision `[3]` records is unchanged and `dialect_for` still answers per virtual schema. The generated MySQL set MUST be exactly the set release 0.2 advertises, asserted element for element rather than by count, and every existing capability test MUST pass unmodified. A capability name stays a bare protocol string: `SqlDialect::capabilities` keeps returning `&'static [&'static str]`, this plan adds no `Capability` type, and the build script MUST NOT carry an allowlist of valid capability names — that would be the second capability list `vs-adapter/pushdown-dialect` forbids. An unknown name is caught by task 3.4's every-advertised-capability-renders invariant test, which finds no rendering path for it. This is the migration decision `[19]` requires, and doing it before the PostgreSQL dialect exists means PostgreSQL's list is authored into the mechanism rather than retrofitted onto it.
- [ ] 3.8 Make the new build inputs prerequisites of the `.so`, and assert it. The `$(SO_FILE)` rule's prerequisites are `ADAPTER_SRCS`, `SCAN_SRCS`, and four manifest paths; `crates/adbc-vs-scan/build.rs` and `crates/adbc-vs-scan/drivers/` are under neither, so after task 3.1 a `version_range`, capability-list, or build-script edit leaves `make cross-udf-build` a no-op and `make test-artifact`, `make test-e2e`, `make test-e2e-postgres`, and `make bench` all certify the previous `.so`. Add `SCAN_BUILD_INPUTS := crates/adbc-vs-scan/build.rs $(shell find crates/adbc-vs-scan/drivers -type f 2>/dev/null)` to the `Makefile` and add it to the `$(SO_FILE)` rule's prerequisite list. **The assertion is the guard, not the Makefile edit**: extend `crates/adbc-vs-adapter/tests/make_targets_test.rs` with a test modelled on `touching_source_invalidates_so` that touches `crates/adbc-vs-scan/drivers/mysql.toml` and asserts `make -q cross-udf-build` reports the `.so` out of date, so a later prerequisite-list edit that drops the TOML files fails a test rather than silently returning stale binaries. Lands with task 3.1, which creates both inputs.

### Phase 4: PostgreSQL dialect rendering

- [ ] 4.1 Add `postgres_dialect.rs` with `PostgresDialect` implementing `SqlDialect`: double-quote identifier quoting doubling an embedded double quote, projection rendering, table qualification by the recorded namespace, the single-constant projection for an empty select list, and positional `EMITS` naming. Advertise MySQL's set verbatim for now; tasks 4.2 to 4.4 adjust it. Register the dialect in `dialect_for`.
- [ ] 4.2 [expert] Render PostgreSQL filters: string literals as `E'<body>'` with a backslash doubled and a single quote doubled, rendered unconditionally so no value changes the statement's shape; `DATE`/`TIMESTAMP` typed literals; numeric and null literals; every advertised predicate; `<>` for not-equal; parenthesized nesting. Apply `COLLATE "C"` to a character operand of `predicate_less`, `predicate_lessequal`, and `predicate_between` and to nothing else, per gate 2.3(c), (d), and (f). "Nothing else" is load-bearing at exactly three sites where MySQL does render a suffix and `vs-adapter/mysql-collation` states it: a `LIKE` operand, an `IN` compared operand, and a predicate in select-list position. Each MUST render bare here, and each MUST be asserted rather than left implied. Read both comparison operands by position.
- [ ] 4.3 [expert] Render PostgreSQL aggregation: `COUNT`, `COUNT(*)`, `COUNT(DISTINCT)`, `MIN`, `MAX` in canonical upper case whatever case arrived; `COLLATE "C"` on a character operand of `MIN`/`MAX`; no collation on a `GROUP BY` key, on a bare select-list occurrence of a grouped column, or on a `DISTINCT` operand; `single_group` renders no `GROUP BY`. Per gate 2.3(b), render rather than reject a select-list predicate reading the query's character grouping key, and keep MySQL's rejection bound to the MySQL dialect. Reject `having`, `orderBy`, and `limit` elements with the same named errors MySQL's dialect returns.
- [ ] 4.4 Render `FULL OUTER JOIN` and add `JOIN_TYPE_FULL_OUTER` to the PostgreSQL capability set, conditional on gate 2.3(a); render `INNER`/`LEFT OUTER`/`RIGHT OUTER` and the positional aliasing and column qualification MySQL's dialect uses, with no collation on a character join key. Add the test asserting the two dialects' sets differ in exactly `JOIN_TYPE_FULL_OUTER`, and `avg_and_sum_are_withheld_per_dialect`, asserting that neither dialect advertises `FN_AGG_SUM`, `FN_AGG_SUM_DISTINCT`, `FN_AGG_AVG`, or `FN_AGG_AVG_DISTINCT` — the framework all-or-nothing rule `vs-adapter/pushdown-aggregation` states over every dialect, which needs a per-dialect assertion now that there are two. If gate 2.3(a) measured a rejection, withhold the capability, record it in the decision log, and revise `pushdown-join`'s and `pushdown-dialect`'s scenarios to state that the two sets coincide in this release.
- [ ] 4.5 Unit tests for `PostgresDialect` in `postgres_dialect_tests.rs`: identifier quoting including an embedded double quote and a mixed-case name; every literal form; every predicate form; the full collation-placement matrix over character, numeric, and temporal operands in comparison, ordering, grouping, `DISTINCT`, and `MIN`/`MAX` positions; each join type; nested joins; a shared column name; an unresolvable column; an unrenderable aggregate and join type; and a byte-identical re-render of one request. Name three of them after the framework scenarios `vs-adapter/pushdown-filter` states for every dialect, so the coverage audit finds one test per dialect: `like_renders_its_escape_clause`, `between_and_in_keep_operand_order`, and `select_list_predicate_renders_as_projected_boolean`. Their MySQL arms need no new code and MUST NOT be renamed — `mysql_dialect_tests.rs::like_renders_pattern_and_escape`, `::between_and_in_keep_operand_order`, and `::a_select_list_predicate_renders_where_the_query_groups_by_nothing` already assert the framework steps plus the MySQL halves `vs-adapter/mysql-collation` and `vs-adapter/mysql-pushdown` now state. Verify each still asserts `COLLATE utf8mb4_0900_as_cs` on the `LIKE` operand, on the `IN` compared operand rather than on the list, and on a select-list predicate, and the hex form on a `LIKE` pattern and its `escapeChar`; the three MySQL scenarios that stated those rules as unqualified framework `MUST`s are removed from `pushdown-filter` by this plan, so these tests are what keeps them asserted.

### Phase 5: PostgreSQL metadata, mapping, and connection

- [ ] 5.1 Add the PostgreSQL metadata source: `information_schema.tables` and `information_schema.columns` filtered to `table_schema = 'public'`, reading `table_name`, `column_name`, `ordinal_position`, `data_type`, `character_maximum_length`, `numeric_precision`, `numeric_scale`, `is_nullable`, and `collation_name`, exposing both `BASE TABLE` and `VIEW`. Record `public` as the namespace in `adapterNotes`. Do not reuse MySQL's query text: `COLUMN_TYPE` does not exist in PostgreSQL and `table_schema` names a schema rather than a database.
- [ ] 5.2 Add the PostgreSQL type mapping and its unit tests: `integer`, `bigint`, `numeric(p,s)`, `character varying(n)`, `date`, `timestamp without time zone`, `double precision` mapped; a `numeric` precision above 36, an absent precision, an absent character length, `character`, `boolean`, `text`, `smallint`, `real`, both `time` types, `timestamptz`, `uuid`, `json`, `jsonb`, `bytea`, `interval`, `money`, and `ARRAY` unmapped with the reason naming the type; a character column of a nondeterministic collation unmapped, treating a server that reports no determinism flag as deterministic. Include `oversize_numeric_is_unmapped` and `nullability_carries_through`, the PostgreSQL half of the two framework rules `vs-adapter/type-mapping` states over every source.
- [ ] 5.3 Add PostgreSQL's source precondition check through the `MetadataSource` trait: the database encoding MUST be `UTF8`, checked before any table is discovered and on every request that resolves the source `CONNECTION`, with the error naming the reported encoding and `UTF8`. Assert MySQL's collation check does not run for a PostgreSQL source.
- [ ] 5.4 [expert] Build PostgreSQL's connection value as a libpq keyword/value connection string from the `CONNECTION` object's address and credentials: host, port, and `dbname` from the address, every address query parameter carried across as a keyword of the same name, `user` and `password` from the object, and every value single-quoted with an embedded quote or backslash escaped. Pass it as the driver's only accepted database option, and pass no `username` or `password` option for this driver. Strip the resolved password from every surfaced error, including text the driver or libpq produced, while keeping the host and database in the message. The connection value MUST NOT be interpolated as an error subject: `connect_error` today passes `&connection.uri` as the subject and redacts only the driver's message, which for PostgreSQL would print the conninfo string verbatim. Redaction applies to every caller of the shared open path, not to the scan alone — `create_schema.rs:193-199` opens the source connection through it, so `createVirtualSchema`, `refresh`, and `setProperties` traverse the same value. Unit-test the escaping over a password containing a single quote, a backslash, a space, and an ampersand, and test that the redaction survives a driver error carrying the connection string.

### Phase 6: Range resolution, driver path, and PostgreSQL provisioning

This phase implements all three `driver-provisioning` features. Tasks 6.1, 6.2, and 6.4 carry `driver-version-resolution`; task 6.3 carries `driver-path-override`; tasks 6.5 and 6.6 carry `driver-self-install`. The split is one of spec ownership only: no task, file, or ordering edge changes because of it, and decision `[26]` states why.

- [ ] 6.1 [expert] Replace the registry index's exact-match version lookup with range resolution. `registry_index::package_url` today finds a block with `block.version == version`, raw `String` equality; change it to parse every published `version` scalar for the driver, normalize one optional leading `v`, and select the **highest** version satisfying the driver's `version_range`, then read that block's `linux_amd64` `url` as it does now. Add `semver` as a direct dependency of `adbc-vs-adapter` (already in `Cargo.lock` at 1.0.28 via `rustc_version`, `MIT OR Apache-2.0`, so no `deny.toml` or `about.toml` change). Return the resolved version alongside the URL, because tasks 6.2 and 6.4 both need it. A published scalar that does not parse MUST be skipped rather than failing the lookup, so one malformed entry cannot make a driver uninstallable; a range satisfied by no published version MUST fail with an error naming the driver, the range, and the versions the index publishes. Extend the `tests/fixtures/index.yaml` fixture with several in-range and out-of-range versions per driver, deliberately including both the `v`-prefixed and bare spellings and one unparseable scalar, and assert the highest in-range version wins in each case. Replace `LookupError::NoVersion`'s "the version adbc-vs pins" wording, which the range makes false.
- [ ] 6.2 [expert] Version the BucketFS install path and rebind the presence check to the resolved version. `driver_install_path` is `mount.join(DRIVER_DIRECTORY).join(DRIVER_LIBRARY)` and `ensure_driver` returns early on `install_path.is_file()` before any network call; change the path to `<mount>/adbc-drivers/<driver>/<version>/<library>` and its `driver_url` twin to match, and move the presence check to **after** range resolution so it tests the version actually resolved. Without this the range resolves exactly once per cluster and never again, which is decision `[23]`. Assert explicitly that a mount already holding an older in-range version does **not** short-circuit the install of a newer resolved one, and that installing the newer version leaves the older file in place rather than deleting or overwriting it. Record the consequence in the error path too: an unreachable registry can no longer be answered from the mount, so `RegistryUnreachable` and `RegistryRejected` MUST name `DRIVER_PATH` as the supported offline route rather than a fixed path to pre-load, per task 6.3.
- [ ] 6.3 Add the `DRIVER_PATH` property. Read it in `create_schema::open` beside the three existing properties as an **optional** one — note `SchemaError::MissingProperty` carries `&'static str`, which a new optional property does not need but the new error variants do. When present, skip `provisioning::ensure_driver` entirely: no index read, no download, no integrity check, no BucketFS write. Fail with a named error carrying the property name, the path, and the driver when the path holds no readable file, and MUST NOT fall back to the registry. Make `BUCKETFS_CONNECTION` optional when `DRIVER_PATH` is present, since its only purpose is the self-install write credential, and keep it required otherwise. Record the path in `adapterNotes.driverPath` as today, so `refresh` and `setProperties` reuse it without the property being re-supplied, and let `setProperties` change it. All three handlers funnel through `discover` → `open`, so one change covers `createVirtualSchema`, `refresh`, and `setProperties`. Update the `create_statement` helper in `tests/common/schema.rs` and the property spellings in `dispatch_test.rs`. Extend `dispatch_test.rs::missing_property_named_in_error` in the same task with two cases — it exercises only the `DRIVER`-omitted case today: a statement omitting `BUCKETFS_CONNECTION` while carrying no `DRIVER_PATH`, asserting the error names `BUCKETFS_CONNECTION`; and the same statement with `DRIVER_PATH` set, asserting that whatever error `FakeCtx` produces is **not** a missing-property error naming `BUCKETFS_CONNECTION`. That negative is the strongest claim this tier can make: `dispatch_test.rs` covers only request paths that need no live connection, and a `createVirtualSchema` passing property validation goes on to open the source and discover tables. The success half of the scenario's last step is carried by `e2e_postgres_test.rs::driver_path_is_accepted_and_bypasses_provisioning`, and the required half by `create_schema_tests.rs::bucketfs_connection_required_without_driver_path`.
- [ ] 6.4 Add `crates/adbc-vs-scan/drivers/postgresql.toml`: `[package]` with `registry_path = "postgresql"`, `library = "libadbc_driver_postgresql.so"`, `entry_point = "AdbcDriverPostgresqlInit"`, and `version_range = ">=1.12.0, <2.0.0"`, plus the `[capabilities]` list tasks 4.1 to 4.4 settle. State no exact version and no digest. Add the test asserting each driver's range resolves against the **real** index to a published version — the one assertion that would catch a range no longer satisfiable because the registry moved on — and that the two drivers' ranges are independent rather than derived from a shared rule, since MySQL's stops below `0.7.0` while PostgreSQL's admits every `1.x`.
- [ ] 6.5 Extend provisioning tests for two drivers: add a PostgreSQL entry to the stub registry in `tests/provisioning_test.rs`; assert both libraries install side by side without either overwriting or satisfying the other's presence check, now under distinct driver **and** version path segments; assert a missing platform package and the offline error each name the driver and its own path; assert only the file the `MANIFEST` names is uploaded, so the package's differently-spelled license files are not — `LICENSE` in the MySQL package and `LICENSE.txt` in the PostgreSQL one. Assert the `DRIVER_PATH` route performs no HTTP request at all, against a stub registry that fails the test if contacted.
- [ ] 6.6 Remove digest verification from the provisioning path, per decision `[22]`. Delete `adbc-vs-scan::driver`'s `PACKAGE_SHA256`, `Registry`'s `sha256` field, and the `verify_digest` call and function, so a downloaded package is extracted without any checksum or signature check and the TLS certificate verification the registry client already performs is the only integrity guarantee. Delete the digest-mismatch provisioning test with the mechanism it asserts, and delete any digest field from the stub registry in `tests/provisioning_test.rs`. Add nothing in its place: **no `pgp` or other crypto dependency, no pinned key fingerprint, and no operator-supplied digest property** — the middle grounds were considered and rejected in decision `[22]`, and reintroducing one here would ship an interface nobody asked for. `deny.toml` and `about.toml` are unchanged, because no dependency is added. Keep the `MANIFEST` `[Files] driver` check, which is a filename check rather than an integrity check and survives this removal. Assert the absence rather than leaving it merely unimplemented: `missing_driver_is_fetched_per_driver` MUST install successfully from a stub package the registry serves with no digest and no valid signature. **This MUST run immediately after task 6.1, before task 6.2**: once a range can resolve to a version other than `v0.6.0`, a surviving compiled-in digest turns every newer in-range MySQL package into a false mismatch.

### Phase 7: Scan coercion and the PostgreSQL integration tier

- [ ] 7.1 Coerce a `numeric` column the driver returns as an Arrow string into its declared `DECIMAL(p,s)`, with the source precision and scale intact, treating the string form as a first-class case rather than a fallback. Name that assertion `string_form_numeric_coerces_to_declared_decimal`; it is the first of the two halves `scan-udf/postgresql-scan` states, and only the failing half had a scenario before this revision. Fail with an error naming the value and the declared type for `NaN`, `Infinity`, and `-Infinity`, per value rather than per column, and keep a finite value in the same column coercing. Add the cases to `coerce_tests.rs`.
- [ ] 7.2 Add the PostgreSQL integration tier: a `postgres-it` cargo feature, default off, and `tests/postgres_it_test.rs` in `adbc-vs-scan` covering driver load by the PostgreSQL entry point, statement execution, Arrow batch streaming across more than one batch, and coercion of every mapped type including the string-form `numeric` and a microsecond timestamp. Fetch the driver library through task 3.6's per-driver helper, which is this task's prerequisite alongside task 6.4's package row — there is no other route to a driver library outside BucketFS. Needs the `postgres` container only, never Exasol, and links the crate as an `rlib` so it does not depend on `cross-udf-build`.

### Phase 8: End-to-end tier

- [ ] 8.1 Grow `docker/postgres/init.sql` into the differential fixture and add local Exasol mirror tables seeded from identical values: character values differing only by case and only by accent, the ordering triple `B`/`a`/`C`, a trailing-space pair, `numeric(12,4)`, an unconstrained `numeric`, `character(8)`, `character varying(100)`, `text`, `timestamp`, `timestamptz`, `uuid`, `jsonb`, `bytea`, a nullable numeric column, two tables sharing a key with an unmatched row on each side, and a table whose every column is unmapped. Put the case and accent variants in the column the aggregation tests group by.
- [ ] 8.2 [expert] Differential PostgreSQL E2E tests in `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` behind the `exasol-e2e` feature, with its own shared setup module creating both a PostgreSQL and a MySQL virtual schema so task 8.3 has both: for projection, filter, aggregation, and join, assert through `EXPLAIN VIRTUAL` that the fragment reached PostgreSQL and through a row comparison against the mirror that it produced Exasol's answer. Cover the collation placements explicitly — `COLLATE "C"` present on an ordering comparison and on `MIN`/`MAX`, absent on equality, `LIKE`, the `GROUP BY` key and its select-list occurrence, and a `DISTINCT` operand — and assert the uncollated `GROUP BY` and `COUNT(DISTINCT)` return the mirror's group count. Assert a delegated `FULL OUTER JOIN` returns both unmatched rows with nulls; a select-list predicate over the grouped character column delegates and matches the mirror; `AVG` and `SUM` render in no generated statement and still return the mirror's values; a two-column `GROUP BY` is withdrawn; and no generated statement carries `ORDER BY`, `LIMIT`, or `OFFSET`.
- [ ] 8.3 Add the two-driver end-to-end test to `e2e_postgres_test.rs`, over the two virtual schemas task 8.2's setup created: assert both driver libraries are present in BucketFS at their distinct paths, and assert a query over each returns its own source's rows. Task 8.5 owns the capability half of the two-driver assertion and states its mechanism, so this task does not restate it. This is the test that would fail if package resolution or metadata discovery defaulted to one driver. Add `every_supported_driver_creates_a_virtual_schema` in the same file, asserting that `CREATE VIRTUAL SCHEMA` succeeds once per value `Driver::parse` accepts, enumerated from the identity rather than listed — so a third driver inherits the assertion. It replaces the earlier PostgreSQL-only "driver value is accepted" scenario, which named one technology inside a framework feature.
- [ ] 8.4 Add the PostgreSQL omission E2E test: `adapterNotes` records every unmapped column with its PostgreSQL type and reason under `sourceType`, the all-unmapped table is absent from `schemaMetadata`, and creation still succeeds. Add the adapter-side credential assertion in the same file: point the `CONNECTION` at a wrong password, and separately at a host and port nothing listens on, and assert that no error surfaced by `createVirtualSchema`, `refresh`, or `setProperties` contains the password or the libpq conninfo string, while each still names the host and the database.
- [ ] 8.5 Add `two_virtual_schemas_advertise_their_own_capability_sets` to `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs`, over the two virtual schemas task 8.2's setup module created — one `DRIVER = 'postgresql'`, one `DRIVER = 'mysql'`, both through the ordinary `CREATE VIRTUAL SCHEMA` path against the shipped `ADBC_ADAPTER`. Plan a `FULL OUTER JOIN` over each schema's own pair of joinable fixture tables and read the `PUSHDOWN_SQL` rows `EXPLAIN VIRTUAL` reports for it, the way `e2e_mysql_test.rs`'s `pushdown_statement` helper and `two_column_group_by_is_not_delegated` already read a delegation decision — reading every reported row rather than the first, because a withdrawn join leaves one pushed-down statement per scanned table. Assert the PostgreSQL schema's generated statement carries the delegated `FULL OUTER JOIN`, and that no statement generated for the MySQL schema carries a join at all, since Exasol withdraws a join the advertised set omits and performs it itself. What the two answers prove is the per-driver dispatch itself — `getCapabilities` resolving its dialect from `adapterNotes.driver`, live, through the shipped code path — which is strictly more than the presence of a JSON field. Add no probe adapter, no request-echo instrumentation to the shipped adapter, and no test-only code path through it. This is the standing regression test behind the plan's keystone precondition: gate 2.1 measures the request shape once, and a later loss of the identity on the `getCapabilities` request collapses this difference and MUST fail this test rather than be absorbed.

### Phase 9: Operational surface and mission

- [ ] 9.1 Add `test-e2e-postgres` to the `Makefile` — prereqs `cross-udf-build stack-up`, running `cargo test -p adbc-vs-adapter --features exasol-e2e --test e2e_postgres_test -- --test-threads=1` — as a second target beside `test-e2e` rather than folding both drivers into one binary. Two binaries keep each driver's shared setup independent, so a MySQL-only run provisions no PostgreSQL virtual schema and a failure names which driver's tier broke. Update `tests/make_targets_test.rs`.
- [ ] 9.2 Extend `.github/workflows/ci.yml`: run `make test-it-postgres` in `integration-tests` and the PostgreSQL end-to-end target in `e2e`, both as `make` invocations with no inline shell. Confirm the `e2e` job's disk cleanup still leaves room for a third image, and update `tests/workflow_test.rs` for the new target references.
- [ ] 9.3 Update `CLAUDE.md`'s dense bullets: the `postgres` service, its image tag, static IP `172.28.5.11`, and deliberate `en_US.utf8` locale; the `postgres-it` tier and what it does and does not cover; the per-technology `drivers/*.toml` files, the `build.rs` that generates the tables from them, and the rule that a new technology edits a TOML file *and* ships Rust in a release rather than being loaded at run time; that `crates/adbc-vs-scan/build.rs` and `crates/adbc-vs-scan/drivers/*.toml` are `cross-udf-build` prerequisites, so a TOML edit alone rebuilds the `.so` and no tier ever certifies a stale binary; that the build validates the `[package]` fields only and an unknown capability name is caught by the capability invariant test rather than by the build; each driver's `version_range` and the rule that the two ranges are independent; the versioned BucketFS install path; the `DRIVER_PATH` property and that it is the supported air-gapped route; that self-install verifies no checksum and no signature, so the download's TLS connection is its only integrity guarantee, and that the compiled-in digest release 0.2 checked is gone for MySQL too; the PostgreSQL credential path and its redaction requirement; the `public`-schema-only boundary. Delete the version-spelling-trap bullet if one was added, since range resolution removes it. Record the two consequences of task 1.2's `stack-up` edge in the stack-lifecycle and Benchmarking bullets: `make test-e2e` now starts a container it does not use, and `make bench` samples throughput on a host carrying a third database.
- [ ] 9.5 Record the spec library's framework/technology convention in `CLAUDE.md`'s **Spec governance** section, so a later planner extends the structure rather than re-deriving it. State: every feature is either framework or technology; a framework feature keeps the protocol's own vocabulary as its name; a technology feature is named `<technology>-<concern>` and sits flat inside one of the three existing domains. State the reason nesting is not used — `speq` represents exactly two path levels and drops a nested feature from `feature list` silently rather than rejecting it, so a nested spec would be invisible to search and validation. State that a new technology adds `<technology>-pushdown`, `<technology>-collation`, `<technology>-type-mapping`, and `<technology>-scan`, and adds no provisioning feature, because provisioning's technology-dependence is data in `drivers/<technology>.toml` rather than behavior. State that `driver-provisioning` holds three framework features — `driver-self-install`, `driver-version-resolution`, `driver-path-override` — split along framework-internal lines rather than per technology, per decision `[26]`. Keep the existing rule that the live domains are `vs-adapter/`, `scan-udf/`, and `driver-provisioning/`, and that no `e2e/`, `toolchain/`, `ci/`, or `licensing/` group may be created. Point at decisions `[25]` and `[26]` for the reasoning rather than restating it. Two bullets this task would otherwise write are **already in `CLAUDE.md`**, added when decision `[26]` was made: the `vs-adapter` feature-count acceptance and the `driver-provisioning` three-feature shape. Verify both still read true against the merged library and extend them; do not duplicate them.
- [ ] 9.4 Update `specs/mission.md`: PostgreSQL moves from the aspirational v1 driver set to a shipped driver; the MySQL 8.0.17 source-server constraint becomes one driver's precondition beside PostgreSQL's UTF-8 encoding requirement; the `SqlDialect` row in the tech-stack table names both dialects; note that the credential-by-separate-option rule holds for MySQL and not for PostgreSQL.

### Phase 10: Review and verification

- [ ] 10.1 Code review of all changed files
- [ ] 10.2 Apply standard review fixes (if any)
- [ ] 10.3 Apply expert review fixes (if any)
- [ ] 10.4 Run the automated checklist (build, test, artifact, integration both drivers, e2e both drivers, lint, format, deny)
- [ ] 10.5 Scenario coverage audit
- [ ] 10.6 Manual verification steps
- [ ] 10.7 Confirm the record-time Background and description rewrites are still owed, and hand them to whoever runs `/speq:record`. Re-derive § Verification § Recording notes' eleven-feature list from the deltas rather than trusting the table: a feature is on the list when `specs/<domain>/<feature>/spec.md` exists and the delta's description line or `## Background` differs from it. Run that section's post-record check against the unmerged library; it MUST report `FAIL` for all eleven and exit 1, because the merge has not happened yet — a `ok` row means the list drifted from the deltas and the table needs correcting. Record in the verification report that `/speq:record` owes the manual head replacement, naming the section and the eleven features, so the obligation reaches the recorder rather than sitting only in plan.md.
- [ ] 10.8 Generate verification report

## Parallelization

| Parallel Group | Tasks |
|----------------|-------|
| Group 0 | 2.2a → 2.2b |
| Group A | 1.1, 1.2 |
| Group B | 2.1, 2.4 |
| Group C | 2.3 |
| Group D | 3.1 → 3.2 → 3.3, and 3.5 after 3.1; 3.4 after 3.2; 3.7 after 3.1; 3.8 after 3.1. Task 3.6 moves to Group G, after 6.1 |
| Group E | 4.1 → 4.2 → 4.3 → 4.4 → 4.5 |
| Group F | 5.1, 5.2 in parallel; then 5.3, 5.4 |
| Group G | 6.1 → 6.6 → 6.2 → 6.3 → 6.4 → 6.5 (all three `driver-provisioning` features) |
| Group H | 7.1, 7.2 |
| Group I | 8.1 → 8.2 → 8.3, 8.4, 8.5 |
| Group J | 9.1, 9.2, 9.3, 9.4, 9.5 |
| Group K | 10.1 → 10.8 |

Sequential dependencies:

- Group 0 → every other group. The driver-load gate blocks the whole plan, and its failure is fixed in `language-container-rs` and returns through the release chain, so it runs first. It needs no PostgreSQL container: 2.2a needs HTTPS egress and BucketFS, and 2.2b needs `make cross-udf-build`, `make stack-up`, and the installed SLC. None of those is Group A's work
- Group 0 → Group B (gate 2.4 reads the `MANIFEST` of the package gate 2.2a fetched)
- Group A → Group C (gate 2.3 is the only gate that needs the PostgreSQL container; gate 2.1 needs only Exasol and runs in Group B alongside it)
- Group B → Group D (gate 2.1 decides whether per-dialect capability sets are legal, so it precedes the resolution work). Gate 2.4 feeds only the `library` value task 6.4's TOML row states; it gates nothing else, because no task verifies a digest or a signature
- Group C → Groups E and H (gate 2.3 grounds every PostgreSQL rendering rule)
- Group D → Groups E, F, G, H — **the plan's main fan-out.** The driver-identity refactor and the TOML mechanism are done once, then four workstreams run concurrently on largely disjoint files: the dialect renderer (`postgres_dialect.rs`), the metadata source and mapping (`postgres_source.rs`, `create_schema.rs`), provisioning (`registry_index.rs`, `provisioning.rs`, `drivers/postgresql.toml`, `provisioning_test.rs`), and the scan coercion and integration tier (`coerce.rs`, `postgres_it_test.rs`). Group G is now the longest chain rather than the shortest — range resolution, the versioned path, and `DRIVER_PATH` are three sequential changes to one module where release 0.2 had a single package row to extend — so it is the critical path and should start first among the four
- Group G's `DRIVER_PATH` task (6.3) touches `create_schema.rs`, which Group F's task 3.5 work also touches. Sequence 6.3 after Group F rather than running them concurrently, or accept a merge in one file
- Group G stays one sequential chain despite now serving three spec features. The three features cut the *specification* into three owners, not the code: `registry_index.rs`, `provisioning.rs`, and `create_schema.rs` are the same three files in the same order, and tasks 6.1, 6.2, and 6.3 remain three sequential edits to overlapping code. Decision `[26]` split the specs where the questions differ, which is not where the file boundaries fall, so it adds no parallelism and removes none
- Task 6.1 → task 3.6, a new edge the range creates: the integration tier can no longer construct a package URL from a pinned version, so its helper needs range resolution before it can resolve anything. Task 3.6 therefore moves out of Group D's fan-out and lands after Group G's first task
- Task 6.4 → task 7.2, the one remaining edge crossing the fan-out: the integration tier fetches the PostgreSQL package through the row 6.4 adds, using task 3.6's helper. Task 7.1 needs nothing from Group G
- Groups E, F, G, H → Group I (the end-to-end tier exercises all four)
- Task 8.2 → task 8.5. The keystone capability test needs two fully working virtual schemas, one per driver, which is exactly what task 8.2's shared setup module creates; it observes them through the shipped adapter and adds no capture mechanism of its own, so it can start no earlier than that setup exists
- Group I → Group J (a CI job may reference only a target whose test exists)
- Group J → Group K

The foundational work that both MySQL and PostgreSQL build on — the driver identity, the three tables, the resolution points, and the per-dialect invariant test — is Group D alone and is never repeated per driver. Each of Groups E, F, G, and H adds one row or one implementation to a seam Group D established.

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| Constant | `create_schema.rs::SUPPORTED_DRIVER` | Replaced by the `Driver` enum's own supported set |
| Function | `create_schema.rs::validate_driver` | Replaced by `Driver::parse`, whose error lists every supported value |
| Re-export | `provisioning.rs`'s `pub use adbc_vs_scan::driver::{DRIVER_LIBRARY, DRIVER_VERSION}` | Bare globals cannot describe two drivers; replaced by `DriverPackage` fields read through the identity |
| Constant | `adbc-vs-scan::driver`'s `DRIVER_LIBRARY`, `REGISTRY_DRIVER_PATH` | Become fields of the MySQL `DriverPackage` row, generated from `drivers/mysql.toml` |
| Constant | `adbc-vs-scan::driver`'s `DRIVER_VERSION` | Removed outright, not relocated. A range replaces it and the resolved version is a run-time value, not a compiled-in one |
| Constant | `adbc-vs-scan::driver`'s `PACKAGE_SHA256` | Removed outright, nothing replacing it. A digest cannot describe a version the range has yet to resolve to, and decision `[22]` adds no substitute anchor |
| Field | `provisioning.rs`'s `Registry` `sha256` field | No caller supplies a digest once `PACKAGE_SHA256` is gone |
| Function | `provisioning.rs::verify_digest` and its call site | Self-install verifies no checksum and no signature; the registry client's TLS certificate verification is the only integrity guarantee |
| Test | The digest-mismatch provisioning test and the stub registry's digest field | Removed with the mechanism they assert |
| Expression | `registry_index::package_url`'s `block.version == version` exact string match | Replaced by semantic comparison over a range, which also removes the `v`-spelling hazard it created |
| Function | `mysql_it_test.rs::package_url` and its convention-based URL assembly | The tier resolves through the index like the product; the constants it interpolated no longer exist |
| Fallback | The "place `<library>` at `<fixed path>` by hand" clause in `RegistryUnreachable` and `RegistryRejected` | Superseded by `DRIVER_PATH`; the path is no longer fixed once it carries a version, per decision `[20]` |
| Constant | `connection.rs::DRIVER_ENTRY_POINT` | Becomes a per-driver field; the scan reads it from the identity |
| Import | `dispatch.rs`'s `use crate::mysql_dialect::MySqlDialect` | No call site names a concrete dialect after resolution lands |
| Method | `metadata.rs::AdbcMetadataSource::discover_collations` as an inherent method | Becomes the MySQL implementation of the trait's precondition check |
| Call sites | `create_schema.rs`'s direct `type_mapping::map_column` calls | Mapping moves behind `MetadataSource`, which returns mapped columns |
| Field | `adapter_notes.rs::Omission::mysql_type` | Renamed to `source_type`; the old wire name survives only as a deserialization alias |
| Type alias | `MysqlConnection`, `open_mysql_connection` | Renamed to source-neutral names taking the driver identity |
| Tests | `create_schema_tests.rs` mapping assertions | Move to the per-driver mapping test modules |

## Verification

### Recording notes: apply the Background and description rewrites by hand

**`/speq:record` MUST hand-apply this plan's Background and feature-description rewrites. The marker merge silently drops them.** `/speq:spec-merge`'s marker table defines exactly three actions, and all three are scoped to a scenario: `DELTA:NEW` appends a scenario, `DELTA:CHANGED` replaces a scenario of the same name, `DELTA:REMOVED` deletes one. Neither the feature-description line nor the `## Background` block has a marker verb. A `DELTA:CHANGED` block wrapping a Background is therefore discarded with no warning, and `speq feature validate` still passes — a dropped Background is not invalid, it just never lands. Round 6 confirmed this against the `speq record` CLI in a throwaway project rather than inferring it from the table.

Eleven of this plan's twenty-two deltas target a feature the library already holds. **Every one of the eleven rewrites its `## Background`, and eight also rewrite the feature-description line.** None of those rewrites merges on its own. The eleven brand-new features are unaffected: no library file exists, so the merge copies each delta whole and its Background lands intact. That asymmetry is what hides the defect — the new MySQL and PostgreSQL technology features read correctly after recording, and only the framework features they defer to keep saying MySQL.

**Skipping the step below recreates the exact defect this restructuring removes.** Each framework file would keep its MySQL-only Background and description standing over now dialect-neutral scenarios. `vs-adapter/pushdown-filter` would still declare `COLLATE utf8mb4_0900_as_cs` "the collation of every equality question this release delegates" while `vs-adapter/postgresql-collation` requires no suffix there. That is round 5's `[REQUIREMENT_CONFLICT]` with its halves swapped, which is why this is a checked step and not a note.

#### The manual step

For each feature in the table below, before stripping DELTA markers from the merged file:

1. Replace the library file's feature-description line — the first non-blank line under `# Feature:`, line 3 in all eleven files — with the delta's.
2. Replace the library file's entire `## Background` block — every line from `## Background` down to the next `## ` heading — with the delta's, DELTA markers removed.
3. Merge `## Scenarios` the ordinary marker-driven way. Nothing about the scenario merge changes.

Steps 1 and 2 together replace everything above `## Scenarios`, because `## Scenarios` is the heading that follows `## Background` in all eleven library files and all eleven deltas. Each delta wraps its whole Background in exactly one `<!-- DELTA:CHANGED -->` … `<!-- /DELTA:CHANGED -->` pair with no bullet outside it, so step 2 is a whole-block copy rather than a per-bullet reconciliation.

| # | Feature | Delta holding the final text | Background bullets library → delta | Description line |
|---|---------|------------------------------|------------------------------------|------------------|
| 1 | `specs/vs-adapter/pushdown-dialect/spec.md` | `vs-adapter/pushdown-dialect/spec.md` | 17 → 21 | unchanged |
| 2 | `specs/vs-adapter/pushdown-projection/spec.md` | `vs-adapter/pushdown-projection/spec.md` | 8 → 9 | rewritten |
| 3 | `specs/vs-adapter/pushdown-filter/spec.md` | `vs-adapter/pushdown-filter/spec.md` | 12 → 12 | rewritten |
| 4 | `specs/vs-adapter/pushdown-aggregation/spec.md` | `vs-adapter/pushdown-aggregation/spec.md` | 23 → 19 | rewritten |
| 5 | `specs/vs-adapter/pushdown-join/spec.md` | `vs-adapter/pushdown-join/spec.md` | 8 → 10 | rewritten |
| 6 | `specs/vs-adapter/type-mapping/spec.md` | `vs-adapter/type-mapping/spec.md` | 8 → 8 | rewritten |
| 7 | `specs/vs-adapter/create-virtual-schema/spec.md` | `vs-adapter/create-virtual-schema/spec.md` | 10 → 11 | rewritten |
| 8 | `specs/vs-adapter/result-ordering/spec.md` | `vs-adapter/result-ordering/spec.md` | 7 → 9 | unchanged |
| 9 | `specs/vs-adapter/adapter-request-dispatch/spec.md` | `vs-adapter/adapter-request-dispatch/spec.md` | 5 → 6 | unchanged |
| 10 | `specs/scan-udf/arrow-scan/spec.md` | `scan-udf/arrow-scan/spec.md` | 9 → 12 | rewritten |
| 11 | `specs/driver-provisioning/driver-self-install/spec.md` | `driver-provisioning/driver-self-install/spec.md` | 11 → 15 | rewritten |

Delta paths are relative to this plan directory. Background text is not duplicated here: each delta's `## Background` block is the final text, and duplicating roughly 130 bullets would create a second copy to keep in step. The eight rewritten description lines are one line each, so they are quoted in full below and the recorder needs no second file open for them.

| Feature | Final description line |
|---------|------------------------|
| `vs-adapter/pushdown-projection` | Reports the query fragments this release can delegate to the source, and rewrites the projection fragment into a `SELECT` the scan UDF executes at that source. |
| `vs-adapter/pushdown-filter` | Delegates a query's `WHERE` clause to the source, rendering each predicate so the source selects exactly the rows Exasol would have selected itself. |
| `vs-adapter/pushdown-aggregation` | Delegates a single-group or `GROUP BY` aggregation to the source, declaring for each aggregate the Exasol type Exasol's own compiler computed for it. |
| `vs-adapter/pushdown-join` | Delegates an equi-join between two virtual tables of one virtual schema to the source, so the source joins the rows instead of shipping both tables to Exasol. |
| `vs-adapter/type-mapping` | States the contract every source-to-Exasol column mapping obeys: the closed set of Exasol types a mapped column may declare, how nullability carries, and how a column the mapping cannot represent without loss is omitted. |
| `vs-adapter/create-virtual-schema` | Turns a `CREATE VIRTUAL SCHEMA` statement into a browsable set of Exasol virtual tables backed by an ADBC-fronted source database, provisioning that source's ADBC driver on the way. |
| `scan-udf/arrow-scan` | Executes the pushed-down source statement through the ADBC driver the virtual schema names and streams the driver's Arrow record batches into Exasol without a row-by-row conversion step. |
| `driver-provisioning/driver-self-install` | Installs the ADBC driver a virtual schema names into BucketFS by downloading it from the public driver registry, so no operator packaging step stands between installing adbc-vs and querying that source. |

The three features whose description line is unchanged — `pushdown-dialect`, `result-ordering`, `adapter-request-dispatch` — still need step 2. Their Backgrounds are all rewritten.

#### The post-record check

Run this from the repo root under `bash`, after `/speq:record` merges. It asserts the post-record state directly rather than trusting that step 1 and step 2 were performed.

```bash
# DELTA_ROOT is specs/_plans/add-postgresql-support before /speq:record archives
# the plan, and specs/_recorded/NNN-add-postgresql-support after.
DELTA_ROOT=specs/_plans/add-postgresql-support
head_of() { sed -n '1,/^## Scenarios$/p' "$1" | sed '$d' | grep -v '^<!-- /\?DELTA:'; }
fail=0
for f in vs-adapter/pushdown-dialect vs-adapter/pushdown-projection \
         vs-adapter/pushdown-filter vs-adapter/pushdown-aggregation \
         vs-adapter/pushdown-join vs-adapter/type-mapping \
         vs-adapter/create-virtual-schema vs-adapter/result-ordering \
         vs-adapter/adapter-request-dispatch scan-udf/arrow-scan \
         driver-provisioning/driver-self-install; do
  if diff -q <(head_of "specs/$f/spec.md") <(head_of "$DELTA_ROOT/$f/spec.md") >/dev/null
  then echo "ok    $f"; else echo "FAIL  $f"; fail=1; fi
done
exit $fail
```

The check compares each library file's description line and Background against the delta's, marker-stripped, and exits non-zero on any difference. Both halves were verified rather than reasoned about:

- **It detects the defect.** Run against the library as it stands today, before any merge, it reports `FAIL` for all eleven features and exits 1.
- **The manual step satisfies it.** Applied to copies of the eleven library files — each head replaced by its delta's, per steps 1 and 2 — the check reports `ok` for all eleven and exits 0, and `speq feature validate` reports no new error and no new warning on the result.

### Scenario Coverage

Scenarios are grouped by the feature that owns them after decision `[25]`. A scenario a technology feature now owns is listed under that feature, never under the framework feature it left. A framework scenario that must hold for every dialect maps to one test per dialect.

#### Framework — vs-adapter

`vs-adapter/driver-selection`

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| Each supported DRIVER value resolves to its own dialect and package | Unit | `crates/adbc-vs-adapter/src/driver_selection_tests.rs` | `each_driver_resolves_to_its_own_tables` |
| DRIVER value is matched case-insensitively and recorded in canonical form | Unit | `crates/adbc-vs-adapter/src/driver_selection_tests.rs` | `driver_value_case_is_folded_to_canonical` |
| Unsupported driver value names every supported value | Integration | `crates/adbc-vs-adapter/tests/dispatch_test.rs` | `unsupported_driver_names_every_supported_value` |
| getCapabilities resolves the driver from the request | Integration | `crates/adbc-vs-adapter/tests/dispatch_test.rs` | `capabilities_follow_the_driver_in_adapter_notes` |
| Each virtual schema is served the capability set its own driver advertises | Integration | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `two_virtual_schemas_advertise_their_own_capability_sets` |
| getCapabilities without adapter notes fails rather than guessing a driver | Integration | `crates/adbc-vs-adapter/tests/dispatch_test.rs` | `capabilities_without_adapter_notes_fails_named` |
| Pushdown and capabilities resolve the driver from one field | Integration | `crates/adbc-vs-adapter/tests/dispatch_test.rs` | `capabilities_and_pushdown_resolve_one_field` |
| Connection address scheme must match the named driver | Integration | `crates/adbc-vs-adapter/tests/dispatch_test.rs` | `connection_scheme_must_match_named_driver` |
| Scan specification carries the driver identity | Unit | `crates/adbc-vs-adapter/src/scan_spec_tests.rs` | `scan_spec_carries_driver_identity` |
| Each technology's package row and capability list come from its own TOML file | Unit | `crates/adbc-vs-scan/src/driver_tests.rs` | `generated_tables_match_the_checked_in_toml` |
| A malformed or incomplete driver TOML file fails the build | Integration | `crates/adbc-vs-scan/tests/driver_toml_test.rs` | `malformed_driver_toml_fails_the_build` |
| A capability name no dialect renders fails the invariant test rather than the build | Unit | `crates/adbc-vs-adapter/src/capabilities_tests.rs` | `every_advertised_capability_renders_per_dialect` |
| Adding a technology requires code, not only a TOML file | Unit | `crates/adbc-vs-adapter/src/driver_selection_tests.rs` | `supported_set_is_fixed_at_build_time` |
| Every driver identity has a complete set of tables | Unit | `crates/adbc-vs-adapter/src/driver_selection_tests.rs` | `every_identity_yields_every_table` |

`vs-adapter/pushdown-dialect`

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| Every advertised capability has a rendering path | Unit | `crates/adbc-vs-adapter/src/capabilities_tests.rs` | `every_advertised_capability_renders_per_dialect` |
| The two dialects' capability sets differ only where their SQL differs | Unit | `crates/adbc-vs-adapter/src/capabilities_tests.rs` | `dialect_sets_differ_only_in_full_outer_join` |
| Unrepresentable request fragment fails rather than returning wrong rows | Integration | `crates/adbc-vs-adapter/tests/dispatch_test.rs` | `unrenderable_fragment_named_in_error` |
| Comparison operands are read by position, not by assumed role | Unit | `crates/adbc-vs-adapter/src/mysql_dialect_tests.rs`, `crates/adbc-vs-adapter/src/postgres_dialect_tests.rs` | `swapped_comparison_keeps_operand_order`, `comparison_operands_keep_request_order` |
| Generated statement is deterministic for a given request | Unit | `crates/adbc-vs-adapter/src/capabilities_tests.rs` | `same_request_renders_identically_per_dialect` |

`vs-adapter/pushdown-projection`

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| Absent select list projects every mapped column | Unit | `crates/adbc-vs-adapter/src/mysql_dialect_tests.rs`, `crates/adbc-vs-adapter/src/postgres_dialect_tests.rs` | `absent_select_list_projects_all`, `absent_select_list_projects_every_column` |
| Pushdown response carries no credential | Integration | `crates/adbc-vs-adapter/tests/dispatch_test.rs` | `pushdown_response_carries_no_credential` |

`vs-adapter/pushdown-filter`

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| Numeric comparison is rendered as a WHERE clause | Unit | `crates/adbc-vs-adapter/src/mysql_dialect_tests.rs`, `crates/adbc-vs-adapter/src/postgres_dialect_tests.rs` | `each_comparison_operator_renders_its_mysql_counterpart`, `standard_predicate_forms_render` |
| LIKE renders its pattern and its escape character | Unit | `crates/adbc-vs-adapter/src/mysql_dialect_tests.rs`, `crates/adbc-vs-adapter/src/postgres_dialect_tests.rs` | `like_renders_pattern_and_escape`, `like_renders_its_escape_clause` |
| BETWEEN and IN keep their operand order | Unit | `crates/adbc-vs-adapter/src/mysql_dialect_tests.rs`, `crates/adbc-vs-adapter/src/postgres_dialect_tests.rs` | `between_and_in_keep_operand_order`, `between_and_in_keep_operand_order` |
| Predicate in select-list position renders as a projected boolean | Unit | `crates/adbc-vs-adapter/src/mysql_dialect_tests.rs`, `crates/adbc-vs-adapter/src/postgres_dialect_tests.rs` | `a_select_list_predicate_renders_where_the_query_groups_by_nothing`, `select_list_predicate_renders_as_projected_boolean` |

`vs-adapter/pushdown-aggregation`

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| Predicate over an aggregate is delegated as a BOOLEAN column | Unit | `crates/adbc-vs-adapter/src/mysql_dialect_tests.rs`, `crates/adbc-vs-adapter/src/postgres_dialect_tests.rs` | `a_select_list_predicate_over_an_aggregate_renders`, `select_list_predicate_over_an_aggregate_renders` |
| AVG and SUM are computed by Exasol rather than delegated | Unit + Integration | `crates/adbc-vs-adapter/src/capabilities_tests.rs`, `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `avg_and_sum_are_withheld_per_dialect`, `avg_and_sum_match_local_exasol_result` |
| Aggregate value that overflows its declared type fails rather than truncating | Unit | `crates/adbc-vs-scan/src/coerce_tests.rs` | `overflowing_aggregate_fails_named` |

`vs-adapter/pushdown-join`

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| Each join type renders its counterpart | Unit | `crates/adbc-vs-adapter/src/mysql_dialect_tests.rs`, `crates/adbc-vs-adapter/src/postgres_dialect_tests.rs` | `each_join_type_renders_its_mysql_keyword_in_the_requests_operand_order`, `each_join_type_renders` |
| Unsupported join type fails with a named error | Unit | `crates/adbc-vs-adapter/src/mysql_dialect_tests.rs`, `crates/adbc-vs-adapter/src/postgres_dialect_tests.rs` | `an_unrenderable_join_type_fails_naming_the_value_it_received`, `unsupported_join_type_named_per_dialect` |
| Inner join of two virtual tables renders one statement | Unit | `crates/adbc-vs-adapter/src/mysql_dialect_tests.rs`, `crates/adbc-vs-adapter/src/postgres_dialect_tests.rs` | `a_join_renders_one_statement_with_every_table_aliased_and_every_column_qualified`, `inner_join_renders_one_statement` |
| Nested join renders as a nested join | Unit | `crates/adbc-vs-adapter/src/mysql_dialect_tests.rs`, `crates/adbc-vs-adapter/src/postgres_dialect_tests.rs` | `a_nested_join_renders_the_inner_join_parenthesized_inside_the_outer`, `nested_join_renders_nested` |
| Outer join emits nulls for the unmatched side | Integration | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `outer_join_emits_nulls_for_unmatched_side` |
| Join composes with a delegated filter and aggregation | Unit | `crates/adbc-vs-adapter/src/mysql_dialect_tests.rs`, `crates/adbc-vs-adapter/src/postgres_dialect_tests.rs` | `join_composes_with_filter_and_group_by`, `join_composes_with_filter_and_grouping` |
| Every joined table is aliased and every column is qualified | Unit | `crates/adbc-vs-adapter/src/mysql_dialect_tests.rs`, `crates/adbc-vs-adapter/src/postgres_dialect_tests.rs` | `every_column_position_in_a_join_carries_its_tables_alias`, `joined_tables_are_aliased_and_qualified` |
| Column that matches no table in the from tree fails with a named error | Unit | `crates/adbc-vs-adapter/src/mysql_dialect_tests.rs`, `crates/adbc-vs-adapter/src/postgres_dialect_tests.rs` | `a_column_naming_no_table_in_the_from_tree_fails_naming_the_column_and_the_reference`, `unresolvable_column_named_in_error` |

`vs-adapter/type-mapping`

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| DECIMAL beyond Exasol's maximum precision is unmapped | Unit | `crates/adbc-vs-adapter/src/mysql_type_mapping_tests.rs`, `crates/adbc-vs-adapter/src/postgres_type_mapping_tests.rs` | `oversize_decimal_unmapped`, `oversize_numeric_is_unmapped` |
| Column nullability carries through unchanged | Unit | `crates/adbc-vs-adapter/src/mysql_type_mapping_tests.rs`, `crates/adbc-vs-adapter/src/postgres_type_mapping_tests.rs` | `nullability_carries_through`, `nullability_carries_through` |
| Each technology states its own mapping table | Unit | `crates/adbc-vs-adapter/src/metadata_source_tests.rs` | `each_source_maps_its_own_catalog_row` |
| A mapped column declares only an Exasol type this release produces | Unit | `crates/adbc-vs-adapter/src/metadata_source_tests.rs` | `every_mapped_type_is_in_the_declarable_set` |
| Omission records the source type under a source-neutral field | Unit | `crates/adbc-vs-adapter/src/adapter_notes_tests.rs` | `omission_source_type_accepts_legacy_alias` |

`vs-adapter/create-virtual-schema`

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| Creating a virtual schema exposes the source tables | Integration | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `creating_virtual_schema_exposes_source_tables` |
| Source identifiers keep their original case | Integration | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `source_identifiers_keep_their_case` |
| Views are exposed alongside base tables | Integration | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `views_are_exposed_alongside_base_tables` |
| Missing required property fails with a named error | Integration + Unit | `crates/adbc-vs-adapter/tests/dispatch_test.rs`, `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs`, `crates/adbc-vs-adapter/src/create_schema_tests.rs` | `missing_property_named_in_error`, `driver_path_is_accepted_and_bypasses_provisioning`, `bucketfs_connection_required_without_driver_path` |
| Unsupported driver value fails with a named error | Integration | `crates/adbc-vs-adapter/tests/dispatch_test.rs` | `unsupported_driver_value_fails_named` |
| Every supported driver value creates a virtual schema | Integration | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `every_supported_driver_creates_a_virtual_schema` |
| DRIVER_PATH is accepted and bypasses driver provisioning | Integration | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `driver_path_is_accepted_and_bypasses_provisioning` |
| DRIVER_PATH naming no readable file fails with a named error | Integration | `crates/adbc-vs-adapter/tests/dispatch_test.rs` | `driver_path_missing_file_names_property_and_path` |
| Connection address without a database fails with a named error | Unit | `crates/adbc-vs-adapter/src/create_schema_tests.rs` | `address_without_database_fails_for_each_scheme` |
| Adapter notes record the state pushdown needs and no credential | Integration | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `adapter_notes_record_state_and_no_credential` |
| Columns of unmappable type are omitted and recorded | Integration | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `unmappable_columns_are_omitted_and_recorded` |
| Table with no mappable column is omitted | Integration | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `table_with_no_mappable_column_is_omitted` |
| Empty source database yields an empty table list | Unit | `crates/adbc-vs-adapter/src/create_schema_tests.rs` | `empty_namespace_yields_empty_table_list` |
| Unreachable source fails creation with a diagnosable message | Integration | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `unreachable_source_names_the_address` |

`vs-adapter/result-ordering`

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| Capability set withholds ordering and limiting | Unit | `crates/adbc-vs-adapter/src/capabilities_tests.rs` | `every_dialect_withholds_ordering` |
| Pushed-down statement carries no ordering or limiting clause | Integration | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `generated_statement_carries_no_ordering` |
| Request carrying an order-by or limit element fails rather than dropping it | Unit | `crates/adbc-vs-adapter/src/postgres_dialect_tests.rs` | `order_by_and_limit_elements_fail_named` |

`vs-adapter/adapter-request-dispatch`

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| Refresh and setProperties re-run metadata discovery | Integration | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `refresh_rediscovers_through_the_resolved_source` |
| Handler failure surfaces to the querying session | Integration | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `handler_failure_names_the_driver_failure` |
| Dropping a virtual schema releases no external resource | Integration | `crates/adbc-vs-adapter/tests/dispatch_test.rs` | `drop_releases_no_driver_library` |

#### Technology — vs-adapter

`vs-adapter/mysql-pushdown`

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| Capability set advertises the fragments this dialect renders | Unit | `crates/adbc-vs-adapter/src/capabilities_tests.rs` | `mysql_capability_set_is_exact` |
| Filter capabilities cover the predicates and literals this release renders | Unit | `crates/adbc-vs-adapter/src/capabilities_tests.rs` | `filter_capabilities_match_per_dialect` |
| Aggregation capabilities cover single-group and column grouping | Unit | `crates/adbc-vs-adapter/src/capabilities_tests.rs` | `aggregation_capabilities_match_per_dialect` |
| Join capabilities cover inner and one-sided outer joins only | Unit | `crates/adbc-vs-adapter/src/capabilities_tests.rs` | `mysql_join_capabilities_are_exact` |
| Backtick in an identifier is escaped | Unit | `crates/adbc-vs-adapter/src/mysql_dialect_tests.rs` | `backtick_is_doubled` |
| String literal is rendered as a hex literal | Unit | `crates/adbc-vs-adapter/src/mysql_dialect_tests.rs` | `string_literal_renders_as_hex`, `like_renders_pattern_and_escape` |
| Temporal literals are rendered as typed literals | Unit | `crates/adbc-vs-adapter/src/mysql_dialect_tests.rs` | `temporal_literals_render_typed` |
| Projection pushdown reaches MySQL rather than Exasol | Integration | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` | `projection_reaches_mysql` |
| Filter pushdown reaches MySQL rather than Exasol | Integration | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` | `filter_matches_local_exasol_result` |
| Aggregation pushdown reaches MySQL rather than Exasol | Integration | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` | `aggregation_matches_local_exasol_result` |

`vs-adapter/mysql-collation`

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| Character comparison reproduces Exasol's case and accent sensitivity | Integration | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` | `filter_matches_local_exasol_result` |
| A LIKE operand, an IN comparison, and a select-list predicate carry the comparison collation | Unit | `crates/adbc-vs-adapter/src/mysql_dialect_tests.rs` | `like_renders_pattern_and_escape`, `between_and_in_keep_operand_order`, `a_select_list_predicate_renders_where_the_query_groups_by_nothing` |
| Grouped character columns carry the comparison collation in both the grouping list and the select list | Unit | `crates/adbc-vs-adapter/src/mysql_dialect_tests.rs` | `grouped_character_column_carries_the_comparison_collation_in_both_clauses` |
| Only a grouped character column carries the grouping collation | Unit | `crates/adbc-vs-adapter/src/mysql_dialect_tests.rs` | `numeric_grouping_key_carries_no_collation`, `ungrouped_projection_carries_no_collation` |
| A DISTINCT aggregate over a character column carries the comparison collation | Unit | `crates/adbc-vs-adapter/src/mysql_dialect_tests.rs` | `distinct_character_operand_carries_the_comparison_collation` |
| Delegated grouping and DISTINCT over case-only variants return Exasol's groups | Integration | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` | `grouping_over_character_variants_matches_local_exasol_result` |
| MIN and MAX over a character column reproduce Exasol's code-point order | Integration | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` | `aggregation_matches_local_exasol_result` |
| Select-list predicate over the grouped character column fails with a named error | Unit | `crates/adbc-vs-adapter/src/mysql_dialect_tests.rs` | `grouped_character_predicate_fails_mysql_only` |
| Select-list predicate shapes that stay delegated | Unit | `crates/adbc-vs-adapter/src/mysql_dialect_tests.rs` | `a_select_list_predicate_over_a_numeric_grouping_key_renders`, `a_select_list_predicate_over_an_aggregate_renders`, `a_select_list_predicate_renders_where_the_query_groups_by_nothing` |
| Grouped-column select-list predicate fails before reaching MySQL | Integration | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` | `select_list_predicate_over_grouped_character_column_fails` |

`vs-adapter/mysql-type-mapping`

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| Signed integer types map to DECIMAL with the matching digit count | Unit | `crates/adbc-vs-adapter/src/mysql_type_mapping_tests.rs` | `signed_integers_map_to_decimal` |
| DECIMAL preserves the source precision and scale | Unit | `crates/adbc-vs-adapter/src/mysql_type_mapping_tests.rs` | `decimal_preserves_precision_and_scale` |
| Character types preserve the declared length and use UTF8 | Unit | `crates/adbc-vs-adapter/src/mysql_type_mapping_tests.rs` | `character_types_preserve_length` |
| Character column of a non-utf8mb4 character set is unmapped | Unit | `crates/adbc-vs-adapter/src/mysql_type_mapping_tests.rs` | `non_utf8mb4_character_columns_unmapped` |
| Temporal and floating-point types map to their Exasol counterparts | Unit | `crates/adbc-vs-adapter/src/mysql_type_mapping_tests.rs` | `temporal_and_double_map` |
| Unsigned integer columns are unmapped | Unit | `crates/adbc-vs-adapter/src/mysql_type_mapping_tests.rs` | `unsigned_integers_unmapped` |
| Non-core MySQL types are unmapped | Unit | `crates/adbc-vs-adapter/src/mysql_type_mapping_tests.rs` | `non_core_types_unmapped` |
| Source server without the required collations is rejected at creation | Unit | `crates/adbc-vs-adapter/src/create_schema_tests.rs` | `mysql_collation_check_binds_mysql_only` |

`vs-adapter/postgresql-pushdown`

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| Capability set advertises the fragments this dialect renders | Unit | `crates/adbc-vs-adapter/src/capabilities_tests.rs` | `postgres_capability_set_is_exact` |
| Join capabilities add the full outer join | Unit | `crates/adbc-vs-adapter/src/capabilities_tests.rs` | `postgres_join_capabilities_add_full_outer` |
| Double quote in an identifier is escaped | Unit | `crates/adbc-vs-adapter/src/postgres_dialect_tests.rs` | `double_quote_in_identifier_is_doubled` |
| String literal is rendered as an escape-string constant | Unit | `crates/adbc-vs-adapter/src/postgres_dialect_tests.rs` | `string_literal_is_escape_string_constant` |
| LIKE, IS NULL, BETWEEN, and IN render in their standard forms | Unit | `crates/adbc-vs-adapter/src/postgres_dialect_tests.rs` | `standard_predicate_forms_render` |
| Delegated full outer join is rendered and executed | Integration | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `full_outer_join_matches_local_exasol_result` |
| Projection pushdown reaches PostgreSQL rather than Exasol | Integration | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `projection_reaches_postgres` |
| Filter pushdown reaches PostgreSQL rather than Exasol | Integration | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `filter_reaches_postgres` |
| Aggregation pushdown reaches PostgreSQL rather than Exasol | Integration | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `aggregation_matches_local_exasol_result` |

`vs-adapter/postgresql-collation`

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| An ordering comparison is collated and equality is left bare | Unit | `crates/adbc-vs-adapter/src/postgres_dialect_tests.rs` | `collation_placement_matrix` |
| Character comparison reproduces Exasol's case, accent, and code-point order | Integration | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `character_comparison_matches_local_exasol_result` |
| MIN and MAX over a character column render the C collation | Integration | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `min_max_reproduce_code_point_order` |
| A grouping key and a DISTINCT operand render without a collation | Unit | `crates/adbc-vs-adapter/src/postgres_dialect_tests.rs` | `grouping_and_distinct_render_uncollated` |
| Delegated grouping and DISTINCT over case-only variants return Exasol's groups | Integration | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `grouping_and_distinct_match_local_exasol_result` |
| A select-list predicate over the grouped character column is delegated | Integration | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `grouped_character_predicate_delegates_on_postgres` |

`vs-adapter/postgresql-type-mapping`

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| Integer types map to DECIMAL with the matching digit count | Unit | `crates/adbc-vs-adapter/src/postgres_type_mapping_tests.rs` | `integer_types_map_to_decimal` |
| numeric preserves the source precision and scale | Unit | `crates/adbc-vs-adapter/src/postgres_type_mapping_tests.rs` | `numeric_preserves_precision_and_scale` |
| numeric without a declared precision is unmapped | Unit | `crates/adbc-vs-adapter/src/postgres_type_mapping_tests.rs` | `unconstrained_numeric_is_unmapped` |
| character varying preserves the declared length and uses UTF8 | Unit | `crates/adbc-vs-adapter/src/postgres_type_mapping_tests.rs` | `character_varying_preserves_length` |
| Character column of a nondeterministic collation is unmapped | Unit | `crates/adbc-vs-adapter/src/postgres_type_mapping_tests.rs` | `nondeterministic_collation_is_unmapped` |
| character is unmapped because it is blank-padded | Unit | `crates/adbc-vs-adapter/src/postgres_type_mapping_tests.rs` | `bpchar_is_unmapped_for_padding` |
| Temporal and floating-point types map to their Exasol counterparts | Unit | `crates/adbc-vs-adapter/src/postgres_type_mapping_tests.rs` | `temporal_and_float_types_map` |
| Non-core types are unmapped | Unit | `crates/adbc-vs-adapter/src/postgres_type_mapping_tests.rs` | `non_core_types_are_unmapped_named` |
| Database whose encoding is not UTF8 is rejected at creation | Unit | `crates/adbc-vs-adapter/src/create_schema_tests.rs` | `non_utf8_encoding_is_rejected_named` |
| Virtual schema exposes the public schema only | Integration | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `only_the_public_schema_is_exposed` |

#### scan-udf

`scan-udf/arrow-scan` (framework)

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| Scan connects using credentials resolved at run time | Unit | `crates/adbc-vs-scan/src/connection_tests.rs` | `credentials_are_resolved_per_open_and_never_persisted` |
| Provisioned driver loads inside the UDF sandbox | Integration | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `provisioned_driver_loads_in_udf_sandbox` |
| Decimal width variation is normalized | Unit | `crates/adbc-vs-scan/src/coerce_tests.rs` | `decimal_width_variation_is_normalized` |
| Timestamp unit variation is normalized | Unit | `crates/adbc-vs-scan/src/coerce_tests.rs` | `timestamp_unit_variation_is_normalized` |
| Source failure surfaces the driver's own diagnosis | Integration | `crates/adbc-vs-scan/tests/postgres_it_test.rs` | `source_failure_surfaces_adbc_status` |
| Missing driver library fails with a named error | Integration | `crates/adbc-vs-scan/tests/postgres_it_test.rs` | `missing_driver_library_names_path_and_driver` |
| Scan streams every result batch to Exasol | Integration | `crates/adbc-vs-scan/tests/postgres_it_test.rs` | `scan_streams_every_batch` |
| Declared BOOLEAN column emits every representation the driver returns | Unit | `crates/adbc-vs-scan/src/coerce_tests.rs` | `declared_boolean_accepts_bool_and_integer` |

`scan-udf/mysql-scan` (technology)

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| Credentials pass as separate driver options | Unit | `crates/adbc-vs-scan/src/connection_tests.rs` | `mysql_passes_credentials_as_separate_options` |
| Connection string carries no credential | Unit | `crates/adbc-vs-scan/src/connection_tests.rs` | `mysql_connection_string_carries_no_credential` |

`scan-udf/postgresql-scan` (technology)

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| Connection string is built from the address and escapes every credential byte | Unit | `crates/adbc-vs-scan/src/connection_tests.rs` | `conninfo_escapes_every_credential_byte` |
| Credential never appears in any surfaced text | Integration | `crates/adbc-vs-scan/tests/postgres_it_test.rs` | `credential_never_appears_in_surfaced_text` |
| Adapter-side source error surfaces no credential | Integration | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `postgres_source_error_surfaces_no_credential` |
| A numeric arriving as a string is coerced into its declared DECIMAL | Unit | `crates/adbc-vs-scan/src/coerce_tests.rs` | `string_form_numeric_coerces_to_declared_decimal` |
| Non-finite numeric fails with a named error | Unit | `crates/adbc-vs-scan/src/coerce_tests.rs` | `non_finite_numeric_fails_named` |

#### driver-provisioning

All three features are framework, with no technology sibling, per decisions `[25]` and `[26]`.

`driver-provisioning/driver-self-install`

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| Missing driver is fetched and installed | Integration | `crates/adbc-vs-adapter/tests/provisioning_test.rs` | `missing_driver_is_fetched_per_driver` |
| Two drivers install side by side without collision | Integration | `crates/adbc-vs-adapter/tests/provisioning_test.rs` | `two_drivers_install_side_by_side` |
| Archive manifest is checked against the compiled-in filename | Integration | `crates/adbc-vs-adapter/tests/provisioning_test.rs` | `manifest_filename_checked_per_driver` |
| Registry client always verifies the server certificate | Integration | `crates/adbc-vs-adapter/tests/provisioning_test.rs` | `registry_client_rejects_untrusted_certificate` |
| BucketFS client accepts the cluster's own certificate | E2E | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` | `bucketfs_put_accepts_self_signed_certificate` |
| BucketFS write failure surfaces rather than being retried silently | Integration | `crates/adbc-vs-adapter/tests/provisioning_test.rs` | `bucketfs_write_failure_hides_credential` |
| Locked BucketFS object is waited out rather than surfaced | Integration | `crates/adbc-vs-adapter/tests/provisioning_test.rs` | `locked_bucketfs_path_is_retried_until_the_lock_clears` |
| Driver visibility wait is bounded | Integration | `crates/adbc-vs-adapter/tests/provisioning_test.rs` | `visibility_wait_times_out` |
| Release artifact bundles no driver binary | Integration | `crates/adbc-vs-adapter/tests/artifact_test.rs` | `release_artifact_bundles_no_driver_binary` |

The five transport and BucketFS rows carry forward from the library untouched by any delta, and their tests already exist under the names release 0.1 gave them. They are listed because the split moves the feature's other scenarios out from around them, and a coverage audit reads this table rather than the merged file. No task in this plan renames or rewrites those five tests.

`driver-provisioning/driver-version-resolution`

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| Version range resolves to the highest published version satisfying it | Unit | `crates/adbc-vs-adapter/src/registry_index_tests.rs` | `range_resolves_to_highest_published_match` |
| Range satisfied by no published version fails with a named error | Unit | `crates/adbc-vs-adapter/src/registry_index_tests.rs` | `unsatisfiable_range_names_driver_and_published_versions` |
| Missing platform package fails with a named error | Integration | `crates/adbc-vs-adapter/tests/provisioning_test.rs` | `missing_platform_names_driver_and_version` |
| Already-installed driver short-circuits the fetch | Integration | `crates/adbc-vs-adapter/tests/provisioning_test.rs` | `installed_driver_short_circuits_per_driver` |
| Newer in-range version is installed beside an older installed one | Integration | `crates/adbc-vs-adapter/tests/provisioning_test.rs` | `newer_in_range_version_is_not_short_circuited` |
| Unreachable registry names the offline fallback | Integration | `crates/adbc-vs-adapter/tests/provisioning_test.rs` | `offline_fallback_names_the_drivers_own_path` |

`driver-provisioning/driver-path-override`

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| DRIVER_PATH skips self-install entirely | Integration | `crates/adbc-vs-adapter/tests/provisioning_test.rs` | `driver_path_skips_self_install_entirely` |
| DRIVER_PATH naming no readable file fails without falling back | Integration | `crates/adbc-vs-adapter/tests/provisioning_test.rs` | `driver_path_missing_file_fails_without_fallback` |

### Manual Testing

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| driver-selection | `exapump sql --dsn "$EXASOL_DSN" "CREATE VIRTUAL SCHEMA pgvs USING ADBC_VS.ADBC_ADAPTER WITH DRIVER = 'snowflake' CONNECTION_NAME = 'PG_CONN' BUCKETFS_CONNECTION = 'BFS_CONN'"` | Fails naming `snowflake` and listing `mysql` and `postgresql` |
| create-virtual-schema | `exapump sql --dsn "$EXASOL_DSN" "CREATE VIRTUAL SCHEMA pgvs USING ADBC_VS.ADBC_ADAPTER WITH DRIVER = 'postgresql' CONNECTION_NAME = 'PG_CONN' BUCKETFS_CONNECTION = 'BFS_CONN'"` | Succeeds; `SELECT * FROM SYS.EXA_ALL_VIRTUAL_TABLES WHERE TABLE_SCHEMA='PGVS'` lists the `public` tables |
| driver-self-install | `exapump bfs ls --dsn "$BFS_DSN" --recursive /adbc-drivers/` | Lists `mysql/<version>/libadbc_driver_mysql.so` and `postgresql/<version>/libadbc_driver_postgresql.so`, each under its own driver and resolved-version segments |
| driver-version-resolution | `exapump sql --dsn "$EXASOL_DSN" "SELECT ADAPTER_NOTES FROM SYS.EXA_ALL_VIRTUAL_SCHEMAS WHERE SCHEMA_NAME='PGVS'"` | `driverVersion` reports the version the range resolved to, and `driverPath` carries the driver and version segments |
| driver-path-override | `exapump sql --dsn "$EXASOL_DSN" "CREATE VIRTUAL SCHEMA pgvs2 USING ADBC_VS.ADBC_ADAPTER WITH DRIVER = 'postgresql' CONNECTION_NAME = 'PG_CONN' DRIVER_PATH = '/buckets/bfsdefault/default/custom/pg-driver.so'"` | Succeeds with no `BUCKETFS_CONNECTION` and no registry access; querying the schema loads the operator-supplied library |
| driver-path-override | The same statement with `DRIVER_PATH` naming a path holding no file | Fails naming `DRIVER_PATH`, the path, and `postgresql`; does not fall back to the registry |
| pushdown-dialect | `exapump sql --dsn "$EXASOL_DSN" "SELECT PUSHDOWN_SQL FROM (EXPLAIN VIRTUAL SELECT * FROM pgvs.\"orders\" o FULL OUTER JOIN pgvs.\"customers\" c ON o.\"cid\" = c.\"id\")"`, then the same shape over the MySQL virtual schema | The PostgreSQL schema's generated statement carries `FULL OUTER JOIN`; the MySQL schema's carries no join, so its two table scans are pushed down separately |
| postgresql-pushdown | `exapump sql --dsn "$EXASOL_DSN" "EXPLAIN VIRTUAL SELECT \"id\", \"note\" FROM pgvs.\"orders\""` | Generated statement projects only `"id"` and `"note"`, every identifier double-quoted |
| postgresql-collation | `exapump sql --dsn "$EXASOL_DSN" "EXPLAIN VIRTUAL SELECT \"id\" FROM pgvs.\"orders\" WHERE \"note\" < 'm'"` | `WHERE` clause carries `COLLATE "C"` on the column operand and an `E'...'` literal |
| postgresql-collation | `exapump sql --dsn "$EXASOL_DSN" "EXPLAIN VIRTUAL SELECT \"id\" FROM pgvs.\"orders\" WHERE \"note\" = 'Widget'"` | `WHERE` clause carries no collation suffix |
| postgresql-collation | `exapump sql --dsn "$EXASOL_DSN" "SELECT \"note\", COUNT(*) FROM pgvs.\"orders\" GROUP BY \"note\""` | One group per distinct value including case-only variants; generated `GROUP BY` carries no collation suffix |
| postgresql-pushdown | `exapump sql --dsn "$EXASOL_DSN" "EXPLAIN VIRTUAL SELECT * FROM pgvs.\"orders\" o FULL OUTER JOIN pgvs.\"customers\" c ON o.\"cid\" = c.\"id\""` | Generated statement renders `FULL OUTER JOIN` |
| mysql-collation | `exapump sql --dsn "$EXASOL_DSN" "EXPLAIN VIRTUAL SELECT \`id\` FROM myvs.\`orders\` WHERE \`note\` = 'Widget'"` | `WHERE` clause carries MySQL's comparison collation suffix on the column operand, where the PostgreSQL step above carries none — the two technologies' collation rules read side by side |
| result-ordering | `exapump sql --dsn "$EXASOL_DSN" "EXPLAIN VIRTUAL SELECT \"id\" FROM pgvs.\"orders\" ORDER BY \"id\" LIMIT 5"` | Generated statement carries no `ORDER BY`, `LIMIT`, or `OFFSET` |
| postgresql-type-mapping | `exapump sql --dsn "$EXASOL_DSN" "SELECT ADAPTER_NOTES FROM SYS.EXA_ALL_VIRTUAL_SCHEMAS WHERE SCHEMA_NAME='PGVS'"` | `omitted` lists the `boolean`, `text`, `character`, `jsonb`, and unconstrained-`numeric` columns, each under `sourceType` with a reason |
| postgresql-scan | `exapump sql --dsn "$EXASOL_DSN" "SELECT \"amount\" FROM pgvs.\"orders\""` | Returns the seeded `numeric(12,4)` values with scale intact, having arrived from the driver as strings |
| postgresql-scan | Point `PG_CONN` at a wrong password, then `exapump sql --dsn "$EXASOL_DSN" "SELECT * FROM pgvs.\"orders\""` | Fails naming the host and database, with the password absent from the message |
| adapter-request-dispatch | `exapump sql --dsn "$EXASOL_DSN" "ALTER VIRTUAL SCHEMA pgvs REFRESH"` | Succeeds and re-discovers through the PostgreSQL metadata source |

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Build | `make cross-udf-build` | Exit 0; `cargo exasol-udf validate` reports no fingerprint mismatch |
| Unit tests | `make test` | 0 failures, no container needed |
| Artifact | `make test-artifact` | Exit 0; both entry symbols exported |
| Integration (MySQL) | `make test-it` | 0 failures |
| Integration (PostgreSQL) | `make test-it-postgres` | 0 failures |
| E2E | `make test-e2e` and the PostgreSQL end-to-end target task 9.1 settles | 0 failures; no test skipped |
| Policy | `make deny` | 0 findings; allow list unchanged |
| Lint | `make lint` | 0 warnings |
| Format | `make fmt-check` | No changes |
| Spec validation | `speq plan validate add-postgresql-support` and `speq feature validate` | Pass |
| Recorded Backgrounds | § Recording notes' post-record check, under `bash`, after `/speq:record` merges | `ok` for all eleven features; exit 0. A `FAIL` means the merge dropped that feature's Background or description line and § Recording notes' manual step was skipped |

### Library organization after recording

Recording this plan leaves the library at 22 features. `/speq:spec-merge`'s feature-count threshold still trips on `vs-adapter`, and a human has accepted that count as the standing shape rather than a defect to fix. Decisions `[25]` and `[26]` record both halves; nothing here is left for `/speq:record` to escalate. Recording does owe one manual step, unrelated to these thresholds: see § Recording notes at the top of this section.

| Domain | Features | Threshold | Over-10 spec files |
|--------|----------|-----------|--------------------|
| `vs-adapter` | 16 | 8 | `pushdown-dialect` 13, `pushdown-aggregation` 13, `create-virtual-schema` 14, `driver-selection` 14 |
| `scan-udf` | 3 | 8 | `arrow-scan` 13 |
| `driver-provisioning` | 3 | 8 | none |

The two thresholds were **already jointly unsatisfiable for `vs-adapter` before this plan**: it held 9 features and roughly 96 scenarios, and 96 scenarios need at least ten files at ten scenarios each. Only a new top-level domain resolves that, and this repo's spec governance forbids one. The restructuring improves both numbers rather than worsening them — `pushdown-aggregation` drops from 23 scenarios to 13, and `type-mapping` from a merged 19 to 5 — but it cannot bring `vs-adapter` under 8 features while also keeping files under 10 scenarios.

**`vs-adapter`'s 16 features are the accepted shape, not a backlog item.** A human read the arithmetic above and chose to accept the count rather than revisit the no-new-domain rule. The count is the direct, intended result of separating framework from technology, so re-consolidating features to satisfy the generic guideline would undo exactly what this plan was instructed to do. `CLAUDE.md`'s Spec governance section now says so, so a later planner or auditor reads the acceptance rather than re-deriving the question.

**`driver-self-install`'s 17 scenarios split three ways**, on the same human decision, along the framework-internal axis decision `[25]` predicted: `driver-self-install` at 9, `driver-version-resolution` at 6, `driver-path-override` at 2. That clears the ten-scenario threshold for the whole domain. Four `vs-adapter` files and one `scan-udf` file stay above it. Each is a single coherent feature whose scenario count reflects how much behavior it owns, and none was raised as a candidate for a further split.

**Every open question this plan escalated is now answered.** Decision `[22]` settles the integrity model — TLS only, uniformly for both drivers. Decision `[26]` settles the library's organization. No task is blocked on a pending decision and the plan is ready to implement in full. Three measurement gates remain, as designed: gates 2.1, 2.2b, and 2.3 can each falsify a design assumption, and each names the re-planning its failure forces.
