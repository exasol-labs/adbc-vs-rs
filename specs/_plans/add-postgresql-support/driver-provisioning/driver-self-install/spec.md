# Feature: driver_self_install

Installs the ADBC driver a virtual schema names into BucketFS by downloading it from the public driver registry, so no operator packaging step stands between installing adbc-vs and querying that source.

## Background

<!-- DELTA:CHANGED -->
- This feature owns the install mechanism alone: downloading the package a resolution produced, extracting the one file the archive's `MANIFEST` names, uploading that file to BucketFS, and returning a path a UDF can load. Which version to install, and whether the mount already holds it, belong to `driver-provisioning/driver-version-resolution`. Skipping the whole mechanism on an operator-supplied library belongs to `driver-provisioning/driver-path-override`. Three features rather than one, because the install mechanism is uniform across drivers while version resolution and the override each answer a question of their own.
- Every per-driver package fact is one row of one table keyed on the driver identity `vs-adapter/driver-selection` owns: the version range, the installed shared-library filename, the registry driver path, and the ADBC entry-point symbol. Those values live in one checked-in TOML file per technology and are compiled into the adapter at build time, so a technology's package facts are a data edit rather than a code edit. They are never read from BucketFS or the filesystem at run time. Release 0.2 held them as bare module constants, which is why a second driver could not coexist with the first.
- A technology cannot be added by adding a TOML file alone. Each one also needs a dialect and a metadata source, which are code, so the supported set is fixed at build time and adding to it requires a new adbc-vs release.
- Each package is a gzipped tar archive holding the driver shared library, a `MANIFEST` TOML file whose `[Files] driver` key names that library, a detached OpenPGP signature the adapter does not read, and license files. The two packages differ in incidental contents — one spells its license file `LICENSE` and the other `LICENSE.txt` — so extraction selects the file the `MANIFEST` names rather than every file the archive holds.
- The download's TLS connection is the only integrity guarantee. The registry index publishes no checksum for any driver, in any field: a package block carries exactly `platform` and `url`. Self-install therefore verifies **neither a checksum nor a signature** of a downloaded package, and this feature's certificate-verification scenario states the guarantee that remains. The compiled-in SHA-256 digest that release 0.2 checked one pinned MySQL version against is removed rather than replaced, because a range resolves to versions no maintainer has reviewed. This narrows a released guarantee deliberately: verifying the packages' embedded OpenPGP signature would need a signing key whose public half is obtainable only from the same CDN that serves the artifact, which adds ceremony without adding independent trust. Adding checksum or signature verification is deferred to a later change rather than abandoned, and an operator who needs a fixed, vouched-for artifact supplies it through `DRIVER_PATH`.
- The BucketFS install path is `<mount>/adbc-drivers/<driver>/<version>/<library>`. Both segments earn their place: the driver segment keeps two technologies apart, and the version segment is what `driver-provisioning/driver-version-resolution` binds the presence check to. Release 0.2 installed at `<mount>/adbc-drivers/<library>`, which could hold only one driver and only one version of it.
- Both drivers install under distinct paths and do not collide, because the driver segment differs and the shared-library filenames differ — `libadbc_driver_mysql.so` and `libadbc_driver_postgresql.so`. That is a property to assert rather than to assume: two drivers sharing an install path would make each one's presence check answer for the other, so the second driver would never be fetched and the first would be loaded in its place.
- The entry-point symbol is per driver and is not derived from the filename. The MySQL package exports `AdbcDriverMysqlInit` and the PostgreSQL package exports `AdbcDriverPostgresqlInit`, and only the latter's capitalization follows from its path; `scan-udf/arrow-scan` already requires an explicitly named entry point rather than a naming heuristic.
- The PostgreSQL driver is a C++ library and links `libstdc++` and `libgcc_s` at run time, where the Go-built MySQL driver links neither. A provisioned library that BucketFS holds is not yet a library the UDF sandbox can load, so the loadability of each driver is asserted rather than inferred from a successful upload; `scan-udf/arrow-scan` owns that assertion.
- Self-install runs in the adapter script during `createVirtualSchema`, not in the scan UDF. The adapter runs once per statement, whereas scan UDFs run concurrently on every node and would race on the same BucketFS write.
- UDFs see BucketFS as a read-only mount. Writing requires an HTTPS `PUT` to the BucketFS endpoint with the write credentials held by the `BUCKETFS_CONNECTION` object.
- The registry client and the BucketFS client are two separate HTTP clients with two separate certificate policies. The registry is a public internet host reached by name; BucketFS is an in-cluster endpoint serving the cluster's own certificate. One shared client cannot serve both without weakening the public-internet leg.
- A BucketFS write is not immediately visible on every node. The adapter waits for the file to appear on its own mount before reporting success.
- BucketFS answers HTTP 423 to a write of an object it is still finishing an earlier write or delete of, for as long as that work takes. That is a "not yet" rather than a refusal, so it is the one status the upload waits out instead of surfacing.
- adbc-vs never ships a driver binary in its own release artifact, for any driver.
<!-- /DELTA:CHANGED -->

## Scenarios

<!-- DELTA:REMOVED -->
### Scenario: Already-installed driver short-circuits the fetch

* *GIVEN* the pinned driver's shared library already present at its BucketFS path
* *WHEN* the adapter provisions the driver
* *THEN* the adapter MUST return that BucketFS path
* *AND* the adapter MUST NOT request the registry index
* *AND* the adapter MUST NOT write to BucketFS
<!-- /DELTA:REMOVED -->

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
* *THEN* the adapter MUST download the package that `driver-provisioning/driver-version-resolution` resolved for that driver, from the absolute URL the resolution produced rather than from a URL built out of the version
* *AND* the download's server-certificate verification MUST be its only integrity check, so the adapter MUST NOT verify a checksum or a signature of the package, the registry publishing neither and no compiled-in digest being able to describe a version the range has yet to resolve to
* *AND* it MUST install the library at `<mount>/adbc-drivers/<driver>/<version>/<library>`, carrying that driver and the resolved version
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

<!-- DELTA:REMOVED -->
### Scenario: Unreachable registry names the offline fallback

* *GIVEN* a cluster whose UDFs have no outbound HTTPS egress
* *WHEN* the adapter provisions the driver
* *THEN* the adapter MUST return an error
* *AND* the error message MUST name the registry host it could not reach
* *AND* the error message MUST state the exact BucketFS path at which an operator places the shared library ahead of time
<!-- /DELTA:REMOVED -->

<!-- DELTA:REMOVED -->
### Scenario: Missing platform package fails with a named error

* *GIVEN* a registry index whose pinned MySQL version publishes no `linux_amd64` package
* *WHEN* the adapter provisions the driver
* *THEN* the adapter MUST return an error
* *AND* the error message MUST name the platform tuple it looked for and the version it looked in
<!-- /DELTA:REMOVED -->

<!-- DELTA:CHANGED -->
### Scenario: Release artifact bundles no driver binary

* *GIVEN* the adbc-vs repository and its build output
* *WHEN* the tracked files and the release artifact are inspected
* *THEN* neither MUST contain an ADBC driver shared library, for any supported driver
* *AND* the only shared library the build produces MUST be adbc-vs's own UDF object
<!-- /DELTA:CHANGED -->
