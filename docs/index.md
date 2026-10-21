# `adbc-vs-rs` Documentation

`adbc-vs-rs` is an Exasol Virtual Schema for MySQL, PostgreSQL, and Trino. It reads the source data through ADBC and returns it to Exasol as Apache Arrow record batches.

## Documentation

- [Install & Set Up](install.md) - Requirements, the Script Language Container, the adapter scripts, and the driver
- [Create a Virtual Schema](create-virtual-schema.md) - Connections, properties, and one example for each source
- [How Does It Work](how-it-works.md) - The adapter and the scan UDF, and the [limits](how-it-works.md#limits) of `adbc-vs-rs`

## Additional Resources

- [Exasol Virtual Schemas](https://docs.exasol.com/db/latest/database_concepts/virtual_schemas.htm)
- [Exasol BucketFS](https://docs.exasol.com/db/latest/administration/on-premise/bucketfs/bucketfs.htm)
- [Exasol CREATE CONNECTION](https://docs.exasol.com/db/latest/sql/create_connection.htm)
- [Exasol UDF scripts](https://docs.exasol.com/db/latest/database_concepts/udf_scripts.htm)
- [Apache Arrow ADBC](https://arrow.apache.org/adbc/)
- [ADBC Driver Foundry](https://github.com/adbc-drivers)
