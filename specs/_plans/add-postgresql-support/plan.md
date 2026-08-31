# Plan: add-postgresql-support

## Summary

Adds PostgreSQL as a second ADBC source behind a driver identity that keys every per-driver fact — dialect, driver package, metadata source — to one closed enum, so `getCapabilities` and `pushdown` resolve the same dialect per virtual schema and each dialect advertises the capability set its own SQL can render. PostgreSQL matches MySQL's delegation scope and adds `JOIN_TYPE_FULL_OUTER`, the one capability the two targets genuinely disagree on.

## Design

### Context

mission.md names PostgreSQL first in its v1 driver set and uses `DRIVER = 'postgresql'` as the persona example, and `pushdown-dialect`'s design already states that the dialect owns its advertised capability set. What does not exist is any way to have two of them. Six forces shape the design.

**The dialect boundary is right but has no selection point.** `SqlDialect` already exposes exactly `capabilities()` and `render()`, and `capabilities.rs` derives the response from the dialect rather than from a list of its own. The single-driver assumption lives one layer up: `dispatch.rs` names `MySqlDialect` at both call sites, and its module doc justifies that as the property that keeps the advertised set and the renderer in agreement. Two drivers must preserve that property while letting the answer differ per virtual schema, which means both arms resolve the dialect from one field rather than neither naming one.

**Per-driver facts are scattered as module constants, not keyed on anything.** `adbc-vs-scan::driver` holds one `DRIVER_VERSION`, one `DRIVER_LIBRARY`, one `PACKAGE_SHA256`, one `REGISTRY_DRIVER_PATH`; `connection.rs` holds one `DRIVER_ENTRY_POINT`; `create_schema.rs` holds one `SUPPORTED_DRIVER`. Six places restate "there is one driver". A second driver either threads a parameter through all six or gives the decision one owner. The owner is a closed enum, because a Rust `match` over it is exhaustive: adding a variant fails the build at every table missing a row, which is what makes "a new driver adds an implementation rather than editing a renderer" a compiler guarantee rather than a convention.

**`getCapabilities` may not know which source it is being asked about.** `pushdown` reads `schemaMetadataInfo.adapterNotes`, whose `driver` field release 0.2 writes and never reads. Whether `getCapabilities` carries the same field is unverified: `virtual-schema-common-java` models `schemaMetadataInfo` on every adapter request, which is why the plan expects it, but the adapter reads the wire rather than the framework. The answer decides whether a per-dialect capability set is legal at all — without it, an adapter can advertise only the intersection of every dialect's set. Gate 2.1 measures it before any table is built.

**PostgreSQL needs less collation machinery than MySQL, not a translation of it.** Roughly a third of `mysql_dialect.rs` exists to undo MySQL's default collation, which answers both of Exasol's character questions differently: `utf8mb4_0900_ai_ci` matches `widget` to `Widget` and orders by collation weight. PostgreSQL's built-in collations are all deterministic, and a deterministic collation equates two values only when their bytes are identical — Exasol's own equality. So PostgreSQL needs no collation on an equality operand, a `GROUP BY` key, or a `DISTINCT` operand. It needs one on an ordering operand: the official `postgres` image sets `LANG=en_US.utf8`, so `MIN` over `B`, `a`, `C` returns `a` where Exasol returns `B`, and `COLLATE "C"` sorts by byte value, which for UTF-8 is code-point order. One collation where MySQL needs two, and rendered at fewer sites.

That difference cascades. MySQL's grouping collation turns the `GROUP BY` key into an expression, which forces the same suffix onto every bare select-list occurrence of that column and creates the one composed shape MySQL rejects outright — a select-list predicate reading its query's character grouping key, which fails with error 1055 at every collation placement. PostgreSQL renders a bare grouping column, so the key and its select-list occurrence are the same expression and the shape is expected to delegate. Gate 2.3 measures that rather than reasoning it away, because the failing direction is a hard source error.

**PostgreSQL's driver accepts one connection option, which breaks a stated invariant.** `arrow-scan` requires that the user and password pass as separate ADBC options so no credential travels in a connection string. The ADBC PostgreSQL driver's `SetOption` accepts exactly `uri` and rejects `username` and `password` as unknown options, handing `uri`'s value verbatim to libpq's connect call. There is no design that preserves the invariant for PostgreSQL; the only choice is where the credential goes instead. It goes into a libpq keyword/value connection string built in memory at connect time, because that form escapes any byte by single-quoting and backslash-escaping, where URI form would need percent-encoding and a password containing `&`, `#`, `@`, or `/` would change the parse. Every invariant about persistence and disclosure survives and is asserted: nothing reaches the scan specification, `adapterNotes`, SQL text, `EXPLAIN VIRTUAL`, or any log line, and the scan strips the password from libpq's own diagnostics rather than trusting the driver to omit it. This is the plan's one deliberate narrowing of a security property; decision-log `[6]` records it.

**The two drivers disagree in small, load-bearing details.** The registry spells MySQL's pinned version `v0.6.0` and PostgreSQL's `1.12.0`, and the index lookup matches by exact string equality, so a shared "add a `v`" rule finds neither one reliably — the version string is per-driver data, not a derived form. The PostgreSQL driver returns every `numeric`, constrained or not, as a UTF-8 string, so a column declared `DECIMAL(p,s)` is parsed from text rather than cast from a decimal, and `NaN`/`±Infinity` are legal source values no Exasol `DECIMAL` can hold. The PostgreSQL library is C++ and links `libstdc++` and `libgcc_s`, where the Go-built MySQL driver links neither, so a provisioned library is not yet a loadable one — and if it does not load, the fix lands in `language-container-rs`, outside this repo. Gates 2.2a and 2.2b test that first, because late discovery is the most expensive outcome in the plan: the fix returns through the release chain rather than through this repo.

- **Goals** — `DRIVER = 'postgresql'` reaching PostgreSQL through the same self-install, pushdown, and scan path MySQL uses; one closed driver identity owning every per-driver fact, so a third driver adds rows and an implementation; a per-dialect advertised capability set, resolved from the same field the renderer is resolved from, proven by an invariant test that enumerates dialects from the type; PostgreSQL delegation matching MySQL's scope plus `FULL OUTER JOIN`; every delegated PostgreSQL fragment verified against Exasol's own answer for the same query.
- **Non-Goals** — any driver beyond MySQL and PostgreSQL; a PostgreSQL schema other than `public`, and the property that would name one; mapping PostgreSQL `boolean`, `character(n)`, `text`, `uuid`, `jsonb`, `timestamptz`, or an unconstrained `numeric`; `AVG` and `SUM` delegation for either dialect, withheld for PostgreSQL by the same all-or-nothing rule that withholds them for MySQL despite PostgreSQL's exact-numeric `sum` being exact; `AGGREGATE_HAVING`, grouping by an expression, grouping by more than one column, scalar-function delegation, `REGEXP_LIKE`, `JOIN_CONDITION_ALL`; `ORDER BY` and `LIMIT`/`OFFSET`, withheld for every dialect because the emitting-script transport rather than the source is what makes them unsound; a benchmark against `postgresql-virtual-schema`; and preserving the separate-credential-option invariant for PostgreSQL, which its driver makes impossible.

### Decision

#### Architecture

```
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
   │ MySqlDialect       │  │ MySqlSource      │  │ version · library    │
   │ PostgresDialect    │  │ PostgresSource   │  │ sha256 · registry    │
   └─────┬──────────┬───┘  └──────────────────┘  │ path · entry point   │
         │          │       catalog SQL +        └──────────────────────┘
         │          │       its own type mapping   provisioning + scan load
         ▼          ▼
  capabilities()  render(&Query)
         │              │
   getCapabilities   pushdown → scan spec (carries Driver) → scan UDF
```

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
| Per-driver data stored, never derived | Pinned version strings `v0.6.0` and `1.12.0` | The registry's spellings are not uniform and the lookup is exact string equality |
| Measure before advertising | Gates 2.1, 2.2b, 2.3 precede the capability sets they justify | An advertised capability the source refuses converts a query Exasol would answer into a hard failure |
| Differential verification against a local Exasol mirror | PostgreSQL E2E tier | Catches collation, type, and aggregate gaps generically instead of predicting them one at a time |

### Consequences

| Decision | Alternatives Considered | Rationale |
|----------|------------------------|-----------|
| Closed `Driver` enum owning three tables | A string threaded to each call site; a trait object bundling all per-driver behavior | The enum makes a missing table row a build failure. A bundled trait would put `DriverPackage` data behind a method in the adapter crate, which the scan crate cannot reach |
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

## Features

| Feature | Status | Spec |
|---------|--------|------|
| vs-adapter/driver-selection | NEW | `vs-adapter/driver-selection/spec.md` |
| vs-adapter/pushdown-dialect | CHANGED | `vs-adapter/pushdown-dialect/spec.md` |
| vs-adapter/create-virtual-schema | CHANGED | `vs-adapter/create-virtual-schema/spec.md` |
| vs-adapter/type-mapping | CHANGED | `vs-adapter/type-mapping/spec.md` |
| vs-adapter/pushdown-projection | CHANGED | `vs-adapter/pushdown-projection/spec.md` |
| vs-adapter/pushdown-filter | CHANGED | `vs-adapter/pushdown-filter/spec.md` |
| vs-adapter/pushdown-aggregation | CHANGED | `vs-adapter/pushdown-aggregation/spec.md` |
| vs-adapter/pushdown-join | CHANGED | `vs-adapter/pushdown-join/spec.md` |
| vs-adapter/result-ordering | CHANGED | `vs-adapter/result-ordering/spec.md` |
| vs-adapter/adapter-request-dispatch | CHANGED | `vs-adapter/adapter-request-dispatch/spec.md` |
| driver-provisioning/driver-self-install | CHANGED | `driver-provisioning/driver-self-install/spec.md` |
| scan-udf/arrow-scan | CHANGED | `scan-udf/arrow-scan/spec.md` |

## Impact

`CREATE VIRTUAL SCHEMA ... WITH DRIVER = 'postgresql'` works, exposing the `public` schema of the addressed PostgreSQL database, with the driver self-installed from the same registry MySQL uses. A PostgreSQL-backed schema delegates projection, filter, aggregation, and equi/outer joins on MySQL's terms, and additionally delegates `FULL OUTER JOIN`.

Existing MySQL virtual schemas keep working without recreation. Three visible changes reach an operator:

- **The unsupported-driver error text changes.** It now lists every supported value instead of naming `mysql` as the only one. A script matching that message breaks.
- **PostgreSQL credentials travel inside the connection string handed to the driver, in process memory.** The ADBC PostgreSQL driver accepts no separate credential option. No credential is written into the scan specification, `adapterNotes`, generated SQL, `EXPLAIN VIRTUAL` output, or any log line or error message, and the scan strips the password from libpq's own diagnostics. MySQL's separate-option path is unchanged. This is a narrowing of the previous posture, not an equivalent, and it is the reason a PostgreSQL `CONNECTION` object deserves a least-privilege, read-only role.
- **PostgreSQL columns of unmapped types are omitted, and the omitted set is wider than MySQL's.** `boolean`, `text`, `character(n)`, an unconstrained `numeric`, `timestamptz`, `uuid`, `jsonb`, and `bytea` are all omitted with a recorded reason. A `character(n)` column omission is the one likely to surprise, because MySQL's `CHAR` maps.

`adapterNotes` records an omitted column's source type under `sourceType` rather than `mysqlType`; notes written by an earlier release still parse. Running the full test stack now needs a third container, and the CI `e2e` job's disk cleanup carries a PostgreSQL image on top of Exasol and MySQL.

## Dependencies

| Dependency | Detail |
|------------|--------|
| ADBC PostgreSQL driver | Registry path `postgresql`, library `libadbc_driver_postgresql.so`, entry point `AdbcDriverPostgresqlInit`, Apache-2.0. Fetched at run time, never bundled, so outside `cargo deny`'s scope. The pinned version string and the package URL are whatever gate 2.4 records from the index — this table quotes that one measured spelling and states no second guess. Research read `1.12.0` as the `version` scalar; the gate settles it |
| Pinned package SHA-256 | Task 2.4 verifies and records it. The registry publishes no checksum, so the digest is compiled in as MySQL's is |
| `postgres:18.6-trixie` | Test container. Within the driver's documented tested range of PostgreSQL 14–18; Debian trixie matches the `rust:1.94-trixie` builder |
| `language-container-rs` SLC | Must supply `libstdc++.so.6` and `libgcc_s.so.1` for the C++ driver to load. Gate 2.2b verifies, recording the SLC's actual `ldd` output. A failure is fixed there, not here, and blocks this plan |

## Migration

| Current | New |
|---------|-----|
| `adapterNotes` omission field `mysqlType` | `sourceType`, with `mysqlType` accepted on deserialization so notes Exasol persisted before this release still parse |
| `ScanSpec` without a driver field | Carries the driver identity. No migration: the specification is generated fresh on every pushdown and never persisted |
| Existing MySQL virtual schemas | No action. `adapterNotes.driver` already records `mysql`, which is the field resolution now reads |

## Implementation Tasks

### Phase 1: PostgreSQL test stack

- [ ] 1.1 Add the `postgres` service to `docker-compose.yml`: image `postgres:18.6-trixie`, static IP `172.28.5.11` on the existing `adbc-vs` network, host port `127.0.0.1:5432`, `pg_isready` healthcheck matching the MySQL service's interval and retries, and `./docker/postgres/init.sql` mounted into `/docker-entrypoint-initdb.d/`. Keep the image's default `LANG=en_US.utf8` cluster locale and set no `POSTGRES_INITDB_ARGS`: locale ordering is what makes a missing `COLLATE "C"` fail a test instead of passing silently. Seed a minimal fixture for now; task 8.1 grows it.
- [ ] 1.2 Add `stack-up-postgres` to the `Makefile` (`compose up -d --wait postgres`), make `stack-up` depend on it alongside `stack-up-mysql`, and add `test-it-postgres` with prereq `stack-up-postgres`. Extend `tests/make_targets_test.rs` for the new targets and the new prereq edges.

### Phase 2: Gates

- [ ] 2.1 Gate [expert]: measure whether a `getCapabilities` request on Exasol 2025.2.1 carries `schemaMetadataInfo.adapterNotes`, and whether the `driver` field inside it is the value `createVirtualSchema` recorded. Use the throwaway Lua adapter probe in `tests/common/probe.rs`, or log the raw request body from the Rust adapter — capture the request verbatim rather than inferring from a successful response. Also record whether `schemaMetadataInfo.properties` arrives, as the fallback source. **If the identity does not arrive, stop and re-plan**: per-dialect capability sets become illegal, every dialect must advertise the intersection, `JOIN_TYPE_FULL_OUTER` is withheld from both, and `driver-selection`'s and `pushdown-dialect`'s scenarios change. Record the measurement in decision-log under Review Findings.
- [ ] 2.2a Gate: resolve the PostgreSQL package from the registry index by hand — read the `postgresql` entry, record its exact `version` scalar and the exact relative `url` of its `linux_amd64` package — then fetch and extract `libadbc_driver_postgresql.so` and upload it to BucketFS using `BUCKETFS_CONNECTION`'s credentials. Record the resolved absolute URL for tasks 2.4 and 6.1. Prerequisites: outbound HTTPS egress and a reachable BucketFS write endpoint.
- [ ] 2.2b Gate: load the library task 2.2a uploaded from inside the Exasol UDF sandbox by the entry point `AdbcDriverPostgresqlInit`, calling the driver manager directly rather than through the scan path, from a throwaway raw driver-load entry point compiled into the crates. Prerequisites: task 2.2a, `make cross-udf-build`, `make stack-up`, and the SLC installed. Record the SLC's resolved shared-library set as measured by `ldd` inside the sandbox, so the gate reports what the container actually supplies rather than what the plan predicted. Record the exact loader error if it fails. **A failure blocks this plan** and is fixed in `language-container-rs`, then flows back through the release chain — escalate rather than working around it here.
- [ ] 2.3 Gate [expert]: measure the PostgreSQL dialect's semantic assumptions against `postgres:18.6-trixie` and Exasol 2025.2.1, using `psql` and `exapump` directly rather than the adapter, so the gate does not wait on Phase 4. Measure and record each: (a) `FULL OUTER JOIN` with an equality condition executes rather than being rejected over the merge-joinable restriction; (b) a select-list predicate reading a bare character `GROUP BY` key executes, i.e. there is no analogue of MySQL's error 1055 for this shape; (c) an `E'<body>'` literal carries its exact value under `standard_conforming_strings` both `on` and `off` and under `backslash_quote` set to `off`, for a value containing a backslash, a single quote, and a multi-byte character; (d) `MIN`/`MAX` over `B`, `a`, `C` returns Exasol's answer with `COLLATE "C"` and a different answer without it; (e) an uncollated `GROUP BY` and `COUNT(DISTINCT)` over values differing only by case and only by accent return the four groups Exasol returns; (f) `LIKE` matches the same rows with and without a collation suffix; (g) the driver returns `numeric(12,4)` as an Arrow string, and `NaN`/`Infinity` are storable in such a column; (h) the `selectListDataTypes` Exasol sends for aggregates over a PostgreSQL-backed schema match the values already measured against MySQL. Any result contradicting a spec Background bullet re-plans that bullet and its scenarios before the rendering task starts.
- [ ] 2.4 Independently verify the SHA-256 digest of the PostgreSQL package at the URL task 2.2a resolved, fetched fresh, and confirm the archive's `MANIFEST` `[Files] driver` key is `libadbc_driver_postgresql.so`. Record as this gate's outputs the index's exact `version` scalar and the exact relative `url` for the PostgreSQL entry, verbatim as published: task 6.1 and § Dependencies quote that one measured spelling and no second one, because `mysql_it_test.rs` derives the whole package URL from the pinned version string. Reproduce the already-pinned MySQL digest by the same method as a check on the method. Record the digest for task 6.1; do not carry a digest forward from research notes unverified. Prerequisite: task 2.2a.

### Phase 3: Driver identity foundation

- [ ] 3.1 [expert] Introduce `Driver` in `adbc-vs-scan::driver` as a closed enum with `parse` (case-insensitive), `as_str` (canonical lower case), and `package() -> &'static DriverPackage`. `DriverPackage` carries the pinned version, library filename, package SHA-256, registry driver path, and ADBC entry-point symbol. Populate the MySQL row from today's constants verbatim, including the `v0.6.0` spelling. Replace `Registry::pinned()`'s reads of the bare constants and `provisioning.rs`'s `DRIVER_LIBRARY`/`DRIVER_VERSION` re-export with per-driver reads threaded from the identity, and make `driver_install_path` take it. Add no PostgreSQL row yet — task 6.1 does — so this task's diff is provably behavior-preserving for MySQL. Every existing provisioning test MUST pass unchanged.
- [ ] 3.2 [expert] Route dialect resolution through the identity. Add `dialect_for(Driver) -> &'static dyn SqlDialect` beside the `SqlDialect` trait, resolve `Driver` from `schemaMetadataInfo.adapterNotes` in both `dispatch`'s `getCapabilities` and `pushdown` arms, and remove `dispatch`'s import of `MySqlDialect` so no call site names a concrete dialect. A `getCapabilities` request carrying no notes MUST error naming the missing field rather than defaulting to a driver. Replace `create_schema::SUPPORTED_DRIVER` and `validate_driver` with `Driver::parse`, whose error lists every supported value generated from the identities. Add the connection-address scheme check against the resolved driver. Update `dispatch_test.rs` and `capabilities_tests.rs` for the notes-bearing request shape.
- [ ] 3.3 Add the driver identity to `ScanSpec` and to the scan's driver load: rename `MysqlConnection` to `SourceConnection` and `open_mysql_connection` to an open taking the identity, reading the entry-point symbol from `Driver::package()` instead of the module-level `DRIVER_ENTRY_POINT`. Update `scan_entry.rs`, `scan_spec.rs`, their tests, and `mysql_it_test.rs`'s call sites. MySQL's rendered scan specification gains one field and nothing else changes.
- [ ] 3.4 Make the capability invariant test enumerate dialects from the `Driver` type rather than naming one: run the every-advertised-capability-renders arm and the byte-identical-re-render arm once per variant. Add the test that every identity yields a dialect, a package, and a metadata source with no fallback, and the test asserting the installed library filenames are pairwise distinct across identities. The set-difference test lands with task 4.4, once a second set exists.
- [ ] 3.5 [expert] Move type mapping behind `MetadataSource`. `discover_tables` returns columns already mapped to `MappedType` with omissions recorded, so `create_schema` no longer calls `type_mapping::map_column` and branches on no driver; add the dialect-stated source precondition check to the trait, with MySQL's collation-and-version check as its first implementation and `discover_collations` no longer an inherent method. Rename `Omission.mysql_type` to `source_type` serialized as `sourceType`, with `#[serde(alias = "mysqlType")]` so notes Exasol persisted before this release still parse. Relocate the mapping assertions that move out of `create_schema_tests.rs`.
- [ ] 3.6 Generalize the integration tier's driver-fetch helper over the driver identity. `mysql_it_test.rs`'s `package_url()` derives the whole package URL from `REGISTRY_BASE_URL`, `REGISTRY_DRIVER_PATH`, `DRIVER_VERSION`, and `PLATFORM`, and `install_driver` caches the extracted library under `PACKAGE_SHA256` — both read module constants that task 3.1 turns into `DriverPackage` fields. Move both into a shared per-driver helper taking a `Driver` and reading its package row, so a second tier reuses it rather than copying it. `mysql_it_test.rs` MUST fetch, verify, and cache exactly as it does today. This is the only route the repo has to a driver library outside BucketFS, and it is a prerequisite of task 7.2.

### Phase 4: PostgreSQL dialect rendering

- [ ] 4.1 Add `postgres_dialect.rs` with `PostgresDialect` implementing `SqlDialect`: double-quote identifier quoting doubling an embedded double quote, projection rendering, table qualification by the recorded namespace, the single-constant projection for an empty select list, and positional `EMITS` naming. Advertise MySQL's set verbatim for now; tasks 4.2 to 4.4 adjust it. Register the dialect in `dialect_for`.
- [ ] 4.2 [expert] Render PostgreSQL filters: string literals as `E'<body>'` with a backslash doubled and a single quote doubled, rendered unconditionally so no value changes the statement's shape; `DATE`/`TIMESTAMP` typed literals; numeric and null literals; every advertised predicate; `<>` for not-equal; parenthesized nesting. Apply `COLLATE "C"` to a character operand of `predicate_less`, `predicate_lessequal`, and `predicate_between` and to nothing else, per gate 2.3(c), (d), and (f). Read both comparison operands by position.
- [ ] 4.3 [expert] Render PostgreSQL aggregation: `COUNT`, `COUNT(*)`, `COUNT(DISTINCT)`, `MIN`, `MAX` in canonical upper case whatever case arrived; `COLLATE "C"` on a character operand of `MIN`/`MAX`; no collation on a `GROUP BY` key, on a bare select-list occurrence of a grouped column, or on a `DISTINCT` operand; `single_group` renders no `GROUP BY`. Per gate 2.3(b), render rather than reject a select-list predicate reading the query's character grouping key, and keep MySQL's rejection bound to the MySQL dialect. Reject `having`, `orderBy`, and `limit` elements with the same named errors MySQL's dialect returns.
- [ ] 4.4 Render `FULL OUTER JOIN` and add `JOIN_TYPE_FULL_OUTER` to the PostgreSQL capability set, conditional on gate 2.3(a); render `INNER`/`LEFT OUTER`/`RIGHT OUTER` and the positional aliasing and column qualification MySQL's dialect uses, with no collation on a character join key. Add the test asserting the two dialects' sets differ in exactly `JOIN_TYPE_FULL_OUTER`. If gate 2.3(a) measured a rejection, withhold the capability, record it in the decision log, and revise `pushdown-join`'s and `pushdown-dialect`'s scenarios to state that the two sets coincide in this release.
- [ ] 4.5 Unit tests for `PostgresDialect` in `postgres_dialect_tests.rs`: identifier quoting including an embedded double quote and a mixed-case name; every literal form; every predicate form; the full collation-placement matrix over character, numeric, and temporal operands in comparison, ordering, grouping, `DISTINCT`, and `MIN`/`MAX` positions; each join type; nested joins; a shared column name; an unresolvable column; an unrenderable aggregate and join type; and a byte-identical re-render of one request.

### Phase 5: PostgreSQL metadata, mapping, and connection

- [ ] 5.1 Add the PostgreSQL metadata source: `information_schema.tables` and `information_schema.columns` filtered to `table_schema = 'public'`, reading `table_name`, `column_name`, `ordinal_position`, `data_type`, `character_maximum_length`, `numeric_precision`, `numeric_scale`, `is_nullable`, and `collation_name`, exposing both `BASE TABLE` and `VIEW`. Record `public` as the namespace in `adapterNotes`. Do not reuse MySQL's query text: `COLUMN_TYPE` does not exist in PostgreSQL and `table_schema` names a schema rather than a database.
- [ ] 5.2 Add the PostgreSQL type mapping and its unit tests: `integer`, `bigint`, `numeric(p,s)`, `character varying(n)`, `date`, `timestamp without time zone`, `double precision` mapped; a `numeric` precision above 36, an absent precision, an absent character length, `character`, `boolean`, `text`, `smallint`, `real`, both `time` types, `timestamptz`, `uuid`, `json`, `jsonb`, `bytea`, `interval`, `money`, and `ARRAY` unmapped with the reason naming the type; a character column of a nondeterministic collation unmapped, treating a server that reports no determinism flag as deterministic.
- [ ] 5.3 Add PostgreSQL's source precondition check through the `MetadataSource` trait: the database encoding MUST be `UTF8`, checked before any table is discovered and on every request that resolves the source `CONNECTION`, with the error naming the reported encoding and `UTF8`. Assert MySQL's collation check does not run for a PostgreSQL source.
- [ ] 5.4 [expert] Build PostgreSQL's connection value as a libpq keyword/value connection string from the `CONNECTION` object's address and credentials: host, port, and `dbname` from the address, every address query parameter carried across as a keyword of the same name, `user` and `password` from the object, and every value single-quoted with an embedded quote or backslash escaped. Pass it as the driver's only accepted database option, and pass no `username` or `password` option for this driver. Strip the resolved password from every surfaced error, including text the driver or libpq produced, while keeping the host and database in the message. The connection value MUST NOT be interpolated as an error subject: `connect_error` today passes `&connection.uri` as the subject and redacts only the driver's message, which for PostgreSQL would print the conninfo string verbatim. Redaction applies to every caller of the shared open path, not to the scan alone — `create_schema.rs:193-199` opens the source connection through it, so `createVirtualSchema`, `refresh`, and `setProperties` traverse the same value. Unit-test the escaping over a password containing a single quote, a backslash, a space, and an ampersand, and test that the redaction survives a driver error carrying the connection string.

### Phase 6: PostgreSQL driver provisioning

- [ ] 6.1 Add the PostgreSQL `DriverPackage` row: the `version` scalar exactly as gate 2.4 recorded it from the index, library `libadbc_driver_postgresql.so`, the digest task 2.4 verified, registry path `postgresql`, entry point `AdbcDriverPostgresqlInit`. Take the spelling from the gate output, never from this plan's prose. Add the test asserting each driver's pinned version string resolves against the real index's spelling, and that the version is stored per driver rather than derived by adding or stripping a leading `v`.
- [ ] 6.2 Extend provisioning tests for two drivers: add a PostgreSQL entry to the stub registry in `tests/provisioning_test.rs`; assert both libraries install side by side without either overwriting or satisfying the other's presence check; assert a per-driver digest mismatch, missing platform package, and offline-fallback message each name the driver and its own BucketFS path; assert only the file the `MANIFEST` names is uploaded, so the package's differently-spelled license files are not.

### Phase 7: Scan coercion and the PostgreSQL integration tier

- [ ] 7.1 Coerce a `numeric` column the driver returns as an Arrow string into its declared `DECIMAL(p,s)`, with the source precision and scale intact, treating the string form as a first-class case rather than a fallback. Fail with an error naming the value and the declared type for `NaN`, `Infinity`, and `-Infinity`, per value rather than per column, and keep a finite value in the same column coercing. Add the cases to `coerce_tests.rs`.
- [ ] 7.2 Add the PostgreSQL integration tier: a `postgres-it` cargo feature, default off, and `tests/postgres_it_test.rs` in `adbc-vs-scan` covering driver load by the PostgreSQL entry point, statement execution, Arrow batch streaming across more than one batch, and coercion of every mapped type including the string-form `numeric` and a microsecond timestamp. Fetch the driver library through task 3.6's per-driver helper, which is this task's prerequisite alongside task 6.1's package row — there is no other route to a driver library outside BucketFS. Needs the `postgres` container only, never Exasol, and links the crate as an `rlib` so it does not depend on `cross-udf-build`.

### Phase 8: End-to-end tier

- [ ] 8.1 Grow `docker/postgres/init.sql` into the differential fixture and add local Exasol mirror tables seeded from identical values: character values differing only by case and only by accent, the ordering triple `B`/`a`/`C`, a trailing-space pair, `numeric(12,4)`, an unconstrained `numeric`, `character(8)`, `character varying(100)`, `text`, `timestamp`, `timestamptz`, `uuid`, `jsonb`, `bytea`, a nullable numeric column, two tables sharing a key with an unmatched row on each side, and a table whose every column is unmapped. Put the case and accent variants in the column the aggregation tests group by.
- [ ] 8.2 [expert] Differential PostgreSQL E2E tests in `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` behind the `exasol-e2e` feature, with its own shared setup module creating both a PostgreSQL and a MySQL virtual schema so task 8.3 has both: for projection, filter, aggregation, and join, assert through `EXPLAIN VIRTUAL` that the fragment reached PostgreSQL and through a row comparison against the mirror that it produced Exasol's answer. Cover the collation placements explicitly — `COLLATE "C"` present on an ordering comparison and on `MIN`/`MAX`, absent on equality, `LIKE`, the `GROUP BY` key and its select-list occurrence, and a `DISTINCT` operand — and assert the uncollated `GROUP BY` and `COUNT(DISTINCT)` return the mirror's group count. Assert a delegated `FULL OUTER JOIN` returns both unmatched rows with nulls; a select-list predicate over the grouped character column delegates and matches the mirror; `AVG` and `SUM` render in no generated statement and still return the mirror's values; a two-column `GROUP BY` is withdrawn; and no generated statement carries `ORDER BY`, `LIMIT`, or `OFFSET`.
- [ ] 8.3 Add the two-driver end-to-end test to `e2e_postgres_test.rs`, over the two virtual schemas task 8.2's setup created: assert both driver libraries are present in BucketFS at their distinct paths, and assert a query over each returns its own source's rows. Task 8.5 owns the capability half of the two-driver assertion and states its mechanism, so this task does not restate it. This is the test that would fail if package resolution or metadata discovery defaulted to one driver.
- [ ] 8.4 Add the PostgreSQL omission E2E test: `adapterNotes` records every unmapped column with its PostgreSQL type and reason under `sourceType`, the all-unmapped table is absent from `schemaMetadata`, and creation still succeeds. Add the adapter-side credential assertion in the same file: point the `CONNECTION` at a wrong password, and separately at a host and port nothing listens on, and assert that no error surfaced by `createVirtualSchema`, `refresh`, or `setProperties` contains the password or the libpq conninfo string, while each still names the host and the database.
- [ ] 8.5 Add `two_virtual_schemas_advertise_their_own_capability_sets` to `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs`, over the two virtual schemas task 8.2's setup module created — one `DRIVER = 'postgresql'`, one `DRIVER = 'mysql'`, both through the ordinary `CREATE VIRTUAL SCHEMA` path against the shipped `ADBC_ADAPTER`. Plan a `FULL OUTER JOIN` over each schema's own pair of joinable fixture tables and read the `PUSHDOWN_SQL` rows `EXPLAIN VIRTUAL` reports for it, the way `e2e_mysql_test.rs`'s `pushdown_statement` helper and `two_column_group_by_is_not_delegated` already read a delegation decision — reading every reported row rather than the first, because a withdrawn join leaves one pushed-down statement per scanned table. Assert the PostgreSQL schema's generated statement carries the delegated `FULL OUTER JOIN`, and that no statement generated for the MySQL schema carries a join at all, since Exasol withdraws a join the advertised set omits and performs it itself. What the two answers prove is the per-driver dispatch itself — `getCapabilities` resolving its dialect from `adapterNotes.driver`, live, through the shipped code path — which is strictly more than the presence of a JSON field. Add no probe adapter, no request-echo instrumentation to the shipped adapter, and no test-only code path through it. This is the standing regression test behind the plan's keystone precondition: gate 2.1 measures the request shape once, and a later loss of the identity on the `getCapabilities` request collapses this difference and MUST fail this test rather than be absorbed.

### Phase 9: Operational surface and mission

- [ ] 9.1 Add `test-e2e-postgres` to the `Makefile` — prereqs `cross-udf-build stack-up`, running `cargo test -p adbc-vs-adapter --features exasol-e2e --test e2e_postgres_test -- --test-threads=1` — as a second target beside `test-e2e` rather than folding both drivers into one binary. Two binaries keep each driver's shared setup independent, so a MySQL-only run provisions no PostgreSQL virtual schema and a failure names which driver's tier broke. Update `tests/make_targets_test.rs`.
- [ ] 9.2 Extend `.github/workflows/ci.yml`: run `make test-it-postgres` in `integration-tests` and the PostgreSQL end-to-end target in `e2e`, both as `make` invocations with no inline shell. Confirm the `e2e` job's disk cleanup still leaves room for a third image, and update `tests/workflow_test.rs` for the new target references.
- [ ] 9.3 Update `CLAUDE.md`'s dense bullets: the `postgres` service, its image tag, static IP `172.28.5.11`, and deliberate `en_US.utf8` locale; the `postgres-it` tier and what it does and does not cover; the pinned PostgreSQL driver version, library, and entry point, and the per-driver version-spelling trap; the PostgreSQL credential path and its redaction requirement; the `public`-schema-only boundary. Record the two consequences of task 1.2's `stack-up` edge in the stack-lifecycle and Benchmarking bullets: `make test-e2e` now starts a container it does not use, and `make bench` samples throughput on a host carrying a third database.
- [ ] 9.4 Update `specs/mission.md`: PostgreSQL moves from the aspirational v1 driver set to a shipped driver; the MySQL 8.0.17 source-server constraint becomes one driver's precondition beside PostgreSQL's UTF-8 encoding requirement; the `SqlDialect` row in the tech-stack table names both dialects; note that the credential-by-separate-option rule holds for MySQL and not for PostgreSQL.

### Phase 10: Review and verification

- [ ] 10.1 Code review of all changed files
- [ ] 10.2 Apply standard review fixes (if any)
- [ ] 10.3 Apply expert review fixes (if any)
- [ ] 10.4 Run the automated checklist (build, test, artifact, integration both drivers, e2e both drivers, lint, format, deny)
- [ ] 10.5 Scenario coverage audit
- [ ] 10.6 Manual verification steps
- [ ] 10.7 Generate verification report

## Parallelization

| Parallel Group | Tasks |
|----------------|-------|
| Group 0 | 2.2a → 2.2b |
| Group A | 1.1, 1.2 |
| Group B | 2.1, 2.4 |
| Group C | 2.3 |
| Group D | 3.1 → 3.2 → 3.3, and 3.5 after 3.1; 3.4 after 3.2; 3.6 after 3.1 |
| Group E | 4.1 → 4.2 → 4.3 → 4.4 → 4.5 |
| Group F | 5.1, 5.2 in parallel; then 5.3, 5.4 |
| Group G | 6.1 → 6.2 |
| Group H | 7.1, 7.2 |
| Group I | 8.1 → 8.2 → 8.3, 8.4, 8.5 |
| Group J | 9.1, 9.2, 9.3, 9.4 |
| Group K | 10.1 → 10.7 |

Sequential dependencies:

- Group 0 → every other group. The driver-load gate blocks the whole plan, and its failure is fixed in `language-container-rs` and returns through the release chain, so it runs first. It needs no PostgreSQL container: 2.2a needs HTTPS egress and BucketFS, and 2.2b needs `make cross-udf-build`, `make stack-up`, and the installed SLC. None of those is Group A's work
- Group 0 → Group B (gate 2.4 verifies the digest and records the spelling of the package gate 2.2a resolved)
- Group A → Group C (gate 2.3 is the only gate that needs the PostgreSQL container; gate 2.1 needs only Exasol and runs in Group B alongside it)
- Group B → Group D (gate 2.1 decides whether per-dialect capability sets are legal, so it precedes the resolution work; 2.4 supplies the digest and the version spelling task 6.1 needs)
- Group C → Groups E and H (gate 2.3 grounds every PostgreSQL rendering rule)
- Group D → Groups E, F, G, H — **the plan's main fan-out.** The driver-identity refactor is done once, then four workstreams run concurrently on disjoint files: the dialect renderer (`postgres_dialect.rs`), the metadata source and mapping (`postgres_source.rs`, `create_schema.rs`), the driver package row and provisioning tests (`driver.rs`, `provisioning_test.rs`), and the scan coercion and integration tier (`coerce.rs`, `postgres_it_test.rs`)
- Task 6.1 → task 7.2, the one edge crossing the fan-out: the integration tier fetches the PostgreSQL package through the row 6.1 adds, using task 3.6's helper. Task 7.1 needs nothing from Group G
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
| Constant | `adbc-vs-scan::driver`'s `DRIVER_VERSION`, `DRIVER_LIBRARY`, `PACKAGE_SHA256`, `REGISTRY_DRIVER_PATH` | Become fields of the MySQL `DriverPackage` row |
| Constant | `connection.rs::DRIVER_ENTRY_POINT` | Becomes a per-driver field; the scan reads it from the identity |
| Import | `dispatch.rs`'s `use crate::mysql_dialect::MySqlDialect` | No call site names a concrete dialect after resolution lands |
| Method | `metadata.rs::AdbcMetadataSource::discover_collations` as an inherent method | Becomes the MySQL implementation of the trait's precondition check |
| Call sites | `create_schema.rs`'s direct `type_mapping::map_column` calls | Mapping moves behind `MetadataSource`, which returns mapped columns |
| Field | `adapter_notes.rs::Omission::mysql_type` | Renamed to `source_type`; the old wire name survives only as a deserialization alias |
| Type alias | `MysqlConnection`, `open_mysql_connection` | Renamed to source-neutral names taking the driver identity |
| Tests | `create_schema_tests.rs` mapping assertions | Move to the per-driver mapping test modules |

## Verification

### Scenario Coverage

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
| Every driver identity has a complete set of tables | Unit | `crates/adbc-vs-adapter/src/driver_selection_tests.rs` | `every_identity_yields_every_table` |

`vs-adapter/pushdown-dialect`

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| Every advertised capability has a rendering path | Unit | `crates/adbc-vs-adapter/src/capabilities_tests.rs` | `every_advertised_capability_renders_per_dialect` |
| Capability set advertises the fragments this release renders | Unit | `crates/adbc-vs-adapter/src/capabilities_tests.rs` | `mysql_capability_set_is_exact` |
| PostgreSQL advertises its own capability set | Unit | `crates/adbc-vs-adapter/src/capabilities_tests.rs` | `postgres_capability_set_is_exact` |
| The two dialects' capability sets differ only where their SQL differs | Unit | `crates/adbc-vs-adapter/src/capabilities_tests.rs` | `dialect_sets_differ_only_in_full_outer_join` |
| Unrepresentable request fragment fails rather than returning wrong rows | Unit | `crates/adbc-vs-adapter/src/postgres_dialect_tests.rs` | `unrenderable_fragment_named_in_error` |
| Comparison operands are read by position, not by assumed role | Unit | `crates/adbc-vs-adapter/src/postgres_dialect_tests.rs` | `comparison_operands_keep_request_order` |
| Generated statement is deterministic for a given request | Unit | `crates/adbc-vs-adapter/src/capabilities_tests.rs` | `same_request_renders_identically_per_dialect` |

`vs-adapter/create-virtual-schema`

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| Creating a virtual schema exposes the source tables | Integration | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `creating_virtual_schema_exposes_source_tables` |
| Source identifiers keep their original case | Integration | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `source_identifiers_keep_their_case` |
| Views are exposed alongside base tables | Integration | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `views_are_exposed_alongside_base_tables` |
| Unsupported driver value fails with a named error | Integration | `crates/adbc-vs-adapter/tests/dispatch_test.rs` | `unsupported_driver_value_fails_named` |
| PostgreSQL driver value is accepted | Integration | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `postgresql_driver_value_is_accepted` |
| PostgreSQL virtual schema exposes the public schema only | Integration | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `only_the_public_schema_is_exposed` |
| PostgreSQL database whose encoding is not UTF8 is rejected at creation | Integration | `crates/adbc-vs-adapter/src/create_schema_tests.rs` | `non_utf8_encoding_is_rejected_named` |
| Connection address without a database fails with a named error | Unit | `crates/adbc-vs-adapter/src/create_schema_tests.rs` | `address_without_database_fails_for_each_scheme` |
| Adapter notes record the state pushdown needs and no credential | Integration | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `adapter_notes_record_state_and_no_credential` |
| Columns of unmappable type are omitted and recorded | Integration | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `unmappable_columns_are_omitted_and_recorded` |
| Table with no mappable column is omitted | Integration | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `table_with_no_mappable_column_is_omitted` |
| Empty source database yields an empty table list | Unit | `crates/adbc-vs-adapter/src/create_schema_tests.rs` | `empty_namespace_yields_empty_table_list` |
| Source server without the required collations is rejected at creation | Unit | `crates/adbc-vs-adapter/src/create_schema_tests.rs` | `mysql_collation_check_binds_mysql_only` |
| Unreachable source fails creation with a diagnosable message | Integration | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `unreachable_source_names_the_address` |
| PostgreSQL source error surfaces no credential | Integration | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `postgres_source_error_surfaces_no_credential` |

`vs-adapter/type-mapping`

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| PostgreSQL integer types map to DECIMAL with the matching digit count | Unit | `crates/adbc-vs-adapter/src/postgres_type_mapping_tests.rs` | `integer_types_map_to_decimal` |
| PostgreSQL numeric preserves the source precision and scale | Unit | `crates/adbc-vs-adapter/src/postgres_type_mapping_tests.rs` | `numeric_preserves_precision_and_scale` |
| PostgreSQL numeric without a declared precision is unmapped | Unit | `crates/adbc-vs-adapter/src/postgres_type_mapping_tests.rs` | `unconstrained_numeric_is_unmapped` |
| PostgreSQL character varying preserves the declared length and uses UTF8 | Unit | `crates/adbc-vs-adapter/src/postgres_type_mapping_tests.rs` | `character_varying_preserves_length` |
| PostgreSQL character column of a nondeterministic collation is unmapped | Unit | `crates/adbc-vs-adapter/src/postgres_type_mapping_tests.rs` | `nondeterministic_collation_is_unmapped` |
| PostgreSQL character is unmapped because it is blank-padded | Unit | `crates/adbc-vs-adapter/src/postgres_type_mapping_tests.rs` | `bpchar_is_unmapped_for_padding` |
| PostgreSQL temporal and floating-point types map to their Exasol counterparts | Unit | `crates/adbc-vs-adapter/src/postgres_type_mapping_tests.rs` | `temporal_and_float_types_map` |
| Non-core PostgreSQL types are unmapped | Unit | `crates/adbc-vs-adapter/src/postgres_type_mapping_tests.rs` | `non_core_types_are_unmapped_named` |
| PostgreSQL column nullability carries through unchanged | Unit | `crates/adbc-vs-adapter/src/postgres_type_mapping_tests.rs` | `nullability_carries_through` |
| Omission records the source type under a source-neutral field | Unit | `crates/adbc-vs-adapter/src/adapter_notes_tests.rs` | `omission_source_type_accepts_legacy_alias` |

`vs-adapter/pushdown-projection`

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| Absent select list projects every mapped column | Unit | `crates/adbc-vs-adapter/src/postgres_dialect_tests.rs` | `absent_select_list_projects_every_column` |
| Backtick in an identifier is escaped | Unit | `crates/adbc-vs-adapter/src/mysql_dialect_tests.rs` | `backtick_in_identifier_is_doubled` |
| Double quote in a PostgreSQL identifier is escaped | Unit | `crates/adbc-vs-adapter/src/postgres_dialect_tests.rs` | `double_quote_in_identifier_is_doubled` |
| Pushdown response carries no credential | Integration | `crates/adbc-vs-adapter/tests/dispatch_test.rs` | `pushdown_response_carries_no_credential` |
| Projection pushdown reaches PostgreSQL rather than Exasol | Integration | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `projection_reaches_postgres` |

`vs-adapter/pushdown-filter`

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| Filter capabilities cover the predicates and literals this release renders | Unit | `crates/adbc-vs-adapter/src/capabilities_tests.rs` | `filter_capabilities_match_per_dialect` |
| PostgreSQL collates an ordering comparison and leaves equality bare | Unit | `crates/adbc-vs-adapter/src/postgres_dialect_tests.rs` | `collation_placement_matrix` |
| PostgreSQL character comparison reproduces Exasol's case, accent, and code-point order | Integration | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `character_comparison_matches_local_exasol_result` |
| PostgreSQL string literal is rendered as an escape-string constant | Unit | `crates/adbc-vs-adapter/src/postgres_dialect_tests.rs` | `string_literal_is_escape_string_constant` |
| PostgreSQL renders LIKE, IS NULL, BETWEEN, and IN in their standard forms | Unit | `crates/adbc-vs-adapter/src/postgres_dialect_tests.rs` | `standard_predicate_forms_render` |
| Filter pushdown reaches PostgreSQL rather than Exasol | Integration | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `filter_reaches_postgres` |

`vs-adapter/pushdown-aggregation`

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| Aggregation capabilities cover single-group and column grouping | Unit | `crates/adbc-vs-adapter/src/capabilities_tests.rs` | `aggregation_capabilities_match_per_dialect` |
| PostgreSQL renders MIN and MAX over a character column with the C collation | Integration | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `min_max_reproduce_code_point_order` |
| PostgreSQL renders a grouping key and a DISTINCT operand without a collation | Unit | `crates/adbc-vs-adapter/src/postgres_dialect_tests.rs` | `grouping_and_distinct_render_uncollated` |
| Delegated PostgreSQL grouping and DISTINCT over case-only variants return Exasol's groups | Integration | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `grouping_and_distinct_match_local_exasol_result` |
| Select-list predicate over the grouped character column fails with a named error | Unit | `crates/adbc-vs-adapter/src/mysql_dialect_tests.rs` | `grouped_character_predicate_fails_mysql_only` |
| PostgreSQL delegates a select-list predicate over its grouped character column | Integration | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `grouped_character_predicate_delegates_on_postgres` |
| Aggregate value that overflows its declared type fails rather than truncating | Unit | `crates/adbc-vs-scan/src/coerce_tests.rs` | `overflowing_aggregate_fails_named` |
| Aggregation pushdown reaches PostgreSQL rather than Exasol | Integration | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `aggregation_matches_local_exasol_result` |

`vs-adapter/pushdown-join`

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| Join capabilities cover inner and one-sided outer joins only | Unit | `crates/adbc-vs-adapter/src/capabilities_tests.rs` | `mysql_join_capabilities_are_exact` |
| PostgreSQL join capabilities add the full outer join | Unit | `crates/adbc-vs-adapter/src/capabilities_tests.rs` | `postgres_join_capabilities_add_full_outer` |
| PostgreSQL renders and executes a delegated full outer join | Integration | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `full_outer_join_matches_local_exasol_result` |
| Each join type renders its counterpart | Unit | `crates/adbc-vs-adapter/src/postgres_dialect_tests.rs` | `each_join_type_renders` |
| Unsupported join type fails with a named error | Unit | `crates/adbc-vs-adapter/src/postgres_dialect_tests.rs` | `unsupported_join_type_named_per_dialect` |
| Inner join of two virtual tables renders one statement | Unit | `crates/adbc-vs-adapter/src/postgres_dialect_tests.rs` | `inner_join_renders_one_statement` |
| Outer join emits nulls for the unmatched side | Integration | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `outer_join_emits_nulls_for_unmatched_side` |
| Join composes with a delegated filter and aggregation | Unit | `crates/adbc-vs-adapter/src/postgres_dialect_tests.rs` | `join_composes_with_filter_and_grouping` |
| Every joined table is aliased and every column is qualified | Unit | `crates/adbc-vs-adapter/src/postgres_dialect_tests.rs` | `joined_tables_are_aliased_and_qualified` |

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

`driver-provisioning/driver-self-install`

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| Already-installed driver short-circuits the fetch | Integration | `crates/adbc-vs-adapter/tests/provisioning_test.rs` | `installed_driver_short_circuits_per_driver` |
| Missing driver is fetched, verified, and installed | Integration | `crates/adbc-vs-adapter/tests/provisioning_test.rs` | `missing_driver_is_fetched_per_driver` |
| Pinned version strings are matched exactly as the registry spells them | Unit | `crates/adbc-vs-adapter/src/registry_index_tests.rs` | `pinned_versions_match_registry_spellings` |
| Two drivers install side by side without collision | Integration | `crates/adbc-vs-adapter/tests/provisioning_test.rs` | `two_drivers_install_side_by_side` |
| Archive manifest is checked against the compiled-in filename | Integration | `crates/adbc-vs-adapter/tests/provisioning_test.rs` | `manifest_filename_checked_per_driver` |
| Checksum mismatch aborts the install | Integration | `crates/adbc-vs-adapter/tests/provisioning_test.rs` | `digest_mismatch_names_driver_and_digests` |
| Unreachable registry names the offline fallback | Integration | `crates/adbc-vs-adapter/tests/provisioning_test.rs` | `offline_fallback_names_the_drivers_own_path` |
| Missing platform package fails with a named error | Integration | `crates/adbc-vs-adapter/tests/provisioning_test.rs` | `missing_platform_names_driver_and_version` |
| Release artifact bundles no driver binary | Integration | `crates/adbc-vs-adapter/tests/artifact_test.rs` | `release_artifact_bundles_no_driver_binary` |

`scan-udf/arrow-scan`

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| Scan connects using credentials resolved at run time | Unit | `crates/adbc-vs-scan/src/connection_tests.rs` | `credential_options_differ_per_driver` |
| PostgreSQL connection string is built from the address and escapes every credential byte | Unit | `crates/adbc-vs-scan/src/connection_tests.rs` | `conninfo_escapes_every_credential_byte` |
| PostgreSQL credential never appears in any surfaced text | Integration | `crates/adbc-vs-scan/tests/postgres_it_test.rs` | `credential_never_appears_in_surfaced_text` |
| Provisioned driver loads inside the UDF sandbox | Integration | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `provisioned_driver_loads_in_udf_sandbox` |
| Decimal width variation is normalized | Unit | `crates/adbc-vs-scan/src/coerce_tests.rs` | `decimal_width_variation_is_normalized` |
| Non-finite PostgreSQL numeric fails with a named error | Unit | `crates/adbc-vs-scan/src/coerce_tests.rs` | `non_finite_numeric_fails_named` |
| Timestamp unit variation is normalized | Unit | `crates/adbc-vs-scan/src/coerce_tests.rs` | `timestamp_unit_variation_is_normalized` |
| Source failure surfaces the driver's own diagnosis | Integration | `crates/adbc-vs-scan/tests/postgres_it_test.rs` | `source_failure_surfaces_adbc_status` |
| Missing driver library fails with a named error | Integration | `crates/adbc-vs-scan/tests/postgres_it_test.rs` | `missing_driver_library_names_path_and_driver` |
| Scan streams every result batch to Exasol | Integration | `crates/adbc-vs-scan/tests/postgres_it_test.rs` | `scan_streams_every_batch` |
| Declared BOOLEAN column emits every representation the driver returns | Unit | `crates/adbc-vs-scan/src/coerce_tests.rs` | `declared_boolean_accepts_bool_and_integer` |

### Manual Testing

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| driver-selection | `exapump sql --dsn "$EXASOL_DSN" "CREATE VIRTUAL SCHEMA pgvs USING ADBC_VS.ADBC_ADAPTER WITH DRIVER = 'snowflake' CONNECTION_NAME = 'PG_CONN' BUCKETFS_CONNECTION = 'BFS_CONN'"` | Fails naming `snowflake` and listing `mysql` and `postgresql` |
| create-virtual-schema | `exapump sql --dsn "$EXASOL_DSN" "CREATE VIRTUAL SCHEMA pgvs USING ADBC_VS.ADBC_ADAPTER WITH DRIVER = 'postgresql' CONNECTION_NAME = 'PG_CONN' BUCKETFS_CONNECTION = 'BFS_CONN'"` | Succeeds; `SELECT * FROM SYS.EXA_ALL_VIRTUAL_TABLES WHERE TABLE_SCHEMA='PGVS'` lists the `public` tables |
| driver-self-install | `exapump bfs ls --dsn "$BFS_DSN" /adbc-drivers/` | Lists both `libadbc_driver_mysql.so` and `libadbc_driver_postgresql.so` |
| pushdown-dialect | `exapump sql --dsn "$EXASOL_DSN" "SELECT PUSHDOWN_SQL FROM (EXPLAIN VIRTUAL SELECT * FROM pgvs.\"orders\" o FULL OUTER JOIN pgvs.\"customers\" c ON o.\"cid\" = c.\"id\")"`, then the same shape over the MySQL virtual schema | The PostgreSQL schema's generated statement carries `FULL OUTER JOIN`; the MySQL schema's carries no join, so its two table scans are pushed down separately |
| pushdown-projection | `exapump sql --dsn "$EXASOL_DSN" "EXPLAIN VIRTUAL SELECT \"id\", \"note\" FROM pgvs.\"orders\""` | Generated statement projects only `"id"` and `"note"`, every identifier double-quoted |
| pushdown-filter | `exapump sql --dsn "$EXASOL_DSN" "EXPLAIN VIRTUAL SELECT \"id\" FROM pgvs.\"orders\" WHERE \"note\" < 'm'"` | `WHERE` clause carries `COLLATE "C"` on the column operand and an `E'...'` literal |
| pushdown-filter | `exapump sql --dsn "$EXASOL_DSN" "EXPLAIN VIRTUAL SELECT \"id\" FROM pgvs.\"orders\" WHERE \"note\" = 'Widget'"` | `WHERE` clause carries no collation suffix |
| pushdown-aggregation | `exapump sql --dsn "$EXASOL_DSN" "SELECT \"note\", COUNT(*) FROM pgvs.\"orders\" GROUP BY \"note\""` | One group per distinct value including case-only variants; generated `GROUP BY` carries no collation suffix |
| pushdown-join | `exapump sql --dsn "$EXASOL_DSN" "EXPLAIN VIRTUAL SELECT * FROM pgvs.\"orders\" o FULL OUTER JOIN pgvs.\"customers\" c ON o.\"cid\" = c.\"id\""` | Generated statement renders `FULL OUTER JOIN` |
| result-ordering | `exapump sql --dsn "$EXASOL_DSN" "EXPLAIN VIRTUAL SELECT \"id\" FROM pgvs.\"orders\" ORDER BY \"id\" LIMIT 5"` | Generated statement carries no `ORDER BY`, `LIMIT`, or `OFFSET` |
| type-mapping | `exapump sql --dsn "$EXASOL_DSN" "SELECT ADAPTER_NOTES FROM SYS.EXA_ALL_VIRTUAL_SCHEMAS WHERE SCHEMA_NAME='PGVS'"` | `omitted` lists the `boolean`, `text`, `character`, `jsonb`, and unconstrained-`numeric` columns, each under `sourceType` with a reason |
| arrow-scan | `exapump sql --dsn "$EXASOL_DSN" "SELECT \"amount\" FROM pgvs.\"orders\""` | Returns the seeded `numeric(12,4)` values with scale intact, having arrived from the driver as strings |
| arrow-scan | Point `PG_CONN` at a wrong password, then `exapump sql --dsn "$EXASOL_DSN" "SELECT * FROM pgvs.\"orders\""` | Fails naming the host and database, with the password absent from the message |
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

Recording this plan trips `/speq:spec-merge`'s more-than-10-scenarios-per-spec threshold on `vs-adapter/pushdown-aggregation`, `vs-adapter/type-mapping`, and `vs-adapter/create-virtual-schema`, so `/speq:record` halts and returns for a decision before archiving. Whether to split those specs is a user decision at record time, not an implementation task in this plan.
