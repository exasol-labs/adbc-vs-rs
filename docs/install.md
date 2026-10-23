[Home](index.md) · [Install & Set Up](install.md) · [Create a Virtual Schema](create-virtual-schema.md) · [How Does It Work](how-it-works.md)

---

# Install & Set Up

`adbc-vs-rs` has two parts. The adapter script plans each query. The scan UDF (a script that reads the source data) runs the query at the source. Both parts are in one shared library, `libadbc_vs_adapter.so`.

You install the library once for each Exasol database. After that, you create one virtual schema for each source.

## Requirements

- Exasol 2025.1 or later.
- A Linux x86_64 or aarch64 Exasol cluster. Use the release asset and the SLC asset of the cluster's architecture. The two tables below list them.
- A MySQL, PostgreSQL, or Trino source that the Exasol nodes can reach on the network.
- [`exapump`](https://github.com/exasol/exapump) to upload files to [BucketFS](https://docs.exasol.com/db/latest/administration/on-premise/bucketfs/bucketfs.htm). You can use any other BucketFS client.
- For the automatic driver install only: outbound HTTPS access from the Exasol nodes to `dbc-cdn.columnar.tech`. Without it, you supply the driver yourself. See [Install the driver yourself](#install-the-driver-yourself).

To try `adbc-vs-rs` on your own machine, start a local Exasol with the [Docker image](https://hub.docker.com/r/exasol/docker-db):

```bash
docker run -d --name exasol-test -p 8563:8563 --privileged --shm-size=2g exasol/docker-db:latest
```

The default credentials are `sys` / `exasol`. The Docker image is x86_64 only. On aarch64, use an Exasol Personal local deployment.

## 1. Install the Script Language Container

The Script Language Container (SLC) is the runtime that loads the shared library. The SLC version must match the `exasol-udf-sdk` version that `adbc-vs-rs` uses. The current version is `0.23.1`. A library that does not match the SLC fails to load.

| Architecture | SLC asset |
|--------------|-----------|
| x86_64 | `lc-rust-0.23.1.tar.gz` |
| aarch64 | `lc-rust-0.23.1-aarch64.tar.gz` |

Download the asset of your cluster's architecture from the [language-container-rs releases](https://github.com/exasol-labs/language-container-rs/releases). Then follow the steps for your Exasol.

### Exasol with a BucketFS endpoint

1. Upload the file to BucketFS as `slc/rustslc.tar.gz`.
2. Wait until BucketFS finishes the extraction. The file `slc/rustslc/exaudf/exaudfclient` must exist.
3. Register the language alias `RUST`:

```sql
ALTER SYSTEM SET SCRIPT_LANGUAGES = '<existing value> RUST=localzmq+protobuf:///bfsdefault/default/slc/rustslc?lang=rust#buckets/bfsdefault/default/slc/rustslc/exaudf/exaudfclient';
```

Read the existing value first with `SELECT SYSTEM_VALUE FROM EXA_PARAMETERS WHERE PARAMETER_NAME = 'SCRIPT_LANGUAGES'`. Keep it in the new value. If you replace it, the built-in languages stop working.

### Exasol Personal local deployment

An Exasol Personal local deployment has no BucketFS HTTP endpoint. The `exasol` launcher installs the SLC instead of the upload and the `ALTER SYSTEM` statement:

```bash
exasol slc custom install --source lc-rust-0.23.1.tar.gz --alias RUST --language rust
```

On aarch64, use `lc-rust-0.23.1-aarch64.tar.gz` as the source. The database restarts to load the SLC.

Do not use `exasol slc install rust`. It installs the latest language-container-rs release, not the pinned `0.23.1`, and a library that does not match the SLC fails to load.

## 2. Upload the shared library

The release has one tarball for each architecture. The library loads only on the architecture it was built for.

| Architecture | Release asset |
|--------------|---------------|
| x86_64 | `adbc-vs.tar.gz` |
| aarch64 | `adbc-vs-linux-arm64.tar.gz` |

1. Download the asset of your cluster's architecture from the [GitHub releases](https://github.com/exasol-labs/adbc-vs-rs/releases) and unpack it. It holds `udf/libadbc_vs_adapter.so` and `THIRD-PARTY-LICENSES.md`. Only the library goes to BucketFS.
2. Upload the library to BucketFS:

```bash
exapump bucketfs cp udf/libadbc_vs_adapter.so udf/libadbc_vs_adapter.so
```

On an Exasol Personal local deployment, copy the library into the deployment directory instead. `exasol info --json` reports it as `deploymentDir`.

```bash
DEPLOY_DIR=$(exasol info --json | jq -r '.deploymentDir')
BUCKETFS="$DEPLOY_DIR/local/runtime/exa/bucketfs/bfsdefault/default"
mkdir -p "$BUCKETFS/udf"
cp udf/libadbc_vs_adapter.so "$BUCKETFS/udf/"
```

Add `--deployment <name>` to `exasol info` for a deployment that is not the default one.

The release has no ADBC driver in it. `adbc-vs-rs` gets the driver in step 4.

## 3. Create the scripts

Create a schema for the two scripts. Then create the scripts over the uploaded library. The script names are fixed. The shared library selects its entry point by the script name.

```sql
CREATE SCHEMA IF NOT EXISTS ADBC_VS;

CREATE OR REPLACE RUST ADAPTER SCRIPT ADBC_VS.ADBC_ADAPTER AS
%udf_object /buckets/bfsdefault/default/udf/libadbc_vs_adapter.so;

CREATE OR REPLACE RUST SCALAR SCRIPT ADBC_VS.ADBC_SCAN(scan_spec VARCHAR(2000000)) EMITS (...) AS
%udf_object /buckets/bfsdefault/default/udf/libadbc_vs_adapter.so;
```

## 4. Make the driver available

`adbc-vs-rs` needs one ADBC driver library for each source technology. It gets the library when you create or refresh a virtual schema. Choose one route.

### Let `adbc-vs-rs` install the driver

`adbc-vs-rs` reads the [ADBC Driver Registry](https://dbc-cdn.columnar.tech) at `dbc-cdn.columnar.tech` directly over HTTPS. It does not call `dbc`. It downloads the driver for the architecture of the library, `linux_amd64` or `linux_arm64`, and writes it to BucketFS at `adbc-drivers/<driver>/<version>/<library>`. It does this only if that version is not on BucketFS yet.

This route needs a BucketFS HTTPS endpoint. An Exasol Personal local deployment has none, so use [Install the driver yourself](#install-the-driver-yourself) there.

The write needs a BucketFS write credential. Store it in a `CONNECTION` object with the reserved name `ADBC_VS_BUCKETFS`. The address is the HTTPS address of the BucketFS service, with the bucket name. The user and the password are the BucketFS write user and its password.

```sql
CREATE OR REPLACE CONNECTION ADBC_VS_BUCKETFS
  TO 'https://<exasol-node>:2581/default'
  USER 'w'
  IDENTIFIED BY '<bucketfs-write-password>';
```

To use a different connection for one virtual schema, set the `BUCKETFS_CONNECTION` property. `adbc-vs-rs` then uses only that connection. It never falls back to `ADBC_VS_BUCKETFS`.

### Install the driver yourself

Use this route if the Exasol nodes have no outbound HTTPS access. `adbc-vs-rs` then reads no registry, downloads nothing, and writes nothing to BucketFS.

1. Get the driver library for your source from the [ADBC Driver Foundry](https://github.com/adbc-drivers). Pick the `linux_amd64` package on x86_64 and the `linux_arm64` package on aarch64.
2. Upload it to BucketFS at `adbc-drivers/<driver>/supplied/<library>`. The `<driver>` is `mysql`, `postgresql`, or `trino`. The `<library>` is, for example, `libadbc_driver_mysql.so`.

```bash
exapump bucketfs cp libadbc_driver_mysql.so adbc-drivers/mysql/supplied/libadbc_driver_mysql.so
```

On an Exasol Personal local deployment, copy the library to `$BUCKETFS/adbc-drivers/<driver>/supplied/<library>`, with `$BUCKETFS` as in step 2.

`adbc-vs-rs` uses this file when `ADBC_VS_BUCKETFS` does not exist, or when the registry host cannot be reached. To use a library at another path, set the `DRIVER_PATH` property on the virtual schema. With `DRIVER_PATH`, you need no BucketFS connection.

`adbc-vs-rs` does not check a checksum or a signature of a downloaded driver. The TLS certificate check of the registry is the only integrity check. If you need more, use this route and check the file yourself.

## Next

[Create a Virtual Schema](create-virtual-schema.md)
