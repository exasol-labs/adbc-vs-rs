# Feature: driver_self_install

Makes the ADBC driver a virtual schema names available to the scan UDF by fetching it from the public driver registry into BucketFS on first use, so no operator packaging step stands between installing adbc-vs and querying that source.

## Background

<!-- DELTA:CHANGED -->
- The registry index is the YAML document at `https://dbc-cdn.columnar.tech/index.yaml`. Its single top-level key `drivers` holds one entry per driver; each supported driver has its own `path` — `mysql` and `postgresql` — and its `pkginfo` list holds one block per version, each with a `version` scalar and a `packages` list of `platform` and `url` pairs.
- Package URLs are relative and resolve against `https://dbc-cdn.columnar.tech`. The platform tuple is `<os>_<arch>` in lower case; the SLC target is `linux_amd64`.
- Each supported driver pins one version, and the pinned version string is stored per driver rather than derived from a shared rule, because the registry's own version spellings are not uniform across drivers. The MySQL entry publishes `v0.6.0` and the PostgreSQL entry publishes `1.12.0`: one carries a leading `v` and the other does not. The index lookup matches the `version` scalar by exact string equality, so a shared "prefix with `v`" or "strip the `v`" rule would fail to find one of the two published versions. The adapter reads the relative `url` out of the matched block rather than constructing it, so the adapter's own fetch depends on the spelling only through the lookup. The integration tier does construct the URL from the pinned version string, so the spelling is load-bearing there and one recorded spelling per driver serves both.
- The index publishes no checksum for any driver, so integrity is checked against a SHA-256 digest compiled into adbc-vs alongside the pinned version, one digest per driver. No property lets an operator request a different version in this release.
- Every per-driver package fact is one row of one table keyed on the driver identity `vs-adapter/driver-selection` owns: the pinned version, the SHA-256 digest, the installed shared-library filename, the registry driver path, and the ADBC entry-point symbol. Release 0.2 held those as bare module constants, which is why a second driver could not coexist with the first; they become fields read through the identity so a request provisions the driver it named rather than the only driver there was.
- The BucketFS path is therefore computable from the driver identity alone, before any network call, which is what lets the presence check short-circuit and what lets an operator pre-load a driver by hand.
- Both drivers install into one BucketFS directory and do not collide, because their shared-library filenames differ — `libadbc_driver_mysql.so` and `libadbc_driver_postgresql.so`. That is a property to assert rather than to assume: two drivers sharing a filename would make each one's presence check answer for the other, so the second driver would never be fetched and the first would be loaded in its place.
- The entry-point symbol is per driver and is not derived from the filename. The MySQL package exports `AdbcDriverMysqlInit` and the PostgreSQL package exports `AdbcDriverPostgresqlInit`, and only the latter's capitalization follows from its path; `scan-udf/arrow-scan` already requires an explicitly named entry point rather than a naming heuristic.
- Each package is a gzipped tar archive holding the driver shared library, a `MANIFEST` TOML file whose `[Files] driver` key names that library, a detached OpenPGP signature, and license files. The two packages differ in incidental contents — one spells its license file `LICENSE` and the other `LICENSE.txt` — so extraction selects the file the `MANIFEST` names rather than every file the archive holds.
- The PostgreSQL driver is a C++ library and links `libstdc++` and `libgcc_s` at run time, where the Go-built MySQL driver links neither. A provisioned library that BucketFS holds is not yet a library the UDF sandbox can load, so the loadability of each driver is asserted rather than inferred from a successful upload; `scan-udf/arrow-scan` owns that assertion.
- Self-install runs in the adapter script during `createVirtualSchema`, not in the scan UDF. The adapter runs once per statement, whereas scan UDFs run concurrently on every node and would race on the same BucketFS write.
- UDFs see BucketFS as a read-only mount. Writing requires an HTTPS `PUT` to the BucketFS endpoint with the write credentials held by the `BUCKETFS_CONNECTION` object.
- The registry client and the BucketFS client are two separate HTTP clients with two separate certificate policies. The registry is a public internet host reached by name; BucketFS is an in-cluster endpoint serving the cluster's own certificate. One shared client cannot serve both without weakening the public-internet leg.
- A BucketFS write is not immediately visible on every node. The adapter waits for the file to appear on its own mount before reporting success.
- BucketFS answers HTTP 423 to a write of an object it is still finishing an earlier write or delete of, for as long as that work takes. That is a "not yet" rather than a refusal, so it is the one status the upload waits out instead of surfacing.
- adbc-vs never ships a driver binary in its own release artifact, for any driver.
<!-- /DELTA:CHANGED -->

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Already-installed driver short-circuits the fetch

* *GIVEN* the pinned shared library of the driver a request names, already present at its BucketFS path
* *WHEN* the adapter provisions that driver
* *THEN* the adapter MUST return that BucketFS path
* *AND* the adapter MUST NOT request the registry index
* *AND* the adapter MUST NOT write to BucketFS
* *AND* the same MUST hold for each driver this release supports
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Missing driver is fetched, verified, and installed

* *GIVEN* no shared library at the BucketFS path of the driver a request names, for each driver this release supports
* *AND* a reachable registry and a reachable BucketFS write endpoint
* *WHEN* the adapter provisions that driver
* *THEN* the adapter MUST read the registry index and select the `linux_amd64` package of that driver's pinned version, looked up under that driver's own registry path
* *AND* it MUST match the pinned version string exactly as the index spells it, and resolve the matched package's relative `url` against the registry base URL
* *AND* it MUST verify the downloaded archive against that driver's compiled-in SHA-256 digest before extracting it
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: Pinned version strings are matched exactly as the registry spells them

* *GIVEN* the registry index, whose MySQL entry publishes the version `v0.6.0` and whose PostgreSQL entry publishes the version `1.12.0`
* *WHEN* each driver's pinned version is looked up in the index
* *THEN* each lookup MUST find a published version block
* *AND* the pinned version string MUST be stored per driver rather than produced from a shared rule that adds or strips a leading `v`
* *AND* a lookup that finds no block MUST fail with an error naming the driver, the version it looked for, and the versions the index publishes for that driver
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: Two drivers install side by side without collision

* *GIVEN* a BucketFS mount holding no driver
* *WHEN* one virtual schema provisions the MySQL driver and a second provisions the PostgreSQL driver
* *THEN* both shared libraries MUST be present on the mount afterwards, under filenames that differ
* *AND* provisioning the second driver MUST NOT overwrite, delete, or re-upload the first
* *AND* the presence check for either driver MUST NOT be satisfied by the other driver's library
* *AND* the pinned filenames MUST be pairwise distinct across every supported driver, asserted over the driver identities rather than over a written list
<!-- /DELTA:NEW -->

<!-- DELTA:CHANGED -->
### Scenario: Archive manifest is checked against the compiled-in filename

* *GIVEN* a verified driver archive extracted by the adapter
* *WHEN* the adapter installs the driver into BucketFS
* *THEN* it MUST verify that the archive's `MANIFEST` `[Files] driver` key equals the shared-library filename compiled in for that driver
* *AND* it MUST fail with an error naming the driver and both filenames when they differ, because a divergence would make the presence check miss forever
* *AND* it MUST upload that shared library, and only it, to BucketFS over HTTPS using the `BUCKETFS_CONNECTION` credentials, so a package's license or signature files are never installed
* *AND* it MUST return the BucketFS path only after the file is readable on the adapter's own mount
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Checksum mismatch aborts the install

* *GIVEN* a downloaded archive whose SHA-256 digest differs from the digest compiled in for the driver being provisioned
* *WHEN* the adapter provisions that driver
* *THEN* the adapter MUST return an error
* *AND* the error message MUST name the driver and report both the expected and the computed digest
* *AND* the adapter MUST NOT extract the archive
* *AND* the adapter MUST NOT write anything to BucketFS
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Unreachable registry names the offline fallback

* *GIVEN* a cluster whose UDFs have no outbound HTTPS egress
* *WHEN* the adapter provisions a driver
* *THEN* the adapter MUST return an error
* *AND* the error message MUST name the registry host it could not reach
* *AND* the error message MUST state the exact BucketFS path at which an operator places that driver's shared library ahead of time
* *AND* that path MUST be the one belonging to the driver the request named, not the one belonging to another supported driver
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Missing platform package fails with a named error

* *GIVEN* a registry index whose pinned version of the driver being provisioned publishes no `linux_amd64` package
* *WHEN* the adapter provisions that driver
* *THEN* the adapter MUST return an error
* *AND* the error message MUST name the driver, the platform tuple it looked for, and the version it looked in
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Release artifact bundles no driver binary

* *GIVEN* the adbc-vs repository and its build output
* *WHEN* the tracked files and the release artifact are inspected
* *THEN* neither MUST contain an ADBC driver shared library, for any supported driver
* *AND* the only shared library the build produces MUST be adbc-vs's own UDF object
<!-- /DELTA:CHANGED -->
