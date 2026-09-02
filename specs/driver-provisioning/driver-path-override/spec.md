# Feature: driver_path_override

Uses the driver shared library an operator names in `DRIVER_PATH`, so a cluster with no outbound HTTPS egress runs a driver the operator supplied and reaches the registry not at all.

## Background

- `DRIVER_PATH` is the supported way to run without the registry. When `vs-adapter/create-virtual-schema` receives it, the adapter uses exactly that BucketFS path and performs no index read, no download, and no BucketFS write. It applies to every driver technology. A driver an operator supplies this way is one the operator vouches for; adbc-vs asserts nothing about it beyond its readability.
- This feature owns what provisioning does with the property. `vs-adapter/create-virtual-schema` owns the property's place among the statement's properties, including the rule that `BUCKETFS_CONNECTION` becomes optional while `DRIVER_PATH` is present, because its only purpose is the write credential self-install needs.
- The override supersedes the arrangement release 0.2 shipped, in which an unreachable registry named a fixed compiled-in path for an operator to pre-load. That path is no longer fixed once it carries a resolved version, and it was never one an operator could choose. An air-gapped path the operator names is an interface; one discoverable only from an error message is not.
- The override never falls back. A path holding no readable file fails the request rather than reaching the registry, because a silent fallback would defeat the air gap the property exists to serve and would make an operator's typo look like a slow first query.
- Two neighbouring features state the rest of the arrangement. `driver-provisioning/driver-version-resolution` owns the unreachable-registry error that names this property, which is where an operator who did not set it learns it exists. `driver-provisioning/driver-self-install` owns the mechanism this property skips.

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
