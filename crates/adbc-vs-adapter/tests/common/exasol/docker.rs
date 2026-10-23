//! The Docker backend: the `exasol/docker-db` container of the Compose stack.
//!
//! SQL reaches it on a published loopback port, BucketFS through the bucket's
//! HTTPS endpoint, and the tree a UDF reads through `docker exec`, so this
//! module alone knows the container's ports, credentials, and BucketFS layout.

use std::path::Path;

use tempfile::NamedTempFile;

use super::super::readiness::{self, BUCKETFS_DEADLINE};
use super::super::{run, try_run};
use super::{workspace_root, Exasol, BUCKET, BUCKETFS_SERVICE};

/// Host and ports come from `docker-compose.yml`, which binds them to fixed
/// loopback ports; the `sys` password is the `exasol/docker-db` default, which
/// the Compose file does not override.
const HOST: &str = "localhost";
const SQL_PORT: u16 = 8563;
const BUCKETFS_PORT: u16 = 2581;
const USER: &str = "sys";
const PASSWORD: &str = "exasol";

/// The write user baked into `exasol/docker-db`.
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

/// The container the Compose stack runs Exasol in, and what writing to its
/// bucket takes.
pub(super) struct Backend {
    container: String,
    node_address: String,
    bucketfs_write_password: String,
}

/// Binds to the Exasol of the Compose stack.
///
/// Panics when the stack is down, so a test can never quietly pass against
/// no database.
pub(super) fn discover() -> Exasol {
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

    Exasol {
        dsn: dsn(),
        sql_endpoint: sql_endpoint(),
        backend: Backend {
            node_address: node_address(&container),
            container,
            bucketfs_write_password,
        },
    }
}

impl Exasol {
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
    ///
    /// A locked path is waited out rather than reported, on the rule
    /// [`accepted_upload`] owns: writing one path is a transient refusal away
    /// from succeeding, and absorbing that here is what keeps every caller
    /// from having to sequence its own writes around another test's delete.
    pub fn put_file(&self, local: &Path, bucket_path: &str) {
        assert!(
            local.is_file(),
            "uploading to BucketFS at {bucket_path}: {} is not a file",
            local.display()
        );

        let source = local.to_string_lossy().into_owned();
        let url = format!("https://{HOST}:{BUCKETFS_PORT}/{BUCKET}/{bucket_path}");
        let description = format!("uploading {source} to BucketFS at {bucket_path}");

        readiness::wait_until(
            &format!("a writable BucketFS path at {bucket_path}"),
            BUCKETFS_DEADLINE,
            || {
                let status = run(
                    &description,
                    "curl",
                    &[
                        "--silent",
                        "--show-error",
                        "--insecure",
                        "--max-time",
                        "600",
                        "--user",
                        &self.credentials(),
                        "--upload-file",
                        &source,
                        "--output",
                        "/dev/null",
                        "--write-out",
                        "%{http_code}",
                        &url,
                    ],
                );
                accepted_upload(&status, &description)
            },
        );
    }

    /// Copies an already-installed driver library from `from` to `to`, so a
    /// test can prove self-install does not short-circuit off a pre-placed
    /// library sitting elsewhere in BucketFS.
    ///
    /// Downloads through a temporary file rather than into memory: the pinned
    /// MySQL driver library measures about 44 MiB, and a call holding that as
    /// a `Vec<u8>` for its whole lifetime is worse than one writing it to
    /// disk. Waiting for `to` to become readable is [`put_library`]'s job, not
    /// a second call this one might omit.
    ///
    /// [`put_library`]: Self::put_library
    pub fn copy_bucket_object(&self, from: &str, to: &str) {
        let temp = NamedTempFile::new().unwrap_or_else(|error| {
            panic!("creating a temporary file to copy BucketFS object {from} into: {error}")
        });
        let temp_path = temp.path().to_string_lossy().into_owned();
        let url = format!("https://{HOST}:{BUCKETFS_PORT}/{BUCKET}/{from}");

        run(
            &format!("downloading BucketFS object {from} to copy it to {to}"),
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
                "--output",
                &temp_path,
                &url,
            ],
        );

        self.put_library(temp.path(), to);
    }

    /// Asks BucketFS to delete a bucket object, which it acknowledges before
    /// the object leaves the tree a UDF reads.
    pub(super) fn remove_bucket_object(&self, bucket_path: &str) {
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
    }

    /// The size a UDF would see for `bucket_path`, or `None` when nothing is
    /// there.
    pub fn bucket_file_size(&self, bucket_path: &str) -> Option<u64> {
        let size = try_run(
            &format!("measuring BucketFS object {bucket_path}"),
            "docker",
            &[
                "exec",
                &self.backend.container,
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
                &self.backend.container,
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
        format!(
            "https://{}:{BUCKETFS_PORT}/{BUCKET}",
            self.backend.node_address
        )
    }

    /// The bucket's write password, generated per container.
    pub fn bucketfs_write_password(&self) -> &str {
        &self.backend.bucketfs_write_password
    }

    fn credentials(&self) -> String {
        format!(
            "{BUCKETFS_WRITE_USER}:{}",
            self.backend.bucketfs_write_password
        )
    }
}

/// The connection string for the stack's Exasol.
///
/// It always disables certificate validation: the container serves a
/// self-signed certificate, so a connection that validates cannot reach it.
fn dsn() -> String {
    format!("exasol://{USER}:{PASSWORD}@{HOST}:{SQL_PORT}?validateservercertificate=0")
}

/// The `host:port` the stack publishes Exasol's SQL port at.
fn sql_endpoint() -> String {
    format!("{HOST}:{SQL_PORT}")
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

/// The status BucketFS answers an upload with while the path it names is
/// still locked, which is the one status worth another attempt.
const BUCKETFS_LOCKED: &str = "423";

/// Whether BucketFS accepted an upload, given the HTTP status it answered.
///
/// BucketFS keeps a path locked for several seconds after a delete — measured
/// at about eight on `exasol/docker-db:2025.2.1`, well past the point the
/// object leaves the tree [`Exasol::delete_file`] waits on — so a second
/// writer of one path fails for a condition that clears itself. Answering
/// `false` for that one status is what turns it into another attempt.
///
/// Every other refusal is BucketFS's own answer and will not change, so it
/// panics here rather than being retried until the deadline reports it as a
/// timeout: a wrong password must name itself, not the path it was waiting
/// for.
fn accepted_upload(status: &str, description: &str) -> bool {
    match status.trim() {
        BUCKETFS_LOCKED => false,
        accepted if accepted.starts_with('2') => true,
        refused => panic!("{description}: BucketFS answered HTTP {refused}"),
    }
}

#[cfg(test)]
mod tests {
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
    fn unpacked_path_names_the_extraction_tree_of_the_same_bucket() {
        assert_eq!(
            unpacked_path("slc/rustslc/exaudf/exaudfclient"),
            "/exa/data/bucketfs/bfsdefault/.dest/default/slc/rustslc/exaudf/exaudfclient"
        );
    }

    #[test]
    fn a_locked_bucket_path_is_worth_another_upload_attempt() {
        assert!(!accepted_upload(
            "423",
            "uploading to BucketFS at some/path"
        ));
    }

    #[test]
    fn an_accepted_upload_needs_no_further_attempt() {
        assert!(accepted_upload(
            "200\n",
            "uploading to BucketFS at some/path"
        ));
    }

    /// A refusal BucketFS will keep repeating must surface as itself rather
    /// than as a timeout, which would name the deadline and hide the status.
    #[test]
    #[should_panic(expected = "BucketFS answered HTTP 401")]
    fn a_refused_upload_fails_the_run_rather_than_waiting_out_the_deadline() {
        accepted_upload("401", "uploading to BucketFS at some/path");
    }
}
