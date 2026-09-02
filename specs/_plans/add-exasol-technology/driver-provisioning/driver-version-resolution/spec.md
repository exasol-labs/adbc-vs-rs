# Feature: driver_version_resolution

Resolves each driver's declared version range against the registry index to one published version and one package URL, and decides from that resolved version whether the mount already holds the driver.

## Background

This file is a scenario delta. The permanent feature's own Background governs, and the bullets below are what this plan adds to it; task 6.1 applies them to `specs/driver-provisioning/driver-version-resolution/spec.md`, because `/speq:spec-merge` merges an existing feature by scenario name and reads nothing outside `## Scenarios`.

- Each supported driver has its own registry `path` — `mysql`, `postgresql`, and `exasol`.
- The index's version spellings stay non-uniform: the MySQL entry publishes `v0.6.0`, the Exasol entry `v0.13.0`, and the PostgreSQL entry `1.12.0`. Normalizing one optional leading `v` and comparing semantically is what makes one rule serve all three.
- The three ranges are independent data, not one derived rule. MySQL's stops below `0.7.0` because a pre-1.0 minor bump is breaking by SemVer convention; Exasol's stops below `0.14.0` because `0.14.0` is a breaking release nothing has measured and the package's exported init symbol changed after it; PostgreSQL's admits every `1.x`. Two of the three stopping at their next minor is a coincidence of both being pre-1.0, not a shared rule.

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Version range resolves to the highest published version satisfying it

* *GIVEN* a registry index publishing several versions of a driver, some inside that driver's range and some outside it
* *WHEN* the adapter resolves that driver's version range
* *THEN* it MUST select the highest published version satisfying the range
* *AND* it MUST NOT select a version outside the range, whether higher or lower
* *AND* a published `version` scalar carrying a leading `v` MUST resolve identically to the same version spelled without one, because the index spells MySQL's versions `v0.6.0`, Exasol's `v0.13.0`, and PostgreSQL's `1.12.0`
* *AND* a published scalar that cannot be parsed as a version MUST be skipped rather than failing the resolution
* *AND* each supported driver's range MUST be stated per technology rather than derived from a shared rule, because MySQL's stops below `0.7.0` and Exasol's below `0.14.0` while PostgreSQL's admits every `1.x`
<!-- /DELTA:CHANGED -->
