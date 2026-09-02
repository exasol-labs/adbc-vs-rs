# Feature: arrow_scan

Executes the pushed-down source statement through the ADBC driver the virtual schema names and streams the driver's Arrow record batches into Exasol without a row-by-row conversion step.

## Background

This file is a scenario delta. The permanent feature's own Background governs, and the bullets below are what this plan adds to it; task 6.1 applies them to `specs/scan-udf/arrow-scan/spec.md`, because `/speq:spec-merge` merges an existing feature by scenario name and reads nothing outside `## Scenarios`.

- Three drivers are supported. What is per technology is how a driver takes its credentials, which bytes those credentials may hold, and whether the driver bounds its own memory — stated by `scan-udf/mysql-scan`, `scan-udf/postgresql-scan`, and `scan-udf/exasol-scan`.
- The driver is loaded with the ADBC entry-point symbol the driver identity names — `AdbcDriverMysqlInit`, `AdbcDriverPostgresqlInit`, or `AdbcDriverInit` — rather than one derived from the filename. The Exasol package is what proves the derivation would fail rather than merely being unpinned: its library is `libadbc_driver_exasol.so` and the only ADBC init symbol it exports is the unqualified `AdbcDriverInit`.
- Releasing each batch before requesting the next is this feature's rule and holds for every driver. Whether peak memory across a whole scan is bounded is not, because a driver that materializes its whole result before returning a reader defeats the discipline from below. The Exasol driver is one such driver, and `scan-udf/exasol-scan` states what that caps.
- A provisioned library is not necessarily a loadable one, and each driver's own runtime linkage differs: the PostgreSQL driver links `libstdc++` and `libgcc_s`, the Exasol driver links `libgcc_s` and no `libstdc++`, and the Go-built MySQL driver links neither.

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Scan connects using credentials resolved at run time

* *GIVEN* a scan specification naming an Exasol `CONNECTION` object, for each driver this release supports
* *WHEN* the scan runs
* *THEN* it MUST read the address, user, and password from that `CONNECTION` object at scan time
* *AND* it MUST NOT read any credential from the scan specification, and the resolved credential MUST NOT outlive the connection attempt
* *AND* the credential MUST NOT be written into the scan specification, into `adapterNotes`, into SQL text, into `EXPLAIN VIRTUAL` output, or into any log line
* *AND* the mechanism that passes it to a driver MUST be the one that driver's own feature states — `scan-udf/mysql-scan`, `scan-udf/postgresql-scan`, or `scan-udf/exasol-scan`
* *AND* a credential byte a driver's own option handling cannot carry unaltered MUST be refused by name rather than passed on altered, and which bytes those are MUST be that driver's own feature's answer
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Scan streams every result batch to Exasol

* *GIVEN* a source table holding more rows than fit in one Arrow record batch, for each driver this release supports
* *WHEN* the scan executes the pushed-down statement
* *THEN* it MUST emit every batch the driver produces
* *AND* the total emitted row count MUST equal the source row count
* *AND* it MUST release each batch before requesting the next, so the scan's own contribution to peak memory does not grow with the result size
* *AND* whether peak memory across the whole scan is bounded MUST be each driver's own feature's claim rather than this one's, because a driver that materializes its whole result before returning a reader defeats the release discipline from below — `scan-udf/exasol-scan` states that this is one such driver and what it caps
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Provisioned driver loads inside the UDF sandbox

* *GIVEN* each supported driver's shared library provisioned to its BucketFS path
* *WHEN* the scan UDF loads each library by its named entry point inside the Exasol UDF sandbox
* *THEN* each load MUST succeed
* *AND* the PostgreSQL library MUST load despite linking `libstdc++` and `libgcc_s`, and the Exasol library MUST load despite linking `libgcc_s`, both of which the Script Language Container supplies rather than BucketFS
* *AND* a library whose own shared-library dependencies cannot be resolved MUST fail with an error naming the library path and the unresolved dependency, distinct from the error a missing file produces
* *AND* each library MUST be loaded through the entry point its driver identity names — `AdbcDriverMysqlInit`, `AdbcDriverPostgresqlInit`, or `AdbcDriverInit` — rather than one derived from the file name, and the Exasol package MUST be the case that proves the derivation would fail, because its library is named `libadbc_driver_exasol.so` while the only ADBC init symbol it exports is the unqualified `AdbcDriverInit`
<!-- /DELTA:CHANGED -->
