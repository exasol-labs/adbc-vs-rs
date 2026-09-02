//! Drives the real ADBC path — driver load through the PostgreSQL entry
//! point, statement execution, batch streaming, coercion — against the
//! `postgres` service in `docker-compose.yml`, with no Exasol anywhere.
//!
//! Structural twin of `mysql_it_test.rs`: same shared driver-cache and
//! connect helpers, same reliance on `fetch_driver_library` for the only
//! route to a driver library outside BucketFS. What differs is what this
//! tier has to prove about PostgreSQL specifically — that the entry point
//! `postgresql.toml` declares actually loads, and that the driver's own
//! string-form `numeric` representation and its microsecond `timestamp`
//! survive coercion into the declared Exasol type, not just the general
//! MySQL-shaped cases `mysql_it_test.rs` already covers.
//!
//! No skip path. A missing container, an unreachable registry, or a driver
//! that will not load fails the test, because a tier that quietly skips
//! itself proves nothing.

#![cfg(feature = "postgres-it")]

mod common;

use std::path::PathBuf;
use std::sync::LazyLock;

use adbc_driver_manager::ManagedConnection;
use adbc_vs_scan::coerce::{DeclaredColumn, DeclaredType};
use adbc_vs_scan::connection::{open, SourceConnection};
use adbc_vs_scan::driver::Driver;
use adbc_vs_scan::error::ScanError;
use adbc_vs_scan::scan::{execute_scan, BatchStream, ScanRequest};
use arrow::array::{
    Array, Date32Array, Decimal128Array, Float64Array, Int64Array, StringArray,
    TimestampMicrosecondArray,
};
use common::driver_fetch::fetch_driver_library;

/// The `postgres` service as `docker-compose.yml` publishes it to the host.
const POSTGRES_URI: &str = "postgresql://127.0.0.1:5432/adbc_vs";
const POSTGRES_USER: &str = "adbc_vs";
const POSTGRES_PASSWORD: &str = "adbc_vs_password";

/// How many rows the multi-batch scan projects.
///
/// The PostgreSQL driver cuts a batch once it has accumulated
/// `adbc.postgresql.batch_size_hint_bytes` (16 MiB, its own default) of data,
/// so this row count and [`WIDE_RESULT_COLUMN_BYTES`] are sized to clear that
/// threshold several times over — comfortable margin against the default
/// changing, rather than a value tuned to the threshold's edge — without
/// writing anything to the fixture database.
const WIDE_RESULT_ROWS: i64 = 300_000;

/// The fixed width of each of [`streams_a_result_across_more_than_one_batch`]'s
/// two text columns, chosen so the total result size is easy to reason about
/// independent of how many digits `generate_series` produces.
const WIDE_RESULT_COLUMN_BYTES: i64 = 100;

/// Fetched, resolved, and cached once per process — under the system
/// temporary directory, never under `target/`, through the same registry
/// resolution and `MANIFEST` read the product's own provisioning performs.
static DRIVER: LazyLock<PathBuf> = LazyLock::new(|| fetch_driver_library(Driver::Postgres));

fn postgres() -> SourceConnection {
    SourceConnection {
        uri: POSTGRES_URI.to_string(),
        username: POSTGRES_USER.to_string(),
        password: POSTGRES_PASSWORD.to_string(),
    }
}

fn connect() -> ManagedConnection {
    open(
        Driver::Postgres,
        DRIVER.to_str().expect("the cache path is UTF-8"),
        &postgres(),
    )
    .expect("the postgres service in docker-compose.yml accepts an ADBC connection")
}

fn scan(
    connection: &mut ManagedConnection,
    statement: &str,
    columns: &[DeclaredColumn],
) -> Result<BatchStream, ScanError> {
    execute_scan(
        connection,
        ScanRequest {
            statement,
            columns,
            password: POSTGRES_PASSWORD,
        },
    )
}

fn declared(name: &str, declared_type: DeclaredType) -> DeclaredColumn {
    DeclaredColumn {
        name: name.to_string(),
        declared_type,
    }
}

/// Proves the driver loads through the entry point `postgresql.toml`
/// declares (`AdbcDriverPostgresqlInit`), rather than through the driver
/// manager's own filename-derivation heuristic, and that a statement then
/// executes over the opened connection.
#[test]
fn driver_loads_via_declared_entry_point_and_executes_a_statement() {
    let mut connection = connect();
    let columns = vec![declared("name", DeclaredType::Text)];

    let batch = scan(
        &mut connection,
        "SELECT name FROM orders WHERE id = 1",
        &columns,
    )
    .expect("the statement executes over the loaded driver")
    .next()
    .expect("the fixture row returns a batch")
    .expect("the batch coerces");

    let names = batch
        .column(0)
        .as_any()
        .downcast_ref::<StringArray>()
        .expect("the name column is the declared text");
    assert_eq!(names.value(0), "Widget");
}

/// A result large enough that the driver must stream it back as more than
/// one Arrow batch, generated without touching the fixture table.
#[test]
fn streams_a_result_across_more_than_one_batch() {
    let mut connection = connect();
    let columns = vec![
        declared("c1", DeclaredType::Text),
        declared("c2", DeclaredType::Text),
    ];
    let statement = format!(
        "SELECT repeat('a', {WIDE_RESULT_COLUMN_BYTES}) AS c1, \
                repeat('b', {WIDE_RESULT_COLUMN_BYTES}) AS c2 \
         FROM generate_series(1, {WIDE_RESULT_ROWS}) AS g"
    );
    let stream = scan(&mut connection, &statement, &columns).expect("the wide scan executes");

    let mut batches = 0usize;
    let mut rows = 0i64;
    for batch in stream {
        let batch = batch.expect("every batch of the wide result coerces");
        batches += 1;
        rows += batch.num_rows() as i64;
    }

    assert_eq!(
        rows, WIDE_RESULT_ROWS,
        "the scan emitted {rows} rows for a source of {WIDE_RESULT_ROWS}"
    );
    assert!(
        batches > 1,
        "the wide result arrived as {batches} batch(es), so nothing about streaming was exercised"
    );
}

/// Every PostgreSQL type `postgres_type_mapping.rs` maps, coerced to its
/// declared Exasol representation in one pass: `integer` and `bigint` as
/// `DECIMAL(p,0)`, `numeric(p,s)` as the declared `DECIMAL(p,s)` — the
/// string-form path, since the driver returns `numeric` as an Arrow string
/// rather than a decimal-typed array — `character varying(n)` as text,
/// `date`, `timestamp without time zone` down to its declared microsecond
/// precision, and `double precision`.
#[test]
fn every_mapped_type_coerces_to_its_declared_exasol_representation() {
    let mut connection = connect();
    let columns = vec![
        declared(
            "id",
            DeclaredType::Decimal {
                precision: 10,
                scale: 0,
            },
        ),
        declared(
            "big_id",
            DeclaredType::Decimal {
                precision: 19,
                scale: 0,
            },
        ),
        declared(
            "amount",
            DeclaredType::Decimal {
                precision: 12,
                scale: 4,
            },
        ),
        declared("name", DeclaredType::Text),
        declared("order_date", DeclaredType::Date),
        declared("created_at", DeclaredType::Timestamp),
        declared("price", DeclaredType::Double),
    ];
    let batch = scan(
        &mut connection,
        "SELECT id, big_id, amount, name, order_date, created_at, price \
         FROM orders ORDER BY id",
        &columns,
    )
    .expect("the projection executes")
    .next()
    .expect("the fixture table returns a batch")
    .expect("the fixture batch coerces");

    let ids = batch
        .column(0)
        .as_any()
        .downcast_ref::<Int64Array>()
        .expect("a declared DECIMAL(10,0) is an Int64 column inside Exasol");
    assert_eq!(ids.value(0), 1, "the integer column lost its value");

    let big_ids = batch
        .column(1)
        .as_any()
        .downcast_ref::<Decimal128Array>()
        .expect("a declared DECIMAL(19,0) is a Decimal128 column inside Exasol");
    assert_eq!(
        big_ids.value(0),
        9_000_000_001,
        "the bigint column lost its value"
    );

    let amounts = batch
        .column(2)
        .as_any()
        .downcast_ref::<Decimal128Array>()
        .expect("the numeric(12,4) column coerces to the declared DECIMAL(12,4)");
    assert_eq!(
        amounts.value(0),
        12_345_600,
        "the string-form numeric the driver returned did not survive coercion at its declared scale"
    );
    assert!(
        amounts.is_null(1),
        "the seeded NULL numeric was replaced by a value"
    );
    assert_eq!(
        amounts.value(2),
        999_999_999_999,
        "the maximum-scale numeric(12,4) did not survive coercion"
    );

    let names = batch
        .column(3)
        .as_any()
        .downcast_ref::<StringArray>()
        .expect("the character varying column is the declared text");
    assert_eq!(names.value(0), "Widget");

    let dates = batch
        .column(4)
        .as_any()
        .downcast_ref::<Date32Array>()
        .expect("the date column is the declared DATE");
    assert_eq!(
        dates.value(0),
        20_468, // 2026-01-15, days since the Unix epoch
        "the date column did not carry the fixture's calendar date"
    );

    let instants = batch
        .column(5)
        .as_any()
        .downcast_ref::<TimestampMicrosecondArray>()
        .expect("the timestamp without time zone column is the declared TIMESTAMP");
    assert_eq!(
        instants.value(0),
        1_768_465_800_123_456, // 2026-01-15 08:30:00.123456, microseconds since the Unix epoch
        "the timestamp's microsecond component did not survive coercion"
    );
    assert_eq!(
        instants.value(2),
        1_774_483_199_999_999, // 2026-03-25 23:59:59.999999, microseconds since the Unix epoch
        "the maximum-fraction microsecond timestamp did not survive coercion"
    );

    let prices = batch
        .column(6)
        .as_any()
        .downcast_ref::<Float64Array>()
        .expect("the double precision column is the declared DOUBLE");
    assert_eq!(
        prices.value(0),
        19.99,
        "the double precision column lost its value"
    );
}

/// A statement against a table PostgreSQL rejects must surface the ADBC
/// status and the driver's own diagnosis, never a swallowed or generic
/// failure — and never the connection's password. The driver may fail the
/// statement immediately in `execute_scan`, or defer the failure to the
/// first batch of the returned stream, so this checks both shapes.
#[test]
fn source_failure_reports_adbc_status() {
    let mut connection = connect();
    let columns = vec![
        declared("c1", DeclaredType::Text),
        declared("c2", DeclaredType::Text),
    ];

    let statement = "SELECT \"c1\", \"c2\" FROM \"no_such_table\"";
    let error = match scan(&mut connection, statement, &columns) {
        Err(error) => error.to_string(),
        Ok(mut stream) => stream
            .find_map(|batch| batch.err())
            .expect("a statement against a table that does not exist must fail")
            .to_string(),
    };

    assert!(
        error.contains("ADBC status"),
        "error message {error:?} does not carry the ADBC status"
    );
    assert!(
        error.contains("no_such_table"),
        "error message {error:?} does not carry the driver's own diagnosis"
    );
    assert!(
        !error.contains(POSTGRES_PASSWORD),
        "error message leaked the password"
    );
}
