# Feature: driver_path_override

Uses a driver shared library the operator placed themselves — named by `DRIVER_PATH`, or found at the default driver location when no BucketFS connection resolves — so a cluster with no outbound HTTPS egress runs a driver the operator supplied and reaches the registry not at all.

## Background

<!-- DELTA:CHANGED -->
- This feature owns the operator-supplied route: use exactly one file, assert nothing about it beyond its readability, read no registry index, download nothing, write nothing, and never fall back. Two things select that route, and they differ only in where the path came from. `DRIVER_PATH` names it. The default driver location is where the adapter looks when the statement named no `BUCKETFS_CONNECTION` and the reserved connection `ADBC_VS_BUCKETFS` did not resolve either, which is the point at which no write credential exists and self-install cannot run.
- One feature rather than two, because the mechanism is one. A path an operator typed and a path the adapter derived reach the same three obligations — exact file, readability only, no fallback — and splitting them would leave two features to keep in agreement about a decision neither owns alone. The feature keeps its name for reference stability; `vs-adapter/create-virtual-schema` owns the property's place among the statement's properties, including the rule that `BUCKETFS_CONNECTION` is optional on every route.
- The default driver location is `<mount>/adbc-drivers/<driver>/supplied/<library>`, where `<mount>` is the read-only BucketFS mount every UDF already sees. It carries no version segment, unlike the install path `driver-provisioning/driver-self-install` writes, because nothing has resolved a version on this route: the registry was never read. The `supplied` segment states who owns the file. It can never collide with an install path, because that path's third segment is always a parsed version and no version parses as `supplied`, and it keeps the operator's file out of the directory tree the adapter writes and deletes.
- The location is a stated interface, which is what separates it from the arrangement release 0.2 shipped. Release 0.2's fallback named a fixed compiled-in path only from inside an error message, and that path stopped being fixed once the install path carried a resolved version. This one is stated by this feature, is version-independent by construction, and is named by the error an operator gets when nothing is there. `DRIVER_PATH` remains the route for a library that lives anywhere else, including outside the `adbc-drivers` tree entirely.
- The default location is consulted last and never first. A library sitting there MUST NOT short-circuit self-install for a statement that has a BucketFS connection, because an unversioned path answers for whichever version arrived first — the defect the versioned install path exists to remove. `vs-adapter/create-virtual-schema` owns the full precedence and the properties that select each level of it.
- The override never falls back. A path holding no readable file fails the request rather than reaching the registry, because a silent fallback would defeat the air gap the route exists to serve and would make an operator's typo look like a slow first query. That holds for the default location too: an empty default location fails the statement rather than provisioning anything.
- Two neighbouring features state the rest of the arrangement. `driver-provisioning/driver-version-resolution` owns the unreachable-registry error that names `DRIVER_PATH`, which is where an operator who did not set it learns it exists; that error keeps naming `DRIVER_PATH` alone, because reaching the default location requires a cluster with no BucketFS connection at all rather than a change one statement can make. `driver-provisioning/driver-self-install` owns the mechanism this route skips.
<!-- /DELTA:CHANGED -->

## Scenarios

<!-- DELTA:NEW -->
### Scenario: The default driver location is used when no BucketFS connection resolves

* *GIVEN* a request setting neither `BUCKETFS_CONNECTION` nor `DRIVER_PATH`, on a cluster where `ADBC_VS_BUCKETFS` does not resolve
* *AND* a readable driver shared library at `<mount>/adbc-drivers/<driver>/supplied/<library>` for the driver the request names
* *WHEN* the adapter provisions the driver
* *THEN* the adapter MUST use exactly that path
* *AND* the adapter MUST NOT request the registry index, resolve a version range, download a package, or write to BucketFS
* *AND* the adapter MUST assert nothing about the library beyond its readability, exactly as it does for a `DRIVER_PATH` the operator typed
* *AND* the created schema's `adapterNotes` MUST record that path and MUST record no driver version, because nothing resolved one
* *AND* the location MUST be derived from the driver identity for every driver this release supports, so two technologies' pre-placed libraries never share a path
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: A library at the default location does not short-circuit self-install

* *GIVEN* a readable driver shared library at `<mount>/adbc-drivers/<driver>/supplied/<library>`
* *AND* a request whose BucketFS connection resolves, whether from `BUCKETFS_CONNECTION` or from `ADBC_VS_BUCKETFS`
* *WHEN* the adapter provisions the driver
* *THEN* the adapter MUST self-install: resolve the driver's version range against the registry index and use the versioned install path
* *AND* the adapter MUST NOT load the library at the default location
* *AND* the presence of that library MUST NOT satisfy the versioned presence check, because an unversioned path would answer for whichever version arrived first
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: No driver on any route fails naming every route

* *GIVEN* a request setting neither `BUCKETFS_CONNECTION` nor `DRIVER_PATH`, on a cluster where `ADBC_VS_BUCKETFS` does not resolve
* *AND* no readable file at the default driver location for the driver the request names
* *WHEN* the adapter provisions the driver
* *THEN* the adapter MUST return an error
* *AND* the error message MUST name the default driver location it looked at, the driver, the reserved connection `ADBC_VS_BUCKETFS`, and why that connection did not resolve, so a connection the session merely lacks access to is not reported as a connection that does not exist
* *AND* the error message MUST name both `BUCKETFS_CONNECTION` and `DRIVER_PATH` as the properties that state a route explicitly
* *AND* the adapter MUST NOT request the registry index, download a package, or write to BucketFS
* *AND* the adapter MUST NOT create the virtual schema
<!-- /DELTA:NEW -->
