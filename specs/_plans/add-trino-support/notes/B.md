# Group B: measurement gates 2.0 - 2.5

Every result below was executed on 2026-09-23 against **Trino 483** (`trinodb/trino:483`, `SELECT version()` returned `483`) and **Exasol 2025.2.1** (`EXA_METADATA.databaseProductVersion` returned `2025.2.1`), using the ADBC Driver Foundry Trino driver **v0.5.3** (`linux_amd64`), which is the highest version the registry publishes and the only one in `drivers/trino.toml`'s range `>=0.5.3, <0.6.0`.

## Summary

| Gate | Verdict | What it changes |
|------|---------|-----------------|
| 2.0 driver loads in the UDF sandbox | **PASS** | nothing |
| 2.1 connection path | **PASS on the scheme, FAIL on two claims** | `scan-udf/trino-scan` needs re-deciding: a separate `username`/`password` option exists, and a multi-byte password cannot be sent as UTF-8 |
| 2.2 `information_schema` + `char(n)` | **PASS** | one correction: `varchar(0)` is a legal Trino type |
| 2.3 rendering rules | **PASS on all nine sub-claims** | three nuances recorded below, one of them a real correction to the `LITERAL_DOUBLE` rationale |
| 2.4 Arrow types / coercion gaps | **PASS, and no gap exists** | task 5.3 has nothing to close; `Decimal32`/`Decimal64` are reached and already cast |
| 2.5 `COLLATE` is not a Trino keyword | **PASS** | nothing |

Two STOP conditions in the plan's own wording were hit, both in gate 2.1. Neither blocks the whole plan the way a 2.0 failure would; both are `scan-udf/trino-scan` decisions that must be re-made before task 5.1 or 5.2 is written. They are recorded in `decision-log.md` under `## Review Findings` as `[gate] ...` entries.

## Measurement environment

- `make stack-up-trino` brought up `labs-adbc-vs-trino-1` unchanged from Group A's `docker-compose.yml`, `config.properties`, `password-authenticator.properties`, and `init.sql`.
- Gate 2.1 needed Trino users whose passwords carry hostile characters. **No repo file was edited.** A scratchpad compose override replaced only the `password.db` bind mount with a scratchpad copy that keeps Group A's `adbc_vs` line verbatim and appends probe users. The container was restored to Group A's own file afterwards (see "Environment left behind").
- The Exasol container could not bind its compose host ports: an unrelated standalone container named `exasol-test` (`exasol/docker-db:2025.2.0`, up 10 days, not Compose-managed) already holds `0.0.0.0:8563` and `0.0.0.0:2581`. A scratchpad compose override republished the same container ports on `127.0.0.1:18563/12580/12581` for the measurement. **Anyone running `make stack-up-exasol` or `make test-e2e-trino` on this host will hit that port conflict** until `exasol-test` is stopped; it is a host condition, not a defect in this plan.
- One further host condition: an orphaned docker network `labs-adbc-vs-4_adbc-vs` (from a `labs-adbc-vs-4` worktree that no longer exists) owned subnet `172.28.5.0/24` and made `docker compose up` fail with `failed to create network labs-adbc-vs_adbc-vs: Error response from daemon: Pool overlaps with other one on this address space`. It had no containers attached and was removed. A sibling checkout `labs-adbc-vs-3` exists, so only one adbc-vs stack can run host-wide at a time.

---

## Gate 2.0 - the Trino driver library loads inside the Exasol UDF sandbox

**Verdict: PASS.** No change to any plan text.

### Package

`https://dbc-cdn.columnar.tech/index.yaml` publishes nine Trino versions: `v0.1.0, v0.2.0, v0.3.0, v0.3.1, v0.4.0, v0.5.0, v0.5.1, v0.5.2, v0.5.3`. `v0.5.3` is the newest, so `drivers/trino.toml`'s `>=0.5.3, <0.6.0` resolves to exactly one version today.

`trino/v0.5.3/trino_linux_amd64_v0.5.3.tar.gz` is **11.5 MB** compressed. Its `MANIFEST` states:

```
version = "v0.5.3"

[ADBC]
version = "v1.1.0"

[Files]
driver = "libadbc_driver_trino.so"
signature = "libadbc_driver_trino.so.sig"
```

The library is **43,064,128 bytes** (41.07 MiB). The plan's "roughly 43MB" is right to the byte count and `drivers/trino.toml`'s `library`, `entry_point`, and ADBC `1.1.0` values all match the manifest.

### Link surface

```
nm -D --defined-only libadbc_driver_trino.so | grep AdbcDriver
  0000000001367170 T AdbcDriverInit
  0000000001366a50 T AdbcDriverTrinoInit

readelf -d:  NEEDED libpthread.so.0, libresolv.so.2, libc.so.6
max GLIBC symbol version required: GLIBC_2.3.2
```

`libresolv.so.2` is the only dependency beyond libc/libpthread and it is present in the SLC. The maximum required glibc symbol version is 2.3.2, so the SLC's glibc is never the binding constraint.

### BucketFS upload and visibility

Uploaded to `adbc-drivers/trino/0.5.3/libadbc_driver_trino.so` through the `default` bucket's HTTPS write endpoint:

- `PUT` returned HTTP **200**, elapsed under 1 s.
- The object became readable under the container's `/exa/data/bucketfs/bfsdefault/.dest/default/...` tree after **96 ms** of 250 ms polling, at exactly 43,064,128 bytes.

### Load inside the sandbox

Two independent sandboxes were measured.

1. **`builtin_python3` sandbox**, via `ctypes.CDLL` plus `getattr(lib, 'AdbcDriverTrinoInit')`:
   `OK exists=True size=43064128 symbol_addr=7f0210d66a50`

2. **The real `RUST` SLC** (`SCRIPT_LANGUAGES` carries `RUST=localzmq+protobuf:///bfsdefault/default/slc/rustslc?lang=rust#buckets/bfsdefault/default/slc/rustslc/exaudf/exaudfclient`, the same `language-container-rs` 0.23.0 container `make test-e2e` installs). A throwaway `cdylib` was built in `rust:1.94-trixie` against `exasol-udf-sdk =0.23.0` and calls exactly what `connection.rs` calls:

   ```rust
   ManagedDriver::load_dynamic_from_filename(path, Some(b"AdbcDriverTrinoInit"), AdbcVersion::V110)
   ```

   Result: `LOAD_OK size=43064128`.

3. **Bonus, not required by the gate:** the same UDF then opened a database and a connection with `uri = trino://adbc_vs:adbc_vs_password@172.28.5.12:8443/memory/adbc_vs?SSLVerification=NONE` and executed a query, returning `LOAD_OK size=43064128 QUERY_OK rows=[adbc_vs,3]`. So the sandbox reaches the Trino container over HTTPS on the static IP, authenticates, and streams Arrow back. Group H's E2E tier has no unknown left on this path.

**Note for anyone repeating this:** pinning `exasol-udf-sdk` at `"0.23.0"` rather than `"=0.23.0"` resolves to 0.23.1 and the SLC rejects the object with `F-UDF-CL-RUST-9001: Fingerprint mismatch: expected 0.23.0:rustc_1.94.1__e408947bf_2026-03-25_, found 0.23.1:...`. The repo's own `Cargo.lock` pins it, so `make cross-udf-build` is unaffected.

---

## Gate 2.1 - the connection path

**Verdict: the scheme claim PASSES. Two of the feature's own claims FAIL and `scan-udf/trino-scan` must be re-decided before tasks 5.1 and 5.2.**

### 2.1a - URI scheme: PASS, no STOP

`vs-adapter/driver-selection` requires the address's scheme to equal the driver's canonical name, i.e. `trino`. Measured, with the credential in the userinfo and `SSLVerification=NONE`:

| scheme | result |
|--------|--------|
| `trino://` | **OPEN OK, query returned `adbc_vs \| 483`** |
| `https://` | OPEN OK, query returned `adbc_vs \| 483` |
| `http://` | opens, then the query fails: `"Post \"http://localhost:8443/v1/statement\": net/http: HTTP/1.x transport connection broken: malformed HTTP response \"\\x15\\x03\\x03\\x00\\x02\\x02P\""` |
| `trino+https://` | opens, then: `"Post \"https://trino+https/v1/statement\": dial tcp: lookup trino+https: no such host"` |

So `trino://` is accepted and is not merely tolerated: it selects HTTPS. The scheme-match rule in `vs-adapter/driver-selection` needs no change. Note that `trino+https://` was **not rejected** - the driver silently mis-parsed it into a host named `trino+https`, so an address the adapter does not validate itself fails late and obscurely.

**`new_database_with_opts` and `new_connection` are lazy.** Every row above "opened" successfully; the transport and the credential are only exercised on `execute()`. Any test that asserts a connection authenticates MUST run a statement - `scan-udf/trino-scan`'s "assert the connection authenticates rather than merely opening" is therefore a real requirement, not a formality.

### 2.1b - separate `username` / `password` database options: **THEY EXIST. STOP CONDITION HIT.**

`scan-udf/trino-scan`'s Background states: "The ADBC Trino driver accepts the connection as one `uri` option and documents no `username` or `password` option beside it." Measured, that is false for this driver version.

With a URI carrying **no userinfo at all** (`trino://localhost:8443/memory/adbc_vs?SSLVerification=NONE`) plus the two options:

```
username = "g_plain", password = "plainpassword"   ->  OK user=g_plain
```

The options are effective for every hostile ASCII character too (full matrix in 2.1c). They are also real registered options rather than pass-through noise: setting `password` through `AdbcDatabaseSetOptionBytes` is rejected with

```
NotImplemented: [Trino] Unknown database option 'password'
```

which proves the driver maintains an option registry and that `password` is registered as a string option only. The library's own string table contains `uri`, `username`, `password`, `passwordSet`, `SSL`, `SSLCert`, `SSLCertPath`.

**Precedence, measured:** the separate options **override** the URI userinfo, they do not merge with it as a fallback.

| URI userinfo | separate options | result |
|---|---|---|
| `g_at` valid | `g_plain` valid | `OK user=g_plain` - the separate options won |
| `g_plain` valid | `g_plain` + wrong password | `401 "Access Denied: Invalid credentials"` - the separate options won |
| `g_plain` user only, no password | `password` only | `OK user=g_plain` - they compose per field |
| `nobody:plainpassword` | `username` only | `OK user=g_plain` - per field again, user from the option, password from the URI |

`uri` remains mandatory: omitting it gives `InvalidArguments: [Trino] failed to create database: missing required option uri`.

**Consequence for the plan.** The plan's own wording is "If a separate option exists, STOP and report: `scan-udf/trino-scan` needs re-planning, the credential would leave the URI and the percent-encoding rule and its redaction scenarios go with it." That is now a decision a human has to make, and 2.1c below gives it a second, harder input.

### 2.1c - percent-encoded hostile credential: PASS for every ASCII character, **FAILS for a multi-byte character**

Probe users were configured in Trino with one hostile character each. Each row is one authentication attempt executing `SELECT current_user`.

| password | URI userinfo, percent-encoded | URI userinfo, verbatim | separate `username`/`password` |
|---|---|---|---|
| `plainpassword` | OK | OK | OK |
| `abc@def` | OK | OK | OK |
| `abc/def` | OK | `InvalidArguments` at database creation | OK |
| `abc:def` | OK | OK | OK |
| `abc?def` | OK | `InvalidArguments` at database creation | OK |
| `abc#def` | OK | `InvalidArguments` at database creation | OK |
| `abc&def` | OK | OK | OK |
| `abc%def` | OK | `401 Access Denied: Invalid credentials` | OK |
| `abc def` | OK | `InvalidArguments` at database creation | OK |
| `abcüdef` | **401 Access Denied: Invalid credentials** | `InvalidArguments` at database creation | **401 Access Denied: Invalid credentials** |
| `p@ss/w:rd?#&x-y` | **OK** | `"Post \"https://...\""` transport error | **OK** |
| `p@s%s/w:rd?#&x ü` | **401 Access Denied: Invalid credentials** | `invalid Trino URI format: parse "trino://gate_user:p@s%s/w:rd?": invalid URL escape "%s"` | **401 Access Denied: Invalid credentials** |

Two conclusions.

1. **The unconditional percent-encoding rule is proven for every ASCII character the feature names** (`@ / : ? # & %` and a space). `p@ss/w:rd?#&x-y` authenticates through the userinfo when encoded and fails to parse when not, which is exactly the rule `scan-udf/trino-scan` states and exactly why it is needed.
2. **A multi-byte character cannot be sent as UTF-8 at all, by either mechanism.** This is a Trino **server** property, not a driver defect. Probing the server's REST endpoint directly with a hand-built `Authorization: Basic` header:

   | header payload | HTTP |
   |---|---|
   | base64 of `g_plain:plainpassword` (control) | **200** |
   | base64 of the **UTF-8** bytes of `g_utf8:abcüdef` | **401** |
   | base64 of the **ISO-8859-1** bytes of the same credential | **200** |
   | base64 of `g_utf8:abcXdef` (negative control) | 401 |

   Trino 483 decodes HTTP Basic credentials as **ISO-8859-1**, which is RFC 7617's default charset. The Java Trino CLI works because OkHttp's `Credentials.basic` also defaults to ISO-8859-1. The ADBC driver sends the UTF-8 bytes.

   There is a workaround and it is ugly but real: the driver percent-**decodes** the userinfo into raw bytes and base64s those bytes unchanged, so encoding the password's **ISO-8859-1** bytes rather than its UTF-8 bytes authenticates:

   ```
   trino://g_utf8:abc%C3%BCdef@...  ->  401 Access Denied: Invalid credentials
   trino://g_utf8:abc%FCdef@...     ->  OK user=g_utf8
   ```

   That only covers U+0000..U+00FF. A password holding `€`, a CJK character, or an emoji cannot be expressed in ISO-8859-1 and therefore cannot reach Trino 483 through HTTP Basic from any client.

**Consequence for the plan.** `scan-udf/trino-scan`'s scenario "The credential is percent-encoded into the connection URI" requires "the connection MUST succeed against a Trino server whose configured password holds each of those characters", where "those characters" includes "a multi-byte character". As written that scenario is **unsatisfiable against Trino 483**. It has to be narrowed to the ASCII set, or restated over the ISO-8859-1 byte encoding, or the multi-byte clause has to be dropped with the reason recorded.

### 2.1d - wrong password: PASS

```
401 Unauthorized: "Access Denied: Invalid credentials"
```

surfaced as `IO: [Trino] failed to create record reader: failed to execute query: trino: query failed (401 Unauthorized): "Access Denied: Invalid credentials"`. The message quotes **neither** the URI nor the password, so the driver itself discloses nothing here - but `scan-udf/trino-scan`'s "redaction MUST be applied by the scan rather than relied on from the driver" still binds, because the `InvalidArguments` parse failure above **does** quote the URI:

```
InvalidArguments: [Trino] failed to create database: invalid Trino URI format: parse "trino://gate_user:p@s%s/w:rd?": invalid URL escape "%s"
```

That message carries a prefix of the password verbatim. Task 5.2's redaction must therefore cover the `new_database_with_opts` failure path, not only the connect and execute paths.

### 2.1e - certificate verification: PASS

Against the self-signed certificate `make stack-up-trino` generates:

| address parameter | result |
|---|---|
| `?SSLVerification=NONE` | OK |
| `?SSLVerification=none` | OK (the value is case-insensitive) |
| `?SSLVerification=FULL` | `"tls: failed to verify certificate: x509: certificate relies on legacy Common Name field, use SANs instead"` |
| `?SSLVerification=CA` | same x509 error |
| no parameter at all | same x509 error |
| `?SSLInsecure=true` | same x509 error - **not a recognised parameter** |

`SSLVerification=NONE` is the one spelling that works, which is the spelling `scan-udf/trino-scan` and the plan already use.

### 2.1f - a password over plain HTTP: PASS, with a caveat about which layer refuses

`scan-udf/trino-scan` states that the *driver* sends Basic authentication only over an encrypted connection and that the attempt therefore "MUST fail with the driver's own diagnosis". Measured, the refusal comes from the **Trino server**, not the driver:

| address | result |
|---|---|
| `trino://user:pw@localhost:8080/...` | `"Post \"https://localhost:8080/v1/statement\": http: server gave HTTP response to HTTPS client"` - the driver used HTTPS by default on port 8080, as the feature says |
| `trino://user:pw@localhost:8080/...?SSL=false` | `403 Forbidden: "Error 403 Forbidden: Authentication over HTTP is not enabled"` |
| `http://user:pw@localhost:8080/...` | `403 Forbidden: "Error 403 Forbidden: Authentication over HTTP is not enabled"` |

Both halves of the feature's claim hold - HTTPS is the default including on port 8080, and a password over plain HTTP fails rather than being sent silently - but the second row shows the driver **did** transmit the credential over plain HTTP; Trino's own `http-server.authentication.allow-insecure-over-http=false` default is what refused it. A Trino deployment that enabled that setting would accept the credential in the clear. The scenario's wording "fails inside the driver" is a correction: the driver does not refuse it, the server does, and `?SSL=false` is the parameter that selects plain HTTP.

---

## Gate 2.2 - `information_schema`, `data_type` spellings, and `char(n)`

**Verdict: PASS.** Every `data_type` spelling `vs-adapter/trino-type-mapping` names is exactly what Trino 483 returns, and the `char(n)` disagreement is real.

### `information_schema.columns` shape

The table has exactly eight columns and no more:

```
table_catalog:varchar, table_schema:varchar, table_name:varchar, column_name:varchar,
ordinal_position:bigint, column_default:varchar, is_nullable:varchar, data_type:varchar
```

Selecting the columns MySQL's and PostgreSQL's catalogs carry fails, exactly as the feature states:

```
SELECT character_maximum_length ...  ->  line 1:8: Column 'character_maximum_length' cannot be resolved
SELECT numeric_precision, numeric_scale ...  ->  line 1:8: Column 'numeric_precision' cannot be resolved
SELECT datetime_precision ...  ->  line 1:8: Column 'datetime_precision' cannot be resolved
```

`information_schema.tables` has exactly four columns: `table_catalog, table_schema, table_name, table_type`, all `varchar`.

### Exact `data_type` strings

One column of each type the type-mapping feature names, read back through the **driver** (identical to what the CLI returns):

| column | `data_type` | `is_nullable` |
|---|---|---|
| `INTEGER` | `integer` | `YES` |
| `BIGINT` | `bigint` | `YES` |
| `DECIMAL(12,4)` | `decimal(12,4)` | `YES` |
| `DECIMAL(30,2)` | `decimal(30,2)` | `YES` |
| `DECIMAL(38,2)` | `decimal(38,2)` | `YES` |
| `VARCHAR(100)` | `varchar(100)` | `YES` |
| `VARCHAR` | `varchar` | `YES` |
| `DATE` | `date` | `YES` |
| `TIMESTAMP(0)` | `timestamp(0)` | `YES` |
| `TIMESTAMP(3)` | `timestamp(3)` | `YES` |
| `TIMESTAMP(6)` | `timestamp(6)` | `YES` |
| `TIMESTAMP(9)` | `timestamp(9)` | `YES` |
| `DOUBLE` | `double` | `YES` |
| `CHAR(5)` | `char(5)` | `YES` |
| `BOOLEAN` | `boolean` | `YES` |
| `REAL` | `real` | `YES` |
| `SMALLINT` | `smallint` | `YES` |
| `TINYINT` | `tinyint` | `YES` |
| `TIME(3)` | `time(3)` | `YES` |
| `TIME(3) WITH TIME ZONE` | `time(3) with time zone` | `YES` |
| `TIMESTAMP(3) WITH TIME ZONE` | `timestamp(3) with time zone` | `YES` |
| `VARBINARY` | `varbinary` | `YES` |
| `JSON` | `json` | `YES` |
| `UUID` | `uuid` | `YES` |
| `IPADDRESS` | `ipaddress` | `YES` |
| `ARRAY(INTEGER)` | `array(integer)` | `YES` |
| `MAP(VARCHAR, INTEGER)` | `map(varchar, integer)` | `YES` |
| `ROW("a" INTEGER)` | `row("a" integer)` | `YES` |
| `INTERVAL DAY TO SECOND` | `interval day to second` | `YES` |
| `INTERVAL YEAR TO MONTH` | `interval year to month` | `YES` |

Every spelling is lower case, the decimal parameters are separated by a comma **with no space**, and `map(varchar, integer)` carries a comma **followed by a space** - the two are spelled differently and the parser in task 7.1 must not assume one rule covers both. `row("a" integer)` embeds double quotes, so a `data_type` value carried verbatim into an omission reason can contain quote characters.

`is_nullable` is the string `YES` (not a boolean, not `t`). `ordinal_position` is 1-based and contiguous.

`table_type` values, measured over three base tables, one view: `BASE TABLE` and `VIEW`, exactly as the feature states.

### One correction: `varchar(0)` is legal

`vs-adapter/trino-type-mapping` says a declared length of 0 must map to Exasol size 1, and separately that bare `varchar` is unbounded rather than zero-length. Both halves hold, and the first is not hypothetical: `CREATE TABLE ... (c VARCHAR(0))` **succeeds** on Trino 483 and the catalog reports `varchar(0)`. The parser therefore really does meet a zero length, and the scenario's clause is load-bearing rather than defensive.

### `char(n)`: the disagreement is real - PASS

```sql
SELECT CAST('abc' AS CHAR(5)) LIKE 'abc'   ->  false
SELECT CAST('abc' AS CHAR(5)) =    'abc'   ->  true
SELECT CAST('abc' AS CHAR(5)) LIKE 'abc  ' ->  true
SELECT length(CAST('abc' AS CHAR(5)))      ->  5
```

`LIKE` sees the blank padding, `=` does not. The second reason `vs-adapter/trino-type-mapping` gives for leaving `char(n)` unmapped stands and no spec correction is needed. The value also arrives through the driver padded - `c_char_5` returned `abc  ` as Arrow `Utf8` - so mapping it to Exasol `VARCHAR` would additionally return the padding to the user.

---

## Gate 2.3 - the rendering rules against Trino 483 and Exasol 2025.2.1

**Verdict: PASS on all nine sub-claims.** Three results need a sentence changed in plan text; none changes a rendering rule.

### (a) Ordering, equality, `MIN`/`MAX`, `GROUP BY`, `COUNT(DISTINCT)` - PASS

Every Exasol query below casts to `VARCHAR(10)` explicitly, because an Exasol **string literal is `CHAR`** and `CHAR` comparison is blank-padded: `'a' = 'a '` is **true** for bare literals and **false** once both sides are `VARCHAR`. The mapped columns are `VARCHAR`, so the `VARCHAR` answers are the binding ones. **Group H must cast in the local mirror too**, or the trailing-space differential assertion will compare the wrong thing.

`ORDER BY` over `a, B, Z, z, A, ä, Ä, é, e`:

```
Trino 483:      A  B  Z  a  e  z  Ä  ä  é
Exasol 2025.2.1: A  B  Z  a  e  z  Ä  ä  é
```

Identical, and identical to the order `vs-adapter/trino-collation` predicted without having executed it.

`min`/`max` over `a, B, Z, ä`:

```
Trino:  min = B   max = ä
Exasol: min = B   max = ä
```

Identical, and identical to the feature's prediction.

`min`/`max` over `B, a, C` (the values `vs-adapter/trino-collation`'s scenario names):

```
Trino:  min = B   max = a
Exasol: min = B   max = a
```

Comparison predicates, both engines returning the same answer for each:

| predicate | Trino | Exasol (VARCHAR) |
|---|---|---|
| `'a' < 'B'` | false | false |
| `'Z' < 'a'` | true | true |
| `'a' = 'A'` | false | false |
| `'ä' = 'a'` | false | false |
| `'ä' = 'Ä'` | false | false |

`GROUP BY` over `a, A, ä, Ä, B, a` - both return five groups, `a` with count 2:

```
Trino:  A/1  B/1  a/2  Ä/1  ä/1
Exasol: A/1  B/1  a/2  Ä/1  ä/1
```

`COUNT(DISTINCT)` over the same six values: **5** on both.

Trailing space, `VARCHAR` on both sides: `'a' = 'a '` false, `'a' < 'a '` true, `min = 'a'`, `max = 'a '` - identical on both engines.

The no-collation rule is proven rather than assumed.

### (b) `DOUBLE '<value>'` versus a bare constant - PASS on the typing claim, but the stated consequence is wrong

The typing claim is exactly right:

```
typeof(123.45)         ->  decimal(5,2)
typeof(19.99)          ->  decimal(4,2)
typeof(-0.5)           ->  decimal(1,1)
typeof(DOUBLE '123.45')->  double
```

But `vs-adapter/trino-pushdown` says "rendering a `LITERAL_DOUBLE` bare would hand Trino an exact-numeric operand where Exasol meant a floating-point one", implying a wrong answer. Measured, **a bare decimal constant compared against a `double` column returns the same rows as the typed form** - Trino promotes the decimal operand to `double` (`typeof(price + 0.01)` is `double`):

```
WHERE price = 19.99           ->  row 1
WHERE price = DOUBLE '19.99'  ->  row 1
WHERE price = 0.01            ->  row 3
WHERE price = DOUBLE '0.01'   ->  row 3
WHERE amount = 1234.5600          ->  row 1   (decimal column, bare constant)
WHERE amount = DOUBLE '1234.56'   ->  row 1
```

The typed form is still the right rule, for three measured reasons that are about **totality** rather than about a wrong comparison:

1. A literal spelled with an exponent is already `double` (`typeof(1e308)` and `typeof(1.7976931348623157E308)` are both `double`), so the bare form's type depends on the spelling Exasol happens to send.
2. `Infinity` and `NaN` have **no bare spelling** - `SELECT Infinity` fails with `line 1:8: Column 'infinity' cannot be resolved` and `SELECT NaN` with `Column 'nan' cannot be resolved`, while `DOUBLE 'Infinity'` and `DOUBLE 'NaN'` both work.
3. A bare constant above 38 significant digits is rejected outright - see (h).

**Correction to record:** the reason `LITERAL_DOUBLE` renders `DOUBLE '<value>'` is that the typed form is total over every value Exasol can send, not that a bare constant returns wrong rows. The rendering rule is unchanged.

### (c) `LIKE` with and without `ESCAPE '\'` - PASS, and the divergence is larger than the feature states

| test | Trino 483 | Exasol 2025.2.1 |
|---|---|---|
| `'a%b' LIKE 'a\%b' ESCAPE '\'` | true | true |
| `'axb' LIKE 'a\%b' ESCAPE '\'` | false | false |
| `'a%b' LIKE 'a\%b'` (no ESCAPE) | **false** | **true** |
| `'a\%b' LIKE 'a\%b'` (no ESCAPE) | **true** | **false** |
| `'a_b' LIKE 'a\_b' ESCAPE '\'` | true | true |
| `'a_b' LIKE 'a\_b'` (no ESCAPE) | **false** | **true** |
| `'a\b' LIKE 'a\\b' ESCAPE '\'` | true | true |
| `'a\b' LIKE 'a\\b'` (no ESCAPE) | **false** | **true** |
| `'ab' LIKE 'a\b' ESCAPE '\'` | **error** | **true** |
| `'a\b' LIKE 'a\b' ESCAPE '\'` | **error** | false |
| `'a\b' LIKE 'a\b'` (no ESCAPE) | true | false |
| `'aXb' LIKE 'a\Xb' ESCAPE '\'` | **error** | true |

Two measured facts, both of which the plan needs:

1. **Trino defines no default escape character and Exasol's default IS the backslash.** Every "no ESCAPE" row disagrees between the two engines. This is the strongest possible justification for `vs-adapter/trino-pushdown`'s rule that the Trino dialect renders an explicit `ESCAPE` clause for *every* pattern, not only for a request carrying an `escapeChar`.
2. **Trino's escape rule is strict and Exasol's is permissive.** The exact Trino error, with no line or column because it is raised during execution rather than parsing:

   ```
   Escape character must be followed by '%', '_' or the escape character itself
   ```

   Exasol instead treats the escape character plus any other character as that character taken literally, which is why `'ab' LIKE 'a\b' ESCAPE '\'` is true on Exasol and an error on Trino.

   **The set task 6.3 must reject is therefore precise: any character that is not `%`, not `_`, and not the escape character itself.** The rendered statement would otherwise fail at the source with the message above, and `vs-adapter/pushdown-filter`'s exclusion is what keeps that from reaching a user.

### (d) `FULL OUTER JOIN` - PASS, all four forms execute and null-extend both sides

Equality condition over a fixture where each side holds one unmatched row:

```
ak   | av   | bk   | bv
1    | a1   | NULL | NULL
2    | a2   | 2    | b2
3    | a3   | 3    | b3
NULL | NULL | 4    | b4
```

Non-equi conditions, each over a fixture built so that a row on each side matches nothing:

| condition | result |
|---|---|
| `ON a.k < b.k` over `a(1,99)` and `b(0,2)` | `1\|2`, `99\|NULL`, `NULL\|0` - executes, null-extends both sides |
| `ON a.k <> b.k` over `a(5)` and `b(5)` | `5\|NULL`, `NULL\|5` - executes, null-extends both sides |
| `ON true` over `a(1,2)` and an empty `b` | `1\|NULL`, `2\|NULL` - executes, null-extends |

No condition restriction of any kind. `vs-adapter/trino-pushdown`'s claim that PostgreSQL's merge-joinable/hash-joinable restriction has no Trino counterpart is confirmed, and `JOIN_TYPE_FULL_OUTER` is safe to advertise on the keyword alone.

### (e) A select-list predicate over the query's own bare `GROUP BY` key - PASS

```sql
SELECT "name", "name" = 'Widget' AS pred, count(*) AS c
FROM memory.adbc_vs.orders GROUP BY "name"
```

returns `Gadget|false|1`, `Gizmo|false|1`, `Widget|true|1`. A scalar function over the same key (`substr("name", 1, 2)`) is accepted too. Trino's strict grouping rule is satisfied by the bare key, exactly as `vs-adapter/trino-collation` states, and MySQL's composed-shape exclusion has no Trino analogue.

### (f) `E'a'` - PASS, but the error is not a parse error

```
SELECT E'a'  ->  Query 20260923_134439_00083_zwktf failed:
  com.google.common.util.concurrent.UncheckedExecutionException: io.trino.spi.type.TypeNotFoundException: Unknown type: e
```

`vs-adapter/trino-pushdown` predicted "parses as a typed literal of an unknown type `e` and fails" and that is literally what happens - `typeof(E'a')` gives the same message. The surfaced text is a Java exception chain rather than a clean SQL diagnostic, which matters only for what an E2E assertion may match on. The string-literal rule (double the single quote, escape nothing else) is unchanged.

### (g) A bare quoted string against a temporal column - PASS

```
SELECT DATE '2024-01-01' = '2024-01-01'
  ->  line 1:26: Cannot apply operator: date = varchar(10)

WHERE order_date = '2026-01-15'
  ->  line 1:55: Cannot apply operator: date = varchar(10)

WHERE order_date BETWEEN '2026-01-01' AND '2026-02-01'
  ->  line 1:55: Cannot check if date is BETWEEN varchar(10) and varchar(10)

WHERE created_at = '2026-01-15 08:30:00.123456'
  ->  line 1:55: Cannot apply operator: timestamp(6) = varchar(26)
```

The typed forms work (`WHERE order_date BETWEEN DATE '2026-01-01' AND DATE '2026-02-01'` returns row 1; `typeof(TIMESTAMP '2026-01-15 08:30:00.123456')` is `timestamp(6)`). The typed temporal literal is mandatory rather than defensive, and the failure is a **loud analyzer error rather than an empty result** - which is what makes it survivable if a rendering bug ever slipped through.

### (h) A decimal literal above 38 significant digits - PASS, rejected rather than widened

```
SELECT typeof(123456789012345678901234567890123456.78)  ->  decimal(38,2)
SELECT typeof(1234567890123456789012345678901234567.8)  ->  decimal(38,1)
SELECT 12345678901234567890123456789012345678.9
  ->  line 1:8: '12345678901234567890123456789012345678.9' is not a valid DECIMAL literal
SELECT 123456789012345678901234567890123456789.0
  ->  line 1:8: '123456789012345678901234567890123456789.0' is not a valid DECIMAL literal
SELECT typeof(0.123456789012345678901234567890123456789)
  ->  line 1:15: '0.123456789012345678901234567890123456789' is not a valid DECIMAL literal
```

39 significant digits is rejected outright, never silently widened to a floating-point value. Exasol's own maximum `DECIMAL` precision is 36, so no exact-numeric literal Exasol can send reaches the limit, and the bare form is safe.

### (i) A multi-column `GROUP BY` - PASS

Two columns and three columns both execute and return one row per group. `AGGREGATE_GROUP_BY_TUPLE` is safe to advertise.

---

## Gate 2.4 - Arrow types and coercion gaps

**Verdict: PASS, and there is NO coercion gap. Task 5.3 has nothing to close.**

Every row below is the Arrow type the driver actually returned for a real batch, and the result of replaying `coerce.rs`'s own conversion - `arrow::compute::cast_with_options` with `safe: false` - into the Arrow type `coerce.rs` resolves the declared Exasol type to.

| Trino type | driver Arrow type | Exasol declared | `coerce.rs` target | conversion |
|---|---|---|---|---|
| `integer` | `Int32` | `DECIMAL(10,0)` | `Int64` | CAST OK |
| `bigint` | `Int64` | `DECIMAL(19,0)` | `Decimal128(19,0)` | CAST OK |
| `decimal(12,4)` | **`Decimal64(12,4)`** | `DECIMAL(12,4)` | `Decimal128(12,4)` | **CAST OK** |
| `decimal(30,2)` | `Decimal128(30,2)` | `DECIMAL(30,2)` | `Decimal128(30,2)` | identity |
| `varchar(100)` | `Utf8` | `VARCHAR(100)` | `Utf8` | identity |
| `date` | `Date32` | `DATE` | `Date32` | identity |
| `double` | `Float64` | `DOUBLE` | `Float64` | identity |
| `timestamp(0)` | `Timestamp(Second, None)` | `TIMESTAMP` | `Timestamp(Microsecond, None)` | CAST OK |
| `timestamp(3)` | `Timestamp(Millisecond, None)` | `TIMESTAMP` | `Timestamp(Microsecond, None)` | CAST OK |
| `timestamp(6)` | `Timestamp(Microsecond, None)` | `TIMESTAMP` | `Timestamp(Microsecond, None)` | identity |
| `timestamp(9)` | **`Timestamp(Nanosecond, None)`** | `TIMESTAMP` | `Timestamp(Microsecond, None)` | **CAST OK** |

Both predicted gaps - `timestamp[ns]` and the decimal width - are already handled by arrow-rs 58's cast kernels. `scan-udf/trino-scan`'s statement that the driver returns "a `decimal` as an Arrow decimal" is true but underspecified: **the driver picks the narrowest decimal width**, which the other two drivers do not do.

Measured boundary, one probe per precision:

| Trino type | driver Arrow type | `coerce.rs` target | conversion |
|---|---|---|---|
| `decimal(1,0)` .. `decimal(9,0)` | `Decimal32(p,0)` | `Int32` | CAST OK |
| `decimal(10,0)` .. `decimal(18,0)` | `Decimal64(p,0)` | `Int64` | CAST OK |
| `decimal(19,0)` .. `decimal(38,0)` | `Decimal128(p,0)` | `Decimal128(p,0)` | identity |
| `decimal(10,2)`, `decimal(18,2)` | `Decimal64(p,2)` | `Decimal128(p,2)` | CAST OK |
| `decimal(19,2)` and above | `Decimal128(p,2)` | `Decimal128(p,2)` | identity |

So the driver reaches **three** Arrow decimal widths - `Decimal32`, `Decimal64`, `Decimal128` - and the first two are units neither shipped driver returns. Group E should keep this note even though nothing needs fixing: it is the reason the `timestamp[ns]`/`Decimal32`/`Decimal64` paths deserve a test of their own in `trino_it_test.rs` rather than being assumed.

**Two things worth naming, neither a gap:**

1. **`ns` to `µs` truncates silently, it does not fail.** `TIMESTAMP '2026-01-15 08:30:00.123456789'` casts to `2026-01-15T08:30:00.123456` even under `safe: false`. `scan-udf/trino-scan`'s "a value whose sub-millisecond digits Exasol cannot hold MUST be handled by the same rule `scan-udf/arrow-scan` states for every driver" therefore resolves to truncation rather than an error, and any test asserting the nanosecond path should assert the truncated value rather than a failure.
2. **`coerce.rs`'s failure diagnostic loses information for the new widths.** `unscaled_decimal_values` downcasts to `Decimal128Array` only, so a cast failure over a `Decimal32`/`Decimal64` column produces the message without the unscaled digits that make the offending row findable. Nothing is wrong today - the casts succeed - but this is the one place Trino's narrower widths are visible in existing code.

Other Arrow types measured, for Group G's metadata source and for the unmapped-type reasons:

- Catalog columns: `ordinal_position` arrives as **`Int64`**; `table_catalog`, `table_schema`, `table_name`, `table_type`, `column_name`, `data_type`, `is_nullable` all arrive as `Utf8`.
- A delegated predicate in the select list (`c_integer = 42`) arrives as Arrow `Boolean`, matching `DeclaredType::Boolean` with no cast.
- Aggregate outputs: `count(*)` and `count(col)` are `Int64`; `min(varchar)` is `Utf8`; `max(decimal(12,4))` is `Decimal64(12,4)`; `min(date)` is `Date32`; `max(timestamp(6))` is `Timestamp(µs)`; `min(double)` is `Float64`; `max(bigint)` is `Int64`.
- Unmapped types, for completeness: `char(5)` -> `Utf8` (padded, value `abc  `), `boolean` -> `Boolean`, `real` -> `Float32`, `smallint` -> `Int16`, `tinyint` -> `Int8`, `time(3)` -> `Time32(ms)`, `timestamp(3) with time zone` -> `Timestamp(ms, "UTC")`, `varbinary` -> `Binary`, `json` -> `Utf8`, `uuid` -> `FixedSizeBinary(16)`, `array(integer)` -> `List(Int64)`.

---

## Gate 2.5 - `COLLATE` is not a Trino keyword in any syntactic position

**Verdict: PASS.** `vs-adapter/trino-collation` ships as planned.

Four suffix positions, each rejected, and **no reported set of legal continuations contains `COLLATE`**:

```
ORDER BY "name" COLLATE "C"
  line 1:54: mismatched input 'COLLATE'. Expecting: '%', '*', '+', ',', '-', '.', '/', 'AND',
  'ASC', 'AT', 'DESC', 'FETCH', 'LIMIT', 'NULLS', 'OFFSET', 'OR', '[', '||', <EOF>, <predicate>

WHERE "name" COLLATE "C" = 'Widget'
  line 1:51: mismatched input 'COLLATE'. Expecting: '%', '*', '+', '-', '.', '/', 'AND', 'AT',
  'EXCEPT', 'FETCH', 'GROUP', 'HAVING', 'INTERSECT', 'LIMIT', 'OFFSET', 'OR', 'ORDER', 'UNION',
  'WINDOW', '[', '||', <EOF>, <predicate>

SELECT "name" COLLATE "C", count(*) ... GROUP BY "name" COLLATE "C"
  line 1:21: mismatched input '"C"'. Expecting: ',', 'EXCEPT', 'FETCH', 'FROM', 'GROUP',
  'HAVING', 'INTERSECT', 'LIMIT', 'OFFSET', 'ORDER', 'UNION', 'WHERE', 'WINDOW', <EOF>

count(DISTINCT "name" COLLATE "C")
  line 1:28: mismatched input 'COLLATE'. Expecting: '%', '(', ')', '*', '+', ',', '-', '->', '.',
  '/', '::', '=>', 'AND', 'AT', 'OR', 'ORDER', 'OVER', '[', '||', <predicate>, <string>
```

The third one is the most informative: the parser accepted `COLLATE` as an **alias** for the preceding column and only then choked on `"C"`. That is direct evidence that `COLLATE` is an ordinary unreserved identifier.

Confirmed positively:

```sql
SELECT "name" COLLATE FROM ... ORDER BY 1        -- column header: COLLATE, three rows
SELECT "name" AS COLLATE FROM ... ORDER BY 1     -- column header: COLLATE, three rows
CREATE TABLE memory.adbc_vs.gate25 (collate VARCHAR(10), other INTEGER)  -- CREATE TABLE
SELECT collate FROM memory.adbc_vs.gate25        -- returns x
```

and the catalog reports the column as `collate`/`varchar(10)`. Trino has no collation mechanism to render a suffix through, and every position a suffix could occupy is a parse error. `vs-adapter/trino-collation`'s claim that "this dialect renders no suffix" is a statement about Trino's grammar rather than a stylistic choice.

---

## What Groups E, F, G must take from this

- **Group E (5.1, 5.2):** do not write the percent-encoding arm until the two 2.1 findings are re-decided. The encoding itself is proven correct for every ASCII character; what is open is (1) whether the credential should travel in the `username`/`password` options instead, given that they exist, work, and override the userinfo, and (2) that the multi-byte clause in `scan-udf/trino-scan`'s first scenario cannot be satisfied against Trino 483. Redaction (5.2) must also cover the `new_database_with_opts` failure path, which quotes the URI verbatim - `invalid Trino URI format: parse "trino://gate_user:p@s%s/w:rd?": invalid URL escape "%s"`.
- **Group E (5.3):** nothing to close. Keep the task only if it becomes a test that pins `Decimal32`/`Decimal64`/`Timestamp(ns)` as handled, and assert truncation rather than failure on the nanosecond path.
- **Group F (6.2):** render `DOUBLE '<value>'` as planned, but the rationale in `vs-adapter/trino-pushdown` needs the correction in 2.3(b): the reason is totality over `Infinity`, `NaN`, and exponent spellings, not a wrong comparison result.
- **Group F (6.3):** the rejected set is exactly "the escape character followed by any character other than `%`, `_`, or the escape character itself", and Trino's own message is `Escape character must be followed by '%', '_' or the escape character itself`.
- **Group F (6.1, 6.4):** `FULL OUTER JOIN` is unrestricted and multi-column `GROUP BY` executes, so `JOIN_TYPE_FULL_OUTER` and `AGGREGATE_GROUP_BY_TUPLE` both ship as planned.
- **Group G (7.1):** parse against the exact spellings in gate 2.2. `decimal(12,4)` has no space after the comma while `map(varchar, integer)` does; `row("a" integer)` embeds double quotes; `varchar(0)` is a real declared length; `ordinal_position` arrives as `Int64` and `is_nullable` as the string `YES`.
- **Group G (7.2):** `information_schema.columns` has exactly eight columns and `information_schema.tables` exactly four. Selecting `character_maximum_length`, `numeric_precision`, `numeric_scale`, or `datetime_precision` fails with `Column '<name>' cannot be resolved`.
- **Group H:** cast the Exasol mirror's literals to `VARCHAR` explicitly. An Exasol string literal is `CHAR`, and `CHAR` comparison is blank-padded, so `'a' = 'a '` is true for bare literals and false for `VARCHAR`. The whole trailing-space differential assertion depends on this.

## Environment left behind

- `labs-adbc-vs-trino-1` was restored to Group A's own `docker/trino/password.db` mount and re-seeded from `docker/trino/init.sql`. The probe users exist in no repo file.
- `labs-adbc-vs-exasol-1` runs with the scratchpad port override (`127.0.0.1:18563/12580/12581`). `make stack-up-exasol` will recreate it on the compose ports and will fail while `exasol-test` holds `8563`/`2581`.
- Left in BucketFS: `adbc-drivers/trino/0.5.3/libadbc_driver_trino.so` (the real driver at the real production path, useful to later tiers) and `gate-b/libgateudf.so` (the throwaway measurement UDF). Left in Exasol: schema `GATE_B` with scripts `LOAD_TRINO` and `TRINO_LOAD`. None of these is referenced by any repo file; delete them with `DROP SCHEMA GATE_B CASCADE` and a BucketFS delete of `gate-b/` when convenient.
- Left in Trino's `memory` catalog: `memory.adbc_vs.gate_types`, `gate_a`, `gate_b`, `orders_view`. The `memory` connector drops all of them on the next container restart, and `stack-up-trino` re-seeds only `orders`.
