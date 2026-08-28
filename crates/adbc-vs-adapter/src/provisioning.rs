//! Makes the pinned ADBC MySQL driver readable at a BucketFS path.
//!
//! [`ensure_driver`] is the whole interface. A caller hands it the registry to
//! fetch from and the bucket to install into, and learns one thing back: the
//! path the scan UDF loads the driver from. Whether that driver was already
//! there, or was downloaded, verified against a compiled-in digest, unpacked,
//! uploaded and waited for, is this module's business alone.
//!
//! Three facts about the driver are compiled in rather than discovered: the
//! pinned version, the SHA-256 of its `linux_amd64` package, and the file name
//! of the shared library inside it. They are read from
//! [`adbc_vs_scan::driver`], which owns the identity of the one driver this
//! release loads, so the integration tier that fetches the same package cannot
//! pin a different one. Because the file name is known before any network
//! call, the BucketFS path is computable up front — which is what lets an
//! already-installed driver short-circuit the whole fetch, and what lets an
//! operator with no outbound egress place the library there by hand.
//!
//! Two HTTP clients, never one. The registry client reaches a public internet
//! host by name and verifies its certificate against a bundled root store,
//! with no switch anywhere that turns that off. The BucketFS client reaches an
//! in-cluster endpoint that serves the cluster's own self-signed certificate,
//! and accepts it. Sharing one client would extend the second policy over the
//! first.
//!
//! Concurrent provisioning converges rather than conflicting: every writer
//! uploads the same verified bytes of the same pinned version to the same
//! path.

use std::fmt;
use std::fmt::Write as _;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use flate2::read::GzDecoder;
use reqwest::blocking::Client;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tempfile::TempDir;

use adbc_vs_scan::driver::{PACKAGE_SHA256, PLATFORM, REGISTRY_BASE_URL, REGISTRY_DRIVER_PATH};

use crate::registry_index;

/// The pinned driver's identity, re-exported from the module that owns it so a
/// caller reading `provisioning`'s interface never has to look elsewhere for
/// what it installs.
pub use adbc_vs_scan::driver::{DRIVER_LIBRARY, DRIVER_VERSION};

const REGISTRY_INDEX_FILE: &str = "index.yaml";
const MANIFEST_MEMBER: &str = "MANIFEST";

/// The bucket-relative directory every adbc-vs driver is installed under.
const DRIVER_DIRECTORY: &str = "adbc-drivers";

/// Response size ceilings. A registry that answers with an endless body would
/// otherwise exhaust the UDF's memory instead of failing.
const MAX_INDEX_BYTES: u64 = 4 * 1024 * 1024;
const MAX_PACKAGE_BYTES: u64 = 256 * 1024 * 1024;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const TRANSFER_TIMEOUT: Duration = Duration::from_secs(600);

/// How long a written driver has to become readable on the adapter's own
/// mount, and how often that is checked.
const VISIBILITY_DEADLINE: Duration = Duration::from_secs(120);
const VISIBILITY_POLL_INTERVAL: Duration = Duration::from_millis(500);

/// The status BucketFS answers while it is still finishing an earlier write or
/// delete of the same object.
///
/// It is a "not yet", not a refusal: the object is unwritable for as long as
/// that work takes — around ten seconds for a driver-sized file — and writable
/// afterwards with no further action. Surfacing it would fail a statement that
/// merely arrived during another one's clean-up, and would make concurrent
/// provisioning of the same pinned driver conflict rather than converge.
const BUCKETFS_LOCKED: u16 = 423;

/// How long a locked BucketFS object gets to become writable, and how often
/// that is retried.
const LOCK_DEADLINE: Duration = Duration::from_secs(120);
const LOCK_POLL_INTERVAL: Duration = Duration::from_millis(500);

/// The write credentials of a BucketFS bucket.
///
/// Renders without its password under every formatting trait it implements,
/// so no accidental diagnostic can put the secret into a message, a log line,
/// or an audit table.
#[derive(Clone)]
pub struct Credentials {
    user: String,
    password: String,
}

impl Credentials {
    /// Takes the user and password of a resolved `BUCKETFS_CONNECTION`
    /// object. Provisioning never resolves the connection itself: it is handed
    /// the pieces, so nothing here depends on the UDF context.
    pub fn new(user: &str, password: &str) -> Self {
        Self {
            user: user.to_owned(),
            password: password.to_owned(),
        }
    }
}

impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Credentials {{ user: {:?}, password: <redacted> }}",
            self.user
        )
    }
}

/// The BucketFS bucket the driver is installed into, in both the forms
/// provisioning needs it: the HTTPS endpoint writes go through, and the
/// read-only mount a UDF reads back from.
///
/// The two must address the same bucket. A mismatch is not silent — the
/// upload succeeds and the bounded visibility wait then fails naming the path
/// it watched.
#[derive(Debug)]
pub struct Bucket {
    write_url: String,
    mount: PathBuf,
    credentials: Credentials,
    visibility_deadline: Duration,
}

impl Bucket {
    /// Binds to the bucket a `BUCKETFS_CONNECTION` object addresses.
    ///
    /// `write_url` is the bucket's HTTPS endpoint (`https://host:2581/default`)
    /// and `mount` the directory the same bucket appears at inside a UDF
    /// (`/buckets/bfsdefault/default`).
    pub fn new(write_url: &str, mount: impl Into<PathBuf>, credentials: Credentials) -> Self {
        Self {
            write_url: write_url.trim_end_matches('/').to_owned(),
            mount: mount.into(),
            credentials,
            visibility_deadline: VISIBILITY_DEADLINE,
        }
    }

    /// Shortens how long an upload has to become readable on the mount. The
    /// default is generous enough for a 46 MB replication; a test that means
    /// to observe the timeout sets its own.
    pub fn visible_within(mut self, deadline: Duration) -> Self {
        self.visibility_deadline = deadline;
        self
    }

    /// Where the driver is, or will be, readable on this mount.
    pub fn driver_path(&self) -> PathBuf {
        driver_install_path(&self.mount)
    }

    fn driver_url(&self) -> String {
        format!("{}/{DRIVER_DIRECTORY}/{DRIVER_LIBRARY}", self.write_url)
    }
}

/// Where driver packages come from, and which package this release pins.
#[derive(Debug)]
pub struct Registry {
    base_url: String,
    version: String,
    sha256: String,
}

impl Registry {
    /// The public `dbc` registry and the one package release 0.1 installs.
    pub fn pinned() -> Self {
        Self {
            base_url: REGISTRY_BASE_URL.to_owned(),
            version: DRIVER_VERSION.to_owned(),
            sha256: PACKAGE_SHA256.to_owned(),
        }
    }

    /// Binds to another host serving the same registry layout, expecting a
    /// package of the given version and digest.
    ///
    /// Adapter code always provisions from [`Registry::pinned`]; this exists
    /// so the fetch, verify, unpack and upload path can be driven end to end
    /// against a stub registry rather than the public internet.
    pub fn at(base_url: &str, version: &str, sha256: &str) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_owned(),
            version: version.to_owned(),
            sha256: sha256.to_owned(),
        }
    }

    fn index_url(&self) -> String {
        registry_index::resolve_url(&self.base_url, REGISTRY_INDEX_FILE)
    }
}

/// Everything that stops the driver from being readable at its BucketFS path.
///
/// No variant carries a credential: BucketFS messages are built from the
/// endpoint and the HTTP status, and every borrowed text is redacted against
/// the password before it is stored.
#[derive(Debug)]
pub enum ProvisioningError {
    RegistryUnreachable {
        url: String,
        install_path: PathBuf,
        cause: String,
    },
    RegistryRejected {
        url: String,
        status: u16,
        install_path: PathBuf,
    },
    ResponseTooLarge {
        url: String,
        limit: u64,
    },
    IndexUnreadable {
        url: String,
        cause: String,
    },
    DriverNotPublished {
        driver: String,
        url: String,
    },
    VersionNotPublished {
        driver: String,
        version: String,
        url: String,
    },
    PlatformNotPublished {
        platform: String,
        version: String,
    },
    ChecksumMismatch {
        url: String,
        expected: String,
        computed: String,
    },
    PackageUnreadable {
        url: String,
        cause: String,
    },
    ManifestMissing {
        url: String,
    },
    ManifestUnreadable {
        url: String,
        cause: String,
    },
    LibraryMissing {
        url: String,
        filename: String,
    },
    LibraryFilenameDiverged {
        declared: String,
        expected: String,
    },
    BucketFsUnreachable {
        url: String,
        cause: String,
    },
    BucketFsRejected {
        url: String,
        status: u16,
    },
    DriverNotVisible {
        path: PathBuf,
        deadline: Duration,
    },
    LocalFailure {
        attempt: String,
        cause: String,
    },
}

impl fmt::Display for ProvisioningError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RegistryUnreachable {
                url,
                install_path,
                cause,
            } => write!(
                f,
                "could not reach the driver registry at {url}: {cause}. Without outbound \
                 access, place {DRIVER_LIBRARY} at {} by hand before creating the virtual \
                 schema",
                install_path.display()
            ),
            Self::RegistryRejected {
                url,
                status,
                install_path,
            } => write!(
                f,
                "the driver registry at {url} answered HTTP {status}. Without registry \
                 access, place {DRIVER_LIBRARY} at {} by hand before creating the virtual \
                 schema",
                install_path.display()
            ),
            Self::ResponseTooLarge { url, limit } => write!(
                f,
                "{url} answered with more than {limit} bytes, which no driver registry \
                 response is; the download was abandoned"
            ),
            Self::IndexUnreadable { url, cause } => {
                write!(
                    f,
                    "the driver registry index at {url} is unreadable: {cause}"
                )
            }
            Self::DriverNotPublished { driver, url } => write!(
                f,
                "the driver registry index at {url} publishes no driver at path `{driver}`"
            ),
            Self::VersionNotPublished {
                driver,
                version,
                url,
            } => write!(
                f,
                "the driver registry index at {url} publishes no version {version} of \
                 driver `{driver}`, which is the version adbc-vs pins"
            ),
            Self::PlatformNotPublished { platform, version } => write!(
                f,
                "version {version} of driver `{REGISTRY_DRIVER_PATH}` publishes no `{platform}` \
                 package, and `{platform}` is the only platform the script language \
                 container runs on"
            ),
            Self::ChecksumMismatch {
                url,
                expected,
                computed,
            } => write!(
                f,
                "the driver package downloaded from {url} does not match the digest \
                 compiled into adbc-vs: expected {expected}, computed {computed}; nothing \
                 was extracted or installed"
            ),
            Self::PackageUnreadable { url, cause } => write!(
                f,
                "the driver package downloaded from {url} is not a readable gzipped tar \
                 archive: {cause}"
            ),
            Self::ManifestMissing { url } => write!(
                f,
                "the driver package downloaded from {url} holds no {MANIFEST_MEMBER} \
                 naming its shared library"
            ),
            Self::ManifestUnreadable { url, cause } => write!(
                f,
                "the {MANIFEST_MEMBER} of the driver package downloaded from {url} does \
                 not declare `[Files] driver`: {cause}"
            ),
            Self::LibraryMissing { url, filename } => write!(
                f,
                "the {MANIFEST_MEMBER} of the driver package downloaded from {url} names \
                 `{filename}` as its shared library, but the archive holds no such file"
            ),
            Self::LibraryFilenameDiverged { declared, expected } => write!(
                f,
                "the driver package declares its shared library as `{declared}`, but \
                 adbc-vs is compiled against `{expected}`; installing it would leave the \
                 presence check looking for a file that is never written"
            ),
            Self::BucketFsUnreachable { url, cause } => {
                write!(
                    f,
                    "could not write the driver to BucketFS at {url}: {cause}"
                )
            }
            Self::BucketFsRejected { url, status } => write!(
                f,
                "BucketFS answered HTTP {status} to the driver upload at {url}"
            ),
            Self::DriverNotVisible { path, deadline } => write!(
                f,
                "the driver was written to BucketFS but did not become readable at {} \
                 within {deadline:?}",
                path.display()
            ),
            Self::LocalFailure { attempt, cause } => {
                write!(f, "provisioning the driver failed while {attempt}: {cause}")
            }
        }
    }
}

impl std::error::Error for ProvisioningError {}

/// Makes the pinned driver readable at its BucketFS path and returns that
/// path.
///
/// Short-circuits on a driver that is already installed, so a second virtual
/// schema costs one stat call rather than a 46 MB round trip, and so an
/// operator can pre-load the library on a cluster with no outbound access.
pub fn ensure_driver(registry: &Registry, bucket: &Bucket) -> Result<PathBuf, ProvisioningError> {
    let install_path = bucket.driver_path();
    if install_path.is_file() {
        return Ok(install_path);
    }

    let registry_client = registry_client()?;
    let index_url = registry.index_url();
    let index = download(&registry_client, &index_url, MAX_INDEX_BYTES)
        .map_err(|failure| provisioning_error(failure, &index_url, &install_path))?;
    let index = String::from_utf8(index).map_err(|error| ProvisioningError::IndexUnreadable {
        url: registry.index_url(),
        cause: error.to_string(),
    })?;

    let package_url = select_package_url(&index, registry)?;
    let driver = {
        let package = download(&registry_client, &package_url, MAX_PACKAGE_BYTES)
            .map_err(|failure| provisioning_error(failure, &package_url, &install_path))?;
        verify_digest(&package, &registry.sha256, &package_url)?;
        extract_driver(&package, &package_url)?
    };
    verify_library_filename(&driver.filename)?;

    let installed_length = driver
        .path
        .metadata()
        .map_err(|error| ProvisioningError::LocalFailure {
            attempt: format!("measuring the extracted {}", driver.filename),
            cause: error.to_string(),
        })?
        .len();

    upload(&bucketfs_client()?, bucket, &driver.path)?;
    await_visibility(&install_path, installed_length, bucket.visibility_deadline)?;

    Ok(install_path)
}

/// Where the driver is installed under a bucket's mount.
fn driver_install_path(mount: &Path) -> PathBuf {
    mount.join(DRIVER_DIRECTORY).join(DRIVER_LIBRARY)
}

/// Resolves the absolute URL of the pinned version's `linux_amd64` package
/// from a registry index document.
fn select_package_url(index: &str, registry: &Registry) -> Result<String, ProvisioningError> {
    let url = registry.index_url();
    registry_index::package_url(
        index,
        REGISTRY_DRIVER_PATH,
        &registry.version,
        PLATFORM,
        &registry.base_url,
    )
    .map_err(|error| match error {
        registry_index::LookupError::Unparseable(cause) => ProvisioningError::IndexUnreadable {
            url: url.clone(),
            cause,
        },
        registry_index::LookupError::NoDriver => ProvisioningError::DriverNotPublished {
            driver: REGISTRY_DRIVER_PATH.to_owned(),
            url: url.clone(),
        },
        registry_index::LookupError::NoVersion => ProvisioningError::VersionNotPublished {
            driver: REGISTRY_DRIVER_PATH.to_owned(),
            version: registry.version.clone(),
            url: url.clone(),
        },
        registry_index::LookupError::NoPlatform => ProvisioningError::PlatformNotPublished {
            platform: PLATFORM.to_owned(),
            version: registry.version.clone(),
        },
    })
}

/// The lower-case hexadecimal SHA-256 of `bytes`.
fn digest_of(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .fold(String::new(), |mut hex, byte| {
            let _ = write!(hex, "{byte:02x}");
            hex
        })
}

/// Checks downloaded bytes against the digest adbc-vs was compiled with,
/// before anything is extracted or written.
fn verify_digest(bytes: &[u8], expected: &str, url: &str) -> Result<(), ProvisioningError> {
    let computed = digest_of(bytes);
    if computed == expected {
        return Ok(());
    }
    Err(ProvisioningError::ChecksumMismatch {
        url: url.to_owned(),
        expected: expected.to_owned(),
        computed,
    })
}

/// The driver shared library unpacked from a verified package, and the
/// directory holding it.
#[derive(Debug)]
struct ExtractedDriver {
    /// Deletes the unpacked package when the driver is dropped. Held, never
    /// read: the upload streams from `path`, which lives inside it.
    _directory: TempDir,
    path: PathBuf,
    filename: String,
}

/// Unpacks a verified package and returns the shared library its `MANIFEST`
/// names.
///
/// Members are written under their file name alone, so an archive naming a
/// parent directory cannot place a file outside the extraction directory.
fn extract_driver(package: &[u8], url: &str) -> Result<ExtractedDriver, ProvisioningError> {
    let directory = TempDir::new().map_err(|error| ProvisioningError::LocalFailure {
        attempt: "creating a directory to unpack the driver package into".to_owned(),
        cause: error.to_string(),
    })?;

    let mut manifest = None;
    let mut archive = tar::Archive::new(GzDecoder::new(package));
    let members = archive
        .entries()
        .map_err(|error| unreadable_package(url, &error))?;

    for member in members {
        let mut member = member.map_err(|error| unreadable_package(url, &error))?;
        if !member.header().entry_type().is_file() {
            continue;
        }

        let path = member
            .path()
            .map_err(|error| unreadable_package(url, &error))?;
        let Some(name) = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
        else {
            continue;
        };

        if name == MANIFEST_MEMBER {
            let mut declared = String::new();
            member
                .read_to_string(&mut declared)
                .map_err(|error| unreadable_package(url, &error))?;
            manifest = Some(declared);
        } else {
            member
                .unpack(directory.path().join(&name))
                .map_err(|error| unreadable_package(url, &error))?;
        }
    }

    let manifest = manifest.ok_or_else(|| ProvisioningError::ManifestMissing {
        url: url.to_owned(),
    })?;
    let filename = declared_library(&manifest, url)?;
    let path = directory.path().join(&filename);
    if !path.is_file() {
        return Err(ProvisioningError::LibraryMissing {
            url: url.to_owned(),
            filename,
        });
    }

    Ok(ExtractedDriver {
        _directory: directory,
        path,
        filename,
    })
}

/// Rejects a package whose shared library is not the file adbc-vs computes its
/// BucketFS path from.
fn verify_library_filename(declared: &str) -> Result<(), ProvisioningError> {
    if declared == DRIVER_LIBRARY {
        return Ok(());
    }
    Err(ProvisioningError::LibraryFilenameDiverged {
        declared: declared.to_owned(),
        expected: DRIVER_LIBRARY.to_owned(),
    })
}

/// Replaces every occurrence of a secret in text bound for an error message.
fn redact(text: &str, secret: &str) -> String {
    if secret.is_empty() {
        return text.to_owned();
    }
    text.replace(secret, "***")
}

/// The `MANIFEST` of a driver package, of which provisioning reads one key.
#[derive(Deserialize)]
struct Manifest {
    #[serde(rename = "Files")]
    files: ManifestFiles,
}

#[derive(Deserialize)]
struct ManifestFiles {
    driver: String,
}

fn declared_library(manifest: &str, url: &str) -> Result<String, ProvisioningError> {
    let parsed: Manifest =
        toml::from_str(manifest).map_err(|error| ProvisioningError::ManifestUnreadable {
            url: url.to_owned(),
            cause: error.to_string(),
        })?;

    Path::new(&parsed.files.driver)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .ok_or_else(|| ProvisioningError::ManifestUnreadable {
            url: url.to_owned(),
            cause: format!("`{}` names no file", parsed.files.driver),
        })
}

fn unreadable_package(url: &str, error: &dyn fmt::Display) -> ProvisioningError {
    ProvisioningError::PackageUnreadable {
        url: url.to_owned(),
        cause: error.to_string(),
    }
}

/// The client that reaches the public registry.
///
/// It verifies the server certificate against the root store compiled into the
/// binary, and takes no argument that could relax that.
fn registry_client() -> Result<Client, ProvisioningError> {
    Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(TRANSFER_TIMEOUT)
        .build()
        .map_err(|error| ProvisioningError::LocalFailure {
            attempt: "building the HTTPS client for the driver registry".to_owned(),
            cause: error.to_string(),
        })
}

/// The client that reaches BucketFS.
///
/// BucketFS serves the cluster's own certificate to an address the operator
/// configured, not a name an attacker can answer for, so this client accepts
/// it. Nothing else in adbc-vs uses this client.
fn bucketfs_client() -> Result<Client, ProvisioningError> {
    Client::builder()
        .danger_accept_invalid_certs(true)
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(TRANSFER_TIMEOUT)
        .build()
        .map_err(|error| ProvisioningError::LocalFailure {
            attempt: "building the HTTPS client for BucketFS".to_owned(),
            cause: error.to_string(),
        })
}

/// Everything that stops a driver-registry download from producing a body.
///
/// Carries only what the download itself knows; `ensure_driver` translates
/// each variant into the matching `ProvisioningError`, adding the install
/// path where the message needs it.
enum DownloadFailure {
    Unreachable(String),
    Rejected(u16),
    TooLarge(u64),
}

fn download(client: &Client, url: &str, limit: u64) -> Result<Vec<u8>, DownloadFailure> {
    let response = client
        .get(url)
        .send()
        .map_err(|error| DownloadFailure::Unreachable(error.to_string()))?;
    let status = response.status();
    if !status.is_success() {
        return Err(DownloadFailure::Rejected(status.as_u16()));
    }

    let mut body = Vec::new();
    response
        .take(limit + 1)
        .read_to_end(&mut body)
        .map_err(|error| DownloadFailure::Unreachable(error.to_string()))?;
    if body.len() as u64 > limit {
        return Err(DownloadFailure::TooLarge(limit));
    }

    Ok(body)
}

fn provisioning_error(
    failure: DownloadFailure,
    url: &str,
    install_path: &Path,
) -> ProvisioningError {
    match failure {
        DownloadFailure::Unreachable(cause) => ProvisioningError::RegistryUnreachable {
            url: url.to_owned(),
            install_path: install_path.to_owned(),
            cause,
        },
        DownloadFailure::Rejected(status) => ProvisioningError::RegistryRejected {
            url: url.to_owned(),
            status,
            install_path: install_path.to_owned(),
        },
        DownloadFailure::TooLarge(limit) => ProvisioningError::ResponseTooLarge {
            url: url.to_owned(),
            limit,
        },
    }
}

/// Writes the driver to BucketFS, waiting out a locked object rather than
/// failing the statement that found it locked.
///
/// Every other status surfaces on the first attempt: a wrong password or a
/// full filesystem does not become right by being asked again.
fn upload(client: &Client, bucket: &Bucket, driver: &Path) -> Result<(), ProvisioningError> {
    let started = Instant::now();
    loop {
        let status = put_driver(client, bucket, driver)?;
        if (200..300).contains(&status) {
            return Ok(());
        }
        if status != BUCKETFS_LOCKED || started.elapsed() >= LOCK_DEADLINE {
            return Err(ProvisioningError::BucketFsRejected {
                url: redact(&bucket.driver_url(), &bucket.credentials.password),
                status,
            });
        }
        thread::sleep(LOCK_POLL_INTERVAL);
    }
}

/// One `PUT` of the driver, reporting the status BucketFS answered.
fn put_driver(client: &Client, bucket: &Bucket, driver: &Path) -> Result<u16, ProvisioningError> {
    let secret = &bucket.credentials.password;

    let library = File::open(driver).map_err(|error| ProvisioningError::LocalFailure {
        attempt: format!("opening {} to upload it", driver.display()),
        cause: error.to_string(),
    })?;

    let response = client
        .put(bucket.driver_url())
        .basic_auth(&bucket.credentials.user, Some(secret))
        .body(library)
        .send()
        .map_err(|error| ProvisioningError::BucketFsUnreachable {
            url: redact(&bucket.driver_url(), secret),
            cause: redact(&error.to_string(), secret),
        })?;

    Ok(response.status().as_u16())
}

/// Waits for a written driver to become readable on the adapter's own mount,
/// giving up at a deadline rather than blocking a statement forever.
///
/// A BucketFS write is not visible on every node the moment it is
/// acknowledged, and a half-visible file would fail the scan's `dlopen` with a
/// far worse diagnosis than a timeout naming the path.
fn await_visibility(path: &Path, length: u64, deadline: Duration) -> Result<(), ProvisioningError> {
    let started = Instant::now();
    loop {
        if is_readable(path, length) {
            return Ok(());
        }

        let elapsed = started.elapsed();
        if elapsed >= deadline {
            return Err(ProvisioningError::DriverNotVisible {
                path: path.to_owned(),
                deadline,
            });
        }
        thread::sleep(VISIBILITY_POLL_INTERVAL.min(deadline - elapsed));
    }
}

fn is_readable(path: &Path, length: u64) -> bool {
    path.metadata()
        .is_ok_and(|file| file.is_file() && file.len() == length)
}

#[cfg(test)]
#[path = "provisioning_tests.rs"]
mod tests;
