//! Makes one technology's ADBC driver readable at a BucketFS path.
//!
//! There are two routes to that and they are the whole interface.
//! [`ensure_driver`] provisions the driver from the registry: a caller hands it
//! the registry to fetch from and the bucket to install into, and learns back
//! the path the scan UDF loads the driver from and the version the range
//! resolved to. Whether that driver was already there, or was downloaded,
//! unpacked, uploaded and waited for, is this module's business alone.
//! [`supplied_driver`] is the `DRIVER_PATH` route, where the operator placed
//! the library themselves and nothing is fetched, resolved, or written.
//!
//! Which package to fetch, which versions of it are acceptable, and which file
//! name it installs are read off the [`Driver`] the [`Registry`] was bound to,
//! so a cluster running two technologies installs two libraries rather than one
//! overwriting the other, and the integration tier that fetches the same
//! package cannot pin a different one.
//!
//! The install path carries the driver *and* the version the range resolved to,
//! which is what makes the presence check answerable only after the index has
//! been read. An unversioned path would have made it answer for whichever
//! version arrived first, so a range would resolve once per cluster and never
//! again. The cost is that an already-installed driver saves the download but
//! not the index read, and that a cluster with no outbound egress can no longer
//! be served by placing a file on the mount — that route is the `DRIVER_PATH`
//! property, and this module's unreachable-registry errors name it.
//!
//! Two HTTP clients, never one. The registry client reaches a public internet
//! host by name and verifies its certificate against a bundled root store,
//! with no switch anywhere that turns that off. The BucketFS client reaches an
//! in-cluster endpoint that serves the cluster's own self-signed certificate,
//! and accepts it. Sharing one client would extend the second policy over the
//! first.
//!
//! Nothing on the way in is checked for integrity beyond the transport. The
//! registry index publishes no checksum for any driver — a package block
//! carries exactly `platform` and `url` — and the signature a package embeds
//! is verifiable only against a key served by the same host as the artifact,
//! which adds ceremony rather than independent trust. So the registry client's
//! certificate verification is the whole guarantee, and an operator who needs
//! a vouched-for artifact supplies it themselves rather than through here.
//!
//! Because nothing vouches for the archive, unpacking assumes nothing about
//! it: it bounds what it reads and writes at [`MAX_UNPACKED_BYTES`] and
//! refuses a package that asks for more, so a response inside the download
//! ceiling cannot expand into one that fills the sandbox's disk or exhausts
//! its memory.
//!
//! Concurrent provisioning converges rather than conflicting: every writer
//! uploads the same bytes of the same resolved version to the same path.

use std::fmt;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use flate2::read::GzDecoder;
use reqwest::blocking::Client;
use semver::Version;
use serde::Deserialize;
use tempfile::TempDir;

use adbc_vs_scan::driver::{Driver, PLATFORM, REGISTRY_BASE_URL};

use crate::registry_index::{self, ResolvedPackage};

const REGISTRY_INDEX_FILE: &str = "index.yaml";
const MANIFEST_MEMBER: &str = "MANIFEST";

/// The property an operator names their own driver library in.
///
/// Spelled here rather than beside the statement's other properties because
/// this module owns both ends of it: the route it selects, and the errors that
/// have to name it — the unreachable-registry ones, which is where an operator
/// who did not set it learns it exists.
pub const DRIVER_PATH_PROPERTY: &str = "DRIVER_PATH";

/// The bucket-relative directory every adbc-vs driver is installed under.
const DRIVER_DIRECTORY: &str = "adbc-drivers";

/// Response size ceilings. A registry that answers with an endless body would
/// otherwise exhaust the UDF's memory instead of failing.
const MAX_INDEX_BYTES: u64 = 4 * 1024 * 1024;
const MAX_PACKAGE_BYTES: u64 = 256 * 1024 * 1024;

/// The ceiling on what one driver package may unpack to.
///
/// [`MAX_PACKAGE_BYTES`] bounds only the compressed body, and gzip has no
/// bounded expansion ratio, so a response inside that ceiling can still ask to
/// write arbitrarily many gigabytes into the UDF sandbox. Nothing has vouched
/// for those bytes by the time they are unpacked, so unpacking bounds itself.
///
/// The same number is enforced from both sides: the sum of the sizes the
/// members declare, which is refused by name, and the total the gzip stream is
/// read for at all, which caps an archive whose headers rather than whose
/// members are the bomb. Roughly eleven times the largest library the registry
/// publishes today, so no real package comes near it.
const MAX_UNPACKED_BYTES: u64 = 512 * 1024 * 1024;

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

    /// Where one version of `driver` is, or will be, readable on this mount.
    ///
    /// Both the driver and the version are arguments rather than something the
    /// bucket was bound to: the same `BUCKETFS_CONNECTION` serves every virtual
    /// schema on the cluster, whatever each one's `DRIVER` says and whatever
    /// version each one's range resolved to.
    pub fn driver_path(&self, driver: Driver, version: &Version) -> PathBuf {
        driver_install_path(&self.mount, driver, version)
    }

    fn driver_url(&self, driver: Driver, version: &Version) -> String {
        format!(
            "{}/{DRIVER_DIRECTORY}/{}/{version}/{}",
            self.write_url,
            driver.as_str(),
            driver.package().library
        )
    }
}

/// Where driver packages come from, and which driver's package to fetch.
///
/// It carries no version. Which version to install is the driver's own
/// declared range resolved against whatever the registry publishes today, so
/// naming one here would be a second answer to a question the driver
/// declaration already answers.
#[derive(Debug)]
pub struct Registry {
    base_url: String,
    driver: Driver,
}

impl Registry {
    /// The public `dbc` registry, and the driver whose range is resolved
    /// against it.
    pub fn public(driver: Driver) -> Self {
        Self {
            base_url: REGISTRY_BASE_URL.to_owned(),
            driver,
        }
    }

    /// Binds to another host serving the same registry layout.
    ///
    /// Adapter code always provisions from [`Registry::public`]; this exists
    /// so the fetch, unpack and upload path can be driven end to end against a
    /// stub registry rather than the public internet.
    pub fn at(base_url: &str, driver: Driver) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_owned(),
            driver,
        }
    }

    /// The versions of this driver the release accepts, as its declaration
    /// states them.
    fn version_range(&self) -> &'static str {
        self.driver.package().version_range
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
        driver: String,
        cause: String,
    },
    RegistryRejected {
        url: String,
        status: u16,
        driver: String,
    },
    ResponseTooLarge {
        url: String,
        limit: u64,
    },
    PackageTooLarge {
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
    RangeUnparseable {
        driver: String,
        range: String,
        cause: String,
    },
    RangeNotSatisfied {
        driver: String,
        range: String,
        published: Vec<String>,
        url: String,
    },
    PlatformNotPublished {
        driver: String,
        platform: String,
        version: String,
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
    SuppliedDriverUnreadable {
        driver: String,
        path: String,
        cause: String,
    },
}

impl fmt::Display for ProvisioningError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RegistryUnreachable { url, driver, cause } => write!(
                f,
                "could not reach the driver registry at {url} while provisioning the \
                 `{driver}` driver: {cause}. Without outbound access, supply the driver \
                 library yourself with the {DRIVER_PATH_PROPERTY} property; there is no path \
                 on the mount to pre-load it at, because the install path carries a version \
                 only the registry could have resolved"
            ),
            Self::RegistryRejected {
                url,
                status,
                driver,
            } => write!(
                f,
                "the driver registry at {url} answered HTTP {status} while provisioning \
                 the `{driver}` driver. Without registry access, supply the driver library \
                 yourself with the {DRIVER_PATH_PROPERTY} property; there is no path on the \
                 mount to pre-load it at, because the install path carries a version only the \
                 registry could have resolved"
            ),
            Self::ResponseTooLarge { url, limit } => write!(
                f,
                "{url} answered with more than {limit} bytes, which no driver registry \
                 response is; the download was abandoned"
            ),
            Self::PackageTooLarge { url, limit } => write!(
                f,
                "the driver package downloaded from {url} asks to unpack more than {limit} \
                 bytes, which no driver package does; nothing vouches for an archive before it \
                 is unpacked, so it was abandoned rather than written"
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
            Self::RangeUnparseable {
                driver,
                range,
                cause,
            } => write!(
                f,
                "adbc-vs declares `{range}` as the version range it accepts for driver \
                 `{driver}`, which is not a version range: {cause}"
            ),
            Self::RangeNotSatisfied {
                driver,
                range,
                published,
                url,
            } => write!(
                f,
                "the driver registry index at {url} publishes no version of driver \
                 `{driver}` satisfying `{range}`, the range adbc-vs accepts for it; it \
                 publishes {}",
                published_list(published)
            ),
            Self::PlatformNotPublished {
                driver,
                platform,
                version,
            } => write!(
                f,
                "version {version} of driver `{driver}` publishes no `{platform}` \
                 package, and `{platform}` is the only platform the script language \
                 container runs on"
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
            Self::SuppliedDriverUnreadable {
                driver,
                path,
                cause,
            } => write!(
                f,
                "the {DRIVER_PATH_PROPERTY} property names `{path}` as the `{driver}` driver \
                 shared library, but no readable file is there: {cause}. A supplied driver \
                 path is used exactly as given and is never fallen back on, so no driver was \
                 fetched from the registry instead"
            ),
        }
    }
}

impl std::error::Error for ProvisioningError {}

/// The driver a virtual schema loads: where it is readable, and which version
/// it is.
///
/// The version travels with the path rather than being left for the caller to
/// recompute, because only the index read knows it. `adapterNotes` records it
/// as the one place an operator can read back which driver version a given
/// virtual schema is running, which under a version range no compiled-in
/// constant can answer.
#[derive(Debug)]
pub struct InstalledDriver {
    pub path: PathBuf,
    pub version: Version,
}

/// Makes the version a driver's range resolves to readable at its BucketFS
/// path, and returns that path and that version.
///
/// The index is read first and always, because the version the presence check
/// tests for is the resolved one. A driver already installed at that version
/// then costs one stat call rather than a 46 MB round trip; one installed at a
/// different in-range version is left where it is and the resolved version is
/// installed beside it, because a running query can still have the older
/// library mapped.
pub fn ensure_driver(
    registry: &Registry,
    bucket: &Bucket,
) -> Result<InstalledDriver, ProvisioningError> {
    let registry_client = registry_client()?;
    let index_url = registry.index_url();
    let index = download(&registry_client, &index_url, MAX_INDEX_BYTES)
        .map_err(|failure| provisioning_error(failure, &index_url, registry.driver))?;
    let index = String::from_utf8(index).map_err(|error| ProvisioningError::IndexUnreadable {
        url: registry.index_url(),
        cause: error.to_string(),
    })?;

    let resolved = select_package(&index, registry)?;
    let install_path = bucket.driver_path(registry.driver, &resolved.version);
    if install_path.is_file() {
        return Ok(InstalledDriver {
            path: install_path,
            version: resolved.version,
        });
    }

    let driver = {
        let package = download(&registry_client, &resolved.url, MAX_PACKAGE_BYTES)
            .map_err(|failure| provisioning_error(failure, &resolved.url, registry.driver))?;
        extract_driver(&package, &resolved.url)?
    };
    verify_library_filename(&driver.filename, registry.driver)?;

    let installed_length = driver
        .path
        .metadata()
        .map_err(|error| ProvisioningError::LocalFailure {
            attempt: format!("measuring the extracted {}", driver.filename),
            cause: error.to_string(),
        })?
        .len();

    let write_url = bucket.driver_url(registry.driver, &resolved.version);
    upload(
        &bucketfs_client()?,
        &bucket.credentials,
        &write_url,
        &driver.path,
    )?;
    await_visibility(&install_path, installed_length, bucket.visibility_deadline)?;

    Ok(InstalledDriver {
        path: install_path,
        version: resolved.version,
    })
}

/// The driver shared library an operator supplied at `path`, checked for
/// nothing but being a readable file.
///
/// This is the whole `DRIVER_PATH` route: no index is read, nothing is
/// downloaded, and nothing is written, so a cluster with no outbound egress
/// runs the driver its operator placed. The library is the operator's to vouch
/// for — adbc-vs asserts nothing else about it — and the path is used exactly
/// as given, neither re-rooted under a mount nor extended with the driver and
/// version segments self-install computes.
///
/// It never falls back to the registry. A silent fallback would defeat the air
/// gap the property exists to serve, and would make an operator's typo look
/// like a slow first query.
pub fn supplied_driver(path: &str, driver: Driver) -> Result<PathBuf, ProvisioningError> {
    let unreadable = |cause: String| ProvisioningError::SuppliedDriverUnreadable {
        driver: driver.as_str().to_owned(),
        path: path.to_owned(),
        cause,
    };

    let library = File::open(path).map_err(|error| unreadable(error.to_string()))?;
    let kind = library
        .metadata()
        .map_err(|error| unreadable(error.to_string()))?;
    if !kind.is_file() {
        return Err(unreadable("it is not a regular file".to_owned()));
    }

    Ok(PathBuf::from(path))
}

/// Where one version of `driver`'s library is installed under a bucket's mount.
///
/// Both segments earn their place: the driver segment keeps two technologies
/// apart, and the version segment is what binds the presence check to the
/// version a range resolved to rather than to whichever one arrived first.
fn driver_install_path(mount: &Path, driver: Driver, version: &Version) -> PathBuf {
    mount
        .join(DRIVER_DIRECTORY)
        .join(driver.as_str())
        .join(version.to_string())
        .join(driver.package().library)
}

/// Resolves the driver's declared version range against a registry index
/// document to one published version and its `linux_amd64` package.
fn select_package(index: &str, registry: &Registry) -> Result<ResolvedPackage, ProvisioningError> {
    let url = registry.index_url();
    let registry_path = registry.driver.package().registry_path;
    let range = registry.version_range();
    registry_index::resolve_package(index, registry_path, range, PLATFORM, &registry.base_url)
        .map_err(|error| match error {
            registry_index::LookupError::Unparseable(cause) => ProvisioningError::IndexUnreadable {
                url: url.clone(),
                cause,
            },
            registry_index::LookupError::NoDriver => ProvisioningError::DriverNotPublished {
                driver: registry_path.to_owned(),
                url: url.clone(),
            },
            registry_index::LookupError::UnparseableRange(cause) => {
                ProvisioningError::RangeUnparseable {
                    driver: registry_path.to_owned(),
                    range: range.to_owned(),
                    cause,
                }
            }
            registry_index::LookupError::NoVersionInRange { published } => {
                ProvisioningError::RangeNotSatisfied {
                    driver: registry_path.to_owned(),
                    range: range.to_owned(),
                    published,
                    url: url.clone(),
                }
            }
            registry_index::LookupError::NoPlatform { resolved } => {
                ProvisioningError::PlatformNotPublished {
                    driver: registry_path.to_owned(),
                    platform: PLATFORM.to_owned(),
                    version: resolved.to_string(),
                }
            }
        })
}

/// The versions an index publishes, for an error that has to say what the
/// range missed.
fn published_list(published: &[String]) -> String {
    if published.is_empty() {
        return "no versions at all".to_owned();
    }
    published.join(", ")
}

/// The driver shared library unpacked from a downloaded package, and the
/// directory holding it.
#[derive(Debug)]
struct ExtractedDriver {
    /// Deletes the unpacked package when the driver is dropped. Held, never
    /// read: the upload streams from `path`, which lives inside it.
    _directory: TempDir,
    path: PathBuf,
    filename: String,
}

/// Unpacks a downloaded package and returns the shared library its `MANIFEST`
/// names.
///
/// This is the first step to touch bytes nobody has vouched for, so it bounds
/// what it takes from them: no member is written or read until the total the
/// members have declared is known to stay inside [`MAX_UNPACKED_BYTES`], and
/// the gzip stream is read for no more than that either.
///
/// Members are written under their file name alone, so an archive naming a
/// parent directory cannot place a file outside the extraction directory.
fn extract_driver(package: &[u8], url: &str) -> Result<ExtractedDriver, ProvisioningError> {
    let directory = TempDir::new().map_err(|error| ProvisioningError::LocalFailure {
        attempt: "creating a directory to unpack the driver package into".to_owned(),
        cause: error.to_string(),
    })?;

    let mut unpacked: u64 = 0;
    let mut manifest = None;
    let mut archive = tar::Archive::new(GzDecoder::new(package).take(MAX_UNPACKED_BYTES));
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

        unpacked = unpacked.saturating_add(member.size());
        if unpacked > MAX_UNPACKED_BYTES {
            return Err(ProvisioningError::PackageTooLarge {
                url: url.to_owned(),
                limit: MAX_UNPACKED_BYTES,
            });
        }

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

/// Rejects a package whose shared library is not the file adbc-vs computes
/// `driver`'s BucketFS path from.
///
/// Public because the integration tier loads a driver out of the same package
/// and must refuse exactly what an install would: a tier applying its own rule
/// could certify a library production would have rejected.
pub fn verify_library_filename(declared: &str, driver: Driver) -> Result<(), ProvisioningError> {
    let expected = driver.package().library;
    if declared == expected {
        return Ok(());
    }
    Err(ProvisioningError::LibraryFilenameDiverged {
        declared: declared.to_owned(),
        expected: expected.to_owned(),
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

/// The shared library a package's `MANIFEST` declares, as a bare file name.
///
/// Public for the same reason [`verify_library_filename`] is: the integration
/// tier reads the same declaration out of the same package, and a second
/// reader of the `[Files] driver` key would be free to drift from this one.
pub fn declared_library(manifest: &str, url: &str) -> Result<String, ProvisioningError> {
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
/// each variant into the matching `ProvisioningError`, adding the driver where
/// the message needs it.
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

fn provisioning_error(failure: DownloadFailure, url: &str, driver: Driver) -> ProvisioningError {
    match failure {
        DownloadFailure::Unreachable(cause) => ProvisioningError::RegistryUnreachable {
            url: url.to_owned(),
            driver: driver.as_str().to_owned(),
            cause,
        },
        DownloadFailure::Rejected(status) => ProvisioningError::RegistryRejected {
            url: url.to_owned(),
            status,
            driver: driver.as_str().to_owned(),
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
fn upload(
    client: &Client,
    credentials: &Credentials,
    write_url: &str,
    library_path: &Path,
) -> Result<(), ProvisioningError> {
    let started = Instant::now();
    loop {
        let status = put_driver(client, credentials, write_url, library_path)?;
        if (200..300).contains(&status) {
            return Ok(());
        }
        if status != BUCKETFS_LOCKED || started.elapsed() >= LOCK_DEADLINE {
            return Err(ProvisioningError::BucketFsRejected {
                url: redact(write_url, &credentials.password),
                status,
            });
        }
        thread::sleep(LOCK_POLL_INTERVAL);
    }
}

/// One `PUT` of the driver, reporting the status BucketFS answered.
fn put_driver(
    client: &Client,
    credentials: &Credentials,
    write_url: &str,
    library_path: &Path,
) -> Result<u16, ProvisioningError> {
    let secret = &credentials.password;

    let library = File::open(library_path).map_err(|error| ProvisioningError::LocalFailure {
        attempt: format!("opening {} to upload it", library_path.display()),
        cause: error.to_string(),
    })?;

    let response = client
        .put(write_url)
        .basic_auth(&credentials.user, Some(secret))
        .body(library)
        .send()
        .map_err(|error| ProvisioningError::BucketFsUnreachable {
            url: redact(write_url, secret),
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
