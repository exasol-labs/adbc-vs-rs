# Feature: driver_self_install

Makes the ADBC driver a virtual schema names available to the scan UDF by fetching it from the public driver registry into BucketFS on first use, so no operator packaging step stands between installing adbc-vs and querying that source.

## Background

<!-- DELTA:CHANGED -->
- The registry index is the YAML document at `https://dbc-cdn.columnar.tech/index.yaml`. Its single top-level key `drivers` holds one entry per driver; each supported driver has its own `path` — `mysql` and `postgresql` — and its `pkginfo` list holds one block per version, each with a `version` scalar and a `packages` list of `platform` and `url` pairs.
- Package URLs are relative and resolve against `https://dbc-cdn.columnar.tech`. The platform tuple is `<os>_<arch>` in lower case; the SLC target is `linux_amd64`.
- Each supported driver states a **version range** rather than one pinned version, and self-install resolves it against the index by selecting the highest published version that satisfies the range. Resolution parses each published `version` scalar and normalizes one optional leading `v`, because the registry's spellings are not uniform: the MySQL entry publishes `v0.6.0` and the PostgreSQL entry publishes `1.12.0`. Comparing versions semantically rather than by string equality is what makes one rule serve both spellings. The adapter reads the relative `url` out of the resolved block rather than constructing it. A published scalar that does not parse is skipped rather than failing the lookup, so one malformed index entry cannot make a driver uninstallable.
- The two ranges are independent data, not one derived rule. MySQL's range stops below `0.7.0` because a pre-1.0 minor bump is breaking by SemVer convention; PostgreSQL's admits every `1.x`. A shared caret rule would be wrong for one of them.
- Resolving a range means the driver version a cluster runs is no longer fixed by the adbc-vs release. A cluster can install a newer in-range version than it installed before, with no adbc-vs release and no operator action. `vs-adapter/create-virtual-schema` records the **resolved** version in `adapterNotes`, which is the only place an operator can read back which version a virtual schema is using.
- The index publishes no checksum for any driver, in any field: a package block carries exactly `platform` and `url`. Self-install therefore verifies **neither a checksum nor a signature** of a downloaded package. The download's TLS connection is the only integrity guarantee, and it is the one this feature's certificate-verification scenario already states. The compiled-in SHA-256 digest that release 0.2 checked one pinned MySQL version against is removed rather than replaced, because a range resolves to versions no maintainer has reviewed. This narrows a released guarantee deliberately: verifying the packages' embedded OpenPGP signature would need a signing key whose public half is obtainable only from the same CDN that serves the artifact, which adds ceremony without adding independent trust. Adding checksum or signature verification is deferred to a later change rather than abandoned, and an operator who needs a fixed, vouched-for artifact supplies it through `DRIVER_PATH`.
- Every per-driver package fact is one row of one table keyed on the driver identity `vs-adapter/driver-selection` owns: the version range, the installed shared-library filename, the registry driver path, and the ADBC entry-point symbol. Those values live in one checked-in TOML file per technology and are compiled into the adapter at build time, so a technology's package facts are a data edit rather than a code edit. They are never read from BucketFS or the filesystem at run time. Release 0.2 held them as bare module constants, which is why a second driver could not coexist with the first.
- A technology cannot be added by adding a TOML file alone. Each one also needs a dialect and a metadata source, which are code, so the supported set is fixed at build time and adding to it requires a new adbc-vs release.
- The BucketFS install path carries the driver and the resolved version: `<mount>/adbc-drivers/<driver>/<version>/<library>`. It is therefore **not** computable before the range is resolved, which is a deliberate change from the release that pinned a version. The presence check tests the resolved version, so a mount already holding an older in-range version does not satisfy it and the newer version is installed beside the older one rather than over it. An unversioned path would make the presence check answer for whichever version arrived first, so the range would resolve once per cluster and never again.
- One consequence follows and is accepted rather than worked around: an already-installed driver no longer lets the adapter skip the network, because the index must be read to learn which version is wanted. A cluster without outbound HTTPS egress supplies the driver through `DRIVER_PATH` instead.
- `DRIVER_PATH` is the supported way to run without the registry. When `vs-adapter/create-virtual-schema` receives it, the adapter uses exactly that BucketFS path and performs no index read, no download, and no BucketFS write. It applies to every driver technology. A driver an operator supplies this way is one the operator vouches for; adbc-vs asserts nothing about it beyond its readability. This supersedes the earlier arrangement, in which an unreachable registry named a fixed compiled-in path for an operator to pre-load — that path is no longer fixed once it carries a resolved version, and it was never one an operator could choose.
- Both drivers install under distinct paths and do not collide, because the driver segment differs and the shared-library filenames differ — `libadbc_driver_mysql.so` and `libadbc_driver_postgresql.so`. That is a property to assert rather than to assume: two drivers sharing an install path would make each one's presence check answer for the other, so the second driver would never be fetched and the first would be loaded in its place.
- The entry-point symbol is per driver and is not derived from the filename. The MySQL package exports `AdbcDriverMysqlInit` and the PostgreSQL package exports `AdbcDriverPostgresqlInit`, and only the latter's capitalization follows from its path; `scan-udf/arrow-scan` already requires an explicitly named entry point rather than a naming heuristic.
- Each package is a gzipped tar archive holding the driver shared library, a `MANIFEST` TOML file whose `[Files] driver` key names that library, a detached OpenPGP signature the adapter does not read, and license files. The two packages differ in incidental contents — one spells its license file `LICENSE` and the other `LICENSE.txt` — so extraction selects the file the `MANIFEST` names rather than every file the archive holds.
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

* *GIVEN* a BucketFS mount already holding the shared library of the version the driver's range resolves to
* *WHEN* the adapter provisions the driver a request names
* *THEN* the adapter MUST return that BucketFS path
* *AND* the adapter MUST NOT download the package
* *AND* the adapter MUST NOT write to BucketFS
* *AND* the adapter MUST still read the registry index, because the version the presence check tests for is the resolved one and it cannot be known before resolution
* *AND* the same MUST hold for each driver this release supports
<!-- /DELTA:CHANGED -->

<!-- DELTA:REMOVED -->
### Scenario: Missing driver is fetched, verified, and installed

* *GIVEN* no driver shared library at the BucketFS path
* *AND* a reachable registry and a reachable BucketFS write endpoint
* *WHEN* the adapter provisions the driver
* *THEN* the adapter MUST read the registry index and select the `linux_amd64` package of the pinned version
* *AND* it MUST resolve that package's relative `url` against the registry base URL
* *AND* it MUST verify the downloaded archive against the compiled-in SHA-256 digest before extracting it
<!-- /DELTA:REMOVED -->

<!-- DELTA:NEW -->
### Scenario: Missing driver is fetched and installed

* *GIVEN* no shared library at the BucketFS path of the driver a request names, for each driver this release supports
* *AND* a reachable registry and a reachable BucketFS write endpoint
* *WHEN* the adapter provisions that driver
* *THEN* the adapter MUST read the registry index and select the `linux_amd64` package of the highest published version satisfying that driver's version range, looked up under that driver's own registry path
* *AND* it MUST resolve the selected package's relative `url` against the registry base URL rather than constructing the URL from the version
* *AND* the download's server-certificate verification MUST be its only integrity check, so the adapter MUST NOT verify a checksum or a signature of the package, the registry publishing neither and no compiled-in digest being able to describe a version the range has yet to resolve to
* *AND* it MUST install the library at a path carrying the driver and the resolved version
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: Version range resolves to the highest published version satisfying it

* *GIVEN* a registry index publishing several versions of a driver, some inside that driver's range and some outside it
* *WHEN* the adapter resolves that driver's version range
* *THEN* it MUST select the highest published version satisfying the range
* *AND* it MUST NOT select a version outside the range, whether higher or lower
* *AND* a published `version` scalar carrying a leading `v` MUST resolve identically to the same version spelled without one, because the index spells MySQL's versions `v0.6.0` and PostgreSQL's `1.12.0`
* *AND* a published scalar that cannot be parsed as a version MUST be skipped rather than failing the resolution
* *AND* each supported driver's range MUST be stated per technology rather than derived from a shared rule, because MySQL's stops below `0.7.0` while PostgreSQL's admits every `1.x`
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: Range satisfied by no published version fails with a named error

* *GIVEN* a registry index publishing no version of a driver that satisfies that driver's range
* *WHEN* the adapter provisions that driver
* *THEN* the adapter MUST return an error
* *AND* the error message MUST name the driver, the range it required, and the versions the index publishes for that driver
* *AND* the adapter MUST NOT download or install any package
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: Newer in-range version is installed beside an older installed one

* *GIVEN* a BucketFS mount holding an in-range version of a driver's shared library
* *AND* a registry index that has since published a higher version still inside that driver's range
* *WHEN* the adapter provisions that driver
* *THEN* the adapter MUST resolve the higher version and install it
* *AND* the presence of the older version MUST NOT short-circuit the install
* *AND* the older version's file MUST remain in place, neither deleted nor overwritten, because a running query can still have it mapped
* *AND* the path the adapter returns MUST be the newly resolved version's path
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: Two drivers install side by side without collision

* *GIVEN* a BucketFS mount holding no driver
* *WHEN* one virtual schema provisions the MySQL driver and a second provisions the PostgreSQL driver
* *THEN* both shared libraries MUST be present on the mount afterwards, under install paths that differ in their driver segment
* *AND* provisioning the second driver MUST NOT overwrite, delete, or re-upload the first
* *AND* the presence check for either driver MUST NOT be satisfied by the other driver's library
* *AND* the install paths MUST be pairwise distinct across every supported driver, asserted over the driver identities rather than over a written list
* *AND* the shared-library filenames MUST also be pairwise distinct, so that a path collision is not the only thing keeping the two apart
<!-- /DELTA:NEW -->

<!-- DELTA:CHANGED -->
### Scenario: Archive manifest is checked against the compiled-in filename

* *GIVEN* a driver archive the adapter downloaded and extracted
* *WHEN* the adapter installs the driver into BucketFS
* *THEN* it MUST verify that the archive's `MANIFEST` `[Files] driver` key equals the shared-library filename compiled in for that driver
* *AND* it MUST fail with an error naming the driver and both filenames when they differ, because a divergence would make the presence check miss forever
* *AND* it MUST upload that shared library, and only it, to BucketFS over HTTPS using the `BUCKETFS_CONNECTION` credentials, so a package's license or signature files are never installed
* *AND* it MUST return the BucketFS path only after the file is readable on the adapter's own mount
<!-- /DELTA:CHANGED -->

<!-- DELTA:REMOVED -->
### Scenario: Checksum mismatch aborts the install

* *GIVEN* a downloaded archive whose SHA-256 digest differs from the compiled-in digest
* *WHEN* the adapter provisions the driver
* *THEN* the adapter MUST return an error
* *AND* the error message MUST report both the expected and the computed digest
* *AND* the adapter MUST NOT extract the archive
* *AND* the adapter MUST NOT write anything to BucketFS
<!-- /DELTA:REMOVED -->

<!-- DELTA:CHANGED -->
### Scenario: Unreachable registry names the offline fallback

* *GIVEN* a cluster whose UDFs have no outbound HTTPS egress
* *AND* a request that set no `DRIVER_PATH`
* *WHEN* the adapter provisions a driver
* *THEN* the adapter MUST return an error
* *AND* the error message MUST name the registry host it could not reach, and the driver it was provisioning
* *AND* the error message MUST name `DRIVER_PATH` as the property that supplies a driver without the registry
* *AND* the error message MUST NOT name a fixed BucketFS path to pre-load, and the adapter MUST NOT load an unrelated version it finds on the mount, because the install path carries a version the adapter could not resolve
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: DRIVER_PATH skips self-install entirely

* *GIVEN* a `CREATE VIRTUAL SCHEMA` request setting `DRIVER_PATH` to a BucketFS path holding a readable driver shared library
* *WHEN* the adapter provisions the driver
* *THEN* the adapter MUST use exactly that path
* *AND* the adapter MUST NOT request the registry index, download a package, or write to BucketFS
* *AND* the adapter MUST assert nothing about the library beyond its readability, because the operator supplied it and vouches for it
* *AND* the same MUST hold for every driver this release supports, because the property is not specific to one technology
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: DRIVER_PATH naming no readable file fails without falling back

* *GIVEN* a `CREATE VIRTUAL SCHEMA` request setting `DRIVER_PATH` to a path holding no readable file
* *WHEN* the adapter provisions the driver
* *THEN* the adapter MUST return an error
* *AND* the error message MUST name the `DRIVER_PATH` property, the path it was given, and the driver
* *AND* the adapter MUST NOT fall back to the registry, because a silent fallback would defeat the air-gapped deployment the property exists to serve
* *AND* the adapter MUST NOT create the virtual schema
<!-- /DELTA:NEW -->

<!-- DELTA:CHANGED -->
### Scenario: Missing platform package fails with a named error

* *GIVEN* a registry index whose highest in-range version of the driver being provisioned publishes no `linux_amd64` package
* *WHEN* the adapter provisions that driver
* *THEN* the adapter MUST return an error
* *AND* the error message MUST name the driver, the platform tuple it looked for, and the resolved version it looked in
* *AND* the adapter MUST NOT silently fall back to a lower in-range version that does publish that platform, because the resolved version is what every other record reports
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Release artifact bundles no driver binary

* *GIVEN* the adbc-vs repository and its build output
* *WHEN* the tracked files and the release artifact are inspected
* *THEN* neither MUST contain an ADBC driver shared library, for any supported driver
* *AND* the only shared library the build produces MUST be adbc-vs's own UDF object
<!-- /DELTA:CHANGED -->
