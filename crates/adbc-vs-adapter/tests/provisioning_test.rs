//! Drives driver provisioning end to end against local doubles: a stub
//! registry, a stub BucketFS endpoint, and a directory standing in for the
//! read-only BucketFS mount a UDF sees.
//!
//! Nothing here reaches the public registry, a real BucketFS, or a container.
//! Every failure the adapter can hit on the way to an installed driver is
//! provoked deliberately, because each one of them surfaces to an operator as
//! a failed `CREATE VIRTUAL SCHEMA` and has to say what to do next.

use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use adbc_vs_adapter::provisioning::{
    default_driver_path, ensure_driver, readable_library, supplied_driver, Bucket, Credentials,
    Registry,
};
use adbc_vs_scan::driver::Driver;
use flate2::write::GzEncoder;
use flate2::Compression;

/// The driver these scenarios provision, and the library its row declares.
const DRIVER: Driver = Driver::MySql;

fn driver_library() -> &'static str {
    DRIVER.package().library
}

const BUCKET_PATH: &str = "/default";
const INDEX_PATH: &str = "/index.yaml";

/// The two MySQL versions the stub registry publishes, and what each one
/// resolves to.
///
/// Both sit inside the range `drivers/mysql.toml` declares, so resolution has
/// to choose between them rather than take the only candidate there is. The
/// published spelling carries the leading `v` the registry writes and the
/// resolved one does not, so a path built from the wrong one of the two shows
/// up as a mismatch rather than passing unnoticed.
const PUBLISHED_OLDER: &str = "v0.6.0";
const PUBLISHED_NEWEST: &str = "v0.6.1";
const OLDER_VERSION: &str = "0.6.0";
const RESOLVED_VERSION: &str = "0.6.1";

const LIBRARY_BYTES: &[u8] = b"\x7fELF a driver, for the purposes of this test";
const OLDER_LIBRARY_BYTES: &[u8] = b"\x7fELF the driver an earlier statement installed";
const WRITE_USER: &str = "w";
const WRITE_PASSWORD: &str = "M4b7-bucketfs-write";

/// A version outside `drivers/mysql.toml`'s declared `>=0.6.0, <0.7.0` range —
/// published, and parseable, but not one the range resolves to. Distinct from
/// an index publishing no version at all: this is what "a registry that
/// answered" looks like on the version-resolution step rather than on the
/// transport, and it is a fact about the package, not about the cluster's
/// egress.
const PUBLISHED_OUT_OF_RANGE: &str = "v0.9.0";

/// The bytes of a library an operator pre-placed at the default location.
/// Distinct from [`LIBRARY_BYTES`] and [`OLDER_LIBRARY_BYTES`] so a test
/// asserting which bytes survive cannot pass by every fixture happening to
/// share content.
const SUPPLIED_LIBRARY_BYTES: &[u8] = b"\x7fELF a pre-placed mysql driver, never touched \
    by self-install";
const POSTGRESQL_SUPPLIED_LIBRARY_BYTES: &[u8] = b"\x7fELF a pre-placed postgresql driver, \
    never touched by self-install";

/// PostgreSQL's own published version and what it resolves to, spelled bare
/// rather than with MySQL's leading `v` — the same asymmetry
/// `drivers/postgresql.toml` and `drivers/mysql.toml` declare for real, kept
/// here so a scenario provisioning both drivers cannot pass by accident from
/// one spelling rule serving both.
const POSTGRESQL_PUBLISHED: &str = "1.13.0";
const POSTGRESQL_RESOLVED_VERSION: &str = "1.13.0";

const POSTGRESQL_LIBRARY_BYTES: &[u8] =
    b"\x7fELF a postgresql driver, for the purposes of this test";

/// The name the PostgreSQL package's license member carries in its own
/// `MANIFEST`, confirmed by gate measurement to differ from the MySQL
/// package's `LICENSE`. Provisioning uploads only the file `MANIFEST` names
/// as `driver`, so nothing here changing the license file's own spelling
/// should ever be able to make it a candidate for upload.
const POSTGRESQL_LICENSE_FILENAME: &str = "LICENSE.txt";

// ---------------------------------------------------------------------------
// Scenarios
// ---------------------------------------------------------------------------

#[test]
fn installed_driver_short_circuits_per_driver() {
    let mount = MountDirectory::new();
    mount.install(RESOLVED_VERSION, LIBRARY_BYTES);
    let registry = StubServer::serving(routes(&[(
        INDEX_PATH,
        index_publishing(&[PUBLISHED_NEWEST]),
    )]));
    let bucketfs = StubServer::serving(routes(&[]));

    let installed = ensure_driver(&registry.as_registry(), &bucketfs.as_bucket(&mount))
        .expect("a mount already holding the resolved version must provision from it");

    assert_eq!(installed.path, mount.driver_path(RESOLVED_VERSION));
    assert_eq!(
        installed.version.to_string(),
        RESOLVED_VERSION,
        "a short circuit reports the version resolution selected just as an install does; \
         `adapterNotes.driverVersion` is written from it either way"
    );
    assert_eq!(
        registry.requested_paths(),
        vec![INDEX_PATH.to_string()],
        "the index is still read, because the version the presence check tests for is \
         the one the range resolves to; nothing past it may be fetched"
    );
    assert!(
        bucketfs.requests().is_empty(),
        "an installed driver must not be written to BucketFS: {:?}",
        bucketfs.requests()
    );
}

#[test]
fn newer_in_range_version_is_not_short_circuited() {
    let mount = MountDirectory::new();
    mount.install(OLDER_VERSION, OLDER_LIBRARY_BYTES);
    let registry = StubServer::serving(routes(&[
        (
            INDEX_PATH,
            index_publishing(&[PUBLISHED_OLDER, PUBLISHED_NEWEST]),
        ),
        (
            &package_path(PUBLISHED_NEWEST),
            package_declaring(driver_library(), LIBRARY_BYTES),
        ),
    ]));
    let bucketfs = StubServer::serving(mount.accepting_writes());

    let installed = ensure_driver(&registry.as_registry(), &bucketfs.as_bucket(&mount))
        .expect("a higher published version inside the range must install");

    assert_eq!(
        installed.path,
        mount.driver_path(RESOLVED_VERSION),
        "an older in-range version on the mount must not answer the presence check for \
         the version the range resolves to today"
    );
    assert_eq!(
        fs::read(&installed.path).expect("the newly installed driver must be readable"),
        LIBRARY_BYTES
    );
    assert_eq!(
        fs::read(mount.driver_path(OLDER_VERSION))
            .expect("the older driver must still be readable"),
        OLDER_LIBRARY_BYTES,
        "the older version is neither deleted nor overwritten: a running query can still \
         have it mapped"
    );
    assert_eq!(
        bucketfs.requested_paths(),
        vec![driver_put_path(RESOLVED_VERSION)],
        "only the newly resolved version is written, and under its own version segment"
    );
}

#[test]
fn manifest_filename_must_match_constant() {
    let mount = MountDirectory::new();
    let registry = StubServer::serving(routes(&[
        (INDEX_PATH, index_publishing(&[PUBLISHED_NEWEST])),
        (
            &package_path(PUBLISHED_NEWEST),
            package_declaring("libadbc_driver_mysql_v2.so", LIBRARY_BYTES),
        ),
    ]));
    let bucketfs = StubServer::serving(routes(&[]));

    let error = ensure_driver(&registry.as_registry(), &bucketfs.as_bucket(&mount))
        .expect_err("a package whose library is named differently must not install");

    let message = error.to_string();
    assert!(
        message.contains("libadbc_driver_mysql_v2.so"),
        "the error must name the library the package declares: {message}"
    );
    assert!(
        message.contains(driver_library()),
        "the error must name the library adbc-vs is compiled against: {message}"
    );
    assert!(
        bucketfs.requests().is_empty(),
        "a package adbc-vs cannot address must not be written to BucketFS"
    );
}

#[test]
fn registry_client_rejects_untrusted_certificate() {
    let mount = MountDirectory::new();
    let registry = UntrustedTlsServer::start();
    let bucketfs = StubServer::serving(routes(&[]));

    let error = ensure_driver(
        &Registry::at(&registry.url(), DRIVER),
        &bucketfs.as_bucket(&mount),
    )
    .expect_err("a registry certificate no root store signs must not be trusted");

    let message = error.to_string();
    assert_eq!(
        registry.served_requests(),
        0,
        "the registry client must hang up on a certificate it cannot verify, before \
         asking that host for anything: {message}"
    );
    assert!(
        message.contains(&registry.url()),
        "the error must name the registry it refused to trust: {message}"
    );
    assert!(
        !mount.driver_path(RESOLVED_VERSION).exists(),
        "no driver may be installed from a registry that could not be authenticated"
    );
    assert!(
        bucketfs.requests().is_empty(),
        "an unauthenticated registry must not lead to a BucketFS write"
    );
}

#[test]
fn offline_fallback_names_the_drivers_own_path() {
    let mount = MountDirectory::new();
    let closed = closed_port();
    let bucketfs = StubServer::serving(routes(&[]));

    let error = ensure_driver(
        &Registry::at(&format!("http://127.0.0.1:{closed}"), DRIVER),
        &bucketfs.as_bucket(&mount),
    )
    .expect_err("a registry nothing answers for must fail provisioning");

    let message = error.to_string();
    assert!(
        message.contains(&format!("127.0.0.1:{closed}")),
        "the error must name the registry it could not reach: {message}"
    );
    assert!(
        message.contains(DRIVER.as_str()),
        "the error must name the driver it was provisioning: {message}"
    );
    assert!(
        message.contains("DRIVER_PATH"),
        "the error must name the property that supplies a driver without the registry: \
         {message}"
    );
    assert!(
        !message.contains(driver_library()),
        "the error must name no versioned install path, because an unreachable registry \
         never resolved a version to install: {message}"
    );
}

#[test]
fn offline_fallback_names_the_drivers_own_path_for_postgresql() {
    let mount = MountDirectory::new();
    let closed = closed_port();
    let bucketfs = StubServer::serving(routes(&[]));

    let error = ensure_driver(
        &Registry::at(&format!("http://127.0.0.1:{closed}"), Driver::Postgres),
        &bucketfs.as_bucket(&mount),
    )
    .expect_err("a registry nothing answers for must fail provisioning");

    let message = error.to_string();
    assert!(
        message.contains(&format!("127.0.0.1:{closed}")),
        "the error must name the registry it could not reach: {message}"
    );
    assert!(
        message.contains(Driver::Postgres.as_str()),
        "the error must name postgresql, the driver it was provisioning, not mysql: {message}"
    );
    assert!(
        message.contains("DRIVER_PATH"),
        "the error must name the property that supplies a driver without the registry: \
         {message}"
    );
    assert!(
        !message.contains(Driver::Postgres.package().library),
        "the error must name no versioned install path, because an unreachable registry \
         never resolved a version to install: {message}"
    );
    assert!(
        !message.contains(driver_library()),
        "postgresql's offline error must name postgresql's own path, never mysql's library: \
         {message}"
    );
}

#[test]
fn missing_platform_names_driver_and_version() {
    let mount = MountDirectory::new();
    let index = concat!(
        "drivers:\n",
        "  - path: mysql\n",
        "    pkginfo:\n",
        "      - version: v0.6.1\n",
        "        packages:\n",
        "          - platform: darwin_arm64\n",
        "            url: mysql/v0.6.1/mysql_darwin_arm64_v0.6.1.tar.gz\n",
    );
    let registry = StubServer::serving(routes(&[(INDEX_PATH, index.as_bytes().to_vec())]));
    let bucketfs = StubServer::serving(routes(&[]));

    let error = ensure_driver(&registry.as_registry(), &bucketfs.as_bucket(&mount))
        .expect_err("an index without a linux_amd64 package must fail provisioning");

    let message = error.to_string();
    assert!(
        message.contains("linux_amd64"),
        "the error must name the platform it looked for: {message}"
    );
    assert!(
        message.contains(DRIVER.as_str()),
        "the error must name the driver it resolved: {message}"
    );
    assert!(
        message.contains(RESOLVED_VERSION),
        "the error must name the version the range resolved to, rather than fall back to \
         a lower one that does publish the platform: {message}"
    );
    assert_eq!(
        registry.requested_paths(),
        vec![INDEX_PATH.to_string()],
        "no package may be downloaded once the index publishes none for this platform"
    );
}

#[test]
fn missing_platform_names_driver_and_version_for_postgresql() {
    let mount = MountDirectory::new();
    let index = concat!(
        "drivers:\n",
        "  - path: postgresql\n",
        "    pkginfo:\n",
        "      - version: 1.13.0\n",
        "        packages:\n",
        "          - platform: darwin_arm64\n",
        "            url: postgresql/1.13.0/postgresql_darwin_arm64_1.13.0.tar.gz\n",
    );
    let registry = StubServer::serving(routes(&[(INDEX_PATH, index.as_bytes().to_vec())]));
    let bucketfs = StubServer::serving(routes(&[]));

    let error = ensure_driver(
        &Registry::at(&registry.url(), Driver::Postgres),
        &bucketfs.as_bucket(&mount),
    )
    .expect_err("an index without a linux_amd64 package must fail provisioning");

    let message = error.to_string();
    assert!(
        message.contains("linux_amd64"),
        "the error must name the platform it looked for: {message}"
    );
    assert!(
        message.contains(Driver::Postgres.as_str()),
        "the error must name postgresql, the driver it resolved, not mysql: {message}"
    );
    assert!(
        message.contains(POSTGRESQL_RESOLVED_VERSION),
        "the error must name the version postgresql's own range resolved to: {message}"
    );
    assert_eq!(
        registry.requested_paths(),
        vec![INDEX_PATH.to_string()],
        "no package may be downloaded once the index publishes none for this platform"
    );
}

#[test]
fn bucketfs_write_failure_hides_credential() {
    let mount = MountDirectory::new();
    let registry = StubServer::serving(routes(&[
        (INDEX_PATH, index_publishing(&[PUBLISHED_NEWEST])),
        (
            &package_path(PUBLISHED_NEWEST),
            package_declaring(driver_library(), LIBRARY_BYTES),
        ),
    ]));
    let bucketfs = StubServer::serving(Arc::new(|_: &StubRequest| {
        StubReply::complete(403, b"forbidden".to_vec())
    }));

    let error = ensure_driver(&registry.as_registry(), &bucketfs.as_bucket(&mount))
        .expect_err("a BucketFS endpoint that rejects the write must fail provisioning");

    let message = error.to_string();
    assert!(
        message.contains("403"),
        "the error must report the status BucketFS returned: {message}"
    );
    assert!(
        !message.contains(WRITE_PASSWORD),
        "the error must not contain the credential it attempted: {message}"
    );
    assert!(
        !format!("{error:?}").contains(WRITE_PASSWORD),
        "the debug rendering must not contain the credential either: {error:?}"
    );

    let attempt = bucketfs
        .requests()
        .into_iter()
        .next()
        .expect("the driver upload must have been attempted");
    assert_eq!(
        attempt.method, "PUT",
        "the driver is written to BucketFS with a PUT"
    );
    assert!(
        attempt.authorization.is_some(),
        "the upload must authenticate with the BUCKETFS_CONNECTION credentials"
    );
}

#[test]
fn locked_bucketfs_path_is_retried_until_the_lock_clears() {
    let mount = MountDirectory::new();
    let registry = StubServer::serving(routes(&[
        (INDEX_PATH, index_publishing(&[PUBLISHED_NEWEST])),
        (
            &package_path(PUBLISHED_NEWEST),
            package_declaring(driver_library(), LIBRARY_BYTES),
        ),
    ]));
    let bucketfs = StubServer::serving(mount.accepting_writes_after_locking(2));

    let installed = ensure_driver(&registry.as_registry(), &bucketfs.as_bucket(&mount))
        .expect("a locked path is BucketFS asking to be retried, not a refusal");

    assert_eq!(installed.path, mount.driver_path(RESOLVED_VERSION));
    assert_eq!(
        bucketfs.requests().len(),
        3,
        "the upload must be retried once per lock and then accepted"
    );
}

#[test]
fn visibility_wait_times_out() {
    let mount = MountDirectory::new();
    let registry = StubServer::serving(routes(&[
        (INDEX_PATH, index_publishing(&[PUBLISHED_NEWEST])),
        (
            &package_path(PUBLISHED_NEWEST),
            package_declaring(driver_library(), LIBRARY_BYTES),
        ),
    ]));
    let bucketfs = StubServer::serving(Arc::new(|_: &StubRequest| {
        StubReply::complete(200, Vec::new())
    }));
    let deadline = Duration::from_millis(300);

    let started = Instant::now();
    let error = ensure_driver(
        &registry.as_registry(),
        &bucketfs.as_bucket(&mount).visible_within(deadline),
    )
    .expect_err("a driver that never appears on the mount must not provision");
    let waited = started.elapsed();

    let message = error.to_string();
    assert!(
        message.contains(&mount.driver_path(RESOLVED_VERSION).display().to_string()),
        "the error must name the path it waited on: {message}"
    );
    assert!(
        message.contains(&format!("{deadline:?}")),
        "the error must name the deadline that elapsed: {message}"
    );
    assert!(
        waited >= deadline,
        "the wait must run to its deadline before failing, not give up at once"
    );
}

#[test]
fn missing_driver_is_fetched_per_driver() {
    let mount = MountDirectory::new();
    let registry = StubServer::serving(routes(&[
        (INDEX_PATH, index_publishing(&[PUBLISHED_NEWEST])),
        (
            &package_path(PUBLISHED_NEWEST),
            package_declaring(driver_library(), LIBRARY_BYTES),
        ),
    ]));
    let bucketfs = StubServer::serving(mount.accepting_writes());

    let installed = ensure_driver(&registry.as_registry(), &bucketfs.as_bucket(&mount)).expect(
        "the registry serves this package with no digest in the index and a signature \
         that verifies against nothing, and it must still install: the download's server \
         certificate is the only integrity guarantee self-install has",
    );

    assert_eq!(installed.path, mount.driver_path(RESOLVED_VERSION));
    assert_eq!(
        installed.version.to_string(),
        RESOLVED_VERSION,
        "the version the range resolved to is reported back, because it is what \
         `adapterNotes.driverVersion` records and nothing compiled in can answer it"
    );
    assert_eq!(
        fs::read(&installed.path).expect("the installed driver must be readable"),
        LIBRARY_BYTES,
        "the bytes the archive holds must be the bytes BucketFS receives"
    );
    assert_eq!(
        bucketfs.requested_paths(),
        vec![driver_put_path(RESOLVED_VERSION)],
        "the driver must be written once, at the path the presence check reads, and the \
         package's license and signature members must not be written at all"
    );
}

#[test]
fn only_the_manifest_named_file_is_uploaded_whatever_the_license_file_is_spelled() {
    let mount = MountDirectory::new();
    let registry = StubServer::serving(routes(&[
        (
            INDEX_PATH,
            index_publishing_for(Driver::Postgres, &[POSTGRESQL_PUBLISHED]),
        ),
        (
            &package_path_for(Driver::Postgres, POSTGRESQL_PUBLISHED),
            package_declaring_with_license(
                POSTGRESQL_LICENSE_FILENAME,
                Driver::Postgres.package().library,
                POSTGRESQL_LIBRARY_BYTES,
            ),
        ),
    ]));
    let bucketfs = StubServer::serving(mount.accepting_writes());

    let installed = ensure_driver(
        &Registry::at(&registry.url(), Driver::Postgres),
        &bucketfs.as_bucket(&mount),
    )
    .expect(
        "postgresql's package, whose license file is spelled LICENSE.txt rather than \
         mysql's LICENSE, must still install",
    );

    assert_eq!(
        installed.path,
        mount.driver_path_for(Driver::Postgres, POSTGRESQL_RESOLVED_VERSION)
    );
    assert_eq!(
        bucketfs.requested_paths(),
        vec![driver_put_path_for(
            Driver::Postgres,
            POSTGRESQL_RESOLVED_VERSION
        )],
        "only the file the MANIFEST names as `driver` may be written to BucketFS; \
         LICENSE.txt — postgresql's differently-spelled license file — must never be \
         uploaded, the same as LICENSE never is for mysql"
    );
}

#[test]
fn two_drivers_install_side_by_side_at_distinct_driver_and_version_segments() {
    let mount = MountDirectory::new();
    let registry = StubServer::serving(routes(&[
        (
            INDEX_PATH,
            index_publishing_both_drivers(&[PUBLISHED_NEWEST], &[POSTGRESQL_PUBLISHED]),
        ),
        (
            &package_path(PUBLISHED_NEWEST),
            package_declaring(driver_library(), LIBRARY_BYTES),
        ),
        (
            &package_path_for(Driver::Postgres, POSTGRESQL_PUBLISHED),
            package_declaring_with_license(
                POSTGRESQL_LICENSE_FILENAME,
                Driver::Postgres.package().library,
                POSTGRESQL_LIBRARY_BYTES,
            ),
        ),
    ]));
    let bucketfs = StubServer::serving(mount.accepting_writes());

    let mysql_installed = ensure_driver(&registry.as_registry(), &bucketfs.as_bucket(&mount))
        .expect("mysql must install from a registry that also publishes postgresql");
    let postgres_installed = ensure_driver(
        &Registry::at(&registry.url(), Driver::Postgres),
        &bucketfs.as_bucket(&mount),
    )
    .expect("postgresql must install from a registry that also publishes mysql");

    assert_ne!(
        mysql_installed.path, postgres_installed.path,
        "the two drivers must install under distinct driver AND version path segments"
    );
    assert_eq!(mysql_installed.path, mount.driver_path(RESOLVED_VERSION));
    assert_eq!(
        postgres_installed.path,
        mount.driver_path_for(Driver::Postgres, POSTGRESQL_RESOLVED_VERSION)
    );

    assert_eq!(
        fs::read(&mysql_installed.path).expect("mysql's installed library must be readable"),
        LIBRARY_BYTES,
        "mysql's own bytes must survive postgresql's install untouched"
    );
    assert_eq!(
        fs::read(&postgres_installed.path)
            .expect("postgresql's installed library must be readable"),
        POSTGRESQL_LIBRARY_BYTES,
        "postgresql's own bytes must be its own, never mysql's"
    );

    assert_eq!(
        bucketfs.requested_paths(),
        vec![
            driver_put_path(RESOLVED_VERSION),
            driver_put_path_for(Driver::Postgres, POSTGRESQL_RESOLVED_VERSION),
        ],
        "each driver is written once, at its own path; neither driver's presence check is \
         satisfied by the other's file, and neither write overwrites the other's"
    );
}

/// Proves the stub registry is actually reachable -- the self-install route
/// hits it and installs successfully -- before trusting the request count
/// below as evidence the `DRIVER_PATH` route skipped it.
#[test]
fn the_driver_path_route_makes_no_registry_request() {
    let mount = MountDirectory::new();
    let directory = tempfile::TempDir::new().expect("a temporary directory must be creatable");
    let supplied = directory.path().join("operators-own-driver.so");
    fs::write(&supplied, b"\x7fELF").expect("the fixture library must be writable");

    let registry = StubServer::serving(routes(&[
        (
            INDEX_PATH,
            index_publishing_for(Driver::Postgres, &[POSTGRESQL_PUBLISHED]),
        ),
        (
            &package_path_for(Driver::Postgres, POSTGRESQL_PUBLISHED),
            package_declaring(Driver::Postgres.package().library, POSTGRESQL_LIBRARY_BYTES),
        ),
    ]));
    let bucketfs = StubServer::serving(mount.accepting_writes());

    ensure_driver(
        &Registry::at(&registry.url(), Driver::Postgres),
        &bucketfs.as_bucket(&mount),
    )
    .expect(
        "the stub registry must serve a real package so this test can prove the \
         driver-path route skips it, rather than merely being unable to reach an \
         unreachable one",
    );
    let requests_before = registry.requests().len();
    assert!(
        requests_before > 0,
        "the self-install route must have reached the stub registry for the request \
         count below to mean anything"
    );

    let path = supplied_driver(
        supplied.to_str().expect("the fixture path is UTF-8"),
        Driver::Postgres,
    )
    .expect(
        "a readable file at the supplied path must be used as it is, with no registry \
             involved at all",
    );

    assert_eq!(path, supplied);
    assert_eq!(
        registry.requests().len(),
        requests_before,
        "the DRIVER_PATH route must perform no HTTP request at all, to the registry the \
         self-install route already proved is listening: {:?}",
        registry.requests()
    );
}

#[test]
fn self_install_writes_only_the_versioned_path_and_leaves_a_supplied_library_alone() {
    let mount = MountDirectory::new();
    place_supplied_library(&mount, DRIVER, SUPPLIED_LIBRARY_BYTES);
    place_supplied_library(&mount, Driver::Postgres, POSTGRESQL_SUPPLIED_LIBRARY_BYTES);

    let registry = StubServer::serving(routes(&[
        (
            INDEX_PATH,
            index_publishing_both_drivers(&[PUBLISHED_NEWEST], &[POSTGRESQL_PUBLISHED]),
        ),
        (
            &package_path(PUBLISHED_NEWEST),
            package_declaring(driver_library(), LIBRARY_BYTES),
        ),
        (
            &package_path_for(Driver::Postgres, POSTGRESQL_PUBLISHED),
            package_declaring_with_license(
                POSTGRESQL_LICENSE_FILENAME,
                Driver::Postgres.package().library,
                POSTGRESQL_LIBRARY_BYTES,
            ),
        ),
    ]));
    let bucketfs = StubServer::serving(mount.accepting_writes());

    let mysql_installed = ensure_driver(&registry.as_registry(), &bucketfs.as_bucket(&mount))
        .expect("mysql must self-install while a supplied mysql library sits beside it");
    let postgres_installed = ensure_driver(
        &Registry::at(&registry.url(), Driver::Postgres),
        &bucketfs.as_bucket(&mount),
    )
    .expect("postgresql must self-install while a supplied postgresql library sits beside it");

    assert_eq!(mysql_installed.path, mount.driver_path(RESOLVED_VERSION));
    assert_eq!(
        postgres_installed.path,
        mount.driver_path_for(Driver::Postgres, POSTGRESQL_RESOLVED_VERSION)
    );

    assert_eq!(
        fs::read(mount.supplied_path_for(DRIVER))
            .expect("the supplied mysql library must still be readable"),
        SUPPLIED_LIBRARY_BYTES,
        "self-install must leave a pre-placed mysql library's bytes untouched, byte for byte"
    );
    assert_eq!(
        fs::read(mount.supplied_path_for(Driver::Postgres))
            .expect("the supplied postgresql library must still be readable"),
        POSTGRESQL_SUPPLIED_LIBRARY_BYTES,
        "self-install must leave a pre-placed postgresql library's bytes untouched, byte for \
         byte"
    );

    assert_eq!(
        bucketfs.requested_paths(),
        vec![
            driver_put_path(RESOLVED_VERSION),
            driver_put_path_for(Driver::Postgres, POSTGRESQL_RESOLVED_VERSION),
        ],
        "each driver must write exactly once, at its own versioned path, never at either \
         driver's supplied location -- this is what separates \"did not overwrite\" from \
         \"overwrote with identical bytes\""
    );
}

/// Proves the stub registry is actually reachable -- self-install hits it
/// and installs successfully -- before trusting the request count below as
/// evidence the default-location route skipped it.
#[test]
fn the_default_location_route_makes_no_registry_request() {
    let mount = MountDirectory::new();
    let registry = StubServer::serving(routes(&[
        (INDEX_PATH, index_publishing(&[PUBLISHED_NEWEST])),
        (
            &package_path(PUBLISHED_NEWEST),
            package_declaring(driver_library(), LIBRARY_BYTES),
        ),
    ]));
    let bucketfs = StubServer::serving(mount.accepting_writes());

    ensure_driver(&registry.as_registry(), &bucketfs.as_bucket(&mount)).expect(
        "the stub registry must serve a real package so this test can prove the default \
         location route skips it, rather than merely being unable to reach an unreachable \
         one",
    );
    let requests_before = registry.requests().len();
    assert!(
        requests_before > 0,
        "self-install must have reached the stub registry for the request count below to \
         mean anything"
    );
    let bucketfs_requests_before = bucketfs.requests().len();

    let default_path = default_driver_path(mount.path(), DRIVER);
    assert!(
        readable_library(&default_path).is_err(),
        "an empty default location must not be readable"
    );

    fs::create_dir_all(
        default_path
            .parent()
            .expect("the default path has a directory"),
    )
    .expect("the default directory must be creatable");
    fs::write(&default_path, b"\x7fELF a pre-placed driver")
        .expect("the fixture library at the default location must be writable");
    assert!(
        readable_library(&default_path).is_ok(),
        "a readable file at the default location must be usable"
    );

    assert_eq!(
        registry.requests().len(),
        requests_before,
        "neither resolving the default location nor checking whether it is readable may \
         perform an HTTP request, to the registry self-install already proved is listening: \
         {:?}",
        registry.requests()
    );
    assert_eq!(
        bucketfs.requests().len(),
        bucketfs_requests_before,
        "neither resolving the default location nor checking whether it is readable may \
         write to BucketFS: {:?}",
        bucketfs.requests()
    );
}

#[test]
fn registry_unreachability_is_told_from_a_registry_that_answered() {
    let mount = MountDirectory::new();
    let bucketfs = StubServer::serving(routes(&[]));

    let closed = closed_port();
    let unreachable = ensure_driver(
        &Registry::at(&format!("http://127.0.0.1:{closed}"), DRIVER),
        &bucketfs.as_bucket(&mount),
    )
    .expect_err("a registry nothing answers for must fail provisioning");
    assert!(
        unreachable.registry_unreachable().is_some(),
        "a registry nothing answers for is the one failure that is evidence of an air gap: \
         {unreachable}"
    );

    let answering_500 = StubServer::serving(Arc::new(|_: &StubRequest| {
        StubReply::complete(500, Vec::new())
    }));
    let rejected = ensure_driver(
        &Registry::at(&answering_500.url(), DRIVER),
        &bucketfs.as_bucket(&mount),
    )
    .expect_err("a registry answering HTTP 500 must fail provisioning");
    assert!(
        rejected.registry_unreachable().is_none(),
        "a registry that answered any HTTP status is not an unreachable one: {rejected}"
    );

    let no_in_range_version = StubServer::serving(routes(&[(
        INDEX_PATH,
        index_publishing(&[PUBLISHED_OUT_OF_RANGE]),
    )]));
    let unsatisfied = ensure_driver(
        &Registry::at(&no_in_range_version.url(), DRIVER),
        &bucketfs.as_bucket(&mount),
    )
    .expect_err("an index publishing no in-range version must fail provisioning");
    assert!(
        unsatisfied.registry_unreachable().is_none(),
        "an index that answered, but with no version the range accepts, is not an \
         unreachable registry: {unsatisfied}"
    );
}

#[test]
fn a_package_download_that_failed_mid_transfer_is_not_an_unreachable_registry() {
    let mount = MountDirectory::new();
    let bucketfs = StubServer::serving(routes(&[]));

    let index = index_publishing(&[PUBLISHED_NEWEST]);
    let package_route = package_path(PUBLISHED_NEWEST);
    let full_package = package_declaring(driver_library(), LIBRARY_BYTES);
    let truncated_package = full_package[..full_package.len() / 2].to_vec();
    let declared_length = full_package.len();

    let registry = StubServer::serving(Arc::new(move |request: &StubRequest| {
        if request.path == INDEX_PATH {
            StubReply::complete(200, index.clone())
        } else if request.path == package_route {
            StubReply {
                status: 200,
                body: truncated_package.clone(),
                declared_length: Some(declared_length),
            }
        } else {
            StubReply::complete(404, b"not found".to_vec())
        }
    }));

    let error = ensure_driver(&registry.as_registry(), &bucketfs.as_bucket(&mount)).expect_err(
        "a package download whose body stops arriving after its status did must fail \
         provisioning",
    );
    assert!(
        error.registry_unreachable().is_none(),
        "the registry answered the index and the version range resolved -- a package \
         download that ran after the versioned presence check is not evidence of an \
         unreachable registry: {error}"
    );
    assert!(
        error
            .to_string()
            .contains("stopped arriving while provisioning"),
        "a truncated package body must reach the interrupted-transfer variant rather than \
         any other refusal: {error}"
    );
}

#[test]
fn an_index_read_that_failed_mid_transfer_is_not_an_unreachable_registry() {
    let mount = MountDirectory::new();
    let bucketfs = StubServer::serving(routes(&[]));

    let full_index = index_publishing(&[PUBLISHED_NEWEST]);
    let truncated_index = full_index[..full_index.len() / 2].to_vec();
    let declared_length = full_index.len();

    let registry = StubServer::serving(Arc::new(move |_: &StubRequest| StubReply {
        status: 200,
        body: truncated_index.clone(),
        declared_length: Some(declared_length),
    }));

    let error = ensure_driver(&registry.as_registry(), &bucketfs.as_bucket(&mount)).expect_err(
        "an index read whose body stops arriving after its status did must fail provisioning",
    );
    assert!(
        error.registry_unreachable().is_none(),
        "a host that answered a status line before the connection dropped is not evidence \
         of an unreachable registry -- nothing was resolved and nothing was installed, but \
         something answered: {error}"
    );
    assert!(
        error
            .to_string()
            .contains("stopped arriving while provisioning"),
        "a truncated index body must reach the interrupted-transfer variant rather than \
         any other refusal: {error}"
    );
}

// ---------------------------------------------------------------------------
// The BucketFS mount a UDF reads, standing in for /buckets/<service>/<bucket>
// ---------------------------------------------------------------------------

struct MountDirectory {
    root: tempfile::TempDir,
}

impl MountDirectory {
    fn new() -> Self {
        Self {
            root: tempfile::TempDir::new().expect("a mount directory must be creatable"),
        }
    }

    fn path(&self) -> &Path {
        self.root.path()
    }

    /// Where the mount exposes one version of the scenario's own driver
    /// (`DRIVER`, MySQL).
    fn driver_path(&self, version: &str) -> PathBuf {
        self.driver_path_for(DRIVER, version)
    }

    /// Where the mount exposes one version of an explicitly named driver's
    /// library, so a scenario provisioning more than one technology can
    /// address each one's own path.
    fn driver_path_for(&self, driver: Driver, version: &str) -> PathBuf {
        self.root
            .path()
            .join("adbc-drivers")
            .join(driver.as_str())
            .join(version)
            .join(driver.package().library)
    }

    /// Where the mount exposes an operator pre-placed library for `driver`,
    /// with no version segment — the location [`default_driver_path`]
    /// computes and self-install must never write to.
    fn supplied_path_for(&self, driver: Driver) -> PathBuf {
        default_driver_path(self.root.path(), driver)
    }

    fn install(&self, version: &str, bytes: &[u8]) {
        let path = self.driver_path(version);
        fs::create_dir_all(path.parent().expect("the driver path has a directory"))
            .expect("the driver directory must be creatable");
        fs::write(path, bytes).expect("the driver must be writable");
    }

    /// A BucketFS stub that stores what it is sent where the mount exposes it,
    /// which is what the real endpoint does once replication catches up.
    ///
    /// It writes at the path the request names rather than one fixed when the
    /// handler was built, so a scenario installing a second version ends up
    /// with two files rather than the first one written over.
    fn accepting_writes(&self) -> Handler {
        let root = self.root.path().to_owned();
        Arc::new(move |request: &StubRequest| {
            let written = root.join(
                request
                    .path
                    .strip_prefix(BUCKET_PATH)
                    .unwrap_or(&request.path)
                    .trim_start_matches('/'),
            );
            fs::create_dir_all(written.parent().expect("the driver path has a directory"))
                .expect("the driver directory must be creatable");
            fs::write(&written, &request.body).expect("the uploaded driver must be writable");
            StubReply::complete(200, Vec::new())
        })
    }
    /// A BucketFS stub that answers `locked` requests with HTTP 423 before it
    /// accepts anything, which is what the real endpoint does while it is
    /// still finishing an earlier write or delete of the same object.
    fn accepting_writes_after_locking(&self, locked: usize) -> Handler {
        let accepting = self.accepting_writes();
        let remaining = Arc::new(Mutex::new(locked));
        Arc::new(move |request: &StubRequest| {
            let mut remaining = remaining.lock().expect("the lock counter is not poisoned");
            if *remaining > 0 {
                *remaining -= 1;
                return StubReply::complete(423, b"locked".to_vec());
            }
            accepting(request)
        })
    }
}

/// Writes `bytes` at `mount`'s default location for `driver`, as an operator
/// pre-placing a library there would, so self-install can be proven to leave
/// it alone.
fn place_supplied_library(mount: &MountDirectory, driver: Driver, bytes: &[u8]) {
    let path = mount.supplied_path_for(driver);
    fs::create_dir_all(path.parent().expect("the supplied path has a directory"))
        .expect("the supplied directory must be creatable");
    fs::write(path, bytes).expect("the supplied library must be writable");
}

// ---------------------------------------------------------------------------
// Package and index fixtures
// ---------------------------------------------------------------------------

/// Where the stub registry publishes one version's `linux_amd64` package, for
/// the scenario's own driver (`DRIVER`, MySQL).
fn package_path(published: &str) -> String {
    package_path_for(DRIVER, published)
}

/// Where the stub registry publishes one version's `linux_amd64` package, for
/// an explicitly named driver.
fn package_path_for(driver: Driver, published: &str) -> String {
    let path = driver.as_str();
    format!("/{path}/{published}/{path}_linux_amd64_{published}.tar.gz")
}

/// Where BucketFS receives one version of the scenario's own driver's library
/// (`DRIVER`, MySQL), as the write endpoint sees the path.
fn driver_put_path(version: &str) -> String {
    driver_put_path_for(DRIVER, version)
}

/// Where BucketFS receives one version of an explicitly named driver's
/// library, as the write endpoint sees the path.
fn driver_put_path_for(driver: Driver, version: &str) -> String {
    format!(
        "{BUCKET_PATH}/adbc-drivers/{}/{version}/{}",
        driver.as_str(),
        driver.package().library
    )
}

/// An index publishing every given version of the scenario's own driver
/// (`DRIVER`, MySQL), each with the `linux_amd64` package [`package_path`]
/// locates.
fn index_publishing(published: &[&str]) -> Vec<u8> {
    index_publishing_for(DRIVER, published)
}

/// An index publishing every given version of one named driver, each with the
/// `linux_amd64` package [`package_path_for`] locates.
fn index_publishing_for(driver: Driver, published: &[&str]) -> Vec<u8> {
    format!("drivers:\n{}", driver_block(driver, published)).into_bytes()
}

/// An index publishing both drivers side by side, in the one document shape
/// the real registry actually serves: a single `drivers` list with a block
/// per technology, rather than one document per driver.
fn index_publishing_both_drivers(
    mysql_published: &[&str],
    postgresql_published: &[&str],
) -> Vec<u8> {
    format!(
        "drivers:\n{}{}",
        driver_block(Driver::MySql, mysql_published),
        driver_block(Driver::Postgres, postgresql_published)
    )
    .into_bytes()
}

/// One driver's entry within a registry index document: its `path` and every
/// given published version, each with the `linux_amd64` package
/// [`package_path_for`] locates.
fn driver_block(driver: Driver, published: &[&str]) -> String {
    let mut block = format!("  - path: {}\n    pkginfo:\n", driver.as_str());
    for version in published {
        block.push_str(&format!(
            "      - version: {version}\n        packages:\n          - platform: \
             linux_amd64\n            url: {}\n",
            package_path_for(driver, version).trim_start_matches('/')
        ));
    }
    block
}

/// A detached signature no key adbc-vs holds could ever verify, of the shape
/// the published packages carry one in.
///
/// Every scenario's package carries it, because that is what the registry
/// actually publishes and because installing anyway is the assertion: nothing
/// on the install path reads it, and nothing may start to.
const UNVERIFIABLE_SIGNATURE: &[u8] =
    b"-----BEGIN PGP SIGNATURE-----\nnot a signature\n-----END PGP SIGNATURE-----\n";

/// A gzipped tar package whose `MANIFEST` declares `library` as its driver,
/// alongside a `LICENSE` and a signature file a published package also holds
/// and self-install must leave behind.
fn package_declaring(library: &str, bytes: &[u8]) -> Vec<u8> {
    package_declaring_with_license("LICENSE", library, bytes)
}

/// [`package_declaring`], but with the license member named as given rather
/// than assumed to be `LICENSE` — the PostgreSQL package's own is
/// `LICENSE.txt`, confirmed by gate measurement, and self-install must leave
/// it behind exactly as it leaves MySQL's `LICENSE` behind, whatever it is
/// called.
fn package_declaring_with_license(license_name: &str, library: &str, bytes: &[u8]) -> Vec<u8> {
    let manifest = format!("[Files]\ndriver = \"{library}\"\n");
    let signature_name = format!("{library}.asc");
    let members: [(&str, &[u8]); 4] = [
        (license_name, b"Apache-2.0"),
        ("MANIFEST", manifest.as_bytes()),
        (&signature_name, UNVERIFIABLE_SIGNATURE),
        (library, bytes),
    ];

    let mut builder = tar::Builder::new(GzEncoder::new(Vec::new(), Compression::fast()));
    for (name, content) in members {
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Regular);
        header.set_mode(0o644);
        header.set_size(content.len() as u64);
        builder
            .append_data(&mut header, name, content)
            .expect("appending an archive member must succeed");
    }
    builder
        .into_inner()
        .expect("finishing the tar stream must succeed")
        .finish()
        .expect("finishing the gzip stream must succeed")
}

// ---------------------------------------------------------------------------
// A stub HTTP server, and a TLS server no root store signs for
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct StubRequest {
    method: String,
    path: String,
    authorization: Option<String>,
    body: Vec<u8>,
}

struct StubReply {
    status: u16,
    body: Vec<u8>,
    /// Overrides the declared `Content-Length` past `body.len()`, so a
    /// handler can announce a body it never finishes sending — the shape a
    /// connection dropped mid-transfer takes on the wire. `None` for every
    /// ordinary reply, whose declared length is exactly `body.len()`.
    declared_length: Option<usize>,
}

impl StubReply {
    /// A reply whose declared `Content-Length` matches the body sent in
    /// full — every reply this file served before a mid-transfer failure
    /// needed expressing at all.
    fn complete(status: u16, body: Vec<u8>) -> Self {
        Self {
            status,
            body,
            declared_length: None,
        }
    }
}

type Handler = Arc<dyn Fn(&StubRequest) -> StubReply + Send + Sync>;

/// Answers the given bodies at the given paths, and 404 everywhere else.
fn routes(paths: &[(&str, Vec<u8>)]) -> Handler {
    let table: HashMap<String, Vec<u8>> = paths
        .iter()
        .map(|(path, body)| ((*path).to_owned(), body.clone()))
        .collect();

    Arc::new(
        move |request: &StubRequest| match table.get(&request.path) {
            Some(body) => StubReply::complete(200, body.clone()),
            None => StubReply::complete(404, b"not found".to_vec()),
        },
    )
}

/// The request log and shutdown flag a running [`StubServer`] and its worker
/// thread share, held behind one `Arc` so both sides mutate the same state.
struct ServerState {
    requests: Mutex<Vec<StubRequest>>,
    stopped: AtomicBool,
}

struct StubServer {
    port: u16,
    shared: Arc<ServerState>,
    worker: Option<JoinHandle<()>>,
}

impl StubServer {
    fn serving(handler: Handler) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a stub server must be bindable");
        let port = listener
            .local_addr()
            .expect("a bound listener has an address")
            .port();
        listener
            .set_nonblocking(true)
            .expect("a stub listener must poll rather than block its shutdown");

        let shared = Arc::new(ServerState {
            requests: Mutex::new(Vec::new()),
            stopped: AtomicBool::new(false),
        });
        let worker = thread::spawn({
            let shared = Arc::clone(&shared);
            move || Self::serve(&listener, &handler, &shared)
        });

        Self {
            port,
            shared,
            worker: Some(worker),
        }
    }

    fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    fn as_registry(&self) -> Registry {
        Registry::at(&self.url(), DRIVER)
    }

    fn as_bucket(&self, mount: &MountDirectory) -> Bucket {
        Bucket::new(
            &format!("{}{BUCKET_PATH}", self.url()),
            mount.path(),
            Credentials::new(WRITE_USER, WRITE_PASSWORD),
        )
    }

    fn requests(&self) -> Vec<StubRequest> {
        self.shared
            .requests
            .lock()
            .expect("the request log is not poisoned")
            .clone()
    }

    fn requested_paths(&self) -> Vec<String> {
        self.requests()
            .into_iter()
            .map(|request| request.path)
            .collect()
    }

    fn serve(listener: &TcpListener, handler: &Handler, shared: &ServerState) {
        while !shared.stopped.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    stream
                        .set_nonblocking(false)
                        .expect("an accepted connection reads blocking");
                    let Some(request) = read_request(&mut stream) else {
                        continue;
                    };
                    shared
                        .requests
                        .lock()
                        .expect("the request log is not poisoned")
                        .push(request.clone());
                    let reply = handler(&request);
                    write_reply(&mut stream, &reply);
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(2));
                }
                Err(_) => return,
            }
        }
    }
}

impl Drop for StubServer {
    fn drop(&mut self) {
        self.shared.stopped.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn read_request(stream: &mut TcpStream) -> Option<StubRequest> {
    let mut head = Vec::new();
    let mut byte = [0_u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        match stream.read(&mut byte) {
            Ok(0) | Err(_) => return None,
            Ok(_) => head.push(byte[0]),
        }
    }

    let head = String::from_utf8_lossy(&head).into_owned();
    let mut lines = head.lines();
    let mut start = lines.next()?.split(' ');
    let method = start.next()?.to_owned();
    let path = start.next()?.to_owned();

    let mut length = 0_usize;
    let mut authorization = None;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        match name.trim().to_ascii_lowercase().as_str() {
            "content-length" => length = value.trim().parse().unwrap_or(0),
            "authorization" => authorization = Some(value.trim().to_owned()),
            _ => {}
        }
    }

    let mut body = vec![0_u8; length];
    stream.read_exact(&mut body).ok()?;

    Some(StubRequest {
        method,
        path,
        authorization,
        body,
    })
}

fn write_reply(stream: &mut TcpStream, reply: &StubReply) {
    let length = reply.declared_length.unwrap_or(reply.body.len());
    let head = format!(
        "HTTP/1.1 {} STUB\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n",
        reply.status
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(&reply.body);
    let _ = stream.flush();
}

/// A port nothing listens on: bound to learn its number, then released.
fn closed_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a port must be bindable");
    listener
        .local_addr()
        .expect("a bound listener has an address")
        .port()
}

/// How many requests reached the endpoint behind the certificate, and the
/// shutdown flag, shared between a running [`UntrustedTlsServer`] and its
/// worker thread behind one `Arc`.
struct TlsState {
    served: Mutex<usize>,
    stopped: AtomicBool,
}

/// A TLS endpoint presenting a certificate no root store signs for, which is
/// what a host impersonating the registry would present.
///
/// It serves a valid registry index to any client that completes the
/// handshake, so a client that would accept the certificate gets a working
/// registry rather than a broken one. What it counts is how many requests it
/// was ever asked to serve.
struct UntrustedTlsServer {
    port: u16,
    shared: Arc<TlsState>,
    worker: Option<JoinHandle<()>>,
}

impl UntrustedTlsServer {
    fn start() -> Self {
        let certified = rcgen::generate_simple_self_signed(vec!["127.0.0.1".to_owned()])
            .expect("a self-signed certificate must be generatable");
        let certificate = certified.cert.der().clone();
        let key =
            rustls::pki_types::PrivatePkcs8KeyDer::from(certified.signing_key.serialize_der());
        let config = Arc::new(
            rustls::ServerConfig::builder()
                .with_no_client_auth()
                .with_single_cert(vec![certificate], key.into())
                .expect("a generated certificate and its own key must pair"),
        );

        let listener = TcpListener::bind("127.0.0.1:0").expect("a TLS server must be bindable");
        let port = listener
            .local_addr()
            .expect("a bound listener has an address")
            .port();
        listener
            .set_nonblocking(true)
            .expect("a TLS listener must poll rather than block its shutdown");

        let shared = Arc::new(TlsState {
            served: Mutex::new(0),
            stopped: AtomicBool::new(false),
        });
        let worker = thread::spawn({
            let shared = Arc::clone(&shared);
            move || Self::present_certificate(&listener, &config, &shared)
        });

        Self {
            port,
            shared,
            worker: Some(worker),
        }
    }

    fn url(&self) -> String {
        format!("https://127.0.0.1:{}", self.port)
    }

    /// How many requests reached the endpoint behind the certificate.
    fn served_requests(&self) -> usize {
        *self
            .shared
            .served
            .lock()
            .expect("the served count is not poisoned")
    }

    /// Offers the certificate to the first client that connects and, if that
    /// client accepts it, answers the request it then sends.
    fn present_certificate(
        listener: &TcpListener,
        config: &Arc<rustls::ServerConfig>,
        shared: &TlsState,
    ) {
        while !shared.stopped.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    stream
                        .set_nonblocking(false)
                        .expect("an accepted connection handshakes blocking");
                    let Ok(mut connection) = rustls::ServerConnection::new(Arc::clone(config))
                    else {
                        return;
                    };
                    let mut tls = rustls::Stream::new(&mut connection, &mut stream);

                    let mut head = Vec::new();
                    let mut byte = [0_u8; 1];
                    while !head.ends_with(b"\r\n\r\n") {
                        match tls.read(&mut byte) {
                            Ok(0) | Err(_) => return,
                            Ok(_) => head.push(byte[0]),
                        }
                    }

                    *shared
                        .served
                        .lock()
                        .expect("the served count is not poisoned") += 1;
                    let index = index_publishing(&[PUBLISHED_NEWEST]);
                    let reply = format!(
                        "HTTP/1.1 200 STUB\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        index.len()
                    );
                    let _ = tls.write_all(reply.as_bytes());
                    let _ = tls.write_all(&index);
                    let _ = tls.flush();
                    return;
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(2));
                }
                Err(_) => return,
            }
        }
    }
}

impl Drop for UntrustedTlsServer {
    fn drop(&mut self) {
        self.shared.stopped.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// The one check in this repository that reaches the public driver registry.
///
/// `select_package` against a fixture index can pass forever after the real
/// registry stops publishing anything a declared range accepts, because the
/// fixture is this repo's own data and never drifts on its own. Only reading
/// what the CDN publishes today can catch a range that has gone unsatisfiable
/// — and only that reading needs the network, which is why it sits behind the
/// default-off `registry-live` feature and no CI job: a CDN outage or a slow
/// DNS answer must not turn the fast tier red for a reason unrelated to the
/// commit.
#[cfg(feature = "registry-live")]
mod registry_live {
    use adbc_vs_adapter::registry_index;
    use adbc_vs_scan::driver::{Driver, PLATFORM, REGISTRY_BASE_URL};

    const INDEX_FILE: &str = "index.yaml";

    #[test]
    fn each_drivers_range_resolves_against_the_published_index() {
        let index_url = registry_index::resolve_url(REGISTRY_BASE_URL, INDEX_FILE);
        let index = reqwest::blocking::get(&index_url)
            .and_then(reqwest::blocking::Response::error_for_status)
            .and_then(reqwest::blocking::Response::text)
            .unwrap_or_else(|error| {
                panic!("the public registry index at {index_url} must be readable: {error}")
            });

        for driver in Driver::ALL {
            let package = driver.package();
            let resolved = registry_index::resolve_package(
                &index,
                package.registry_path,
                package.version_range,
                PLATFORM,
                REGISTRY_BASE_URL,
            )
            .unwrap_or_else(|error| {
                panic!(
                    "the published index must carry a `{PLATFORM}` package of `{}` satisfying \
                     the range `{}` adbc-vs declares for it: {error:?}",
                    driver.as_str(),
                    package.version_range
                )
            });

            assert!(
                resolved.url.starts_with(REGISTRY_BASE_URL),
                "`{}` must resolve to a package the registry itself serves: {}",
                driver.as_str(),
                resolved.url
            );
        }
    }
}
