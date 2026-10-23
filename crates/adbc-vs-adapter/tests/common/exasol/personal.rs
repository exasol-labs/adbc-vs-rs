//! The Exasol Personal backend: the local deployment
//! `make stack-up-exasol EXASOL_TARGET=personal` installs.
//!
//! The launcher describes the deployment, so SQL reaches it at the address
//! `exasol info --json` reports, as `sys` with the password from the
//! deployment's `secrets.json`. A local deployment publishes no BucketFS HTTP
//! endpoint, so BucketFS is the plain directory the deployment mounts into the
//! database, and every object a UDF opens is a file written there. The
//! database serves a new bucket directory to UDFs only once it has reconciled
//! it into its `bucketfs.conf`, so every readiness check reads that file too.

use std::fs;
use std::io::ErrorKind;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use serde_json::Value;
use tempfile::NamedTempFile;

use super::super::run;
use super::{Exasol, BUCKET, BUCKETFS_SERVICE};

/// The deployment the Makefile installs under the same name, as
/// `EXASOL_PERSONAL_DEPLOYMENT`, so a developer's own default deployment is
/// never the one a run writes to.
pub const DEPLOYMENT: &str = "adbc-vs";

/// Where a deployment directory keeps the tree the database mounts at `/exa`.
const RUNTIME_DIRECTORY: &str = "local/runtime/exa";

/// The runtime's BucketFS tree, which a UDF reads under `/buckets`.
const BUCKETFS_DIRECTORY: &str = "bucketfs";

/// The runtime's list of the buckets the database serves to a UDF. The
/// database adds a bucket to it only some time after its directory appears.
const BUCKETFS_CONFIG: &str = "bucketfs.conf";

/// The file in the deployment directory holding the `sys` password.
const SECRETS_FILE: &str = "secrets.json";

/// The state `exasol info --json` reports for a deployment that is up.
const RUNNING: &str = "running";

/// The mode every bucket object gets: the database reads the bucket as a user
/// other than the one that wrote it.
const OBJECT_MODE: u32 = 0o644;

/// The deployment's runtime directory, which holds the bucket every BucketFS
/// method reads and writes and the configuration that says whether a UDF
/// sees it.
pub(super) struct Backend {
    runtime: PathBuf,
}

/// Binds to the deployment the launcher reports under [`DEPLOYMENT`].
///
/// Panics when that deployment is not running, so a test can never quietly
/// pass against no database.
pub(super) fn discover() -> Exasol {
    let deployment = parse_deployment(&launcher(
        &format!("describing the Exasol Personal deployment `{DEPLOYMENT}`"),
        &["info", "--json"],
    ));
    let secrets_path = deployment.directory.join(SECRETS_FILE);
    let secrets = fs::read_to_string(&secrets_path).unwrap_or_else(|error| {
        panic!(
            "reading the sys password from {}: {error}",
            secrets_path.display()
        )
    });
    let password = password_from_secrets(&secrets, &secrets_path);

    Exasol {
        dsn: dsn(&deployment.user, &password, &deployment.sql_endpoint),
        sql_endpoint: deployment.sql_endpoint,
        backend: Backend {
            runtime: deployment.directory.join(RUNTIME_DIRECTORY),
        },
    }
}

impl Exasol {
    /// Writes a local file into the bucket at `bucket_path`.
    ///
    /// The bucket is a plain directory, so an archive is stored as written,
    /// never extracted. The copy is renamed into place, so a UDF sees either
    /// no object or the whole one, and it is world-readable, because the
    /// database reads it as a user other than the file's owner.
    pub fn put_file(&self, local: &Path, bucket_path: &str) {
        let destination = self.backend.object_path(bucket_path);
        let directory = destination
            .parent()
            .unwrap_or_else(|| panic!("BucketFS path {bucket_path} names no file"));
        let description = format!("writing {} to BucketFS at {bucket_path}", local.display());

        fs::create_dir_all(directory)
            .unwrap_or_else(|error| panic!("{description}: creating its directory: {error}"));
        let staged = NamedTempFile::new_in(directory)
            .unwrap_or_else(|error| panic!("{description}: staging the copy: {error}"));
        fs::copy(local, staged.path())
            .unwrap_or_else(|error| panic!("{description}: copying: {error}"));
        fs::set_permissions(staged.path(), fs::Permissions::from_mode(OBJECT_MODE))
            .unwrap_or_else(|error| panic!("{description}: making it readable: {error}"));
        staged
            .persist(&destination)
            .unwrap_or_else(|error| panic!("{description}: moving it into place: {error}"));
    }

    /// Copies one bucket object to another bucket path, returning once `to`
    /// is readable, as on Docker Exasol.
    pub fn copy_bucket_object(&self, from: &str, to: &str) {
        self.put_library(&self.backend.object_path(from), to);
    }

    /// Deletes the file behind a bucket object.
    pub(super) fn remove_bucket_object(&self, bucket_path: &str) {
        fs::remove_file(self.backend.object_path(bucket_path))
            .unwrap_or_else(|error| panic!("deleting BucketFS object {bucket_path}: {error}"));
    }

    /// The size a UDF would see for `bucket_path`, or `None` when nothing is
    /// there, including while the database has not registered the bucket.
    pub fn bucket_file_size(&self, bucket_path: &str) -> Option<u64> {
        if !self.backend.serves_bucket() {
            return None;
        }
        fs::metadata(self.backend.object_path(bucket_path))
            .ok()
            .filter(fs::Metadata::is_file)
            .map(|metadata| metadata.len())
    }

    /// Whether a UDF would find anything at `bucket_path`, which it does not
    /// while the database has not registered the bucket.
    pub fn bucket_path_exists(&self, bucket_path: &str) -> bool {
        self.backend.serves_bucket() && self.backend.object_path(bucket_path).exists()
    }

    /// Every file in the bucket, as sorted bucket-relative paths.
    pub fn bucket_entries(&self) -> Vec<String> {
        let root = self.backend.bucket_root();
        let mut entries = files_below(&root, &root);
        entries.sort();
        entries
    }
}

impl Backend {
    fn bucket_root(&self) -> PathBuf {
        self.runtime
            .join(BUCKETFS_DIRECTORY)
            .join(BUCKETFS_SERVICE)
            .join(BUCKET)
    }

    fn object_path(&self, bucket_path: &str) -> PathBuf {
        self.bucket_root().join(bucket_path)
    }

    /// Whether the database lists the bucket a UDF opens, which it does only
    /// once it has reconciled the bucket's directory into its configuration.
    fn serves_bucket(&self) -> bool {
        let path = self.runtime.join(BUCKETFS_CONFIG);
        match fs::read_to_string(&path) {
            Ok(config) => registers_bucket(&config),
            Err(error) if error.kind() == ErrorKind::NotFound => false,
            Err(error) => panic!("reading {}: {error}", path.display()),
        }
    }
}

/// Whether a `bucketfs.conf` lists the bucket under the mount a UDF opens it
/// at, `/buckets/<service>/<bucket>`.
fn registers_bucket(config: &str) -> bool {
    let mount = format!("/buckets/{BUCKETFS_SERVICE}/{BUCKET}");
    config
        .lines()
        .any(|line| line.split_whitespace().any(|field| field == mount))
}

/// Runs one `exasol` launcher subcommand against the harness's deployment
/// and returns its standard output.
pub fn launcher(description: &str, args: &[&str]) -> String {
    let mut args = args.to_vec();
    args.extend(["--deployment", DEPLOYMENT]);
    run(description, "exasol", &args)
}

/// Every file at or below `directory`, as paths relative to `root`. A
/// directory that does not exist yet is a bucket nothing was written to.
fn files_below(root: &Path, directory: &Path) -> Vec<String> {
    let listing = match fs::read_dir(directory) {
        Ok(listing) => listing,
        Err(error) if error.kind() == ErrorKind::NotFound => return Vec::new(),
        Err(error) => panic!(
            "listing BucketFS directory {}: {error}",
            directory.display()
        ),
    };
    listing
        .flat_map(|entry| {
            let path = entry
                .unwrap_or_else(|error| {
                    panic!(
                        "listing BucketFS directory {}: {error}",
                        directory.display()
                    )
                })
                .path();
            if path.is_dir() {
                files_below(root, &path)
            } else {
                vec![bucket_relative(root, &path)]
            }
        })
        .collect()
}

fn bucket_relative(root: &Path, path: &Path) -> String {
    let relative = path.strip_prefix(root).unwrap_or_else(|_| {
        panic!(
            "{} lies outside the bucket {}",
            path.display(),
            root.display()
        )
    });
    relative
        .to_str()
        .unwrap_or_else(|| panic!("BucketFS object {} is not named in UTF-8", path.display()))
        .to_owned()
}

/// What `exasol info --json` reports about a running deployment.
#[derive(Debug, PartialEq)]
struct Deployment {
    directory: PathBuf,
    sql_endpoint: String,
    user: String,
}

/// Reads the running deployment out of `exasol info --json`, which carries
/// no credential, so the document itself may appear in a failure.
fn parse_deployment(info: &str) -> Deployment {
    let report: Value = serde_json::from_str(info).unwrap_or_else(|error| {
        panic!("parsing `exasol info --json` for deployment `{DEPLOYMENT}`: {error}\n{info}")
    });
    let state = report["deploymentState"].as_str().unwrap_or("unreported");
    assert!(
        state == RUNNING,
        "Exasol Personal deployment `{DEPLOYMENT}` is `{state}`, not `{RUNNING}`; start it with \
         `make stack-up-exasol EXASOL_TARGET=personal`"
    );
    let field = |pointer: &str| {
        report.pointer(pointer).unwrap_or_else(|| {
            panic!(
                "`exasol info --json` for deployment `{DEPLOYMENT}` reports no {pointer}:\n{info}"
            )
        })
    };
    let text = |pointer: &str| {
        field(pointer)
            .as_str()
            .unwrap_or_else(|| {
                panic!("{pointer} of deployment `{DEPLOYMENT}` is not text:\n{info}")
            })
            .to_owned()
    };
    let port = field("/connection/dbPort").as_u64().unwrap_or_else(|| {
        panic!("/connection/dbPort of deployment `{DEPLOYMENT}` is not a port:\n{info}")
    });

    Deployment {
        directory: PathBuf::from(text("/deploymentDir")),
        sql_endpoint: format!("{}:{port}", text("/connection/host")),
        user: text("/connection/username"),
    }
}

/// The `sys` password out of a deployment's `secrets.json`. Every failure
/// names the file and nothing read from it.
fn password_from_secrets(secrets: &str, path: &Path) -> String {
    let secrets: Value = serde_json::from_str(secrets).unwrap_or_else(|error| {
        panic!(
            "{} is not the JSON document the launcher writes: {error}",
            path.display()
        )
    });
    secrets["dbPassword"]
        .as_str()
        .filter(|password| !password.is_empty())
        .unwrap_or_else(|| panic!("{} records no `dbPassword`", path.display()))
        .to_owned()
}

/// The connection string for the deployment. It disables certificate
/// validation, because the deployment serves a self-signed certificate.
fn dsn(user: &str, password: &str, sql_endpoint: &str) -> String {
    format!("exasol://{user}:{password}@{sql_endpoint}?validateservercertificate=0")
}

#[cfg(test)]
mod tests {
    use std::panic;

    use tempfile::TempDir;

    use super::*;

    /// The shape `exasol info --json` printed for a running local deployment
    /// of launcher 2.3.0, with the fields this backend reads.
    const RUNNING_INFO: &str = r#"{
  "deploymentDir": "/home/user/.exasol/personal/deployments/adbc-vs",
  "deploymentId": "86532c71",
  "deploymentState": "running",
  "connection": {
    "backend": "local",
    "host": "127.0.0.1",
    "displayHost": "127.0.0.1",
    "dbPort": 8564,
    "username": "sys",
    "insecureSkipCertValidation": true,
    "publicIp": "127.0.0.1"
  }
}"#;

    fn panic_message(failing: impl FnOnce() + panic::UnwindSafe) -> String {
        let payload = panic::catch_unwind(failing).expect_err("the call must fail the run");
        payload
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| payload.downcast_ref::<&str>().map(|text| text.to_string()))
            .expect("the failure carries a message")
    }

    #[test]
    fn a_running_deployment_names_its_directory_sql_endpoint_and_user() {
        assert_eq!(
            parse_deployment(RUNNING_INFO),
            Deployment {
                directory: PathBuf::from("/home/user/.exasol/personal/deployments/adbc-vs"),
                sql_endpoint: "127.0.0.1:8564".to_string(),
                user: "sys".to_string(),
            }
        );
    }

    /// The launcher reports a deployment it never installed as
    /// `not_initialized` and with no connection at all.
    #[test]
    fn a_deployment_that_is_not_running_fails_naming_how_to_start_it() {
        let message = panic_message(|| {
            parse_deployment(
                r#"{"deploymentDir": "/somewhere/adbc-vs", "deploymentState": "not_initialized"}"#,
            );
        });

        assert!(
            message.contains("not_initialized")
                && message.contains("make stack-up-exasol EXASOL_TARGET=personal"),
            "the failure must name the state and the command that starts the deployment: \
             {message}"
        );
    }

    #[test]
    fn a_running_deployment_without_a_port_fails_naming_the_field() {
        let message = panic_message(|| {
            parse_deployment(&RUNNING_INFO.replace("\"dbPort\": 8564,", ""));
        });

        assert!(message.contains("/connection/dbPort"), "{message}");
    }

    #[test]
    fn the_secrets_file_yields_the_db_password() {
        assert_eq!(
            password_from_secrets(
                r#"{"dbPassword": "Abc123xyz"}"#,
                Path::new("/d/secrets.json")
            ),
            "Abc123xyz"
        );
    }

    #[test]
    fn an_unreadable_secrets_file_fails_without_echoing_its_contents() {
        let message = panic_message(|| {
            password_from_secrets(
                r#"{"dbPassword": "Abc123xyz""#,
                Path::new("/d/secrets.json"),
            );
        });

        assert!(message.contains("/d/secrets.json"), "{message}");
        assert!(
            !message.contains("Abc123xyz"),
            "a credential must never reach a failure message: {message}"
        );
    }

    #[test]
    fn a_secrets_file_without_a_db_password_fails_naming_the_file() {
        let message = panic_message(|| {
            password_from_secrets(r#"{"adminUiPassword": "x"}"#, Path::new("/d/secrets.json"));
        });

        assert!(
            message.contains("/d/secrets.json") && message.contains("dbPassword"),
            "{message}"
        );
    }

    /// The deployment serves a self-signed certificate, as `exasol info`
    /// reports with `insecureSkipCertValidation`.
    #[test]
    fn the_dsn_addresses_the_reported_endpoint_without_validating_its_certificate() {
        assert_eq!(
            dsn("sys", "Abc123xyz", "127.0.0.1:8564"),
            "exasol://sys:Abc123xyz@127.0.0.1:8564?validateservercertificate=0"
        );
    }

    /// The `bucketfs.conf` Exasol Personal 2.3.0 reconciled once a file was
    /// written below `bucketfs/bfsdefault/default`.
    const REGISTERED_CONFIG: &str = "/exa/slc __builtin__ slc /exa/slc dDE= P\n\
        /exa/bucketfs/bfsdefault/default bfsdefault default /buckets/bfsdefault/default - P\n";

    /// The `bucketfs.conf` of a deployment no bucket file was written to yet.
    const UNREGISTERED_CONFIG: &str = "/exa/slc __builtin__ slc /exa/slc dDE= P\n";

    /// An `Exasol` over a fresh temporary runtime directory, which nothing
    /// but the test writes to, whose configuration is `config`.
    fn exasol_over_a_runtime_configured(config: &str) -> (Exasol, TempDir) {
        let directory = TempDir::new().expect("a temporary directory is creatable");
        fs::write(directory.path().join("bucketfs.conf"), config)
            .expect("the BucketFS configuration is writable");
        let exasol = Exasol {
            dsn: String::new(),
            sql_endpoint: String::new(),
            backend: Backend {
                runtime: directory.path().to_path_buf(),
            },
        };
        (exasol, directory)
    }

    /// An `Exasol` whose default bucket the database has registered.
    fn exasol_over_a_temporary_bucket() -> (Exasol, TempDir) {
        exasol_over_a_runtime_configured(REGISTERED_CONFIG)
    }

    #[test]
    fn the_reconciled_configuration_registers_the_default_bucket() {
        assert!(registers_bucket(REGISTERED_CONFIG));
        assert!(!registers_bucket(UNREGISTERED_CONFIG));
        assert!(
            !registers_bucket(&REGISTERED_CONFIG.replace(
                "/buckets/bfsdefault/default ",
                "/buckets/bfsdefault/defaults "
            )),
            "only the default bucket's own mount registers it"
        );
    }

    /// The database reconciles a new bucket directory into `bucketfs.conf`
    /// a fraction of a second after the first write, and a UDF started before
    /// that finds no file at all.
    #[test]
    fn an_object_in_a_bucket_the_database_has_not_registered_is_not_readable_yet() {
        let (exasol, directory) = exasol_over_a_runtime_configured(UNREGISTERED_CONFIG);
        let local = local_file(&directory, "library.so", b"twelve bytes");

        exasol.put_file(&local, "udf/library.so");

        assert!(!exasol.bucket_path_exists("udf/library.so"));
        assert_eq!(exasol.bucket_file_size("udf/library.so"), None);

        fs::write(directory.path().join("bucketfs.conf"), REGISTERED_CONFIG)
            .expect("the BucketFS configuration is writable");

        assert!(exasol.bucket_path_exists("udf/library.so"));
        assert_eq!(exasol.bucket_file_size("udf/library.so"), Some(12));
    }

    fn local_file(directory: &TempDir, name: &str, contents: &[u8]) -> PathBuf {
        let path = directory.path().join(name);
        fs::write(&path, contents).expect("the local file is writable");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
            .expect("the local file's mode is settable");
        path
    }

    #[test]
    fn a_written_object_is_a_whole_world_readable_copy_at_its_bucket_path() {
        let (exasol, directory) = exasol_over_a_temporary_bucket();
        let local = local_file(&directory, "library.so", b"twelve bytes");

        exasol.put_file(&local, "adbc-drivers/mysql/supplied/library.so");

        let written = directory
            .path()
            .join("bucketfs/bfsdefault/default/adbc-drivers/mysql/supplied/library.so");
        assert_eq!(
            fs::read(&written).expect("the object exists"),
            b"twelve bytes"
        );
        assert_eq!(
            fs::metadata(&written)
                .expect("the object exists")
                .permissions()
                .mode()
                & 0o777,
            0o644,
            "a UDF reads the object as another user, so it must be world-readable"
        );
        assert_eq!(
            exasol.bucket_entries(),
            vec!["adbc-drivers/mysql/supplied/library.so".to_string()],
            "the rename leaves no staging file behind"
        );
    }

    #[test]
    fn a_written_object_reports_its_size_and_presence() {
        let (exasol, directory) = exasol_over_a_temporary_bucket();
        let local = local_file(&directory, "library.so", b"twelve bytes");

        assert_eq!(exasol.bucket_file_size("udf/library.so"), None);
        assert!(!exasol.bucket_path_exists("udf/library.so"));

        exasol.put_file(&local, "udf/library.so");

        assert_eq!(exasol.bucket_file_size("udf/library.so"), Some(12));
        assert!(exasol.bucket_path_exists("udf/library.so"));
        assert!(
            exasol.bucket_path_exists("udf"),
            "a directory a UDF can open is a path that exists"
        );
        assert_eq!(
            exasol.bucket_file_size("udf"),
            None,
            "a directory is not an object of any size"
        );
    }

    #[test]
    fn a_bucket_never_written_to_lists_no_entry() {
        let (exasol, _directory) = exasol_over_a_temporary_bucket();

        assert!(exasol.bucket_entries().is_empty());
    }

    #[test]
    fn bucket_entries_list_every_nested_file_in_path_order() {
        let (exasol, directory) = exasol_over_a_temporary_bucket();
        let local = local_file(&directory, "object", b"x");

        for bucket_path in ["udf/b.so", "adbc-drivers/trino/supplied/a.so", "udf/a.so"] {
            exasol.put_file(&local, bucket_path);
        }

        assert_eq!(
            exasol.bucket_entries(),
            vec![
                "adbc-drivers/trino/supplied/a.so".to_string(),
                "udf/a.so".to_string(),
                "udf/b.so".to_string(),
            ]
        );
    }

    #[test]
    fn a_copied_object_holds_the_bytes_of_its_source_object() {
        let (exasol, directory) = exasol_over_a_temporary_bucket();
        let local = local_file(&directory, "library.so", b"driver bytes");
        exasol.put_file(&local, "adbc-drivers/mysql/0.6.1/library.so");

        exasol.copy_bucket_object(
            "adbc-drivers/mysql/0.6.1/library.so",
            "adbc-drivers/mysql/supplied/library.so",
        );

        assert_eq!(
            exasol.bucket_file_size("adbc-drivers/mysql/supplied/library.so"),
            Some(12)
        );
        assert!(exasol.bucket_path_exists("adbc-drivers/mysql/0.6.1/library.so"));
    }

    #[test]
    fn a_removed_object_is_gone() {
        let (exasol, directory) = exasol_over_a_temporary_bucket();
        let local = local_file(&directory, "library.so", b"x");
        exasol.put_file(&local, "udf/library.so");

        exasol.delete_file("udf/library.so");

        assert!(!exasol.bucket_path_exists("udf/library.so"));
    }

    #[test]
    fn removing_an_object_that_is_not_there_fails_naming_it() {
        let (exasol, _directory) = exasol_over_a_temporary_bucket();

        let message = panic_message(panic::AssertUnwindSafe(|| {
            exasol.delete_file("udf/missing.so");
        }));

        assert!(message.contains("udf/missing.so"), "{message}");
    }
}
