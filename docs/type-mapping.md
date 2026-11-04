[Home](index.md) · [Install & Set Up](install.md) · [Create a Virtual Schema](create-virtual-schema.md) · [How Does It Work](how-it-works.md) · [Type Mapping](type-mapping.md)

---

# Type Mapping

When `adbc-vs-rs` creates or refreshes a virtual schema, it maps the type of each source column to an Exasol type. This page lists the mapping for each database, the cases where a delegated comparison can differ from the same comparison in Exasol, and the limits of Exasol's own types.

## How the mapping works

- The mapping reads only the catalog row of the column. It does not read the Arrow type that the driver returns at query time.
- A column with no mapping is left out of the virtual table. The adapter records the column and the reason in the `adapterNotes` of the schema. A table with no mapped column is left out too.
- Each mapped column records its source type. When a source type needs a conversion to give the value that the scan emits, the adapter reads the column through a *read expression*. It uses the read expression in every position of the SQL that it sends to the source: the select list, a filter, a `GROUP BY` key, an aggregate operand, and a join condition. A filter on a converted column then compares the same value that Exasol receives. A column with no read expression is used as it is, bare.
- The adapter sets no session setting at the source. The source session defaults apply.
- Exasol's type limits are in [Exasol type limits](#exasol-type-limits).

## PostgreSQL

Every PostgreSQL type maps, except an array. A type with an Exasol counterpart maps to it. Any other type maps to `VARCHAR(2000000)` and is read as the text that PostgreSQL prints for the value. In the tables, `<c>` is the quoted column.

| Source type | Exasol type | Read expression | Notes |
|-------------|-------------|-----------------|-------|
| `smallint` | `DECIMAL(5,0)` | none | |
| `integer` | `DECIMAL(10,0)` | none | |
| `bigint` | `DECIMAL(19,0)` | none | |
| `numeric(p,s)`, p at most 36 | `DECIMAL(p,s)` | none | |
| `numeric(p,s)`, p above 36, and `numeric` without a precision | `VARCHAR(2000000)` | `CAST(<c> AS TEXT)` | The text keeps every digit and trailing zero, so `1.0` and `1.00` are two values. `NaN`, `Infinity`, and values beyond `DECIMAL` arrive as text |
| `real`, `double precision` | `DOUBLE` | none | A `real` value arrives widened to double precision, so a stored `0.1` arrives as `0.100000001490116119384765625` |
| `money` | `DOUBLE` | `CAST(CAST(<c> AS NUMERIC) AS DOUBLE PRECISION)` | The currency symbol and separators of `lc_monetary` do not affect the value. PostgreSQL scales the stored amount by the fractional digits of the session `lc_monetary`, 2 under the `C` and `en_US` locales |
| `boolean` | `BOOLEAN` | none | |
| `bit(n)` of any width | `BOOLEAN` | `(position(B'1' IN <c>) > 0)` | True when any bit is set. See [bit columns](#bit-columns) |
| `bit varying(n)` | `VARCHAR(n)` | `CAST(<c> AS TEXT)` | `VARCHAR(2000000)` without `n` |
| `character(n)`, n at most 2000 | `CHAR(n)` | none | See [CHAR columns](#char-columns) |
| `character(n)`, n above 2000 | `VARCHAR(n)` | `CAST(<c> AS TEXT)` | Exasol `CHAR` holds 2000 characters at most. The value arrives without its trailing blanks. `VARCHAR(2000000)` for n above 2000000 |
| `character varying(n)` | `VARCHAR(n)` | none | A length of 0 maps to 1. `VARCHAR(2000000)` without `n` or with `n` above 2000000 |
| `text` | `VARCHAR(2000000)` | none | |
| `time`, `time with time zone` | `VARCHAR(100)` | `CAST(<c> AS TEXT)` | |
| `date` | `DATE` | none | |
| `timestamp(p)` | `TIMESTAMP(p)` | none | The column keeps its own precision |
| `timestamp(p) with time zone` | `TIMESTAMP(p)` | none | The scan emits the UTC wall-clock time. See [time zones](#time-zones) |
| `uuid`, `json`, `jsonb`, `xml`, `bytea`, `interval`, `inet`, `cidr`, `macaddr`, geometric types, `tsvector`, enum types, and every other type | `VARCHAR(2000000)` | `CAST(<c> AS TEXT)` | `bytea` arrives in the form that `bytea_output` selects, hex by default |
| arrays, such as `text[]` | omitted | | The catalog names no element type, and the Exasol JDBC Virtual Schema leaves arrays out too |

A virtual schema shows one database and its `public` schema.

## MySQL

Every MySQL type maps, except binary, blob, and spatial types. Every character column declares character set `UTF8`, whatever character set MySQL stores it in.

| Source type | Exasol type | Read expression | Notes |
|-------------|-------------|-----------------|-------|
| `TINYINT`, `SMALLINT`, `MEDIUMINT`, `INT`, `BIGINT` | `DECIMAL(3,0)`, `DECIMAL(5,0)`, `DECIMAL(7,0)`, `DECIMAL(10,0)`, `DECIMAL(19,0)` | none | `TINYINT(1)` is a number too, not a boolean: a stored `5` arrives as `5` |
| the same types, `UNSIGNED` | `DECIMAL(3,0)`, `DECIMAL(5,0)`, `DECIMAL(8,0)`, `DECIMAL(10,0)`, `DECIMAL(20,0)` | none | The digits of the unsigned range. `18446744073709551615` arrives with all its digits |
| `DECIMAL(p,s)`, p at most 36 | `DECIMAL(p,s)` | none | |
| `DECIMAL(p,s)`, p above 36 | `VARCHAR(2000000)` | `CAST(<c> AS CHAR)` | |
| `FLOAT`, `DOUBLE` | `DOUBLE` | none | A `FLOAT` value arrives widened to double precision, so a stored `0.1` arrives as `0.100000001490116119384765625` |
| `BIT(n)` of any width | `BOOLEAN` | `(<c> <> 0)` | True when any bit is set. See [bit columns](#bit-columns) |
| `CHAR(n)` | `CHAR(n)` | none | A length of 0 maps to 1. See [CHAR columns](#char-columns) |
| `VARCHAR(n)` | `VARCHAR(n)` | none | A length of 0 maps to 1 |
| `TINYTEXT` | `VARCHAR(255)` | none | |
| `TEXT` | `VARCHAR(65535)` | none | |
| `MEDIUMTEXT`, `LONGTEXT` | `VARCHAR(2000000)` | none | |
| `ENUM`, `SET` | `CHAR(n)`, n the catalog length | none | `VARCHAR(n)` when n is above 2000. See [CHAR columns](#char-columns) |
| `JSON` | `VARCHAR(2000000)` | `CAST(<c> AS CHAR)` | The comparison is on MySQL's text, not on the parsed value |
| `DATE` | `DATE` | none | |
| `YEAR` | `DATE` | `MAKEDATE(<c>, 1)` | `2024` arrives as `2024-01-01`. A `0000` year arrives as `2000-01-01` |
| `DATETIME(p)` | `TIMESTAMP(p)` | none | The column keeps its own precision |
| `TIMESTAMP(p)` | `TIMESTAMP(p)` | none | Read in the session `time_zone`. See [time zones](#time-zones) |
| `TIME(p)` | `TIMESTAMP(p)` | `TIMESTAMP(DATE '1970-01-01', <c>)` | A time on 1970-01-01. `25:00:00` arrives as `1970-01-02 01:00:00` |
| `BINARY`, `VARBINARY`, `TINYBLOB`, `BLOB`, `MEDIUMBLOB`, `LONGBLOB`, spatial types such as `GEOMETRY` | omitted | | The Exasol JDBC Virtual Schema leaves these out too |

In a query with `GROUP BY`, MySQL rejects an expression over a grouped column unless it matches a `GROUP BY` expression as a whole. For a column with a read expression, `adbc-vs-rs` groups by the read expression and wraps each other use of the grouped column in the select list as `ANY_VALUE(<read expression>)`. The statement then runs at the source.

## Trino

Trino maps seven types. Every other type is left out. Trino's catalog reports the length, precision, and scale only inside the type string, so the mapping reads them from that string. A type string that the mapping does not recognize leaves the column out, and the reason names the string as the catalog spelled it. No Trino column needs a read expression.

| Source type | Exasol type | Read expression | Notes |
|-------------|-------------|-----------------|-------|
| `integer` | `DECIMAL(10,0)` | none | |
| `bigint` | `DECIMAL(19,0)` | none | |
| `decimal(p,s)`, p at most 36 | `DECIMAL(p,s)` | none | Trino admits 38 digits, Exasol 36. A wider column is left out |
| `varchar(n)` | `VARCHAR(n)` | none | A length of 0 maps to 1. A column with n above 2000000 is left out |
| `date` | `DATE` | none | |
| `timestamp(p)` of any precision | `TIMESTAMP(3)` | none | Milliseconds. See [Trino timestamps](#trino-timestamps) |
| `double` | `DOUBLE` | none | |
| `boolean` | omitted | | Outside the Trino scope of this release |
| `char(n)` | omitted | | Trino blank-pads the value, and a `LIKE` match sees the padding while an equality does not |
| `varchar` without a length | omitted | | |
| `timestamp(p) with time zone` | omitted | | It carries a different instant than `TIMESTAMP` for one wall-clock value |
| `real`, `smallint`, `tinyint`, `time`, `varbinary`, `json`, `uuid`, `ipaddress`, `array`, `map`, `row`, intervals | omitted | | Outside the Trino scope of this release |

A virtual schema shows one catalog and one schema.

## Known differences

A delegated fragment runs at the source and Exasol does not run it again. Some delegated comparisons therefore select other rows than the same comparison over the returned values in Exasol. This section lists each difference.

### Collation

A delegated comparison follows the collation that the source applies to the column. This includes a column mapped from a text type. See [String comparison follows the source](how-it-works.md#string-comparison-follows-the-source) for the scope and the two ways to get the Exasol behavior.

- A MySQL column that the adapter reads as text, such as a `JSON` column, compares under the connection collation of the session, not under a column collation.
- A MySQL `latin1` column keeps its `latin1` collation, `latin1_swedish_ci` by default. It ignores case and accents, so an equality with `'café'` also selects a row that holds `cafe`.
- A delegated comparison of a MySQL `latin1` column with a literal that `latin1` cannot hold, such as `'ł'`, fails with MySQL error 1267 (illegal mix of collations). It does not select or drop rows. The query fails, and you can run it in Exasol after you wrap the operand in an expression that is not delegated, for example `"name" || ''`.

### CHAR columns

Exasol keeps a `CHAR(n)` value padded with spaces to n characters. It ignores the padding when it compares two `CHAR` values or a `CHAR` value and a string literal. It does not pad a `VARCHAR` value. In Exasol, a `CHAR(5)` holding `ab` is unequal to a `VARCHAR` holding `ab`, and `LIKE 'ab'` does not match it.

- PostgreSQL ignores the trailing blanks of a `character` value when it compares it with a `character varying` value. A delegated equality between the two selects the row, and Exasol's own comparison does not. A `LIKE` without a trailing wildcard matches the same rows as in Exasol.
- MySQL returns a `CHAR`, `ENUM`, or `SET` value without trailing spaces. The scan emits it padded, and Exasol compares it as a `CHAR`. MySQL matches the unpadded value, so a delegated `LIKE 'ab'` selects the row where Exasol's own `LIKE 'ab'` over `CHAR(10)` holding `ab` selects none. A delegated equality with a `VARCHAR` or `TINYTEXT` operand selects rows that Exasol's own comparison does not.

### bit columns

A `bit(n)` or `BIT(n)` column of more than one bit maps to `BOOLEAN` and reads as true when any bit is set. Two different source values then read as one `true`. A `GROUP BY` over such a column puts `00000101` and `00001001` in one group. A `bit(1)` or `BIT(1)` column keeps its value.

### Time zones

`adbc-vs-rs` sets no session time zone at the source. A comparison that depends on a time zone follows the session setting of the source.

- A PostgreSQL `timestamp with time zone` column maps to `TIMESTAMP(p)`, and the scan emits the UTC wall-clock time. A delegated comparison of the column with a `TIMESTAMP` or `DATE` value, and a delegated equi-join with a `timestamp without time zone` column, follow the session `TimeZone`. Under a `TimeZone` other than `UTC`, such a comparison selects other rows than Exasol does. To fix it, set the zone for the role or the database that the connection uses: `ALTER ROLE <user> SET TimeZone = 'UTC'` or `ALTER DATABASE <db> SET TimeZone = 'UTC'`.
- A MySQL `TIMESTAMP` column is read in the session `time_zone`. In a daylight-saving fall-back hour, two instants have one wall-clock value, and the emitted values repeat it. Set the session time zone of the MySQL server to `+00:00` to avoid this.
- No source column maps to `TIMESTAMP WITH LOCAL TIME ZONE`. Exasol reads and compares such a value in the Exasol session time zone, and no source session follows that zone.

### Trino timestamps

A Trino `timestamp(p)` column is declared as `TIMESTAMP(3)`, but Trino evaluates a delegated comparison over the column at the column's own precision. A comparison at a precision above milliseconds can then select other rows than Exasol does over the emitted millisecond values.

### MySQL `TIME` and `YEAR`

- A `TIME` value beyond 24 hours reads as a time on a later day: `25:00:00` is `1970-01-02 01:00:00`.
- A `YEAR` value of `0000` is emitted as `2000-01-01`, because `MAKEDATE(0, 1)` reads year 0 as 2000.

### Values that Exasol reads differently

- Exasol reads an empty `VARCHAR` value as null. A source column that holds `''` returns null, and `= ''` selects no row.
- Exasol reads a `NaN` in a `double precision`, `real`, `DOUBLE`, or `FLOAT` column as null, as it does for a JDBC import. It holds no infinity. A PostgreSQL `numeric` `NaN` is not affected, because that column arrives as text.

### The cost of a read expression

A delegated filter, join key, grouping key, or `MIN` or `MAX` operand on a column with a read expression cannot use a source index on the bare column. The main case is a PostgreSQL `uuid` key. An index on the expression restores it:

```sql
CREATE INDEX ON <table> ((CAST(<column> AS text)));
```

Confirmed on PostgreSQL 18: `EXPLAIN` of a delegated `uuid` equality reports an index scan on this index. The bare comparison `"uid" = '...'` cannot use it.

### Text longer than 2000000 characters

A query that reads a text longer than 2000000 characters from a mapped column fails. It does not return a truncated value.

## After an upgrade

- An existing virtual schema keeps its tables, columns, and generated statements until you run `ALTER VIRTUAL SCHEMA ... REFRESH`. Until then, its MySQL `CHAR` columns stay `VARCHAR(n)`.
- A refresh changes the column types of the virtual tables: a PostgreSQL `timestamp` becomes `TIMESTAMP(6)`, a MySQL `CHAR(n)` becomes `CHAR(n)`, and a column that was left out can now appear. A view or `CREATE TABLE AS` over a virtual table takes the new types.
- If you downgrade `adbc-vs-rs` after a refresh under this release, queries over `TIMESTAMP(p)` columns with p other than 3, and over converted columns, fail. Refresh the schema under the older release to fix this.

## Exasol type limits

These limits of Exasol decide the bounds of the mapping.

| Type | Limits | Notes |
|------|--------|-------|
| `DECIMAL(p,s)` | 1 to 36 digits, scale 0 to p | |
| `DOUBLE` | about 15 significant digits | Reads `NaN` as null and holds no infinity |
| `CHAR(n)` | 1 to 2000 characters | Padded with spaces to n characters |
| `VARCHAR(n)` | 1 to 2000000 characters | Not padded. An empty string is null |
| `TIMESTAMP(p)` | 0 to 9 fractional-second digits, default 3 | No time zone |

The full reference, with aliases, defaults, and Exasol's comparison rules, is in `specs/exasol-types.md` of the repository.
