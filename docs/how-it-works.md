[Home](index.md) · [Install & Set Up](install.md) · [Create a Virtual Schema](create-virtual-schema.md) · [How Does It Work](how-it-works.md)

---

# How Does It Work

`adbc-vs-rs` has two parts in one shared library. The adapter script plans queries and reads metadata. The scan UDF (a script that reads the source data) runs the query at the source. The code of both parts is in `libadbc_vs_adapter.so`.

## When you create or refresh a virtual schema

1. The adapter makes sure that the ADBC driver for the source is on BucketFS. See [Install & Set Up](install.md#4-make-the-driver-available).
2. The adapter opens the source through the driver with the `CONNECTION` object that you named.
3. The adapter reads the tables, views, and columns from the `information_schema` of the source.
4. The adapter maps each source type to an Exasol type. It leaves out a column or a table that has no match.
5. The adapter gives the table list to Exasol. It also gives Exasol a record of the state that later queries need, the `adapterNotes`. The record has no credential.

## When you run a query

1. Exasol sends the query to the adapter. The adapter decides which parts the source can run.
2. The adapter writes those parts as SQL for the source. It returns a statement that calls the scan UDF, `ADBC_VS.ADBC_SCAN`, with that SQL.
3. The scan UDF reads the source address and the credential from the `CONNECTION` object. It loads the driver and runs the SQL at the source.
4. The driver returns Arrow record batches. The scan UDF changes each batch to the Exasol types and sends it to Exasol. It holds one batch at a time.
5. Exasol runs the rest of the query on the returned rows.

The adapter keeps no state between calls. The scan UDF is the only part that reads table data.

## Limits

### Scope

- `adbc-vs-rs` is read-only. `INSERT`, `UPDATE`, and `DELETE` are not supported on a virtual schema.
- `adbc-vs-rs` does not cache. Every query runs against the source.
- `adbc-vs-rs` supports Linux x86_64 only. The release library is x86_64, and the automatic install requests only the `linux_amd64` driver. The registry also has `linux_arm64` drivers. On ARM, `adbc-vs-rs` does not run.
- Columns and tables with a type that has no Exasol match are left out.
- `adbc-vs-rs` does not bundle ADBC drivers. The drivers come from the registry or from you.

### What runs at the source

`adbc-vs-rs` sends these parts of a query to the source: column selection, filters, aggregation (also with `GROUP BY`), and equi joins and outer joins.

Exasol runs `ORDER BY`, `LIMIT`, and `OFFSET` on the returned rows. Non-equi joins also run in Exasol. A query that `adbc-vs-rs` cannot send to the source still works, but Exasol must fetch more rows.

### String comparison follows the source

`adbc-vs-rs` renders no collation, and Exasol accepts no `COLLATE` in a query. A delegated comparison uses the collation that the source applies to the column. This applies to character comparison, `LIKE`, `IN`, grouping keys, `DISTINCT`, `MIN`, `MAX`, and join keys. `ORDER BY` runs in Exasol and sorts by code point.

A source collation can ignore case or accents. The result can then differ from the result in Exasol. A group, or a `MIN` or `MAX` tie, shows one of the equal spellings. The source chooses which one.

To get the Exasol behavior, use one of two options:

1. Declare a binary collation at the source, on the column or in a view that the virtual schema shows. The query stays at the source. The binary collation for each database is in [Database details](#database-details).
2. Wrap the operand in an expression that `adbc-vs-rs` does not send, for example `"name" || ''`. Exasol then runs the comparison. This is slower, because Exasol fetches rows that the source could have removed.

### Network and install

- Exasol nodes must reach the source database on the network. A Docker service name does not resolve inside a UDF, so use an IP address or a DNS name that the nodes can resolve.
- The automatic driver install needs outbound HTTPS to `dbc-cdn.columnar.tech`. The size limits are 4 MiB for the registry index, 256 MiB for a download, and 512 MiB for an unpacked driver.
- The automatic install does not check a checksum or a signature.
- Each database has a supported driver version range. `adbc-vs-rs` installs the newest version in that range. A driver that you install yourself is not checked.
- The shared library works only with the SLC that has the matching version. See [Install & Set Up](install.md#1-install-the-script-language-container).

### Database details

#### MySQL

- The default collation `utf8mb4_0900_ai_ci` ignores case and accents. Use `utf8mb4_0900_bin` for the Exasol behavior. It needs MySQL 8.0.17 or later. MariaDB has no equal collation.
- Driver version range: `>=0.6.0, <0.7.0`.

#### PostgreSQL

- One virtual schema shows one database and its `public` schema. Other schemas are not visible.
- Use the collation `"C"` for the Exasol behavior.
- Before version 18, a delegated `LIKE` on a column with a nondeterministic collation fails at the source.
- Driver version range: `>=1.12.0, <2.0.0`.

#### Trino

- The address must have a catalog and a schema.
- A backslash in a `LIKE` pattern is an ordinary character, unless the query has an `ESCAPE` clause. Exasol then runs that predicate.
- Driver version range: `>=0.5.3, <0.6.0`.
