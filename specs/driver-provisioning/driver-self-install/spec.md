# Feature: driver_self_install

Makes the ADBC MySQL driver available to the scan UDF by fetching it from the public driver registry into BucketFS on first use, so no operator packaging step stands between installing adbc-vs and querying MySQL.

## Background

- The registry index is the YAML document at `https://dbc-cdn.columnar.tech/index.yaml`. Its single top-level key `drivers` holds one entry per driver; the MySQL entry has `path: mysql`, and its `pkginfo` list holds one block per version, each with a `version` scalar and a `packages` list of `platform` and `url` pairs.
- Package URLs are relative and resolve against `https://dbc-cdn.columnar.tech`. The platform tuple is `<os>_<arch>` in lower case; the SLC target is `linux_amd64`.
- Release 0.1 installs one pinned driver version. The index publishes no checksum, so integrity is checked against a SHA-256 digest compiled into adbc-vs alongside the pinned version. No property lets an operator request a different version in this release.
- The pinned version, the SHA-256 digest, and the installed shared-library filename are three compiled-in constants owned by one module. The BucketFS path is therefore computable before any network call, which is what lets the presence check short-circuit and what lets an operator pre-load the driver by hand.
- The package is a gzipped tar archive holding the driver shared library, a `MANIFEST` TOML file whose `[Files] driver` key names that library, a detached OpenPGP signature, and license files.
- Self-install runs in the adapter script during `createVirtualSchema`, not in the scan UDF. The adapter runs once per statement, whereas scan UDFs run concurrently on every node and would race on the same BucketFS write.
- UDFs see BucketFS as a read-only mount. Writing requires an HTTPS `PUT` to the BucketFS endpoint with the write credentials held by the `BUCKETFS_CONNECTION` object.
- The registry client and the BucketFS client are two separate HTTP clients with two separate certificate policies. The registry is a public internet host reached by name; BucketFS is an in-cluster endpoint serving the cluster's own certificate. One shared client cannot serve both without weakening the public-internet leg.
- A BucketFS write is not immediately visible on every node. The adapter waits for the file to appear on its own mount before reporting success.
- BucketFS answers HTTP 423 to a write of an object it is still finishing an earlier write or delete of, for as long as that work takes. That is a "not yet" rather than a refusal, so it is the one status the upload waits out instead of surfacing.
- adbc-vs never ships a driver binary in its own release artifact.

## Scenarios

### Scenario: Already-installed driver short-circuits the fetch

* *GIVEN* the pinned driver's shared library already present at its BucketFS path
* *WHEN* the adapter provisions the driver
* *THEN* the adapter MUST return that BucketFS path
* *AND* the adapter MUST NOT request the registry index
* *AND* the adapter MUST NOT write to BucketFS

### Scenario: Missing driver is fetched, verified, and installed

* *GIVEN* no driver shared library at the BucketFS path
* *AND* a reachable registry and a reachable BucketFS write endpoint
* *WHEN* the adapter provisions the driver
* *THEN* the adapter MUST read the registry index and select the `linux_amd64` package of the pinned version
* *AND* it MUST resolve that package's relative `url` against the registry base URL
* *AND* it MUST verify the downloaded archive against the compiled-in SHA-256 digest before extracting it

### Scenario: Archive manifest is checked against the compiled-in filename

* *GIVEN* a verified driver archive extracted by the adapter
* *WHEN* the adapter installs the driver into BucketFS
* *THEN* it MUST verify that the archive's `MANIFEST` `[Files] driver` key equals the compiled-in shared-library filename
* *AND* it MUST fail with an error naming both filenames when they differ, because a divergence would make the presence check miss forever
* *AND* it MUST upload that shared library to BucketFS over HTTPS using the `BUCKETFS_CONNECTION` credentials
* *AND* it MUST return the BucketFS path only after the file is readable on the adapter's own mount

### Scenario: Checksum mismatch aborts the install

* *GIVEN* a downloaded archive whose SHA-256 digest differs from the compiled-in digest
* *WHEN* the adapter provisions the driver
* *THEN* the adapter MUST return an error
* *AND* the error message MUST report both the expected and the computed digest
* *AND* the adapter MUST NOT extract the archive
* *AND* the adapter MUST NOT write anything to BucketFS

### Scenario: Unreachable registry names the offline fallback

* *GIVEN* a cluster whose UDFs have no outbound HTTPS egress
* *WHEN* the adapter provisions the driver
* *THEN* the adapter MUST return an error
* *AND* the error message MUST name the registry host it could not reach
* *AND* the error message MUST state the exact BucketFS path at which an operator places the shared library ahead of time

### Scenario: Registry client always verifies the server certificate

* *GIVEN* the adapter fetching the registry index or the driver package
* *WHEN* the registry client opens its TLS connection
* *THEN* the client MUST verify the server certificate against its bundled root store
* *AND* no property, cargo feature, or environment variable MUST be able to disable that verification
* *AND* a certificate the client cannot verify MUST fail the provisioning with a named error

### Scenario: BucketFS client accepts the cluster's own certificate

* *GIVEN* a BucketFS endpoint serving a self-signed certificate, as the Exasol container does
* *WHEN* the adapter uploads the driver
* *THEN* the BucketFS client MUST complete the `PUT`
* *AND* that relaxation MUST apply to the BucketFS client alone
* *AND* the registry client MUST keep verifying certificates in the same process

### Scenario: Missing platform package fails with a named error

* *GIVEN* a registry index whose pinned MySQL version publishes no `linux_amd64` package
* *WHEN* the adapter provisions the driver
* *THEN* the adapter MUST return an error
* *AND* the error message MUST name the platform tuple it looked for and the version it looked in

### Scenario: BucketFS write failure surfaces rather than being retried silently

* *GIVEN* a `BUCKETFS_CONNECTION` whose write password is wrong
* *WHEN* the adapter provisions the driver
* *THEN* the adapter MUST return an error
* *AND* the error message MUST report the HTTP status the BucketFS endpoint returned
* *AND* the error message MUST NOT contain the credential it attempted

### Scenario: Locked BucketFS object is waited out rather than surfaced

* *GIVEN* a BucketFS endpoint answering HTTP 423 to the driver upload because it is still finishing an earlier write or delete of the same object
* *WHEN* the adapter uploads the driver
* *THEN* it MUST retry the upload until the object accepts the write or a bounded deadline elapses
* *AND* on timeout it MUST return an error reporting that status
* *AND* every other rejection MUST surface on the first attempt, without a retry

### Scenario: Driver visibility wait is bounded

* *GIVEN* an upload that has completed but is not yet visible on the adapter's BucketFS mount
* *WHEN* the adapter waits for visibility
* *THEN* it MUST poll until the file is readable or a bounded deadline elapses
* *AND* on timeout it MUST return an error naming the path and the elapsed deadline
* *AND* it MUST NOT wait indefinitely

### Scenario: Release artifact bundles no driver binary

* *GIVEN* the adbc-vs repository and its build output
* *WHEN* the tracked files and the release artifact are inspected
* *THEN* neither MUST contain an ADBC driver shared library
* *AND* the only shared library the build produces MUST be adbc-vs's own UDF object
