# Feature: driver_selection

Resolves the `DRIVER` property of a virtual schema to the one dialect, driver package, and metadata source that serve it, so every per-driver fact in the adapter is keyed on a single identity rather than restated per call site.

## Background

This file is a scenario delta. The permanent feature's own Background governs, and the bullets below are what this plan adds to it; task 6.1 applies them to `specs/vs-adapter/driver-selection/spec.md`, because `/speq:spec-merge` merges an existing feature by scenario name and reads nothing outside `## Scenarios`.

- The identity has three variants: `mysql`, `postgresql`, and `exasol`. Adding the third needed one row in each of the three tables and no new table, which is the property the closed enum exists to make true rather than aspirational.
- Two identities advertising the same capability set is legitimate rather than a sign one derives from the other. `postgresql` and `exasol` advertise the same membership because both targets implement `FULL OUTER JOIN` directly, and each set is declared in its own technology's file.
- A capability set is therefore no longer sufficient on its own to tell two virtual schemas apart in a test. The scenario that observes a per-dialect set from the shipped adapter distinguishes `mysql` from the other two, which is what the `JOIN_TYPE_FULL_OUTER` difference still supports.

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Each supported DRIVER value resolves to its own dialect and package

* *GIVEN* the `DRIVER` property values `mysql`, `postgresql`, and `exasol`
* *WHEN* each value is resolved to a driver identity
* *THEN* each MUST resolve to a distinct identity
* *AND* each identity MUST yield the dialect, the driver package, and the metadata source belonging to that driver
* *AND* no two identities MUST yield the same shared-library filename or the same registry driver path, because a shared value would make each driver's presence check answer for the other
* *AND* two identities MAY yield the same advertised capability set, as `postgresql` and `exasol` do, because each set is declared in its own technology's file and the equality is a consequence of the targets rather than of one set deriving from the other
<!-- /DELTA:CHANGED -->
