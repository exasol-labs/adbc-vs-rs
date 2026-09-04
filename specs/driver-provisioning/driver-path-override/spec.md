# Feature: driver_path_override

Uses a driver shared library the operator placed themselves — named by `DRIVER_PATH`, or found at the default driver location when self-install through the reserved connection cannot run — so a cluster with no outbound HTTPS egress runs a driver the operator supplied.

## Background

- This feature owns the operator-supplied route: use exactly one file, assert nothing about it beyond its readability, download nothing, write nothing, and never fall back. Two things select that route, and they differ only in where the path came from. `DRIVER_PATH` names it. The default driver location is where the adapter looks when the statement named no `BUCKETFS_CONNECTION` and self-install through the reserved connection `ADBC_VS_BUCKETFS` could not run.
- Two failures mean self-install through the reserved connection could not run, and the default location answers both. The reserved connection does not resolve, so no write credential exists. Or it resolves and the request for the registry index reaches no host at all, so no version can be resolved to install. The second is the air-gapped cluster that followed the deployment instructions: it holds the reserved connection, and it has no outbound HTTPS egress. Serving only the first would deliver the soft landing to a cluster that deleted the connection it was told to create.
- One feature rather than two, because the mechanism is one. A path an operator typed and a path the adapter derived reach the same three obligations — exact file, readability only, no fallback — and splitting them would leave two features to keep in agreement about a decision neither owns alone. The feature keeps its name for reference stability; `vs-adapter/create-virtual-schema` owns the property's place among the statement's properties, including the rule that `BUCKETFS_CONNECTION` is optional on every route.
- The default driver location is `<mount>/adbc-drivers/<driver>/supplied/<library>`, where `<mount>` is the read-only BucketFS mount every UDF already sees. It carries no version segment, unlike the install path `driver-provisioning/driver-self-install` writes, because nothing resolved a version on this route: the index was never read, or reading it failed. The `supplied` segment states who owns the file. It can never collide with an install path, because that path's third segment is always a parsed version and no version parses as `supplied`, and it keeps the operator's file out of the directory tree the adapter writes and deletes.
- The location is a stated interface, which is what separates it from the arrangement release 0.2 shipped. Release 0.2's fallback named a fixed compiled-in path only from inside an error message, and that path stopped being fixed once the install path carried a resolved version. This one is stated by this feature, is version-independent by construction, and is named by the error an operator gets when nothing is there. `DRIVER_PATH` remains the route for a library that lives anywhere else, including outside the `adbc-drivers` tree entirely.
- The default location is consulted last on every route that reaches it, and never before an install would have been attempted. A library sitting there MUST NOT short-circuit self-install for a statement whose registry answered, because an unversioned path answers for whichever version arrived first — the defect the versioned install path exists to remove. Falling back after the index read failed does not reintroduce that defect: no version was resolved, nothing was installed, and the next statement reads the index again, so a pre-placed library shadows self-install for exactly as long as the registry stays unreachable. `vs-adapter/create-virtual-schema` owns the full precedence and the properties that select each level of it.
- Only the registry index request reaching no host at all reaches the default location. Three things are true when it does, and each one stops being true a step later: no version was resolved, nothing was downloaded, and nothing was installed. So the trigger is that request's own connect-, name-resolution-, or TLS-level failure and nothing else. A registry that answered any HTTP status, an index that will not parse, a range no published version satisfies, a resolved version with no `linux_amd64` package, a package request that reached no host, either request's body stopping after its status arrived, a package that will not unpack, a BucketFS write that was refused: in each case something answered, so nothing about the cluster's egress was learned, and each fails the statement. That ground covers an index read that stopped mid-body as well, where no version was resolved and nothing was installed — the after-resolution argument does not reach that failure, and it is still not an air gap. The package leg carries the after-resolution ground as a second reason: it runs once a version was resolved and the versioned presence check ran, so falling back there would pin a cluster to an unversioned pre-placed library on a reset download. Widening the trigger to any provisioning failure would turn a transient CDN outage into a cluster silently pinned to whatever library was pre-placed.
- The failure the adapter reports on this route MUST never deny that the default driver location exists. Before this feature the mount held no version-free path to pre-load a library at, and the unreachable-registry and registry-rejected messages said so. That location exists now and is documented, so every message a statement on this route can surface names `DRIVER_PATH` as the explicit remedy and states nothing about whether a pre-load location exists. The claim is removed on the explicit route too rather than suppressed on this one, because a false sentence is no more true where it is harder to reach.
- The versioned presence check is unreachable on this trigger, so a pre-placed library is preferred over an already-installed one for as long as the index request reaches no host. `driver-provisioning/driver-version-resolution` binds that check to the resolved version, and no version was resolved. A cluster holding a working install and a stale pre-placed library therefore loads the pre-placed one during an outage, and returns to the installed one when the index answers again.
- The route the request took is readable afterwards rather than silent. A statement answered from the default location records that path in `adapterNotes` and records no driver version, which is the same two-field signature a `DRIVER_PATH` statement leaves, so an operator can tell a pre-placed library from a self-installed one without guessing. That is what separates this fallback from the silent one this feature forbids.
- The override never falls back. A path holding no readable file fails the request rather than reaching the registry, because a silent fallback would defeat the air gap the route exists to serve and would make an operator's typo look like a slow first query. That holds for the default location too: an empty default location fails the statement rather than provisioning anything.
- Two neighbouring features state the rest of the arrangement. `driver-provisioning/driver-version-resolution` owns the unreachable-registry error that names `DRIVER_PATH`, which is where an operator on the explicit `BUCKETFS_CONNECTION` route learns the property exists; that error is scoped to that route, because a statement naming neither property reaches this feature's default location instead of that error. `driver-provisioning/driver-self-install` owns the mechanism this route skips.

## Scenarios

### Scenario: DRIVER_PATH skips self-install entirely

* *GIVEN* a `CREATE VIRTUAL SCHEMA` request setting `DRIVER_PATH` to a BucketFS path holding a readable driver shared library
* *WHEN* the adapter provisions the driver
* *THEN* the adapter MUST use exactly that path
* *AND* the adapter MUST NOT request the registry index, resolve a version range, download a package, or write to BucketFS
* *AND* the adapter MUST assert nothing about the library beyond its readability, because the operator supplied it and vouches for it
* *AND* the same MUST hold for every driver this release supports, because the property is not specific to one technology

### Scenario: DRIVER_PATH naming no readable file fails without falling back

* *GIVEN* a `CREATE VIRTUAL SCHEMA` request setting `DRIVER_PATH` to a path holding no readable file
* *WHEN* the adapter provisions the driver
* *THEN* the adapter MUST return an error
* *AND* the error message MUST name the `DRIVER_PATH` property, the path it was given, and the driver
* *AND* the adapter MUST NOT fall back to the registry, because a silent fallback would defeat the air-gapped deployment the property exists to serve
* *AND* the adapter MUST NOT create the virtual schema

### Scenario: The default driver location is used when no BucketFS connection resolves

* *GIVEN* a request setting neither `BUCKETFS_CONNECTION` nor `DRIVER_PATH`, on a cluster where `ADBC_VS_BUCKETFS` does not resolve
* *AND* a readable driver shared library at `<mount>/adbc-drivers/<driver>/supplied/<library>` for the driver the request names
* *WHEN* the adapter provisions the driver
* *THEN* the adapter MUST use exactly that path, asserting nothing about the library beyond its readability, exactly as it does for a `DRIVER_PATH` the operator typed
* *AND* the adapter MUST NOT request the registry index, resolve a version range, download a package, or write to BucketFS, because no write credential resolved and no index read was attempted
* *AND* the created schema's `adapterNotes` MUST record that path and MUST record no driver version, because nothing resolved one
* *AND* the location MUST be derived from the driver identity for every driver this release supports, so two technologies' pre-placed libraries never share a path

### Scenario: The default driver location is used when the registry answers nothing

* *GIVEN* a request setting neither `BUCKETFS_CONNECTION` nor `DRIVER_PATH`, on a cluster where `ADBC_VS_BUCKETFS` resolves and no UDF has outbound HTTPS egress
* *AND* a readable driver shared library at `<mount>/adbc-drivers/<driver>/supplied/<library>` for the driver the request names
* *WHEN* the adapter provisions the driver
* *THEN* the adapter MUST use exactly that path, and the created schema's `adapterNotes` MUST record it with no driver version
* *AND* the adapter MUST NOT write to BucketFS, because no version was resolved: the index was never read
* *AND* a registry that answered anything at all MUST fail the statement instead, whatever it answered, because only a registry reached not at all is evidence of the air gap this route serves
* *AND* a registry that answered MUST fail the statement with an error that does not state that no path on the mount exists to pre-load a library at, because on this route one does and the adapter reads it
* *AND* a package download that reached no host, and either request's body stopping after its status arrived, MUST each fail the statement too, because the registry answered in each case and nothing about the cluster's egress was learned; the package leg's two run after a version was resolved and the versioned presence check ran, which excludes them a second time
* *AND* a failure to unpack a package, or a BucketFS write that was refused, MUST fail the statement too, because neither is a fact about the cluster's egress

### Scenario: A library at the default location does not short-circuit self-install

* *GIVEN* a readable driver shared library at `<mount>/adbc-drivers/<driver>/supplied/<library>`
* *AND* a request whose BucketFS connection resolves, whether from `BUCKETFS_CONNECTION` or from `ADBC_VS_BUCKETFS`, and whose registry index is readable
* *WHEN* the adapter provisions the driver
* *THEN* the adapter MUST self-install: resolve the driver's version range against the registry index and use the versioned install path
* *AND* the adapter MUST NOT load the library at the default location
* *AND* the presence of that library MUST NOT satisfy the versioned presence check, because an unversioned path would answer for whichever version arrived first

### Scenario: No driver on any route fails naming every route

* *GIVEN* a request setting neither `BUCKETFS_CONNECTION` nor `DRIVER_PATH`, on a cluster where `ADBC_VS_BUCKETFS` does not resolve
* *AND* no readable file at the default driver location for the driver the request names
* *WHEN* the adapter provisions the driver
* *THEN* the adapter MUST return an error naming the default driver location it looked at and the driver it looked for
* *AND* the error message MUST name the reserved connection `ADBC_VS_BUCKETFS` and why that connection did not resolve, so a connection the session merely lacks access to is not reported as a connection that does not exist
* *AND* the error message MUST name both `BUCKETFS_CONNECTION` and `DRIVER_PATH` as the properties that state a route explicitly
* *AND* the adapter MUST NOT request the registry index, download a package, write to BucketFS, or create the virtual schema

### Scenario: An empty default location names the registry that answered nothing

* *GIVEN* a request setting neither `BUCKETFS_CONNECTION` nor `DRIVER_PATH`, on a cluster where `ADBC_VS_BUCKETFS` resolves and no UDF has outbound HTTPS egress
* *AND* no readable file at the default driver location for the driver the request names
* *WHEN* the adapter provisions the driver
* *THEN* the adapter MUST return an error naming the default driver location it looked at and the driver it looked for
* *AND* the error message MUST name the registry it could not reach, rather than reporting the reserved connection as the failure, because that connection resolved
* *AND* the error message MUST NOT state that no path on the mount exists to pre-load a library at, which is the remedy text `driver-provisioning/driver-version-resolution`'s error carries for the explicit route, because this message is naming the very location it looked at
* *AND* the error message MUST name both `BUCKETFS_CONNECTION` and `DRIVER_PATH` as the properties that state a route explicitly
* *AND* the adapter MUST NOT write to BucketFS or create the virtual schema
