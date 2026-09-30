//! Drives the real ADBC path — driver load through the Trino entry point,
//! statement execution, batch streaming, coercion — against the `trino`
//! service in `docker-compose.yml`, with no Exasol anywhere.
//!
//! Structural twin of `postgres_it_test.rs`: same shared driver-cache and
//! connect helpers, same reliance on `fetch_driver_library` for the only
//! route to a driver library outside BucketFS. What differs is what this
//! tier has to prove about Trino specifically. Two things, both measured by
//! gate 2.1 and gate 2.4 rather than inferred:
//!
//! The credential travels in the driver's own `username` and `password`
//! options and reaches no value any parser reads, so a password holding
//! every character that delimits, ends, or escapes something in a URI
//! authenticates unencoded. Proving that needs a statement rather than an
//! open: gate 2.1 measured `new_database_with_opts` and `new_connection` to
//! be lazy, so a connection to a wrong password, an unreachable host, or a
//! server that refuses the transport all "open" and fail on first execute.
//!
//! And this driver is the only one that picks an Arrow width and unit from a
//! column's declared precision rather than returning one of each: three
//! decimal widths and four timestamp units, where the other two drivers
//! reach one width and one unit. Nothing in `coerce.rs` needed changing for
//! them — gate 2.4 measured every cast succeeding — so these assertions pin
//! behavior that already holds rather than closing a gap.
//!
//! No skip path. A missing container, an unreachable registry, or a driver
//! that will not load fails the test, because a tier that quietly skips
//! itself proves nothing.

#![cfg(feature = "trino-it")]

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

/// The `trino` service as `docker-compose.yml` publishes it to the host.
///
/// `SSLVerification=NONE` reaches the self-signed certificate
/// `stack-up-trino` generates. Gate 2.1e measured it as the one spelling
/// this driver honours — `SSLInsecure=true` is not a parameter it knows, and
/// omitting the parameter fails the certificate rather than skipping the
/// check.
const TRINO_URI: &str = "trino://127.0.0.1:8443/memory/adbc_vs?SSLVerification=NONE";
const TRINO_HOST: &str = "127.0.0.1:8443";
const TRINO_CATALOG: &str = "memory";
const TRINO_SCHEMA: &str = "adbc_vs";

/// The principal `docker/trino/password.db` configures for this tier, and the
/// plaintext `docker-compose.yml` and the `Makefile` declare as
/// `TRINO_HOSTILE_PASSWORD`.
///
/// Every character in it — `@`, `/`, `:`, `?`, `#`, `&`, `%`, and a space —
/// delimits, ends, or escapes something in some URI component. The whole
/// point of the credential shape `scan-udf/trino-scan` states is that none of
/// that matters, because the credential reaches no URI, so the tier
/// authenticates as this principal rather than as a benign one.
const TRINO_USER: &str = "adbc_vs_hostile";
const TRINO_PASSWORD: &str = "p@ss/w:o?r#d&x% y";

/// The substrings of [`TRINO_PASSWORD`] a surfaced message is checked
/// against, beside the whole value.
///
/// Single characters cannot be checked: `/`, `:`, and `%` occur in any URI
/// and in most driver messages, so asserting their absence would fail on a
/// message that disclosed nothing. These are the runs long enough that no
/// message produces one by coincidence, which makes a partial disclosure —
/// the shape gate 2.1 measured, where a URI parse error quotes a prefix of
/// what it was handed — detectable rather than only a whole-value one.
const PASSWORD_FRAGMENTS: [&str; 3] = ["p@ss", "w:o?r#d", "&x%"];

/// How many rows the multi-batch scan projects.
///
/// Trino returns a result to its client as a sequence of pages and the driver
/// turns each into one Arrow batch, so this row count and
/// [`WIDE_RESULT_COLUMN_BYTES`] are sized to produce tens of megabytes —
/// comfortable margin against any page-size default rather than a value tuned
/// to one — without writing anything to the fixture catalog.
const WIDE_RESULT_ROWS: i64 = 300_000;

/// The fixed width of each of [`streams_a_result_across_more_than_one_batch`]'s
/// two text columns, chosen so the total result size is easy to reason about.
const WIDE_RESULT_COLUMN_BYTES: i64 = 100;

/// Fetched, resolved, and cached once per process — under the system
/// temporary directory, never under `target/`, through the same registry
/// resolution and `MANIFEST` read the product's own provisioning performs.
static DRIVER: LazyLock<PathBuf> = LazyLock::new(|| fetch_driver_library(Driver::Trino));

fn trino() -> SourceConnection {
    source(TRINO_URI, TRINO_PASSWORD)
}

fn source(uri: &str, password: &str) -> SourceConnection {
    SourceConnection {
        uri: uri.to_string(),
        username: TRINO_USER.to_string(),
        password: password.to_string(),
    }
}

fn driver_path() -> &'static str {
    DRIVER.to_str().expect("the cache path is UTF-8")
}

fn connect() -> ManagedConnection {
    open(Driver::Trino, driver_path(), &trino())
        .expect("the trino service in docker-compose.yml accepts an ADBC connection")
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
            password: TRINO_PASSWORD,
        },
    )
}

fn declared(name: &str, declared_type: DeclaredType) -> DeclaredColumn {
    DeclaredColumn {
        name: name.to_string(),
        declared_type,
    }
}

fn decimal(precision: u32, scale: u32) -> DeclaredType {
    DeclaredType::Decimal { precision, scale }
}

/// Runs `statement` over a connection built from `uri` and `password` and
/// returns whatever the failure said, wherever it surfaced.
///
/// Both shapes have to be admitted because gate 2.1 measured the driver
/// deferring the transport and the credential to the first execute: a wrong
/// password and an unreachable host open successfully and fail later, while a
/// URI the driver cannot parse fails at database creation. Which step failed
/// is not what this returns — the redaction has to hold at every one of them.
fn failure_of(uri: &str, password: &str, statement: &str) -> String {
    let mut connection = match open(Driver::Trino, driver_path(), &source(uri, password)) {
        Err(error) => return error.to_string(),
        Ok(connection) => connection,
    };
    let columns = vec![declared("c1", DeclaredType::Text)];

    match execute_scan(
        &mut connection,
        ScanRequest {
            statement,
            columns: &columns,
            password,
        },
    ) {
        Err(error) => error.to_string(),
        Ok(mut stream) => stream
            .find_map(|batch| batch.err())
            .expect("the connection was expected to fail")
            .to_string(),
    }
}

/// Proves the driver loads through the entry point `trino.toml` declares
/// (`AdbcDriverTrinoInit`) and that a statement then executes over the opened
/// connection.
///
/// It cannot prove the declared entry point was preferred over the driver
/// manager's filename-derivation heuristic, because that heuristic derives the
/// same symbol from `libadbc_driver_trino.so`: the test passes identically
/// either way. What it does establish is that the declared symbol resolves at
/// all, which is what a wrong `entry_point` in the declaration would break.
#[test]
fn driver_loads_via_declared_entry_point_and_executes_a_statement() {
    let mut connection = connect();
    let columns = vec![declared("name", DeclaredType::Text)];

    let batch = scan(
        &mut connection,
        "SELECT name FROM memory.adbc_vs.orders WHERE id = 1",
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

/// `scan-udf/trino-scan`: the resolved user and password travel as the
/// driver's own `username` and `password` options, and a password holding
/// every character that would have delimited, ended, or escaped something in
/// a URI authenticates with no encoding applied to it anywhere.
///
/// A statement is what proves it. Gate 2.1 measured that opening a database
/// and a connection exercises neither the transport nor the credential, so a
/// test that stopped at `open` would pass against a credential the server
/// rejects.
#[test]
fn a_hostile_ascii_credential_authenticates_through_the_separate_options() {
    let mut connection = connect();
    let columns = vec![declared("principal", DeclaredType::Text)];

    let batch = scan(
        &mut connection,
        "SELECT current_user AS principal",
        &columns,
    )
    .expect("the hostile-ASCII credential authenticates")
    .next()
    .expect("the statement returns a batch")
    .expect("the batch coerces");

    let principal = batch
        .column(0)
        .as_any()
        .downcast_ref::<StringArray>()
        .expect("current_user is the declared text");
    assert_eq!(
        principal.value(0),
        TRINO_USER,
        "the server authenticated a different principal than the one the credential names"
    );
}

/// A result large enough that the driver must stream it back as more than
/// one Arrow batch, generated from the stock image's `tpch` catalog so the
/// fixture catalog is neither written to nor read.
#[test]
fn streams_a_result_across_more_than_one_batch() {
    let mut connection = connect();
    let columns = vec![
        declared("c1", DeclaredType::Text),
        declared("c2", DeclaredType::Text),
    ];
    let statement = format!(
        "SELECT lpad('', {WIDE_RESULT_COLUMN_BYTES}, 'a') AS c1, \
                lpad('', {WIDE_RESULT_COLUMN_BYTES}, 'b') AS c2 \
         FROM tpch.sf1.orders LIMIT {WIDE_RESULT_ROWS}"
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

/// Every Trino type `trino_type_mapping.rs` maps, coerced to its declared
/// Exasol representation in one pass — including every Arrow decimal width
/// and every timestamp unit this driver reaches, which is the part no other
/// tier can cover.
///
/// Gate 2.4 measured that the driver picks the narrowest representation a
/// declared precision admits rather than returning one of each: `Decimal32`
/// up to precision 9, `Decimal64` from 10 to 18, `Decimal128` from 19 up,
/// and seconds, milliseconds, microseconds, or nanoseconds according to a
/// timestamp's own precision. `coerce.rs` needed nothing added for any of
/// them — `cast_with_options` under `safe: false` already converts every one
/// — so these assertions pin behavior that holds rather than close a gap.
/// One column per measured band, so no band is asserted through a column
/// that shares another's representation.
///
/// Worth knowing if one ever does fail: `coerce.rs::unscaled_decimal_values`
/// downcasts to `Decimal128Array` alone, so a cast failure over a
/// `Decimal32` or `Decimal64` column reports without the unscaled digits
/// that make the offending row findable.
///
/// The nanosecond unit is the one no other driver reaches.
/// `scan-udf/arrow-scan`'s rule for a value Exasol cannot hold resolves to
/// truncation here, which is what `cast_with_options` does even under
/// `safe: false`, as measured against this container; the third nanosecond row
/// is the case that tells truncation from rounding — `.999999999` becomes
/// `.999999` rather than the next second.
#[test]
fn every_mapped_type_coerces_to_its_declared_exasol_representation() {
    let mut connection = connect();
    let columns = vec![
        declared("id", decimal(10, 0)),
        declared("big_id", decimal(19, 0)),
        declared("small_amount", decimal(9, 2)),
        declared("amount", decimal(12, 4)),
        declared("large_amount", decimal(24, 4)),
        declared("name", DeclaredType::Text),
        declared("order_date", DeclaredType::Date),
        declared("created_at_s", DeclaredType::Timestamp),
        declared("created_at_ms", DeclaredType::Timestamp),
        declared("created_at", DeclaredType::Timestamp),
        declared("created_at_ns", DeclaredType::Timestamp),
        declared("price", DeclaredType::Double),
    ];
    let batch = scan(
        &mut connection,
        "SELECT id, big_id, small_amount, amount, large_amount, name, order_date, \
                created_at_s, created_at_ms, created_at, created_at_ns, price \
         FROM memory.adbc_vs.orders ORDER BY id",
        &columns,
    )
    .expect("the projection executes")
    .next()
    .expect("the fixture table returns a batch")
    .expect("the fixture batch coerces");

    assert_eq!(
        batch.num_rows(),
        3,
        "every assertion below indexes rows 0 to 2 of this one batch, so the driver must return \
         the three fixture rows together; a split batch would otherwise panic out of bounds \
         rather than name the assumption it broke"
    );

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

    let small_amounts = batch
        .column(2)
        .as_any()
        .downcast_ref::<Decimal128Array>()
        .expect("the Decimal32 the driver returns for decimal(9,2) coerces to DECIMAL(9,2)");
    assert_eq!(
        small_amounts.value(0),
        123_456,
        "the Decimal32 column lost its value"
    );
    assert_eq!(
        small_amounts.value(1),
        -999_999_999,
        "the most negative decimal(9,2) did not survive the Decimal32 coercion"
    );
    assert_eq!(
        small_amounts.value(2),
        999_999_999,
        "the maximum decimal(9,2) did not survive the Decimal32 coercion"
    );

    let amounts = batch
        .column(3)
        .as_any()
        .downcast_ref::<Decimal128Array>()
        .expect("the Decimal64 the driver returns for decimal(12,4) coerces to DECIMAL(12,4)");
    assert_eq!(
        amounts.value(0),
        12_345_600,
        "the Decimal64 column lost its value"
    );
    assert!(
        amounts.is_null(1),
        "the seeded NULL decimal was replaced by a value"
    );
    assert_eq!(
        amounts.value(2),
        999_999_999_999,
        "the maximum decimal(12,4) did not survive the Decimal64 coercion"
    );

    let large_amounts = batch
        .column(4)
        .as_any()
        .downcast_ref::<Decimal128Array>()
        .expect("the Decimal128 the driver returns for decimal(24,4) coerces to DECIMAL(24,4)");
    assert_eq!(
        large_amounts.value(0),
        123_456_789_012_345_678_901_234,
        "the Decimal128 column lost its value"
    );
    assert_eq!(
        large_amounts.value(2),
        999_999_999_999_999_999_999_999,
        "the maximum decimal(24,4) did not survive coercion"
    );

    let names = batch
        .column(5)
        .as_any()
        .downcast_ref::<StringArray>()
        .expect("the varchar column is the declared text");
    assert_eq!(names.value(0), "Widget");

    let dates = batch
        .column(6)
        .as_any()
        .downcast_ref::<Date32Array>()
        .expect("the date column is the declared DATE");
    assert_eq!(
        dates.value(0),
        20_468,
        "the date column did not carry the fixture's calendar date, 2026-01-15 as days since \
         the Unix epoch"
    );

    let seconds = batch
        .column(7)
        .as_any()
        .downcast_ref::<TimestampMicrosecondArray>()
        .expect("the timestamp(0) column is the declared TIMESTAMP");
    assert_eq!(
        seconds.value(0),
        1_768_465_800_000_000,
        "the second-unit timestamp 2026-01-15 08:30:00 did not widen to microseconds"
    );

    let milliseconds = batch
        .column(8)
        .as_any()
        .downcast_ref::<TimestampMicrosecondArray>()
        .expect("the timestamp(3) column is the declared TIMESTAMP");
    assert_eq!(
        milliseconds.value(0),
        1_768_465_800_123_000,
        "the millisecond-unit timestamp 2026-01-15 08:30:00.123 did not widen to microseconds"
    );

    let microseconds = batch
        .column(9)
        .as_any()
        .downcast_ref::<TimestampMicrosecondArray>()
        .expect("the timestamp(6) column is the declared TIMESTAMP");
    assert_eq!(
        microseconds.value(0),
        1_768_465_800_123_456,
        "the microsecond component of 2026-01-15 08:30:00.123456 did not survive coercion"
    );

    let nanoseconds = batch
        .column(10)
        .as_any()
        .downcast_ref::<TimestampMicrosecondArray>()
        .expect("the timestamp(9) column is the declared TIMESTAMP");
    assert_eq!(
        nanoseconds.value(0),
        1_768_465_800_123_456,
        "the nanosecond timestamp 2026-01-15 08:30:00.123456789 did not truncate its \
         sub-microsecond digits"
    );
    assert_eq!(
        nanoseconds.value(1),
        1_771_598_700_000_000,
        "2026-02-20 14:45:00.000000001, a value below one microsecond, did not truncate to the \
         whole microsecond"
    );
    assert_eq!(
        nanoseconds.value(2),
        1_774_483_199_999_999,
        "the nanosecond timestamp 2026-03-25 23:59:59.999999999 rounded up to the next second \
         rather than truncating"
    );

    let prices = batch
        .column(11)
        .as_any()
        .downcast_ref::<Float64Array>()
        .expect("the double column is the declared DOUBLE");
    assert_eq!(prices.value(0), 19.99, "the double column lost its value");
}

/// A statement against a table Trino rejects must surface the ADBC status
/// and the driver's own diagnosis, never a swallowed or generic failure —
/// and never the connection's password. The driver may fail the statement
/// immediately in `execute_scan`, or defer the failure to the first batch of
/// the returned stream, so this checks both shapes.
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
        !error.contains(TRINO_PASSWORD),
        "error message leaked the password"
    );
}

/// `scan-udf/trino-scan`: no part of the password reaches a surfaced message,
/// at the database-creation step as well as the connect and execute steps,
/// and the failure still names what it tried to reach.
///
/// Three failures, one per step a credential could surface at, and each is
/// checked against the whole password and against every run of it long
/// enough to be recognisable.
///
/// The malformed address is the step gate 2.1 measured quoting a value it
/// was handed: database creation refuses a URI it cannot parse and echoes it
/// back. That is the disclosure the credential travelling outside the URI
/// closes structurally, and asserting it fails at `open` rather than later
/// is what binds the case to that step rather than to a lazy one.
///
/// The other two are lazy — gate 2.1 measured the open path to defer both
/// the transport and the credential to the first execute — so they surface
/// through the scan's own diagnostic, which names the statement.
///
/// Which failure names the host follows from that split, and is asserted
/// only where it holds. The open path names the userinfo-free `uri` value as
/// its subject, so a malformed address carries the host. A transport failure
/// carries it because the driver's own message quotes the URL it dialled. An
/// authentication refusal carries neither: gate 2.1d measured the driver's
/// 401 quoting no URI and no credential, and the scan's diagnostic names the
/// statement rather than the address, so for that one case the catalog and
/// the schema are what keep the failure diagnosable.
///
/// The redaction is asserted rather than assumed of the driver: nothing
/// upstream binds a driver to omit a value it received as an option.
#[test]
fn a_failing_trino_connection_surfaces_no_part_of_the_password() {
    const UNREACHABLE_HOST: &str = "127.0.0.1:1";
    let statement = "SELECT name AS c1 FROM memory.adbc_vs.orders";
    let malformed = "trino://127.0.0.1:8443/memory/adbc_vs/%ZZ";
    let unreachable = "trino://127.0.0.1:1/memory/adbc_vs?SSLVerification=NONE";

    let Err(refused) = open(
        Driver::Trino,
        driver_path(),
        &source(malformed, TRINO_PASSWORD),
    ) else {
        panic!("an address the driver cannot parse must be refused at database creation");
    };

    let wrong_password = failure_of(TRINO_URI, "wrong-password", statement);
    let malformed_address = refused.to_string();
    let unreachable_host = failure_of(unreachable, TRINO_PASSWORD, statement);

    for (case, message) in [
        ("a wrong password", &wrong_password),
        ("a malformed address", &malformed_address),
        ("an unreachable host", &unreachable_host),
    ] {
        assert!(
            !message.contains(TRINO_PASSWORD),
            "{case} leaked the password: {message}"
        );
        for fragment in PASSWORD_FRAGMENTS {
            assert!(
                !message.contains(fragment),
                "{case} leaked `{fragment}` of the password: {message}"
            );
        }
        assert!(
            message.contains(TRINO_CATALOG) && message.contains(TRINO_SCHEMA),
            "{case} must still name the catalog and the schema it addressed: {message}"
        );
    }

    assert!(
        malformed_address.contains(TRINO_HOST),
        "the subject the open path names is the userinfo-free uri value, so a refused address \
         must carry the host: {malformed_address}"
    );
    assert!(
        unreachable_host.contains(UNREACHABLE_HOST),
        "a transport failure must carry the host the driver dialled: {unreachable_host}"
    );
}
