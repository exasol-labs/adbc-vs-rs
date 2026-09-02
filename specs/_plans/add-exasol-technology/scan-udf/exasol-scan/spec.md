# Feature: exasol_scan

States how the scan reaches an Exasol source, passes its credentials, has its source certificate checked, and what this driver's fully-buffered result costs.

## Background

- This is the mechanism `scan-udf/arrow-scan`'s framework invariants are satisfied by for Exasol. That feature requires the credential to be resolved from the named Exasol `CONNECTION` object at scan time and never persisted; this feature states where it goes from there, and states the one framework guarantee this driver cannot give.
- The driver is `libadbc_driver_exasol.so`, the ADBC Driver Foundry's packaging of the `exarrow-rs` crate, loaded through the entry point `AdbcDriverInit` — the only ADBC init symbol the published package exports. The library links `libgcc_s.so.1`, `librt`, `libpthread`, `libm`, `libdl`, and libc and nothing else, so the Script Language Container supplies every dependency it has; it links no `libstdc++`, unlike the PostgreSQL driver.
- The driver accepts `username` and `password` as ADBC database options distinct from `uri`, so the scan sets three options and no credential is written into a value the adapter builds. That is `scan-udf/mysql-scan`'s shape rather than `scan-udf/postgresql-scan`'s, and it means the `uri` value is safe to name as the subject of a connection error.
- The structural guarantee MySQL's shape gives does not carry over, because this driver re-parses a connection string of its own. It splices the two credential options into an `exasol://user:password@host` DSN without escaping them, then parses that DSN back and URL-decodes the user and password it finds. A credential byte that does not survive that round trip therefore reaches the source altered. A percent sign cannot survive it, because the decode step is unconditional: a password containing `%41` is offered to the source as `A`.
- The scan refuses such a credential rather than offering an altered one. Refusing converts a confusing authentication failure into a named diagnosis, and it is preferred over pre-encoding the credential to match the driver's decode step, because pre-encoding would silently double-encode against a driver release that fixed the defect. The refused set is established by measurement against the driver rather than reasoned from the DSN grammar, because which bytes survive depends on the order the driver's parser splits on `@`, `/`, and `?`.
- The address is passed through as the `uri` option verbatim, query string included, so every driver parameter an operator wrote into the `CONNECTION` address reaches the driver unchanged. That is what makes source-certificate handling an operator decision needing no adapter code: `validateservercertificate=0` accepts any certificate, and `certificate_fingerprint=<hex>` pins one.
- Certificate validation cannot be left at the driver's default inside the UDF sandbox. The driver validates against the operating system's trust store, and the Script Language Container ships no CA bundle, so the default fails for every source. One of the two parameters above MUST therefore appear in the address. `certificate_fingerprint` is the route for a source whose certificate is self-signed in production, and `validateservercertificate=0` is the route for a test stack; the fingerprint is the SHA-256 digest of the source's DER certificate as lower-case hex with no separators, and the driver compares it as an exact string, so an upper-case spelling is refused.
- Pinning a fingerprint replaces the chain check rather than adding to it. The driver's fingerprint verifier bypasses hostname verification and CA-chain verification entirely, so a pinned fingerprint is the whole of the source's identity check and has to be re-pinned when the source's certificate is replaced.
- The scan cannot rely on the driver's session schema. The driver stores the ADBC `CurrentSchema` connection option without acting on it, and treats the DSN path component's schema activation as best effort, so a statement naming an unqualified table can resolve against no schema. Every table in the generated statement is therefore qualified with the recorded source schema, which `vs-adapter/exasol-pushdown` renders.
- This driver buffers the whole result set before the scan sees its first batch. Its ADBC `execute` fetches every page from the source, collects them into one vector of record batches, and returns a reader over that vector, so the scan's own release-as-you-go discipline no longer bounds peak memory. Peak memory inside the UDF is proportional to the result the source produces, and a result larger than the UDF's memory limit fails inside the driver rather than streaming. `scan-udf/arrow-scan` states the release discipline as the framework rule and leaves the end-to-end peak-memory claim to each technology, because this is the technology that cannot give it.
- The driver reports a different Arrow schema for an empty result than for a non-empty one from the same table: a zero-row result is built from the driver's catalog-facing type table, and a non-empty one from its wire-facing table, and the two disagree for an Exasol `DECIMAL(18,0)` column. Nothing in the scan reads the driver's schema to decide a coercion — the declared Exasol output column is the sole authority, per `scan-udf/arrow-scan` — which is what makes the divergence harmless rather than a per-result-size branch.
- The driver's wire-facing types for the mapped set are `Decimal128(p,s)` for `DECIMAL(p,s)`, `Utf8` for `VARCHAR`, `Date32` for `DATE`, microsecond `Timestamp` with no time zone for `TIMESTAMP`, and `Float64` for `DOUBLE`; it also returns `Int64` and `Int32` for a scale-0 integral expression and an Arrow boolean for a delegated predicate. Every one of those is an input `scan-udf/arrow-scan`'s coercion already covers, which is why this feature adds no coercion rule of its own.

## Scenarios

### Scenario: Credentials pass as separate driver options and the address carries none

* *GIVEN* a scan specification naming an Exasol `CONNECTION` object whose address is `exasol://host:8563/APPDATA?validateservercertificate=0` and whose user and password fields hold values
* *WHEN* the scan opens the Exasol ADBC connection
* *THEN* the scan MUST pass the address verbatim as the `uri` database option and the user and password as the separate `username` and `password` database options
* *AND* it MUST NOT concatenate either credential into the `uri` value
* *AND* the `uri` value MUST carry the host, the port, the schema, and the address's query string and nothing else
* *AND* no text derived from that value MUST contain the user or the password, including an error naming the address it could not reach
* *AND* the scan MUST still strip the resolved password from a driver message it surfaces, as defense in depth rather than as the mechanism this guarantee rests on

### Scenario: A credential the driver's own DSN round trip cannot carry is refused

* *GIVEN* an Exasol `CONNECTION` object whose password contains a percent sign, and one whose password contains a single quote, a backslash, a space, and an ampersand
* *WHEN* the scan opens the connection for each
* *THEN* the percent-sign credential MUST be refused with an error naming the character that cannot be carried and naming the driver's DSN round trip as the cause
* *AND* the scan MUST NOT hand an altered credential to the driver, because an altered credential reaches the source as a failed authentication rather than as a diagnosis
* *AND* the scan MUST NOT pre-encode the credential to compensate for the driver's decode step
* *AND* the credential holding a quote, a backslash, a space, and an ampersand MUST connect successfully against a live source, so the refusal is narrow rather than a blanket rejection of punctuation
* *AND* the refused set MUST be established by measurement against the driver rather than derived from the DSN grammar, and every byte in that set MUST be refused by name

### Scenario: The address's query string reaches the driver, so certificate handling is operator-set

* *GIVEN* an Exasol source presenting a self-signed certificate, and `CONNECTION` addresses carrying `validateservercertificate=0`, carrying `certificate_fingerprint` set to the source certificate's lower-case SHA-256 hex digest, carrying the same digest in upper case, and carrying neither parameter
* *WHEN* the scan runs against each
* *THEN* the `validateservercertificate=0` address MUST connect
* *AND* the lower-case fingerprint address MUST connect
* *AND* the upper-case fingerprint address MUST fail with the driver's fingerprint-mismatch diagnosis, because the driver compares the digest as an exact string
* *AND* the address carrying neither MUST fail, because the driver validates against the operating system's trust store and the Script Language Container ships no CA bundle
* *AND* the scan MUST NOT add, remove, or rewrite any parameter of the address, so the operator's choice is the only thing that decides how the certificate is checked

### Scenario: The whole result is buffered by the driver, and the scan states what that caps

* *GIVEN* an Exasol source table holding more rows than fit in one Arrow record batch
* *WHEN* the scan executes the pushed-down statement
* *THEN* the scan MUST emit every batch and the total emitted row count MUST equal the source row count
* *AND* the scan MUST release each batch before requesting the next, so its own contribution to peak memory does not grow with the result size
* *AND* peak memory inside the UDF MUST be documented as proportional to the result the source produces, because the driver collects every page before the scan sees the first batch
* *AND* a result larger than the UDF's memory limit MUST fail with an error rather than emit a truncated result
* *AND* the scan MUST NOT claim the end-to-end bounded-memory property `scan-udf/arrow-scan` states for a streaming driver

### Scenario: An empty result set is emitted despite the driver reporting a different schema for it

* *GIVEN* an Exasol source table with a `DECIMAL(18,0)` column, and two pushed-down statements over it, one matching no rows and one matching rows
* *WHEN* the scan runs each
* *THEN* the empty scan MUST complete without error and emit no row
* *AND* the non-empty scan MUST emit its rows as the declared `DECIMAL(18,0)`
* *AND* the scan MUST NOT fail because the driver reported one Arrow type for the empty result and another for the non-empty one over the same column
* *AND* the coercion MUST be keyed on the declared Exasol output column rather than on the schema the driver reports, so the divergence needs no branch on result size

### Scenario: Source failure surfaces the driver's diagnosis without the credential

* *GIVEN* an Exasol source, and scans whose `CONNECTION` object holds a wrong password, addresses a host and port nothing listens on, and names a schema the pushed-down statement cannot resolve
* *WHEN* each scan runs
* *THEN* each MUST return an error carrying the ADBC status and the driver's own message
* *AND* no error MUST contain the source password, in any encoding or escaping
* *AND* each error MUST name the address it tried to reach, so a wrong credential and an unreachable host stay distinguishable
* *AND* the unresolved-schema error MUST name the schema, because the driver does not activate a session schema and an unqualified table would otherwise fail with no subject
