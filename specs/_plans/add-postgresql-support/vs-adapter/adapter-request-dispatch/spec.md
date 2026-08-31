# Feature: adapter_request_dispatch

Routes each Virtual Schema request Exasol sends to the adapter script, and turns every handler failure into a diagnosable SQL error instead of a malformed response.

## Background

<!-- DELTA:CHANGED -->
- The adapter script is registered as `ADBC_VS.ADBC_ADAPTER` and is declared with the `exasol-udf-macros` `vs_adapter` form, whose callback has the signature `fn(ctx: &mut dyn UdfContext, request_json: &str) -> Result<String, UdfError>`.
- Exasol sends exactly six request types, discriminated by the top-level `type` field: `createVirtualSchema`, `refresh`, `setProperties`, `dropVirtualSchema`, `getCapabilities`, and `pushdown`.
- Every response echoes the request's `type` value in its own `type` field.
- The adapter holds no state between calls. All state Exasol must hand back later travels in `schemaMetadata.adapterNotes`, including the driver identity every later request resolves its dialect and metadata source from.
- Dispatch names no concrete dialect. Release 0.2's `getCapabilities` and `pushdown` arms each named `MySqlDialect` directly, which is why the two could not disagree; with two drivers both arms resolve the dialect through `vs-adapter/driver-selection` from one field instead, which preserves that property while letting the answer differ per virtual schema.
- Requests are parsed as untyped JSON. The Virtual Schema wire format carries fields the adapter ignores, and a strict typed deserialization would reject a request over an unrelated field.
<!-- /DELTA:CHANGED -->

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Refresh and setProperties re-run metadata discovery

* *GIVEN* a virtual schema whose source has gained a column since creation
* *WHEN* the adapter receives a `refresh` request
* *THEN* the adapter MUST re-run metadata discovery against the source
* *AND* the discovery MUST run through the metadata source the request's recorded driver resolves to
* *AND* the returned `schemaMetadata` MUST include the new column
* *AND* the adapter MUST echo the request's `requestedTables` field when the request carries one
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Handler failure surfaces to the querying session

* *GIVEN* a virtual schema whose configured source is unreachable
* *WHEN* the adapter receives a `createVirtualSchema` request
* *THEN* the adapter MUST return an error rather than an empty `schemaMetadata`
* *AND* the error message MUST name the underlying failure reported by the ADBC driver
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Dropping a virtual schema releases no external resource

* *GIVEN* an existing virtual schema
* *WHEN* the adapter receives a `dropVirtualSchema` request
* *THEN* the adapter MUST return exactly `{"type":"dropVirtualSchema"}`
* *AND* the adapter MUST NOT delete any driver shared library from BucketFS, because other virtual schemas MAY still reference it, whether or not they name the same driver
<!-- /DELTA:CHANGED -->
