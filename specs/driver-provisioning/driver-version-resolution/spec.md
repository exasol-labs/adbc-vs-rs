# Feature: driver_version_resolution

Resolves each driver's declared version range against the registry index to one published version and one package URL, and decides from that resolved version whether the mount already holds the driver.

## Background

- This feature owns the resolved version, its package for the adapter library's platform, and the presence check; `driver-provisioning/driver-self-install` owns the download, and `driver-provisioning/driver-path-override` owns skipping the index.
- The registry index at `https://dbc-cdn.columnar.tech/index.yaml` lists one entry per driver `path` (`mysql`, `postgresql`, `trino`), each with one `pkginfo` block per version holding a `version` scalar and `platform` and `url` package pairs.
- Package URLs are relative to `https://dbc-cdn.columnar.tech`. A platform is `<os>_<arch>` in lower case: `linux_amd64` for an adapter library compiled for x86_64, and `linux_arm64` for one compiled for aarch64.
- Each driver declares its own version range, and the resolved version keys the install path, the presence check, and the driver version `vs-adapter/create-virtual-schema` records in `adapterNotes`.

## Scenarios

### Scenario: Version range resolves to the highest published version satisfying it

* *GIVEN* a registry index publishing several versions of a driver, some inside that driver's range and some outside it
* *WHEN* the adapter resolves that driver's version range
* *THEN* it MUST select the highest published version satisfying the range
* *AND* it MUST NOT select a version outside the range, whether higher or lower
* *AND* a published `version` scalar carrying a leading `v` MUST resolve identically to the same version spelled without one, because the index spells MySQL's and Trino's versions with a leading `v` and PostgreSQL's without one
* *AND* a published scalar that cannot be parsed as a version MUST be skipped rather than failing the resolution
* *AND* each supported driver's range MUST be stated per technology rather than derived from a shared rule, because MySQL's and Trino's stop below the next breaking minor version while PostgreSQL's admits every release of its current major version

### Scenario: Range satisfied by no published version fails with a named error

* *GIVEN* a registry index publishing no version of a driver that satisfies that driver's range
* *WHEN* the adapter provisions that driver
* *THEN* the adapter MUST return an error
* *AND* the error message MUST name the driver, the range it required, and the versions the index publishes for that driver
* *AND* the adapter MUST NOT download or install any package

### Scenario: Missing platform package fails with a named error

* *GIVEN* a registry index whose highest in-range version of the driver being provisioned publishes no package for the platform of the architecture the adapter library was compiled for
* *WHEN* the adapter provisions that driver
* *THEN* the adapter MUST return an error
* *AND* the error message MUST name the driver, the platform tuple it looked for, and the resolved version it looked in
* *AND* the adapter MUST NOT silently fall back to a lower in-range version that does publish that platform, because the resolved version is what every other record reports
* *AND* the adapter MUST NOT download the package of another platform that the resolved version does publish, such as `linux_amd64` for an adapter library compiled for aarch64, because the adapter library cannot load a driver built for another architecture

### Scenario: Already-installed driver short-circuits the fetch

* *GIVEN* a BucketFS mount already holding the shared library of the version the driver's range resolves to
* *WHEN* the adapter provisions the driver a request names
* *THEN* the adapter MUST return that BucketFS path
* *AND* the adapter MUST NOT download the package
* *AND* the adapter MUST NOT write to BucketFS
* *AND* the adapter MUST still read the registry index, because the version the presence check tests for is the resolved one and it cannot be known before resolution
* *AND* the same MUST hold for each driver this release supports

### Scenario: Newer in-range version is installed beside an older installed one

* *GIVEN* a BucketFS mount holding an in-range version of a driver's shared library
* *AND* a registry index that has since published a higher version still inside that driver's range
* *WHEN* the adapter provisions that driver
* *THEN* the adapter MUST resolve the higher version and install it
* *AND* the presence of the older version MUST NOT short-circuit the install
* *AND* the older version's file MUST remain in place, neither deleted nor overwritten, because a running query can still have it mapped
* *AND* the path the adapter returns MUST be the newly resolved version's path

### Scenario: Unreachable registry names the offline fallback

* *GIVEN* a cluster whose UDFs have no outbound HTTPS egress
* *AND* a request that set no `DRIVER_PATH` and whose `BUCKETFS_CONNECTION` property named a `CONNECTION` object that resolved, which is the one route this error belongs to
* *WHEN* the adapter provisions a driver
* *THEN* the adapter MUST return an error
* *AND* the error message MUST name the registry host it could not reach, and the driver it was provisioning
* *AND* the error message MUST name `DRIVER_PATH` as the property that supplies a driver without the registry
* *AND* the error message MUST NOT name a fixed BucketFS path to pre-load, and the adapter MUST NOT load an unrelated version it finds on the mount, because the install path carries a version the adapter could not resolve
* *AND* the error message MUST NOT state that no path on the mount exists to pre-load a library at, because `driver-provisioning/driver-path-override`'s default driver location exists on every cluster and this route declines to read it rather than lacking it
* *AND* the adapter MUST NOT fall back to the default driver location `driver-provisioning/driver-path-override` owns, because an explicit `BUCKETFS_CONNECTION` is an instruction rather than a preference

### Scenario: Package platform follows the architecture the adapter library was compiled for

* *GIVEN* a registry index whose resolved version of a driver publishes a `linux_amd64` package and a `linux_arm64` package
* *WHEN* the adapter provisions that driver
* *THEN* the adapter MUST download the package whose platform matches the architecture the adapter library was compiled for: `linux_amd64` for x86_64, `linux_arm64` for aarch64
* *AND* it MUST NOT download the other package, because an adapter library runs only on the architecture it was compiled for
