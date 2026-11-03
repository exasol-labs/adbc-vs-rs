# Architecture Delta: change-type-mapping-coverage
<!-- BASE: 31874c94ddb80247336906b185436e88592a2b15 -->

<!-- DELTA:CHANGED -->
## Constraints

- The UDF `.so` is built only inside the Rust build image that the `Makefile` names by `make cross-udf-build`, on a host of the `.so`'s own architecture, so its glibc fingerprint matches the `language-container-rs` SLC and `cargo exasol-udf validate` can load it
- The Rust toolchain version lives in `rust-toolchain.toml`, and `SLC_VERSION` in the `Makefile` tracks the `exasol-udf-sdk` version
- The minimum supported Exasol version is 2025.1
- The supported cluster architectures are Linux x86_64 and Linux aarch64, and each runs the `language-container-rs` SLC of its own architecture
- The supported source technologies are MySQL, PostgreSQL, and Trino
- Drivers are fetched only for the registry platform of the architecture the `.so` was compiled for, which is the cluster's architecture because the `.so` loads nowhere else
- Outbound HTTPS egress from the UDF enables registry self-install only, and `DRIVER_PATH` or the `supplied` default location works without egress
- No numeric throughput or latency target exists
- The adapter holds no state between calls, and all persisted state is in `adapterNotes`
- Driver self-install runs only in the adapter script during `createVirtualSchema`, `refresh`, and `setProperties`, never in the scan UDF or during `pushdown`
- The UDF reads BucketFS through a read-only mount, so self-install writes through the BucketFS HTTPS endpoint with credentials from a resolved CONNECTION
- `DRIVER_PATH` skips the registry read, the download, and the BucketFS write
- An explicit `BUCKETFS_CONNECTION` never falls back on failure, and only the reserved-connection route falls back to the `supplied` default location, when the connection does not resolve or the registry index request reaches no host
- Self-install verifies no checksum and no signature, and registry TLS certificate verification against the bundled webpki roots is the only integrity check
- The BucketFS client accepts the cluster's self-signed certificate, and the registry client is a separate client that always verifies certificates
- Download limits are 4 MiB for the index and 256 MiB for a package, and an unpacked package is limited to 512 MiB
- Registry and BucketFS requests use a 15 s connect timeout and a 600 s transfer timeout, and a written driver must become readable on the mount within 120 s
- No dialect advertises `ORDER BY`, `LIMIT`, or `LIMIT ... OFFSET`, because the emitting UDF carries no ordering guarantee, so Exasol applies them over the emitted rows
- Each advertised capability must be renderable by its dialect, because Exasol does not re-apply a delegated fragment
- No dialect renders a collation, so a delegated character comparison, grouping key, `DISTINCT` operand, and `MIN` or `MAX` operand follow the collation the source applies to the column, and a user who needs Exasol's code-point comparison either declares that collation at the source or wraps the operand in an expression no dialect advertises, so that Exasol evaluates it
- A source type maps only when the source answers every delegated comparison, grouping key, `DISTINCT` operand, `MIN` or `MAX` operand, and join key over it as Exasol answers it over the declared type and the value the scan emits, because no per-column mechanism keeps a fragment in Exasol. Two exceptions are stated: a character comparison follows the source's collation, and a comparison, grouping key, `DISTINCT` operand, or join key over a source timestamp acts at the source's fractional-second precision, while Exasol holds the emitted value at millisecond precision
- The PostgreSQL dialect renders a `VARCHAR` column reference as `text` and a `DOUBLE` column reference as `double precision`, so a delegated operation acts on the value the scan emits rather than on the source column's own type
- The `connection` component opens every PostgreSQL and MySQL connection with its session time zone pinned to UTC, before any statement runs, for the adapter's metadata reads and the scan alike, so a time-zone-aware source timestamp is emitted as its UTC wall-clock value and the source reads every `TIMESTAMP` or `DATE` operand compared against it in UTC
- Each driver's source precondition is checked on every request that resolves the source CONNECTION, before any table is discovered
- Data moves as Arrow record batches from the driver to Exasol, and the scan holds one batch at a time
- Adding a technology is a reviewed code change: a TOML declaration and dialect code compiled into a release, never a file loaded at run time
- No release artifact bundles an ADBC driver binary
- The dependency tree accepts only the permissive licenses listed in `deny.toml`
<!-- /DELTA:CHANGED -->
