//! Drives the real ADBC path — driver load, statement execution, batch
//! streaming, coercion — against the `mysql` service in `docker-compose.yml`,
//! with no Exasol anywhere.
//!
//! This is the tier where the riskiest code in the repository is debuggable:
//! a failure here names an Arrow type or an ADBC status directly, instead of
//! surfacing as a virtual-schema query that returned the wrong rows. There
//! is no skip path. A missing container, an unreachable registry, or a
//! driver that will not load fails the test, because a tier that quietly
//! skips itself proves nothing.

#![cfg(feature = "mysql-it")]

mod common;

use std::fs;
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex, MutexGuard};

use adbc_driver_manager::ManagedConnection;
use adbc_vs_scan::coerce::{DeclaredColumn, DeclaredType};
use adbc_vs_scan::connection::{open, SourceConnection};
use adbc_vs_scan::driver::Driver;
use adbc_vs_scan::error::ScanError;
use adbc_vs_scan::scan::{execute_scan, BatchStream, ScanRequest};
use arrow::array::{Array, Decimal128Array, StringArray, TimestampMicrosecondArray};
use arrow::datatypes::{DataType, TimeUnit};
use common::driver_fetch::fetch_driver_library;

/// The `mysql` service as `docker-compose.yml` publishes it to the host.
const MYSQL_URI: &str = "mysql://127.0.0.1:3306/adbc_vs";
const MYSQL_USER: &str = "adbc_vs";
const MYSQL_PASSWORD: &str = "adbc_vs_password";

/// How many rows [`wide_result_sql`] projects: enough that holding every
/// batch would be unmistakable in the process's resident size (at roughly
/// 105 bytes per row, about 40 MB — well past the 16 MiB streamed-bytes
/// threshold `streams_every_batch_without_growth` asserts), without paying
/// to materialize the full, unbounded cross join.
const WIDE_RESULT_ROWS: usize = 400_000;

/// A result large enough that holding every batch would be unmistakable in
/// the process's resident size, built without writing anything to the
/// fixture database.
fn wide_result_sql() -> String {
    format!(
        "SELECT REPEAT(a.COLUMN_NAME, 4) AS c1, REPEAT(b.COLUMN_NAME, 4) AS c2 \
         FROM information_schema.COLUMNS a CROSS JOIN information_schema.COLUMNS b \
         LIMIT {WIDE_RESULT_ROWS}"
    )
}

fn wide_result_count_sql() -> String {
    format!("SELECT COUNT(*) AS n FROM ({}) t", wide_result_sql())
}

/// Resident set size is a property of the whole process, and loading the
/// 46 MB driver is a large step change in it, so the growth measurement is
/// only meaningful while no other test is opening a connection. Every test
/// takes this lock, which makes the file sequential whatever `--test-threads`
/// says.
static SERIALIZED: Mutex<()> = Mutex::new(());

fn serialized() -> MutexGuard<'static, ()> {
    SERIALIZED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Fetched, resolved, and cached once per process — under the system
/// temporary directory, never under `target/`, which the artifact assertions
/// scan for exactly this kind of binary — through the same registry
/// resolution and `MANIFEST` read the product's own provisioning performs.
static DRIVER: LazyLock<PathBuf> = LazyLock::new(|| fetch_driver_library(Driver::MySql));

fn mysql() -> SourceConnection {
    SourceConnection {
        uri: MYSQL_URI.to_string(),
        username: MYSQL_USER.to_string(),
        password: MYSQL_PASSWORD.to_string(),
    }
}

fn connect() -> ManagedConnection {
    open(
        Driver::MySql,
        DRIVER.to_str().expect("the cache path is UTF-8"),
        &mysql(),
    )
    .expect("the mysql service in docker-compose.yml accepts an ADBC connection")
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
            password: MYSQL_PASSWORD,
        },
    )
}

fn declared(name: &str, declared_type: DeclaredType) -> DeclaredColumn {
    DeclaredColumn {
        name: name.to_string(),
        declared_type,
    }
}

fn text_pair() -> Vec<DeclaredColumn> {
    vec![
        declared("c1", DeclaredType::Text),
        declared("c2", DeclaredType::Text),
    ]
}

/// The page size `/proc/self/statm` reports its counts in, which is 4 KiB on
/// x86-64 but 16 KiB or 64 KiB on some aarch64 kernels.
fn page_bytes() -> usize {
    static PAGE_BYTES: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *PAGE_BYTES.get_or_init(|| {
        let output = std::process::Command::new("getconf")
            .arg("PAGESIZE")
            .output()
            .expect("`getconf PAGESIZE` must run");
        assert!(
            output.status.success(),
            "`getconf PAGESIZE` exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout)
            .expect("`getconf PAGESIZE` prints text")
            .trim()
            .parse()
            .expect("`getconf PAGESIZE` prints a number")
    })
}

fn resident_bytes() -> usize {
    let statm = fs::read_to_string("/proc/self/statm").expect("this tier runs on Linux");
    let resident_pages: usize = statm
        .split_whitespace()
        .nth(1)
        .expect("/proc/self/statm reports a resident page count")
        .parse()
        .expect("the resident page count is a number");
    resident_pages * page_bytes()
}

fn count_of(connection: &mut ManagedConnection, statement: &str) -> i128 {
    let columns = vec![declared(
        "n",
        DeclaredType::Decimal {
            precision: 19,
            scale: 0,
        },
    )];
    let batch = scan(connection, statement, &columns)
        .expect("the count statement executes")
        .next()
        .expect("a count statement returns a row")
        .expect("the count batch coerces");
    batch
        .column(0)
        .as_any()
        .downcast_ref::<Decimal128Array>()
        .expect("the count column is the declared DECIMAL")
        .value(0)
}

#[test]
fn streams_every_batch_without_growth() {
    let _guard = serialized();
    let mut connection = connect();
    let expected_rows = count_of(&mut connection, &wide_result_count_sql());
    let columns = text_pair();
    let stream =
        scan(&mut connection, &wide_result_sql(), &columns).expect("the wide scan executes");

    let mut batches = 0usize;
    let mut rows = 0i128;
    let mut streamed_bytes = 0usize;
    let mut baseline = 0usize;
    let mut peak = 0usize;
    for batch in stream {
        let batch = batch.expect("every batch of the wide result coerces");
        batches += 1;
        rows += batch.num_rows() as i128;
        streamed_bytes += batch.get_array_memory_size();
        drop(batch);
        if batches == 1 {
            baseline = resident_bytes();
            peak = baseline;
        } else {
            peak = peak.max(resident_bytes());
        }
    }

    assert_eq!(
        rows, expected_rows,
        "the scan emitted {rows} rows for a source of {expected_rows}"
    );
    assert!(
        batches > 1,
        "the wide result arrived as {batches} batch(es), so nothing about streaming was exercised"
    );
    assert!(
        streamed_bytes >= 16 * 1024 * 1024,
        "the wide result carried only {streamed_bytes} bytes, too few to tell streaming from accumulating"
    );
    let growth = peak.saturating_sub(baseline);
    assert!(
        growth < streamed_bytes / 3,
        "resident memory grew by {growth} bytes while streaming {streamed_bytes} bytes, \
         so the scan is accumulating batches rather than releasing them"
    );
}

#[test]
fn emitted_columns_match_declaration() {
    let _guard = serialized();
    let mut connection = connect();
    let columns = vec![
        declared(
            "id",
            DeclaredType::Decimal {
                precision: 10,
                scale: 0,
            },
        ),
        declared("name", DeclaredType::Text),
        declared("created_at", DeclaredType::Timestamp),
    ];
    let stream = scan(
        &mut connection,
        "SELECT `id`, `name`, `created_at` FROM `orders` ORDER BY `id`",
        &columns,
    )
    .expect("the projection executes");
    let declared_schema = stream.schema();

    let mut batches = 0usize;
    for batch in stream {
        let batch = batch.expect("every batch coerces");
        batches += 1;
        assert_eq!(batch.schema(), declared_schema);
    }

    assert!(batches > 0, "the fixture table returned no batch at all");
    let fields: Vec<_> = declared_schema
        .fields()
        .iter()
        .map(|field| (field.name().as_str(), field.data_type().clone()))
        .collect();
    assert_eq!(
        fields,
        vec![
            ("id", DataType::Int64),
            ("name", DataType::Utf8),
            (
                "created_at",
                DataType::Timestamp(TimeUnit::Microsecond, None)
            ),
        ],
        "a declared DECIMAL(10,0) is an INT64 column inside Exasol, and the emitted \
         batch has to carry the representation the container accepts"
    );
}

#[test]
fn declared_values_survive_the_drivers_own_representation() {
    let _guard = serialized();
    let mut connection = connect();
    let columns = vec![
        declared(
            "amount",
            DeclaredType::Decimal {
                precision: 12,
                scale: 4,
            },
        ),
        declared("name", DeclaredType::Text),
        declared("created_at", DeclaredType::Timestamp),
    ];
    let batch = scan(
        &mut connection,
        "SELECT `amount`, `name`, `created_at` FROM `orders` ORDER BY `id`",
        &columns,
    )
    .expect("the projection executes")
    .next()
    .expect("the fixture table returns a batch")
    .expect("the fixture batch coerces");

    let amounts = batch
        .column(0)
        .as_any()
        .downcast_ref::<Decimal128Array>()
        .expect("the amount column is the declared DECIMAL(12,4)");
    assert_eq!(
        amounts.value(0),
        12_345_600,
        "a plain DECIMAL lost its scale"
    );
    assert!(
        amounts.is_null(1),
        "the seeded NULL DECIMAL was replaced by a value"
    );
    assert_eq!(
        amounts.value(2),
        999_999_999_999,
        "the maximum-scale DECIMAL(12,4) did not survive coercion"
    );

    let names = batch
        .column(1)
        .as_any()
        .downcast_ref::<StringArray>()
        .expect("the name column is the declared text");
    assert_eq!(names.value(0), "Widget");

    let instants = batch
        .column(2)
        .as_any()
        .downcast_ref::<TimestampMicrosecondArray>()
        .expect("the created_at column is the declared TIMESTAMP");
    assert_eq!(instants.value(0), 1_768_465_800_000_000);
}

#[test]
fn empty_result_emits_nothing() {
    let _guard = serialized();
    let mut connection = connect();
    let columns = text_pair();
    let stream = scan(
        &mut connection,
        "SELECT `name` AS c1, `code` AS c2 FROM `orders` WHERE 1 = 0",
        &columns,
    )
    .expect("a statement matching no row still executes");
    let declared_schema = stream.schema();

    let mut rows = 0usize;
    let mut batches = 0usize;
    for batch in stream {
        let batch = batch.expect("an empty result produces no failing batch");
        batches += 1;
        rows += batch.num_rows();
    }

    assert_eq!(rows, 0, "an empty result emitted {rows} rows");
    assert_eq!(
        batches, 0,
        "an empty result emitted {batches} batches rather than none"
    );
    assert_eq!(
        declared_schema.fields().len(),
        2,
        "an empty result lost the declared output columns"
    );
}

#[test]
fn missing_driver_path_named_in_error() {
    let _guard = serialized();
    let missing = std::env::temp_dir().join("adbc-vs-no-such-driver.so");

    let message = match open(
        Driver::MySql,
        missing.to_str().expect("the path is UTF-8"),
        &mysql(),
    ) {
        Ok(_) => panic!("a driver path that does not exist must fail to load"),
        Err(error) => error.to_string(),
    };
    assert!(
        message.contains(missing.to_str().expect("the path is UTF-8")),
        "error message {message:?} does not name the path it tried to load"
    );
    assert!(
        !message.contains(MYSQL_PASSWORD),
        "error message leaked the password"
    );
}

#[test]
fn source_failure_reports_adbc_status() {
    let _guard = serialized();
    let mut connection = connect();
    let columns = text_pair();

    let statement = "SELECT `c1`, `c2` FROM `no_such_table`";
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
        !error.contains(MYSQL_PASSWORD),
        "error message leaked the password"
    );
}
