# Feature: adapter_request_dispatch

Routes each Virtual Schema request Exasol sends to the adapter script, and turns every handler failure into a diagnosable SQL error instead of a malformed response.

<!-- DELTA:CHANGED -->
## Background

- The adapter script is registered as `ADBC_VS.ADBC_ADAPTER` and is declared with the `exasol-udf-macros` `vs_adapter` form, whose callback has the signature `fn(ctx: &mut dyn UdfContext, request_json: &str) -> Result<String, UdfError>`.
- Exasol sends exactly six request types, discriminated by the top-level `type` field: `createVirtualSchema`, `refresh`, `setProperties`, `dropVirtualSchema`, `getCapabilities`, and `pushdown`.
- Every response echoes the request's `type` value in its own `type` field.
- The adapter holds no state between calls. All state Exasol must hand back later travels in `schemaMetadata.adapterNotes`, including the driver identity every later request resolves its dialect and metadata source from.
- Dispatch names no concrete dialect. Both the `getCapabilities` arm and the `pushdown` arm resolve the dialect through `vs-adapter/driver-selection` from one recorded field, which is what keeps the advertised set and the renderer from belonging to different drivers while letting the answer differ per virtual schema.
- Requests are parsed as untyped JSON. The Virtual Schema wire format carries fields the adapter ignores, and a strict typed deserialization would reject a request over an unrelated field.
<!-- /DELTA:CHANGED -->

## Scenarios

### Scenario: Every supported request type reaches its handler

* *GIVEN* a registered adapter script
* *WHEN* the adapter receives a request whose `type` is `createVirtualSchema`, `refresh`, `setProperties`, `dropVirtualSchema`, `getCapabilities`, or `pushdown`
* *THEN* the adapter MUST return a JSON object
* *AND* that object's `type` field MUST equal the request's `type` field
