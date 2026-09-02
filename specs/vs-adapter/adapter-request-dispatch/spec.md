# Feature: adapter_request_dispatch

Routes each Virtual Schema request Exasol sends to the adapter script, and turns every handler failure into a diagnosable SQL error instead of a malformed response.

## Background

- The adapter script is registered as `ADBC_VS.ADBC_ADAPTER` and is declared with the `exasol-udf-macros` `vs_adapter` form, whose callback has the signature `fn(ctx: &mut dyn UdfContext, request_json: &str) -> Result<String, UdfError>`.
- Exasol sends exactly six request types, discriminated by the top-level `type` field: `createVirtualSchema`, `refresh`, `setProperties`, `dropVirtualSchema`, `getCapabilities`, and `pushdown`.
- Every response echoes the request's `type` value in its own `type` field.
- The adapter holds no state between calls. All state Exasol must hand back later travels in `schemaMetadata.adapterNotes`, including the driver identity every later request resolves its dialect and metadata source from.
- Dispatch names no concrete dialect. Release 0.2's `getCapabilities` and `pushdown` arms each named `MySqlDialect` directly, which is why the two could not disagree; with two drivers both arms resolve the dialect through `vs-adapter/driver-selection` from one field instead, which preserves that property while letting the answer differ per virtual schema.
- Requests are parsed as untyped JSON. The Virtual Schema wire format carries fields the adapter ignores, and a strict typed deserialization would reject a request over an unrelated field.

## Scenarios

### Scenario: Shared object exports the adapter entry symbol

* *GIVEN* a `.so` produced by `make cross-udf-build`
* *WHEN* its dynamic symbol table is inspected
* *THEN* it MUST export the adapter entry symbol `__exa_udf_entry_ADBC_ADAPTER`

### Scenario: Every supported request type reaches its handler

* *GIVEN* a registered adapter script
* *WHEN* the adapter receives a request whose `type` is `createVirtualSchema`, `refresh`, `setProperties`, `dropVirtualSchema`, `getCapabilities`, or `pushdown`
* *THEN* the adapter MUST return a JSON object
* *AND* that object's `type` field MUST equal the request's `type` field

### Scenario: Dropping a virtual schema releases no external resource

* *GIVEN* an existing virtual schema
* *WHEN* the adapter receives a `dropVirtualSchema` request
* *THEN* the adapter MUST return exactly `{"type":"dropVirtualSchema"}`
* *AND* the adapter MUST NOT delete any driver shared library from BucketFS, because other virtual schemas MAY still reference it, whether or not they name the same driver

### Scenario: Refresh re-runs metadata discovery

* *GIVEN* a virtual schema whose source has gained a column since creation, for each driver this release supports
* *WHEN* the adapter receives a `refresh` request
* *THEN* the adapter MUST re-run metadata discovery against the source
* *AND* the discovery MUST run through the metadata source the request's recorded driver resolves to
* *AND* the returned `schemaMetadata` MUST include the new column
* *AND* the adapter MUST echo the request's `requestedTables` field when the request carries one

### Scenario: setProperties re-runs metadata discovery

* *GIVEN* a virtual schema whose changed properties name a different source than creation did
* *WHEN* the adapter receives a `setProperties` request
* *THEN* the adapter MUST re-run metadata discovery against the source the changed properties name
* *AND* the discovery MUST run through the metadata source the request's recorded driver resolves to
* *AND* the returned `schemaMetadata` MUST describe that source rather than the one creation discovered

### Scenario: Unsupported request type fails with a named error

* *GIVEN* a registered adapter script
* *WHEN* the adapter receives a request whose `type` is absent or is not one of the six supported values
* *THEN* the adapter MUST return an error
* *AND* the error message MUST quote the unsupported `type` value it received

### Scenario: Malformed request JSON fails with a diagnosable error

* *GIVEN* a registered adapter script
* *WHEN* the adapter receives a request body that is not valid JSON
* *THEN* the adapter MUST return an error
* *AND* the error message MUST identify the failure as a request-parsing failure
* *AND* the adapter MUST NOT return a response object

### Scenario: Handler failure surfaces to the querying session

* *GIVEN* a virtual schema whose configured source is unreachable
* *WHEN* the adapter receives a `createVirtualSchema` request
* *THEN* the adapter MUST return an error rather than an empty `schemaMetadata`
* *AND* the error message MUST name the underlying failure reported by the ADBC driver
