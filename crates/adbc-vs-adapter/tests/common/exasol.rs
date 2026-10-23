//! The Exasol instance under test, and the two ways the harness reaches it:
//! SQL and BucketFS.
//!
//! Which instance that is, is a compile-time choice of backend. The default
//! [`docker`] backend binds to the Compose stack's `exasol` service; under the
//! `exasol-personal` feature, the [`personal`] backend binds to the Exasol
//! Personal deployment the Makefile installs. Each backend discovers its own
//! instance and owns its BucketFS route. Both serve the same SQL and BucketFS
//! methods, so no caller spells out a host, a port, a credential, or a BucketFS
//! layout. Each backend also exports what only its own route needs: the Docker
//! backend the BucketFS write endpoint, user, and password that self-install
//! writes through, and the Personal backend its deployment name and launcher.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use super::readiness::{self, BUCKETFS_DEADLINE, SERVICE_DEADLINE};
use super::{run, try_run};

#[cfg(not(feature = "exasol-personal"))]
mod docker;
#[cfg(feature = "exasol-personal")]
mod personal;

#[cfg(not(feature = "exasol-personal"))]
pub use docker::BUCKETFS_WRITE_USER;
#[cfg(not(feature = "exasol-personal"))]
use docker::{discover, Backend};
#[cfg(feature = "exasol-personal")]
use personal::{discover, Backend};
#[cfg(feature = "exasol-personal")]
pub use personal::{launcher, DEPLOYMENT as PERSONAL_DEPLOYMENT};

/// The BucketFS service and bucket every path a UDF opens names. Both
/// backends serve the database's default pair.
const BUCKETFS_SERVICE: &str = "bfsdefault";
const BUCKET: &str = "default";

/// The Exasol instance under test.
pub struct Exasol {
    /// The `exapump` connection string. It carries the `sys` password, so it
    /// is passed to `exapump` and never into a message.
    dsn: String,
    /// The `host:port` Exasol answers SQL at.
    sql_endpoint: String,
    backend: Backend,
}

impl Exasol {
    /// Binds to the Exasol of the stack and waits until it answers SQL.
    ///
    /// Panics when the stack is down, so a test can never quietly pass against
    /// no database.
    pub fn from_stack() -> Self {
        let exasol = discover();
        exasol.await_sql();
        exasol
    }

    /// Executes one statement, failing the test if Exasol rejects it.
    pub fn execute(&self, sql: &str) {
        run(
            &format!("executing SQL against Exasol:\n{sql}"),
            "exapump",
            &["sql", "--dsn", &self.dsn, sql],
        );
    }

    /// Executes one statement that must fail, and returns the error Exasol
    /// surfaced to the session.
    ///
    /// Fails the test when the statement succeeds: a failure path that stopped
    /// failing is exactly what this is here to catch.
    pub fn execute_expecting_failure(&self, sql: &str) -> String {
        match try_run(
            &format!("executing SQL against Exasol:\n{sql}"),
            "exapump",
            &["sql", "--dsn", &self.dsn, sql],
        ) {
            Err(failure) => failure,
            Ok(_) => panic!("this statement was expected to fail, but Exasol accepted it:\n{sql}"),
        }
    }

    /// Runs a query and returns its rows, each a column-name to value map.
    ///
    /// A column that is NULL in a row is absent from that row's map, which is
    /// how `exapump` renders it.
    pub fn query_rows(&self, sql: &str) -> Vec<Map<String, Value>> {
        let result = run(
            &format!("querying Exasol:\n{sql}"),
            "exapump",
            &["sql", "--dsn", &self.dsn, "--format", "json", sql],
        );
        parse_rows(&result, sql)
    }

    /// Runs a query of one column and returns its first value, or `None` when
    /// the query returned no row or that row's value is NULL.
    pub fn query_scalar(&self, sql: &str) -> Option<String> {
        let rows = self.query_rows(sql);
        let row = rows.first()?;
        assert!(
            row.len() <= 1,
            "a single-column result was expected, but the row is {row:?}"
        );
        row.values().next().map(render_cell)
    }

    /// Uploads a shared object and returns once a UDF could open it.
    ///
    /// Waiting is part of the upload rather than a second call a caller might
    /// forget: a script created over a half-replicated `.so` fails at run time
    /// with a diagnosis far worse than a timeout naming the path.
    pub fn put_library(&self, local: &Path, bucket_path: &str) {
        let length = local
            .metadata()
            .unwrap_or_else(|error| panic!("measuring {}: {error}", local.display()))
            .len();

        self.put_file(local, bucket_path);

        readiness::wait_until(
            &format!("BucketFS object {bucket_path} of {length} bytes"),
            BUCKETFS_DEADLINE,
            || self.bucket_file_size(bucket_path) == Some(length),
        );
    }

    /// Deletes a bucket object and returns once it is gone from the tree a UDF
    /// reads.
    pub fn delete_file(&self, bucket_path: &str) {
        self.remove_bucket_object(bucket_path);

        readiness::wait_until(
            &format!("BucketFS object {bucket_path} to disappear"),
            BUCKETFS_DEADLINE,
            || !self.bucket_path_exists(bucket_path),
        );
    }

    /// Waits until `bucket_path` is readable by a UDF, which for an uploaded
    /// archive means BucketFS has finished extracting it.
    pub fn await_bucket_path(&self, bucket_path: &str) {
        readiness::wait_until(
            &format!("BucketFS path {bucket_path}"),
            BUCKETFS_DEADLINE,
            || self.bucket_path_exists(bucket_path),
        );
    }

    /// Waits until Exasol answers a query, not merely until its port accepts.
    ///
    /// Exasol starts listening well before the database is loaded, and a
    /// restart drops every session, so the port alone would let the next
    /// statement race the start-up and fail for a reason that has nothing to
    /// do with the test.
    pub fn await_sql(&self) {
        readiness::wait_for_service("exasol", &self.sql_endpoint, SERVICE_DEADLINE);
        readiness::wait_until(
            &format!("exasol at {}", self.sql_endpoint),
            SERVICE_DEADLINE,
            || {
                try_run(
                    "checking whether Exasol answers SQL",
                    "exapump",
                    &["sql", "--dsn", &self.dsn, "SELECT 1"],
                )
                .is_ok()
            },
        );
    }
}

/// The workspace root, the directory holding `docker-compose.yml`.
pub fn workspace_root() -> PathBuf {
    let crate_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = crate_root.join("..").join("..");
    root.canonicalize()
        .unwrap_or_else(|error| panic!("resolving the workspace root from {crate_root:?}: {error}"))
}

/// How Exasol itself addresses `bucket_path` — the form its `SCRIPT_LANGUAGES`
/// alias URLs use.
pub fn bucketfs_path(bucket_path: &str) -> String {
    format!("{BUCKETFS_SERVICE}/{BUCKET}/{bucket_path}")
}

/// The file path a running UDF sees for `bucket_path` — the form a script's
/// `%udf_object` directive names.
pub fn udf_object_path(bucket_path: &str) -> String {
    format!("/buckets/{}", bucketfs_path(bucket_path))
}

/// The text form of one result cell, for a test that compares against a
/// literal: a JSON string without its quotes, any other value as JSON renders
/// it.
pub fn render_cell(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// The value of `column` in `row`, or `None` when the column is NULL —
/// `exapump` omits a NULL column from the row it renders.
pub fn cell(row: &Map<String, Value>, column: &str) -> Option<String> {
    row.get(column).map(render_cell)
}

/// Parses `exapump sql --format json` output into its rows.
fn parse_rows(stdout: &str, sql: &str) -> Vec<Map<String, Value>> {
    let rows: Vec<Value> = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|error| panic!("parsing the result of `{sql}`: {error}\n{stdout}"));
    rows.into_iter()
        .map(|row| match row {
            Value::Object(fields) => fields,
            other => panic!("`{sql}` returned a result row that is not an object: {other}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn bucketfs_path_prefixes_the_service_and_bucket() {
        assert_eq!(
            bucketfs_path("slc/rustslc"),
            "bfsdefault/default/slc/rustslc"
        );
    }

    #[test]
    fn udf_object_path_names_the_in_database_mount() {
        assert_eq!(
            udf_object_path("udf/libadbc_vs_adapter.so"),
            "/buckets/bfsdefault/default/udf/libadbc_vs_adapter.so"
        );
    }

    #[test]
    fn parse_rows_reads_every_row_of_a_result() {
        let rows = parse_rows(r#"[{"A":1,"B":"x,y"},{"A":2,"B":"p"}]"#, "SELECT A, B");

        assert_eq!(rows.len(), 2);
        assert_eq!(cell(&rows[0], "B"), Some("x,y".to_string()));
        assert_eq!(cell(&rows[1], "A"), Some("2".to_string()));
    }

    #[test]
    fn parse_rows_reads_an_empty_result_as_no_rows() {
        assert!(parse_rows("[]\n", "SELECT 1 WHERE FALSE").is_empty());
    }

    #[test]
    fn a_null_column_is_absent_from_its_row() {
        let rows = parse_rows(r#"[{"A":1}]"#, "SELECT A, B");

        assert_eq!(cell(&rows[0], "B"), None);
    }

    #[test]
    fn render_cell_strips_the_quotes_of_a_text_value_only() {
        assert_eq!(render_cell(&json!("Widget")), "Widget");
        assert_eq!(render_cell(&json!(1234.56)), "1234.56");
        assert_eq!(render_cell(&json!(9_000_000_001i64)), "9000000001");
    }
}
