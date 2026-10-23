# Feature: driver_version_resolution

Resolves each driver's declared version range against the registry index to one published version and one package URL, and decides from that resolved version whether the mount already holds the driver.

<!-- DELTA:CHANGED -->
## Background

- This feature owns the resolved version, its package for the adapter library's platform, and the presence check; `driver-provisioning/driver-self-install` owns the download, and `driver-provisioning/driver-path-override` owns skipping the index.
- The registry index at `https://dbc-cdn.columnar.tech/index.yaml` lists one entry per driver `path` (`mysql`, `postgresql`, `trino`), each with one `pkginfo` block per version holding a `version` scalar and `platform` and `url` package pairs.
- Package URLs are relative to `https://dbc-cdn.columnar.tech`. A platform is `<os>_<arch>` in lower case: `linux_amd64` for an adapter library compiled for x86_64, and `linux_arm64` for one compiled for aarch64.
- Each driver declares its own version range, and the resolved version keys the install path, the presence check, and the driver version `vs-adapter/create-virtual-schema` records in `adapterNotes`.
<!-- /DELTA:CHANGED -->

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Missing platform package fails with a named error

* *GIVEN* a registry index whose highest in-range version of the driver being provisioned publishes no package for the platform of the architecture the adapter library was compiled for
* *WHEN* the adapter provisions that driver
* *THEN* the adapter MUST return an error
* *AND* the error message MUST name the driver, the platform tuple it looked for, and the resolved version it looked in
* *AND* the adapter MUST NOT silently fall back to a lower in-range version that does publish that platform, because the resolved version is what every other record reports
* *AND* the adapter MUST NOT download the package of another platform that the resolved version does publish, such as `linux_amd64` for an adapter library compiled for aarch64, because the adapter library cannot load a driver built for another architecture
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: Package platform follows the architecture the adapter library was compiled for

* *GIVEN* a registry index whose resolved version of a driver publishes a `linux_amd64` package and a `linux_arm64` package
* *WHEN* the adapter provisions that driver
* *THEN* the adapter MUST download the package whose platform matches the architecture the adapter library was compiled for: `linux_amd64` for x86_64, `linux_arm64` for aarch64
* *AND* it MUST NOT download the other package, because an adapter library runs only on the architecture it was compiled for
<!-- /DELTA:NEW -->
