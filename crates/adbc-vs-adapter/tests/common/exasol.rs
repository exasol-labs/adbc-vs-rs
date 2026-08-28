//! The Exasol instance of the Compose stack, and the three ways the harness
//! reaches it: SQL, BucketFS, and the container itself.
//!
//! One module owns where that instance lives and how to authenticate to it, so
//! nothing else in the harness spells out a host, a port, a credential, or a
//! BucketFS layout.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use super::readiness::{self, BUCKETFS_DEADLINE, SERVICE_DEADLINE};
use super::{run, try_run};

/// Host and ports come from `docker-compose.yml`, which binds them to fixed
/// loopback ports; the `sys` password is the `exasol/docker-db` default, which
/// the Compose file does not override.
const HOST: &str = "localhost";
const SQL_PORT: u16 = 8563;
const BUCKETFS_PORT: u16 = 2581;
const USER: &str = "sys";
const PASSWORD: &str = "exasol";

/// BucketFS service, bucket, and write user baked into `exasol/docker-db`.
const BUCKETFS_SERVICE: &str = "bfsdefault";
const BUCKET: &str = "default";
pub const BUCKETFS_WRITE_USER: &str = "w";

/// The Compose service whose container hosts the database.
const SERVICE: &str = "exasol";

/// Reads the `default` bucket's write password out of the cluster
/// configuration and decodes it. The image generates the password per
/// container, so the container is the only place it exists.
const READ_WRITE_PASSWORD: &str = r"awk '/\[\[Bucket : default\]\]/{f=1} f && /WritePasswd/{print $3; exit}' /exa/etc/EXAConf | base64 -d";

/// Where the container keeps what a UDF sees under `/buckets`: the unpacked
/// contents of every bucket, including the extracted contents of an uploaded
/// archive.
///
/// The harness watches this tree rather than the bucket's HTTP listing because
/// the listing answers neither question that matters — it reports an uploaded
/// archive's own name rather than what was extracted from it, and it reports
/// every object's length as zero. This tree answers both, and answers them
/// about the same bytes the UDF will open.
const UNPACKED_BUCKETS: &str = "/exa/data/bucketfs";

/// The Exasol instance under test.
pub struct Exasol {
    container: String,
    node_address: String,
    bucketfs_write_password: String,
}

impl Exasol {
    /// Binds to the Exasol of the Compose stack and waits until it answers
    /// SQL.
    ///
    /// Panics when the stack is down, so a test can never quietly pass against
    /// no database.
    pub fn from_stack() -> Self {
        let compose_file = workspace_root().join("docker-compose.yml");
        let compose_file = compose_file.to_string_lossy().into_owned();

        let container = run(
            "locating the Exasol container of the Compose stack",
            "docker",
            &["compose", "--file", &compose_file, "ps", "--quiet", SERVICE],
        );
        let container = container.trim().to_owned();
        assert!(
            !container.is_empty(),
            "the Compose stack described by {compose_file} runs no `{SERVICE}` \
             container; start it with `make stack-up`"
        );

        let bucketfs_write_password = run(
            "reading the BucketFS write password from the Exasol container",
            "docker",
            &["exec", &container, "bash", "-c", READ_WRITE_PASSWORD],
        );
        let bucketfs_write_password = bucketfs_write_password.trim().to_owned();
        assert!(
            !bucketfs_write_password.is_empty(),
            "container {container} reports no write password for bucket \
             `{BUCKET}` in /exa/etc/EXAConf"
        );

        let exasol = Self {
            node_address: node_address(&container),
            container,
            bucketfs_write_password,
        };
        exasol.await_sql();
        exasol
    }

    /// Executes one statement, failing the test if Exasol rejects it.
    pub fn execute(&self, sql: &str) {
        run(
            &format!("executing SQL against Exasol:\n{sql}"),
            "exapump",
            &["sql", "--dsn", &dsn(), sql],
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
            &["sql", "--dsn", &dsn(), sql],
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
            &["sql", "--dsn", &dsn(), "--format", "json", sql],
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

    /// Uploads a local file into the bucket at `bucket_path`.
    ///
    /// `bucket_path` is bucket-relative with no leading slash and no bucket
    /// segment — `slc/rustslc.tar.gz`, not `/default/slc/rustslc.tar.gz`.
    ///
    /// The upload is acknowledged before BucketFS has finished unpacking it,
    /// so a caller that needs the result readable waits for it: through
    /// [`put_library`](Self::put_library) for a shared object, or
    /// [`await_bucket_path`](Self::await_bucket_path) for an archive's
    /// extracted contents.
    pub fn put_file(&self, local: &Path, bucket_path: &str) {
        assert!(
            local.is_file(),
            "uploading to BucketFS at {bucket_path}: {} is not a file",
            local.display()
        );

        let source = local.to_string_lossy().into_owned();
        let url = format!("https://{HOST}:{BUCKETFS_PORT}/{BUCKET}/{bucket_path}");

        run(
            &format!("uploading {source} to BucketFS at {bucket_path}"),
            "curl",
            &[
                "--fail",
                "--silent",
                "--show-error",
                "--insecure",
                "--max-time",
                "600",
                "--user",
                &self.credentials(),
                "--upload-file",
                &source,
                &url,
            ],
        );
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
        let url = format!("https://{HOST}:{BUCKETFS_PORT}/{BUCKET}/{bucket_path}");

        run(
            &format!("deleting BucketFS object {bucket_path}"),
            "curl",
            &[
                "--fail",
                "--silent",
                "--show-error",
                "--insecure",
                "--max-time",
                "600",
                "--user",
                &self.credentials(),
                "--request",
                "DELETE",
                &url,
            ],
        );

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

    /// The size a UDF would see for `bucket_path`, or `None` when nothing is
    /// there.
    pub fn bucket_file_size(&self, bucket_path: &str) -> Option<u64> {
        let size = try_run(
            &format!("measuring BucketFS object {bucket_path}"),
            "docker",
            &[
                "exec",
                &self.container,
                "stat",
                "--format",
                "%s",
                &unpacked_path(bucket_path),
            ],
        )
        .ok()?;
        size.trim().parse().ok()
    }

    /// Whether a UDF would find anything at `bucket_path`.
    pub fn bucket_path_exists(&self, bucket_path: &str) -> bool {
        try_run(
            &format!("looking for BucketFS path {bucket_path}"),
            "docker",
            &[
                "exec",
                &self.container,
                "test",
                "-e",
                &unpacked_path(bucket_path),
            ],
        )
        .is_ok()
    }

    /// Every object the bucket lists, as bucket-relative paths.
    pub fn bucket_entries(&self) -> Vec<String> {
        let listing = run(
            "listing the BucketFS bucket",
            "curl",
            &[
                "--fail",
                "--silent",
                "--show-error",
                "--insecure",
                "--user",
                &self.credentials(),
                &format!("https://{HOST}:{BUCKETFS_PORT}/{BUCKET}/"),
            ],
        );
        listing
            .lines()
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
            .map(str::to_owned)
            .collect()
    }

    /// The bucket's write endpoint as the cluster itself reaches it — the
    /// address a `BUCKETFS_CONNECTION` object carries.
    ///
    /// It is the node's own address on the Compose network rather than
    /// `localhost`, because the adapter writes from inside a UDF sandbox and
    /// nothing guarantees the sandbox shares the node's loopback interface.
    pub fn bucketfs_write_url(&self) -> String {
        format!("https://{}:{BUCKETFS_PORT}/{BUCKET}", self.node_address)
    }

    /// The bucket's write password, generated per container.
    pub fn bucketfs_write_password(&self) -> &str {
        &self.bucketfs_write_password
    }

    fn credentials(&self) -> String {
        format!("{BUCKETFS_WRITE_USER}:{}", self.bucketfs_write_password)
    }

    /// Waits until Exasol answers a query, not merely until its port accepts.
    ///
    /// `exasol/docker-db` starts listening well before the database is
    /// loaded, so the port alone would let the first statement race the
    /// start-up and fail for a reason that has nothing to do with the test.
    fn await_sql(&self) {
        readiness::wait_for_service("exasol", &sql_endpoint(), SERVICE_DEADLINE);
        readiness::wait_until(
            &format!("exasol at {HOST}:{SQL_PORT}"),
            SERVICE_DEADLINE,
            || {
                try_run(
                    "checking whether Exasol answers SQL",
                    "exapump",
                    &["sql", "--dsn", &dsn(), "SELECT 1"],
                )
                .is_ok()
            },
        );
    }
}

/// The connection string for the stack's Exasol.
///
/// It always disables certificate validation: the container serves a
/// self-signed certificate, so a connection that validates cannot reach it.
pub fn dsn() -> String {
    format!("exasol://{USER}:{PASSWORD}@{HOST}:{SQL_PORT}?validateservercertificate=0")
}

/// The `host:port` the stack publishes Exasol's SQL port at.
pub fn sql_endpoint() -> String {
    format!("{HOST}:{SQL_PORT}")
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

/// Where the container keeps the unpacked form of `bucket_path`.
fn unpacked_path(bucket_path: &str) -> String {
    format!("{UNPACKED_BUCKETS}/{BUCKETFS_SERVICE}/.dest/{BUCKET}/{bucket_path}")
}

/// The container's own address on the Compose network.
fn node_address(container: &str) -> String {
    let addresses = run(
        "reading the Exasol container's address on the Compose network",
        "docker",
        &[
            "inspect",
            "--format",
            "{{range .NetworkSettings.Networks}}{{.IPAddress}} {{end}}",
            container,
        ],
    );
    let addresses: Vec<&str> = addresses.split_whitespace().collect();
    assert_eq!(
        addresses.len(),
        1,
        "container {container} must sit on exactly one network for its address to be \
         unambiguous, but reports {addresses:?}"
    );
    addresses[0].to_owned()
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
    fn dsn_accepts_the_containers_self_signed_certificate() {
        assert!(
            dsn().contains("validateservercertificate=0"),
            "DSN must not validate the container's self-signed certificate: {}",
            dsn()
        );
    }

    #[test]
    fn dsn_addresses_the_compose_sql_port() {
        assert_eq!(
            dsn(),
            "exasol://sys:exasol@localhost:8563?validateservercertificate=0"
        );
    }

    #[test]
    fn sql_endpoint_names_the_published_loopback_port() {
        assert_eq!(sql_endpoint(), "localhost:8563");
    }

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
    fn unpacked_path_names_the_extraction_tree_of_the_same_bucket() {
        assert_eq!(
            unpacked_path("slc/rustslc/exaudf/exaudfclient"),
            "/exa/data/bucketfs/bfsdefault/.dest/default/slc/rustslc/exaudf/exaudfclient"
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
