# Decision Log: add-exasol-technology

## Interview

No live interview took place. This plan ran headless, so the record below is the brief's stated intent followed by the four research spikes it asked for, each with the answer the research returned.

**Q:** What is the feature?
**A:** Add "exasol" as a new ADBC driver technology to adbc-vs — an Exasol-to-Exasol Virtual Schema using `exarrow-rs` as the ADBC backend, following the project's existing per-technology pattern.

**Q:** What is the goal?
**A:** Outperform Exasol's own Lua-based Exasol-to-Exasol Virtual Schema on throughput and latency for cross-cluster or intra-Exasol federated queries, by using `exarrow-rs`'s native protocol and Arrow batches end to end instead of the Lua adapter's row-by-row path.

**Q (spike 1):** Does `exarrow-rs`'s ADBC surface support connecting to Exasol cleanly — connection options, auth, TLS?
**A:** Yes, and it is reachable without a Cargo dependency. The crate builds a `cdylib` under its `ffi` feature, and the ADBC Driver Foundry publishes that build to the `dbc` registry under the driver path `exasol`, newest version `v0.13.0`. The `linux_amd64` package installs `libadbc_driver_exasol.so`, its `MANIFEST` declares ADBC `v1.1.0`, and `nm` shows exactly one exported ADBC init symbol, `AdbcDriverInit`. It links `libgcc_s`, `librt`, `libpthread`, `libm`, `libdl`, and libc, and needs at most `GLIBC_2.16`. It accepts `username` and `password` as ADBC database options distinct from `uri`. Auth is user and password only — no token, OpenID, or Kerberos path exists. Configuration beyond the credential travels in the address's query string; the `adbc.exasol.*` option namespace is stored and never read. `validateservercertificate` is accepted as an alias, and `certificate_fingerprint` pins a lower-case hex SHA-256 digest of the DER certificate. Two limits matter: the driver's ADBC `execute` collects every page into one vector before returning a reader, and it reports one Arrow schema for an empty result and another for a non-empty one over the same column.

**Q (spike 2):** How does the existing Exasol-to-Exasol Virtual Schema actually move data, so the comparison is honestly framed?
**A:** It does not move data at all. Both public adapters — the Java `exasol-virtual-schema` and the Lua `exasol-virtual-schema-lua` — answer a `pushdown` request with a SQL string that the Exasol engine then executes. The Lua `ImportQueryBuilder` defaults its source type to `EXA` and renders `IMPORT INTO (...) FROM EXA AT "<connection>" STATEMENT '<select>'`; the same-database variant renders a plain local `SELECT`. Exasol documents that `IMPORT ... FROM EXA` "is always parallelized" across cluster nodes over the existing database port. So the reference path is the native bulk loader, not a row-by-row Lua loop, and the premise behind the throughput goal is false. Two documented defects of that path are real openings: the `EXA` and `JDBC` variants advertise `ORDER_BY_COLUMN` and `LIMIT` although "the outermost order of the imported result rows is not guaranteed", and the same route maps `HASHTYPE` to `VARCHAR` and then fails a join on incomparable types.

**Q (spike 3):** Are there type-mapping or collation gaps unique to Exasol as a source?
**A:** Collation collapses to nothing and type mapping shrinks, but neither becomes free. Exasol documents that it does not support SQL collation and compares strings by binary character value; measured on Exasol 2025.2.1, `SELECT CAST('a' AS VARCHAR(10)) COLLATE "C"` is refused with a syntax error, `'a' = 'A'` is false, and `LEAST('B','a','C')` returns `B`. So the dialect renders no suffix, and rendering one would fail the source. Type mapping is near-identity and the catalog normalizes aliases — measured, `INTEGER` reports `DECIMAL(18,0)` and `FLOAT` reports `DOUBLE` — but three real gaps remain, and each is this adapter's own bound rather than a source disagreement: `CHAR` is blank-padded where the `VARCHAR` this adapter declares is not, `TIMESTAMP(p)` above precision 3 has no declarable counterpart, and `BOOLEAN`, `GEOMETRY`, `HASHTYPE`, and both `INTERVAL` types fall outside the framework's closed five-type set.

**Q (spike 4):** Does this fit as a normal ADBC technology or does it need new architecture?
**A:** A normal technology, with no new architecture. The registry publishes the package, so the existing self-install path serves it; the DSN scheme is `exasol://`, which is exactly what `check_connection_scheme` already requires; the path component names a schema, which is Exasol's own SQL qualifier; and the credential shape matches MySQL's. Nothing in adbc-vs's manifests collides with `exarrow-rs`, because nothing links it — the driver is a run-time `.so`, so its `aws-lc-sys`, `tokio`, and unpatched Thrift advisory stay outside adbc-vs's Cargo graph and `deny.toml` is untouched.

## Design Decisions

### [1] The driver is the registry-published shared library, not a compiled-in Rust dependency

- **Decision:** Add `exasol` as an ordinary technology whose driver is `libadbc_driver_exasol.so`, resolved from the `dbc` registry and loaded through `ManagedDriver::load_dynamic_from_filename`. Do not depend on the `exarrow-rs` crate.
- **Alternatives:** Add `exarrow-rs` to `adbc-vs-scan`'s `[dependencies]` and drive it through its Rust-native or `adbc_core`-trait surface. Rejected: it would pull `aws-lc-sys`, `tokio`, `rustls`, and an unpatched Apache Thrift advisory into adbc-vs's own dependency graph, forcing a `deny.toml` exception and a licence review that the run-time `.so` route avoids entirely. It would also force `scan.rs`, `connection.rs`, and `metadata.rs` off the concrete `ManagedConnection` type they are written against, a cross-file refactor with no behavioral payoff. Also considered and rejected: requiring `DRIVER_PATH` and no registry entry, which is unnecessary now that the registry publishes the package, and bundling the driver in adbc-vs's release artifact, which `specs/mission.md` forbids.
- **Rationale:** Every per-driver fact the registry route needs is a value in a TOML row or an arm in an existing `match`. The technology therefore adds four features and no architecture, which is what `CLAUDE.md`'s per-technology convention describes.
- **Promotes to ADR:** yes

### [2] The requested throughput claim is dropped; ordering and type fidelity are the claim instead

- **Decision:** Do not state, plan, or measure a throughput or latency win over either Exasol-to-Exasol Virtual Schema adapter. State the feature's value as stable result ordering, honest type mapping, and one adapter reaching Exasol the way it reaches every other ADBC source. Correct `specs/mission.md`'s Problem Statement, which currently says Exasol's existing Virtual Schema dialects "move data through JDBC's row-by-row `ResultSet` cursor".
- **Alternatives:** Keep the goal and plan a benchmark to demonstrate it. Rejected on evidence: the reference adapters return an `IMPORT ... FROM EXA ... STATEMENT` string that Exasol's own documentation says is always parallelized across cluster nodes, and the same-instance Lua variant returns a plain local `SELECT` that moves nothing at all. adbc-vs moves the same rows through one UDF instance over one connection, with a driver that buffers the whole result first. Also considered: narrowing the claim to adapter-call latency. Rejected: the Lua adapter runs with no container and states a sub-50ms overhead goal, so even that margin is not available.
- **Rationale:** `CLAUDE.md` already carries the same framing correction for the MySQL benchmark, where the JDBC adapter plans and ExaLoader transfers. Repeating a refuted claim for Exasol would put a false statement in the mission that later work would build on.
- **Promotes to ADR:** yes

### [3] The throughput measurement is a follow-up plan, not part of this one

- **Decision:** Ship no `make bench-exasol` target. Record the measurement's design in this plan's Dependencies as a follow-up: compare adbc-vs against `exasol-virtual-schema-lua` and against a plain local `SELECT`, framed the way `make bench` is framed today.
- **Alternatives:** Add the benchmark beside `make bench` in this plan. Rejected as scope: the plan already adds four features, two test tiers, two CI jobs, and a compose change, and a measurement harness whose expected result is a loss is worth its own review rather than a footnote in a feature plan.
- **Rationale:** Decision [2] already records what the measurement would show and why. Separating them keeps the technology reviewable on its own terms.
- **Promotes to ADR:** no

### [4] `entry_point` is the unqualified `AdbcDriverInit`

- **Decision:** Declare `entry_point = "AdbcDriverInit"` in `drivers/exasol.toml`.
- **Alternatives:** `AdbcDriverExasolInit`, the name the `exarrow-rs` crate exports from `0.15.1`, or `ExarrowDriverInit`, its backward-compatible alias. Both rejected: `nm -D --defined-only` over the published `v0.13.0` `linux_amd64` package shows exactly one global text symbol, and it is `AdbcDriverInit`. Also rejected: letting the driver manager derive the symbol from the filename, which would look for an Exasol-qualified name that does not exist.
- **Rationale:** This is the case that proves why `scan-udf/arrow-scan` requires a named entry point: the library is `libadbc_driver_exasol.so` and its symbol is unqualified, so the filename heuristic would fail here even though it happens to work for the other two drivers.
- **Promotes to ADR:** no

### [5] `version_range` is `>=0.13.0, <0.14.0`

- **Decision:** Accept only `0.13.x`.
- **Alternatives:** Admit every `0.x`. Rejected: `0.14.0` is a breaking release nothing has measured, and the exported init symbol changed after it, so a range crossing it would resolve to a package this declaration's `entry_point` cannot load. Also rejected: pinning `=0.13.0`, which `driver-provisioning/driver-version-resolution` replaced with ranges for every driver.
- **Rationale:** `0.13.0` is the newest version the registry publishes and it carries the fix that makes a zero-row result report its schema, which this scan depends on. `make test-registry-live` catches the range going stale.
- **Promotes to ADR:** no

### [6] Source and target are the same Exasol instance; cross-cluster federation is out of scope

- **Decision:** Test against one `exasol/docker-db` container acting as both the querying database and the source. Give the compose service a static address on the `adbc-vs` network so a UDF sandbox can dial the database it runs inside. Declare cross-cluster federation between two separate instances a Non-Goal.
- **Alternatives:** Add a second `exasol/docker-db` service as the source. Rejected: `CLAUDE.md` records that a GitHub runner has roughly 21GB free and already exhausts it with Rust, Docker, and three container images, and each Exasol service needs `privileged: true` and a 2GB `/dev/shm`. Also rejected: addressing the database by a compose service name, which `CLAUDE.md` records does not resolve from inside a UDF sandbox because `exaconf` rewrites `/etc/hosts`.
- **Rationale:** Intra-instance federation is a first-class case in the brief, and nothing in the adapter distinguishes a source that happens to be the same instance. Declaring cross-cluster out of scope is honest about what the tests cover rather than a claim the stack cannot support.
- **Promotes to ADR:** yes

### [7] Exasol `CHAR` is omitted to hold the VARCHAR-only invariant

- **Decision:** Omit every `CHAR(n)` source column, with a reason naming the blank-padded comparison and the invariant.
- **Alternatives:** Map it to Exasol `VARCHAR`. Rejected on measurement: two `CHAR(10)` values holding `ab` and `ab ` compare equal on Exasol 2025.2.1 while two `VARCHAR(10)` values holding the same two do not, so a delegated equality would match rows the querying Exasol rejects. Map it to Exasol `CHAR`, which would round-trip exactly. Rejected: four features state that every delegated character operand is Exasol `VARCHAR`, and `vs-adapter/type-mapping` closes the mapped set against `CHAR`; reopening that for one technology is a framework change, not a local mapping choice.
- **Rationale:** This omission costs more than any other in the plan, because `CHAR` is common in real Exasol schemas and its values would survive intact. The cost is the invariant's, and naming it as such is what lets a later release weigh lifting the invariant instead of re-deriving why the column is missing.
- **Promotes to ADR:** yes

### [8] `TIMESTAMP` above precision 3 is omitted rather than truncated

- **Decision:** Map `TIMESTAMP`, `TIMESTAMP(0)`, and `TIMESTAMP(3)`; omit `TIMESTAMP(4)` through `TIMESTAMP(9)` with a reason naming both precisions. Omit `TIMESTAMP WITH LOCAL TIME ZONE` at every precision.
- **Alternatives:** Map every precision to a bare `TIMESTAMP`, which is what the adapter can declare. Rejected: `ExasolType::Timestamp` renders without a precision, so Exasol's default of 3 applies and the fourth digit onward is lost with no error — the outcome `vs-adapter/type-mapping`'s decimal rule refuses for a numeric. Also considered: widening the declarable set with a precision-carrying `TIMESTAMP`. Rejected as a framework change outside this plan's scope.
- **Rationale:** An Exasol-to-Exasol federation is where a user most reasonably expects an exact round trip, so a silent truncation is least acceptable here. The measured catalog spells the precision — `TIMESTAMP(3)` and `TIMESTAMP(9)` both occur in practice — so the mapping can see it and refuse.
- **Promotes to ADR:** yes

### [9] Bounded scan memory becomes a per-technology claim rather than a framework guarantee

- **Decision:** Change `scan-udf/arrow-scan` so it requires only that the scan release each batch before requesting the next, and leave the end-to-end peak-memory claim to each technology's own feature. `scan-udf/exasol-scan` states that this driver buffers the whole result and that a result exceeding the UDF's memory limit fails rather than streams.
- **Alternatives:** Keep the framework guarantee and claim it for Exasol too. Rejected as false: the driver's ADBC `execute` fetches every page, collects them into one vector, and returns a reader over that vector, so no discipline in the scan can bound peak memory. Also considered: window the scan with `LIMIT`/`OFFSET` to bound it. Rejected: `vs-adapter/result-ordering` pushes down no ordering, and an offset without a stable order returns wrong rows.
- **Rationale:** The framework rule was stated as an end-to-end property because both existing drivers stream. Splitting it keeps the rule true for them and makes the exception visible rather than quietly violated.
- **Promotes to ADR:** yes

### [10] Source-certificate handling is operator-set through the connection address

- **Decision:** Pass the address verbatim as the `uri` option, query string included, and add no adapter property for certificate handling. Document `certificate_fingerprint=<lower-case hex>` as the production route and `validateservercertificate=0` as the test-stack route, and require one of the two.
- **Alternatives:** Default to `validateservercertificate=0` inside the adapter. Rejected: it would silently disable certificate verification for every Exasol source, and the operator would have no way to see it. Add a dedicated adapter property. Rejected: the driver already reads both parameters from the address, so a property would be a second way to say the same thing.
- **Rationale:** The driver validates against the operating system's trust store and `CLAUDE.md` records that the UDF sandbox guarantees no CA bundle, so the driver's own default cannot succeed. Making the address the only place that decides keeps the choice visible in the `CONNECTION` object an operator wrote. The fingerprint verifier replaces hostname and chain verification rather than adding to it, which the spec states so that a re-pin after a certificate rotation is expected rather than a surprise.
- **Promotes to ADR:** yes

### [11] A credential byte the driver's DSN round trip cannot carry is refused, not pre-encoded

- **Decision:** Pass the credential as separate `username` and `password` ADBC options. Measure which bytes survive the driver's internal DSN round trip and refuse the rest at connect time with an error naming the character and the cause.
- **Alternatives:** Percent-encode the credential so the driver's URL-decode restores it. Rejected: it hard-codes the shape of an upstream defect, and a driver release that stopped decoding would silently break every password. Pass it through unchanged. Rejected: the driver splices both options into an `exasol://user:password@host` string unescaped and then URL-decodes what it parses back, so a password containing `%41` reaches the source as `A` and the operator sees an authentication failure with no diagnosis.
- **Rationale:** Refusing converts a silent wrong-credential failure into a named one, and the measurement rather than the DSN grammar decides the refused set, because which bytes survive depends on the order the driver's parser splits on `@`, `/`, and `?`. The upstream defect is recorded in the plan's Dependencies so `exarrow-rs` can fix it and the refusal can narrow.
- **Promotes to ADR:** yes

### [12] The source precondition is a schema check that also refuses a virtual schema

- **Decision:** Check `SYS.EXA_ALL_SCHEMAS` before discovering any table, and on every request that resolves the source `CONNECTION`. Refuse a schema that does not exist, is invisible, or reports `SCHEMA_IS_VIRTUAL`.
- **Alternatives:** Check only that the schema exists, and filter virtual tables out of discovery instead. Rejected as shallower: every table of a virtual schema is virtual, so the schema-level check answers the whole concern in one place and gives a clear error rather than an empty virtual schema. Also considered: no precondition at all, on the grounds that an Exasol source needs no collation floor or encoding check. Rejected: an Exasol source can be the same instance the adapter runs in, so a virtual schema named as its own source would re-enter the adapter for every row.
- **Rationale:** `vs-adapter/create-virtual-schema` makes the precondition each technology's own to state. This is the only one of the three whose source can be the adapter's own database, so it is the only one that needs a self-reference guard.
- **Promotes to ADR:** yes

### [13] `VARCHAR ASCII` is declared `UTF8`, and the schema name is matched verbatim

- **Decision:** Map a `VARCHAR(n) ASCII` source column to Exasol `VARCHAR(n) UTF8` rather than omitting it. Compare the address's schema name against `SCHEMA_NAME` exactly as written, without folding case. Discover tables from `SYS.EXA_ALL_TABLES`, so views are never discovered.
- **Alternatives:** Omit `ASCII` columns as a character-set mismatch. Rejected: `ASCII` is a subset of `UTF8`, `n` counts characters under both, and the subset's ordering is identical, so nothing is lost and `exa_type.rs` declares `UTF8` for every `VARCHAR` anyway. Upper-case the schema name to match Exasol's identifier folding. Rejected: a schema created under a quoted lower-case name would become unreachable. Discover views alongside tables. Rejected: `EXA_ALL_COLUMNS` reports null nullability for a view's column, and `vs-adapter/type-mapping` requires the catalog's own flag.
- **Rationale:** Each of the three is the choice that loses no information and needs no new mechanism.
- **Promotes to ADR:** no

### [14] A changed feature's Background lands by task, and its delta file states the change rather than restating the whole section

- **Decision:** In each of the six changed features' delta files, carry the Background changes as bullets under a `## Background` section that says the permanent feature's own Background governs, and mark only scenarios with delta markers. Land those bullets in the permanent files through task 6.1.
- **Alternatives:** Restate each permanent feature's full Background in its delta file. Rejected: `/speq:spec-merge` merges an existing feature by scenario name and reads nothing outside `## Scenarios`, so the restated section would be discarded at merge while adding hundreds of duplicated lines for a reviewer to diff. Omit the section entirely. Rejected: `speq plan validate` requires a feature description and a `## Background` heading in every delta file, changed features included. Leave the Background stale. Rejected: eight framework specs name the supported technologies in prose, and prose naming two of three is wrong rather than merely incomplete.
- **Rationale:** The merge procedure decides what a delta can carry and the validator decides what it must carry, and the two disagree for a changed feature. Stating the change and naming the task that applies it satisfies both, and keeps the delta file a readable statement of what moved.
- **Promotes to ADR:** yes

## Review Findings
