# Feature: driver_version_resolution

Resolves each driver's declared version range against the registry index to one published version and one package URL, and decides from that resolved version whether the mount already holds the driver.

## Background

- This feature owns every answer read out of the registry index: which version a driver's range resolves to, which package of that version serves the SLC platform, and whether the resolved version is already installed. Downloading and installing the package that resolution names belongs to `driver-provisioning/driver-self-install`, which also owns the install path's shape. Skipping the index entirely belongs to `driver-provisioning/driver-path-override`.
- The registry index is the YAML document at `https://dbc-cdn.columnar.tech/index.yaml`. Its single top-level key `drivers` holds one entry per driver; each supported driver has its own `path` — `mysql` and `postgresql` — and its `pkginfo` list holds one block per version, each with a `version` scalar and a `packages` list of `platform` and `url` pairs.
- Package URLs are relative and resolve against `https://dbc-cdn.columnar.tech`. The platform tuple is `<os>_<arch>` in lower case; the SLC target is `linux_amd64`.
- Each supported driver states a **version range** rather than one pinned version, in the per-technology package row `driver-provisioning/driver-self-install` describes. Resolution selects the highest published version that satisfies the range. It parses each published `version` scalar and normalizes one optional leading `v`, because the registry's spellings are not uniform: the MySQL entry publishes `v0.6.0` and the PostgreSQL entry publishes `1.12.0`. Comparing versions semantically rather than by string equality is what makes one rule serve both spellings. The adapter reads the relative `url` out of the resolved block rather than constructing it. A published scalar that does not parse is skipped rather than failing the lookup, so one malformed index entry cannot make a driver uninstallable.
- The two ranges are independent data, not one derived rule. MySQL's range stops below `0.7.0` because a pre-1.0 minor bump is breaking by SemVer convention; PostgreSQL's admits every `1.x`. A shared caret rule would be wrong for one of them.
- Resolving a range means the driver version a cluster runs is no longer fixed by the adbc-vs release. A cluster can install a newer in-range version than it installed before, with no adbc-vs release and no operator action. `vs-adapter/create-virtual-schema` records the **resolved** version in `adapterNotes`, which is the only place an operator can read back which version a virtual schema is using.
- The presence check binds to the resolved version, and is therefore not answerable before resolution. The install path carries that version, so a mount already holding an older in-range version does not satisfy the check and the newer version is installed beside the older one rather than over it. An unversioned path would make the presence check answer for whichever version arrived first, so the range would resolve once per cluster and never again.
- One consequence follows and is accepted rather than worked around: an already-installed driver no longer lets the adapter skip the network, because the index must be read to learn which version is wanted. A cluster without outbound HTTPS egress supplies the driver itself, and which route it supplies it on decides what an operator sees. A statement naming its own `BUCKETFS_CONNECTION` supplies it through `DRIVER_PATH`, and this feature's unreachable-registry error is where that operator learns the property exists.
- This feature's unreachable-registry error belongs to that explicit route alone. Two routes never reach it. A statement setting `DRIVER_PATH` reads no index. A statement naming neither `DRIVER_PATH` nor `BUCKETFS_CONNECTION` hands a registry it could not reach to `driver-provisioning/driver-path-override`, which answers from the default driver location instead — so on that route the remedy is a file an operator already placed rather than a property they must add. Scoping this error rather than widening it keeps one route's remedy from being offered on a route where it is not the remedy.
- Scoping the remedy is not the same as denying the other route exists. This error names `DRIVER_PATH` and says nothing about whether a version-free location to pre-load a library at exists on the mount. One does — `driver-provisioning/driver-path-override` owns it — and this route simply does not read it. An earlier release's message asserted the opposite, and that assertion is removed rather than kept on the one route that still reaches it.
- Only the index request reaching no host at all separates those two outcomes, because that is the one failure under which nothing answered. A registry that answered — any HTTP status, an index that will not parse, a range no published version satisfies, a resolved version publishing no `linux_amd64` package — is a fact about the registry rather than about the cluster's egress, so it fails the statement on every route. Three transport failures fail the statement on every route for that same reason: a package request that reaches no host, a package body that stops arriving after its status did, and an index body that stops arriving after its status did. The first two carry a second reason as well, because both happen after this feature already resolved a version and answered the presence check; the third does not, and the shared reason is what excludes it. Each of those failures is a distinct error rather than one shared with the index request's own transport failure, so the route rule cannot mistake one for another. The fallback `driver-provisioning/driver-path-override` owns exists for the air gap, and an air gap is the one condition under which nothing answers.

## Scenarios

### Scenario: Version range resolves to the highest published version satisfying it

* *GIVEN* a registry index publishing several versions of a driver, some inside that driver's range and some outside it
* *WHEN* the adapter resolves that driver's version range
* *THEN* it MUST select the highest published version satisfying the range
* *AND* it MUST NOT select a version outside the range, whether higher or lower
* *AND* a published `version` scalar carrying a leading `v` MUST resolve identically to the same version spelled without one, because the index spells MySQL's versions `v0.6.0` and PostgreSQL's `1.12.0`
* *AND* a published scalar that cannot be parsed as a version MUST be skipped rather than failing the resolution
* *AND* each supported driver's range MUST be stated per technology rather than derived from a shared rule, because MySQL's stops below `0.7.0` while PostgreSQL's admits every `1.x`

### Scenario: Range satisfied by no published version fails with a named error

* *GIVEN* a registry index publishing no version of a driver that satisfies that driver's range
* *WHEN* the adapter provisions that driver
* *THEN* the adapter MUST return an error
* *AND* the error message MUST name the driver, the range it required, and the versions the index publishes for that driver
* *AND* the adapter MUST NOT download or install any package

### Scenario: Missing platform package fails with a named error

* *GIVEN* a registry index whose highest in-range version of the driver being provisioned publishes no `linux_amd64` package
* *WHEN* the adapter provisions that driver
* *THEN* the adapter MUST return an error
* *AND* the error message MUST name the driver, the platform tuple it looked for, and the resolved version it looked in
* *AND* the adapter MUST NOT silently fall back to a lower in-range version that does publish that platform, because the resolved version is what every other record reports

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
