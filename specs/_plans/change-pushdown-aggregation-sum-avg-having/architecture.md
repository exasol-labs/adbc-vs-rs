# Architecture Delta: change-pushdown-aggregation-sum-avg-having
<!-- BASE: da46f6afe2a69c10daa15de927f6a415f77039fd -->

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
- The adapter holds no state between calls, and all persisted state is in the schema and column `adapterNotes`
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
- A delegated `SUM` or `AVG` over a floating-point operand follows the source's accumulation order, and a source `NaN` makes it null, so a user who needs Exasol's value wraps the operand in an expression no dialect advertises, so that Exasol evaluates it
- The PostgreSQL and MySQL type mappings start from the Exasol JDBC Virtual Schema's mapping for the same source, and each technology's type-mapping feature states every departure. A source type with no Exasol counterpart maps to a `VARCHAR` holding the source's text for each value, except the types that feature leaves unmapped
- A mapped column's recorded source type selects the read expression its dialect renders for every reference to the column, so a delegated operation acts on the value the scan emits, and a column without a recorded source type, which a virtual schema created before this release declares, renders bare
- A delegated comparison can still differ from Exasol's comparison of the emitted value, and each technology's type-mapping feature and `docs/type-mapping.md` state every such difference: a character comparison follows the source's collation, a `CHAR` value compared with a `VARCHAR` operand follows the source's blank-padding rule, and a time-zone-aware source timestamp compared with a timestamp or a date follows the source session's time zone
- The adapter and the scan set no source session setting, so the source's own default session time zone governs every time-zone-aware timestamp
- A PostgreSQL or MySQL timestamp declares the source's fractional-second precision as `TIMESTAMP(p)`, a Trino timestamp declares `TIMESTAMP(3)`, and the scan emits timestamps at microsecond resolution
- Each driver's source precondition is checked on every request that resolves the source CONNECTION, before any table is discovered
- Data moves as Arrow record batches from the driver to Exasol, and the scan holds one batch at a time
- Adding a technology is a reviewed code change: a TOML declaration and dialect code compiled into a release, never a file loaded at run time
- No release artifact bundles an ADBC driver binary
- The dependency tree accepts only the permissive licenses listed in `deny.toml`
<!-- /DELTA:CHANGED -->
