[Home](index.md) · [Install & Set Up](install.md) · [Create a Virtual Schema](create-virtual-schema.md) · [How Does It Work](how-it-works.md)

---

# Create a Virtual Schema

Complete [Install & Set Up](install.md) first.

A virtual schema needs two things:

1. A `CONNECTION` object that holds the source address and the source credential.
2. A `CREATE VIRTUAL SCHEMA` statement that names the `CONNECTION` object.

## Properties

| Property | Required | Meaning |
|----------|----------|---------|
| `DRIVER` | Yes | The source technology: `mysql`, `postgresql`, or `trino` |
| `CONNECTION_NAME` | Yes | The name of the `CONNECTION` object for the source |
| `BUCKETFS_CONNECTION` | No | The `CONNECTION` object that holds the BucketFS write credential. This replaces `ADBC_VS_BUCKETFS` |
| `DRIVER_PATH` | No | The path of a driver library that you installed yourself. This turns off the automatic install |

If you change a property later, use `ALTER VIRTUAL SCHEMA ... SET`. To read the source metadata again, use `ALTER VIRTUAL SCHEMA ... REFRESH`.

## Table and column names

Exasol keeps the case of the source names. Put the names in double quotes in your query, for example `MYSQLVS."customers"` for a schema named `MYSQLVS`.

`adbc-vs-rs` leaves out a column that has no matching Exasol type. It leaves out a table that has no usable column. It records each omission in the `adapterNotes` of the schema.

## Errors

An error message never contains a password or a connection string. If the source is not reachable, the message names the source host and database.

## Databases

Each database has its own address form and a few details. The `CREATE VIRTUAL SCHEMA` statement is the same for all of them, except for the `DRIVER` value.

### MySQL

The address is `mysql://host:port/database`. The user and the password of the `CONNECTION` object are the credential for the source.

```sql
CREATE OR REPLACE CONNECTION MYSQL_CONNECTION
  TO 'mysql://mysql.example.com:3306/shop'
  USER 'adbc_vs' IDENTIFIED BY '<password>';

CREATE VIRTUAL SCHEMA MYSQLVS USING ADBC_VS.ADBC_ADAPTER WITH
  DRIVER = 'mysql'
  CONNECTION_NAME = 'MYSQL_CONNECTION';

SELECT * FROM MYSQLVS."customers";
```

### PostgreSQL

The address is `postgresql://host:port/database`. The virtual schema shows the `public` schema of that database.

```sql
CREATE OR REPLACE CONNECTION PG_CONNECTION
  TO 'postgresql://pg.example.com:5432/shop'
  USER 'adbc_vs' IDENTIFIED BY '<password>';

CREATE VIRTUAL SCHEMA PGVS USING ADBC_VS.ADBC_ADAPTER WITH
  DRIVER = 'postgresql'
  CONNECTION_NAME = 'PG_CONNECTION';
```

### Trino

The address is `trino://host[:port]/<catalog>/<schema>[?parameters]`. Put the user and the password in the fields of the `CONNECTION` object, not in the address.

```sql
CREATE OR REPLACE CONNECTION TRINO_CONNECTION
  TO 'trino://trino.example.com:8443/memory/shop'
  USER 'adbc_vs' IDENTIFIED BY '<password>';

CREATE VIRTUAL SCHEMA TRINOVS USING ADBC_VS.ADBC_ADAPTER WITH
  DRIVER = 'trino'
  CONNECTION_NAME = 'TRINO_CONNECTION';
```

A Trino server that requires a password accepts it over HTTPS only. The driver uses HTTPS by default. If the server has a self-signed certificate, add `?SSLVerification=NONE` to the address.

Trino finds a wrong password only when the first statement runs. The error message then names the catalog and the schema, but not the host.

## Next

[How Does It Work](how-it-works.md)
