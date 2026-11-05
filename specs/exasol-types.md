# Exasol Data Types

Reference for the source-to-Exasol type mappings that the `vs-adapter/*-type-mapping` features state. This file records Exasol's own type system, not adbc-vs behavior, and it holds no scenarios. The facts come from the Exasol documentation, fetched 2026-11-03, and from SELECT-only literal queries against Exasol 2025.1.11, marked "measured".

## Types

Source: https://docs.exasol.com/db/latest/sql_references/data_types/datatypedetails.htm

| Type | Syntax | Limits | Default when omitted | Notes |
|------|--------|--------|----------------------|-------|
| `BOOLEAN` | `BOOLEAN` | `TRUE`, `FALSE`, `NULL` | none | Accepts `1` and `0`, and the strings `'1'`, `'T'`, `'TRUE'`, `'0'`, `'F'`, `'FALSE'` in any case |
| `DECIMAL` | `DECIMAL(p,s)` | 1 ≤ p ≤ 36, 0 ≤ s ≤ p; range ±(10^p − 1) / 10^s | p = 18, s = 0 | Exact |
| `DOUBLE PRECISION` | `DOUBLE PRECISION` | ±1.7 · 10^308, about 15 significant digits | none | Reads `NaN` as `NULL`; holds no infinity |
| `CHAR` | `CHAR(n) [CHARACTER SET] ASCII` or `UTF8` | 1 ≤ n ≤ 2000 characters | n = 1, `UTF8` | Stored padded with spaces to n characters |
| `VARCHAR` | `VARCHAR(n) [CHARACTER SET] ASCII` or `UTF8` | 1 ≤ n ≤ 2000000 characters | `UTF8`; the documentation shows no form without n | Not padded; an empty string is `NULL` |
| `DATE` | `DATE` | 0001-01-01 to 9999-12-31 | none | |
| `TIMESTAMP` | `TIMESTAMP(p)` | 0 ≤ p ≤ 9 fractional-second digits; 0001-01-01 00:00:00 to 9999-12-31 23:59:59.999999999 | p = 3 | No time zone |
| `TIMESTAMP WITH LOCAL TIME ZONE` | `TIMESTAMP(p) WITH LOCAL TIME ZONE` | as `TIMESTAMP` | p = 3 | Stored as UTC; read, written, and compared in the session time zone |
| `INTERVAL YEAR TO MONTH` | `INTERVAL YEAR(p) TO MONTH` | 1 ≤ p ≤ 9; ±'999999999-11' | p = 2 | |
| `INTERVAL DAY TO SECOND` | `INTERVAL DAY(lfp) TO SECOND(fsp)` | 1 ≤ lfp ≤ 9, 0 ≤ fsp ≤ 9; ±'999999999 23:59:59.999' | lfp = 2, fsp = 3 | Stores milliseconds at most |
| `GEOMETRY` | `GEOMETRY(srid)` | `POINT`, `LINESTRING`, `POLYGON`, their `MULTI` forms, `GEOMETRYCOLLECTION` | srid = 0 | `EXA_SPATIAL_REF_SYS` lists the supported SRIDs |
| `HASHTYPE` | `HASHTYPE(n BYTE)` or `HASHTYPE(m BIT)` | 1 ≤ n ≤ 1024; 8 ≤ m ≤ 8192, m a multiple of 8 | 16 `BYTE` | Accepts hex, UUID (16 `BYTE` only), base64, and base64url input |

## Aliases

Source: https://docs.exasol.com/db/latest/sql_references/data_types/datatypealiases.htm

| Alias | Resolves to |
|-------|-------------|
| `BOOL` | `BOOLEAN` |
| `TINYINT` | `DECIMAL(3,0)` |
| `SMALLINT`, `SHORTINT` | `DECIMAL(9,0)` |
| `INT`, `INTEGER`, `DEC`, `DECIMAL`, `NUMERIC` | `DECIMAL(18,0)` |
| `BIGINT` | `DECIMAL(36,0)` |
| `DEC(p)`, `DECIMAL(p)`, `NUMERIC(p)`, `NUMBER(p)` | `DECIMAL(p,0)` |
| `DEC(p,s)`, `NUMERIC(p,s)`, `NUMBER(p,s)` | `DECIMAL(p,s)` |
| `NUMBER`, `DOUBLE`, `FLOAT`, `REAL` | `DOUBLE PRECISION` |
| `CHAR`, `CHARACTER`, `NCHAR` | `CHAR(1)` |
| `NCHAR(n)` | `CHAR(n)` |
| `CHAR VARYING(n)`, `CHARACTER VARYING(n)`, `NVARCHAR(n)`, `NVARCHAR2(n)`, `VARCHAR2(n)`, `CLOB(n)`, `CHARACTER LARGE OBJECT(n)` | `VARCHAR(n)` |
| `CLOB`, `CHARACTER LARGE OBJECT`, `LONG VARCHAR` | `VARCHAR(2000000)` |
| `TIMESTAMP`, `TIMESTAMP WITHOUT TIME ZONE` | `TIMESTAMP(3)` |
| `TIMESTAMP(p) WITHOUT TIME ZONE` | `TIMESTAMP(p)` |
| `TIMESTAMP WITH LOCAL TIME ZONE` | `TIMESTAMP(3) WITH LOCAL TIME ZONE` |
| `INTERVAL DAY TO SECOND` | `INTERVAL DAY(2) TO SECOND(3)` |
| `INTERVAL YEAR TO MONTH` | `INTERVAL YEAR(2) TO MONTH` |
| `GEOMETRY` | `GEOMETRY(0)` |
| `HASHTYPE` | `HASHTYPE(16 BYTE)` |
| `HASHTYPE(m BIT)` | `HASHTYPE(m/8 BYTE)` |

- A bare `NUMBER` is `DOUBLE PRECISION`, while `NUMBER(p)` is `DECIMAL(p,0)`.
- Exasol's integer aliases do not match the digit counts of the source integer widths.

## Fractional-second precision

Sources: https://docs.exasol.com/db/latest/changelogs/13409.htm, https://docs.exasol.com/db/latest/release_notes_db/8.32.0.htm, https://docs.exasol.com/db/latest/planning/life_cycle/release_types.htm

- `TIMESTAMP(p)` and `TIMESTAMP(p) WITH LOCAL TIME ZONE` take 0 ≤ p ≤ 9 and default to p = 3. Exasol 8.32.0 introduced them. Before 8.32.0, every timestamp held 3 fractional digits, and `TIMESTAMP(6)` meant `TIMESTAMP(3)`.
- Exasol 2025.1 is a later release of the Exasol 8 line than 8.34.0. The documentation does not state that 2025.1 contains the 8.32.0 change. Measured on Exasol 2025.1.11: `CAST('2024-01-01 12:00:00.123456' AS TIMESTAMP(6))` holds `.123456`, `CAST(... AS TIMESTAMP)` holds `.123`, and a `TIMESTAMP(6)` holding `.123456` is unequal to a `TIMESTAMP(3)` holding `.123`.
- When operands of different precision meet, the result takes the greater precision. A timestamp literal's precision is its number of fractional digits.
- `NLS_TIMESTAMP_FORMAT` defaults to `FF6`, so an implicit string-to-timestamp conversion keeps at most 6 fractional digits.
- A UDF receives a `TIMESTAMP(7)` to `TIMESTAMP(9)` input truncated to 6 digits. A Script Language Container may emit 9 digits.
- The Virtual Schema API carries the precision of a `TIMESTAMP` in the `dataType` field `fractionalSecondsPrecision`. A `dataType` without the field means p = 3.

## Comparison and padding

Sources: https://docs.exasol.com/db/latest/changelogs/31555.htm, https://docs.exasol.com/db/latest/sql_references/functions/alphabeticallistfunctions/length.htm, https://docs.exasol.com/db/latest/database_concepts/character_encoding.htm

- Exasol has no SQL collation. Character comparison and `ORDER BY` compare code points, so they are case-sensitive and accent-sensitive.
- `CHAR` keeps its padding: `LENGTH` of a `CHAR(10)` holding `ABC` is 10, and of a `VARCHAR(10)` holding `ABC` is 3.
- Exasol ignores the padding when it compares two `CHAR` values, and when it compares a `CHAR` value with a string literal. It does not pad a `VARCHAR` value compared with a `CHAR` value. Measured on Exasol 2025.1.11, a `CHAR(5)` holding `ab` equals `'ab'`, equals a `CHAR(3)` holding `ab`, equals a `VARCHAR` holding `ab   `, is unequal to a `VARCHAR` holding `ab`, does not match `LIKE 'ab'`, and matches `LIKE 'ab%'`.
- Changelog 31555 records a join of a `CHAR` key with a `VARCHAR` key that returned wrong rows in 2025.1.0. Exasol fixed it in 2025.1.14 and 2026.1.2.
- A `VARCHAR` holding `ab ` is unequal to `'ab'` (measured).
- An empty `VARCHAR` value is `NULL`, so `= ''` selects no row.
- `DOUBLE` reads `NaN` as `NULL` and holds no infinity.
- A `TIMESTAMP WITH LOCAL TIME ZONE` converted to `TIMESTAMP` takes the session time zone, so one statement can return different values in two sessions. `TIME_ZONE_BEHAVIOR` decides how a time in a daylight-saving gap or overlap is read.

## Character sets

Source: https://docs.exasol.com/db/latest/database_concepts/character_encoding.htm

- The size n of `CHAR(n)` and `VARCHAR(n)` counts characters in both character sets.
- `UTF8` takes 1 to 4 bytes per character. `ASCII` takes 1 byte per character and holds 7-bit values only.

## Virtual Schema `dataType` fields

| Type | Fields |
|------|--------|
| `DECIMAL` | `precision`, `scale` |
| `DOUBLE` | none |
| `VARCHAR`, `CHAR` | `size`, `characterSet` |
| `DATE` | none |
| `TIMESTAMP` | `withLocalTimeZone`, `fractionalSecondsPrecision` |
| `BOOLEAN` | none |
