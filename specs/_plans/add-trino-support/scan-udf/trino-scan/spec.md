# Feature: trino_scan

States how the scan reaches a Trino source, how the credential travels inside the one connection option the driver accepts, and which transport the driver requires before it will send one.

## Background

- This is the mechanism `scan-udf/arrow-scan`'s framework invariant is satisfied by for Trino. That feature requires the credential to be resolved from the named Exasol `CONNECTION` object at scan time and never persisted; this feature states only where it goes from there.
- The ADBC Trino driver accepts the connection as one `uri` option and documents no `username` or `password` option beside it. The separate-credential-option shape `scan-udf/mysql-scan` uses is therefore unavailable, as it is for PostgreSQL, and the credential travels inside that one option's value.
- Where PostgreSQL's driver takes a libpq keyword/value string whose escaping is total, Trino's takes a URI, whose userinfo component is percent-encoded. The credential is written into that component with every byte outside the RFC 3986 unreserved set percent-encoded, which is what makes the encoding total: a password holding `@`, `/`, `:`, `?`, `#`, `&`, or a multi-byte character reaches Trino as the bytes the `CONNECTION` object holds rather than changing where the URI's authority ends. Encoding more bytes than the grammar requires is always safe in a userinfo component, so the rule is one unconditional pass rather than a set of characters to recognize.
- This is why URI form was rejected for PostgreSQL and is required here. For PostgreSQL a total escaping was available in another form, so the partial percent-encoding of a URI was a needless hazard; for Trino no other form exists, so the encoding is made total instead of avoided.
- A userinfo component the connection address already carries is dropped rather than kept. A user and password written into the address are not the credential — Exasol resolves that beside the address — and treating them as part of the authority would both address the wrong host and put a secret into the subject an error names. Every other component of the address, including its query parameters, crosses over unchanged.
- The driver sends HTTP Basic authentication only over an encrypted connection, so a `CONNECTION` carrying a password reaches Trino over HTTPS or not at all. The driver reads the transport from the address: it uses HTTPS by default, including on port 8080, and uses plain HTTP only when the address carries the parameter that selects it. A statement whose address selects plain HTTP and whose `CONNECTION` object holds a password therefore fails inside the driver, and the adapter surfaces that failure rather than stripping the password to make the connection succeed.
- A Trino server serving a certificate the driver cannot verify is reached by the address's own certificate-verification parameter, which the adapter carries across like every other parameter and neither adds nor removes. Whether to disable verification is the operator's decision, recorded in the `CONNECTION` object rather than in a property or a compiled-in setting, so the adapter has no verification policy of its own for the source. This is separate from the registry client's policy, which `driver-provisioning/driver-self-install` fixes at always verifying.
- Every invariant about persistence and disclosure holds, and each is asserted rather than assumed. The URI carrying the credential is built at the moment of connecting and discarded with the connection attempt; it reaches no scan specification, no `adapterNotes`, no SQL text, no `EXPLAIN VIRTUAL` output, and no log line or error message.
- Redaction is applied by the adapter at the boundary it controls rather than trusted from the driver, and it strips both forms the password takes: the verbatim bytes and their percent-encoded spelling. A driver message quoting the URI it failed on would otherwise disclose the encoded form while a check for the verbatim one passed.
- The failure subject the adapter names is built from the address's host, port, catalog, and schema rather than from the value handed to the driver, because that value carries the password.
- The connection-open path is shared, so the redaction binds every caller rather than the scan alone. The adapter opens the source connection through the same path for `createVirtualSchema`, `refresh`, and `setProperties`.
- The driver's own Arrow representations are what the coercion receives. It returns a `varchar` as a string, a `date` as a 32-bit day value, a `double` as a double, a `bigint` as a 64-bit integer, a `boolean` as an Arrow boolean, and a `decimal` as an Arrow decimal. It returns a `timestamp(p)` at the Arrow unit that precision implies — seconds at precision 0, milliseconds up to 3, microseconds up to 6, and nanoseconds above that — so a nanosecond timestamp is a unit this driver reaches and the other two do not. `scan-udf/arrow-scan` owns the normalization of every unit into the declared Exasol type.

## Scenarios

### Scenario: The credential is percent-encoded into the connection URI

* *GIVEN* an Exasol `CONNECTION` whose address is `trino://host:8443/memory/adbc_vs?SSLVerification=NONE` and whose password contains `@`, `/`, `:`, `#`, `&`, a space, and a multi-byte character
* *WHEN* the scan builds the Trino connection value
* *THEN* the value MUST be that address with the resolved user and password written into its userinfo component
* *AND* every byte of the user and the password outside the RFC 3986 unreserved set MUST be percent-encoded, unconditionally rather than for a recognized set of characters
* *AND* the host, port, catalog, schema, and every query parameter the address carries MUST reach the driver unchanged, so a certificate-verification or transport parameter the operator set is not dropped
* *AND* the connection MUST succeed against a Trino server whose configured password holds each of those characters, so the encoding is proven by authentication rather than by inspection alone

### Scenario: A userinfo component already in the address is replaced rather than kept

* *GIVEN* an Exasol `CONNECTION` whose address carries a user and a password in its own userinfo component, and whose user and password fields hold different values
* *WHEN* the scan builds the Trino connection value
* *THEN* the built value MUST carry the credential the `CONNECTION` object's fields hold
* *AND* it MUST NOT carry the user or the password written into the address
* *AND* the host the value addresses MUST be the address's own host rather than any part of the discarded userinfo component

### Scenario: Credential never appears in any surfaced text

* *GIVEN* a scan against a Trino source, and a second scan whose `CONNECTION` object holds a wrong password
* *WHEN* the first scan's generated SQL, scan specification, and `adapterNotes` are read back, and the second scan's error is surfaced
* *THEN* none of the generated SQL, the scan specification, or `adapterNotes` MUST contain the password or the user
* *AND* the error the failing scan surfaces MUST contain neither the password's verbatim bytes nor its percent-encoded spelling, including within any text the driver produced
* *AND* the error MUST still name the host, the catalog, and the schema it tried to reach, so a wrong credential stays diagnosable
* *AND* the redaction MUST be applied by the scan rather than relied on from the driver

### Scenario: Adapter-side source error surfaces no credential

* *GIVEN* an Exasol `CONNECTION` addressing a Trino server, whose password field holds a value
* *WHEN* `createVirtualSchema`, `refresh`, or `setProperties` fails against that source, once with a wrong password and once against a host and port nothing listens on
* *THEN* every error surfaced to the querying session MUST contain neither the source password nor its percent-encoded spelling, in any encoding or escaping
* *AND* it MUST NOT contain the connection URI the adapter built, because that URI carries the credential
* *AND* it MUST still name the host, the catalog, and the schema it addressed, so the failure stays diagnosable
* *AND* the redaction MUST apply to the subject the adapter names as well as to text the driver produced, and MUST hold for every caller of the shared connection-open path rather than for the scan alone

### Scenario: A password over an unencrypted transport fails rather than being sent or dropped

* *GIVEN* an Exasol `CONNECTION` whose address selects plain HTTP and whose password field holds a value
* *WHEN* the scan opens the Trino connection
* *THEN* the attempt MUST fail with the driver's own diagnosis, because the driver sends Basic authentication only over an encrypted connection
* *AND* the adapter MUST NOT strip the password, downgrade the request, or rewrite the address to make the connection succeed, because either would send a credential the operator asked to protect or silently connect as a different principal
* *AND* the surfaced error MUST name the host and MUST contain neither the password nor its percent-encoded spelling

### Scenario: A timestamp at every precision Trino admits coerces to the declared TIMESTAMP

* *GIVEN* Trino `timestamp(p)` columns at precisions the driver returns as seconds, milliseconds, microseconds, and nanoseconds, each declared in Exasol as `TIMESTAMP`
* *WHEN* the scan coerces those batches
* *THEN* every value MUST be emitted as the declared Exasol `TIMESTAMP`
* *AND* the nanosecond unit MUST be handled rather than rejected, because this driver reaches a unit the other two do not
* *AND* a value whose sub-millisecond digits Exasol cannot hold MUST be handled by the same rule `scan-udf/arrow-scan` states for every driver rather than by a rule of this feature's own
