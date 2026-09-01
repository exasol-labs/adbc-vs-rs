# Feature: postgresql_scan

States how the scan reaches a PostgreSQL source, passes its credentials, and coerces the types its driver returns as text.

## Background

- The ADBC PostgreSQL driver's `SetOption` accepts exactly `uri` and rejects `username` and `password` as unknown options, handing `uri`'s value verbatim to libpq's connect call. The separate-credential-option shape `scan-udf/arrow-scan` describes, and `scan-udf/mysql-scan` uses, is therefore impossible here: the credential has to travel inside that one option's value. It travels as a libpq keyword/value connection string built in memory at connect time from the freshly resolved `CONNECTION` object.
- Conninfo form was chosen over URI form because its escaping is total. Single-quoting a value and backslash-escaping an embedded quote or backslash escapes any byte a password can hold, where percent-encoding would let a password containing `&`, `#`, `@`, or `/` change the parse rather than the value. Every parameter the address's query string carries crosses over as a keyword of the same name, so a `sslmode` or `connect_timeout` the operator set is not silently dropped.
- Every invariant about persistence and disclosure still holds, and each is asserted rather than assumed. The connection string is built at the moment of connecting and discarded with the connection attempt; it reaches no scan specification, no `adapterNotes`, no SQL text, no `EXPLAIN VIRTUAL` output, and no log line or error message.
- Redaction is applied by the adapter at the boundary it controls rather than trusted from the driver. The driver's error text quotes libpq, which can echo connection parameters, so the resolved password is stripped from every surfaced message — and from the subject the adapter names as well as from the driver text it wraps, because that subject can be the connection string itself.
- The connection-open path is shared, so the redaction binds every caller rather than the scan alone. The adapter opens the source connection through the same path for `createVirtualSchema`, `refresh`, and `setProperties`.
- This is the plan's one deliberate narrowing of a security property, forced by the driver's API rather than chosen between designs; decision `[6]` records it. What is narrowed is that a credential now passes through a connection string for this driver; what is unchanged is that it is never persisted and never disclosed.
- The driver returns every `numeric` — constrained or not — as a UTF-8 string, documented as the only lossless representation. A column declared `DECIMAL(p,s)` therefore arrives as text and is parsed rather than cast from a decimal, which makes the string form the ordinary case for this driver rather than a fallback the coercion falls back to.
- PostgreSQL `numeric` legitimately holds `NaN`, `Infinity`, and `-Infinity`, none of which any Exasol `DECIMAL` can represent — but the driver spells them on the wire as the lowercase strings `nan`, `inf`, and `-inf`, never PostgreSQL's own `NaN`/`Infinity`/`-Infinity`, so the coercion must match the driver's spellings rather than the source's. A constrained `numeric(p,s)` column — the only shape this release maps — can hold `nan` but not `inf` or `-inf`: PostgreSQL itself rejects an infinite value into a bounded `numeric` with a numeric field overflow error, so `inf`/`-inf` are reachable only through an unconstrained `numeric`, which this release leaves unmapped regardless. Whichever spelling does reach the scan, it is a failure the coercion names rather than a cast that silently yields a zero, a null, or a saturated value.

## Scenarios

### Scenario: Connection string is built from the address and escapes every credential byte

* *GIVEN* an Exasol `CONNECTION` whose address is `postgresql://host:5432/appdb?sslmode=require` and whose password contains a single quote, a backslash, a space, and an ampersand
* *WHEN* the scan builds the PostgreSQL connection value
* *THEN* the value MUST be a libpq keyword/value connection string rather than a URI carrying the credential
* *AND* the host, port, and database MUST be taken from the address, and every parameter its query string carries MUST appear as a keyword of the same name, so `sslmode` is not dropped
* *AND* each value MUST be enclosed in single quotes with an embedded quote or backslash escaped, so the password reaches libpq as the exact bytes the `CONNECTION` object holds
* *AND* the connection MUST succeed against a source whose password contains each of those characters

### Scenario: Credential never appears in any surfaced text

* *GIVEN* a scan against a PostgreSQL source, and a second scan whose `CONNECTION` object holds a wrong password
* *WHEN* the first scan's generated SQL, scan specification, and `adapterNotes` are read back, and the second scan's error is surfaced
* *THEN* none of the generated SQL, the scan specification, or `adapterNotes` MUST contain the password or the user
* *AND* the error the failing scan surfaces MUST NOT contain the password, including within any text the driver or libpq produced
* *AND* the error MUST still name the host and the database it tried to reach, so a wrong credential stays diagnosable
* *AND* the redaction MUST be applied by the scan rather than relied on from the driver

### Scenario: Adapter-side source error surfaces no credential

* *GIVEN* an Exasol `CONNECTION` addressing a PostgreSQL database, whose password field holds a value
* *WHEN* `createVirtualSchema`, `refresh`, or `setProperties` fails against that source, once with a wrong password and once against a host and port nothing listens on
* *THEN* every error surfaced to the querying session MUST NOT contain the source password, in any encoding or escaping
* *AND* it MUST NOT contain the libpq connection string the adapter built, because that string carries the password verbatim
* *AND* it MUST still name the host and the database it addressed, so the failure stays diagnosable
* *AND* the redaction MUST apply to the subject the adapter names as well as to text the driver or libpq produced, and MUST hold for every caller of the shared connection-open path rather than for the scan alone, because the adapter opens the source connection through that same path

### Scenario: A numeric arriving as a string is coerced into its declared DECIMAL

* *GIVEN* a PostgreSQL `numeric(12,4)` column declared in Exasol as `DECIMAL(12,4)`, which the driver returns as a UTF-8 string
* *WHEN* the scan coerces that batch
* *THEN* every value MUST be emitted as the declared `DECIMAL(12,4)` with the source precision and scale intact
* *AND* the string form MUST be parsed as a first-class case rather than reached through a fallback, because this driver returns every `numeric` that way
* *AND* a value whose digits do not fit the declared precision MUST fail with a named error rather than be truncated or rounded

### Scenario: Non-finite numeric fails with a named error

* *GIVEN* a PostgreSQL `numeric(12,4)` column declared in Exasol as `DECIMAL(12,4)`, and a batch arriving with the driver's own spellings `nan`, `inf`, and `-inf` — `nan` reachable from a value the column actually stores, `inf`/`-inf` only from strings built for the test, since PostgreSQL itself refuses to store an infinite value in a constrained `numeric`
* *WHEN* the scan coerces that batch
* *THEN* each value MUST fail with an error naming the value it received and the declared Exasol type
* *AND* the scan MUST NOT emit a zero, a null, or a saturated value in its place
* *AND* a finite value in the same column MUST still coerce, so the failure is per value rather than per column
