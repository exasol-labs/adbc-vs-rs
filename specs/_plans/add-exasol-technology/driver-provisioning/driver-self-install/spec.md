# Feature: driver_self_install

Installs the ADBC driver a virtual schema names into BucketFS by downloading it from the public driver registry, so no operator packaging step stands between installing adbc-vs and querying that source.

## Background

This file is a scenario delta. The permanent feature's own Background governs, and the bullets below are what this plan adds to it; task 6.1 applies them to `specs/driver-provisioning/driver-self-install/spec.md`, because `/speq:spec-merge` merges an existing feature by scenario name and reads nothing outside `## Scenarios`.

- Three drivers install under distinct paths and do not collide, because the driver segment differs and the shared-library filenames differ — `libadbc_driver_mysql.so`, `libadbc_driver_postgresql.so`, and `libadbc_driver_exasol.so`.
- The entry-point symbol is per driver and is not derived from the filename. The Exasol package is the case that shows why: its library is `libadbc_driver_exasol.so` and the only ADBC init symbol it exports is the unqualified `AdbcDriverInit`, so a filename-derived symbol would not be found.
- The Exasol package publishes a detached signature file beside its library, and self-install still verifies neither a checksum nor a signature. The `MANIFEST` names the library in its `[Files] driver` entry and provisioning installs only that file, so the signature is present on the mount for nobody to check.
- The Exasol driver links `libgcc_s` and no `libstdc++`, and requires at most `GLIBC_2.16`, so its loadability rests on less of the Script Language Container than the PostgreSQL driver's does. It is still asserted rather than inferred; `scan-udf/arrow-scan` owns that assertion.

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Two drivers install side by side without collision

* *GIVEN* a BucketFS mount holding no driver
* *WHEN* one virtual schema provisions the MySQL driver, a second provisions the PostgreSQL driver, and a third provisions the Exasol driver
* *THEN* every shared library MUST be present on the mount afterwards, under install paths that differ in their driver segment
* *AND* provisioning a later driver MUST NOT overwrite, delete, or re-upload an earlier one
* *AND* the presence check for any driver MUST NOT be satisfied by another driver's library
* *AND* the install paths MUST be pairwise distinct across every supported driver, asserted over the driver identities rather than over a written list
* *AND* the shared-library filenames MUST also be pairwise distinct, so that a path collision is not the only thing keeping them apart
<!-- /DELTA:CHANGED -->
