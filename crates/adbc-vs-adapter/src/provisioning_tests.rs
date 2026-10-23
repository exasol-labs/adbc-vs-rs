use std::io::Write;
use std::path::Path;
use std::time::Duration;

use flate2::write::GzEncoder;
use flate2::Compression;

use super::*;

const FIXTURE_INDEX: &str = include_str!("../tests/fixtures/index.yaml");

/// Builds a gzipped tar archive of `(name, content)` members, the shape the
/// driver registry publishes packages in.
fn archive(members: &[(&str, &[u8])]) -> Vec<u8> {
    let declared: Vec<(&str, &[u8], u64)> = members
        .iter()
        .map(|(name, content)| (*name, *content, content.len() as u64))
        .collect();
    archive_declaring(&declared)
}

/// Builds a gzipped tar archive whose members declare `size` in their headers
/// rather than the length of the content that follows.
///
/// A header is what an unpacker learns a member's size from before it reads a
/// byte of it, so declaring one larger than the content is how a hostile
/// archive asks for more than it sends — and the only way to drive the
/// unpacked-size bound without building a half-gigabyte fixture.
///
/// Member names are written into the header directly, because the builder
/// refuses to write the `..` an archive would have to carry to attempt a path
/// traversal.
fn archive_declaring(members: &[(&str, &[u8], u64)]) -> Vec<u8> {
    let mut builder = tar::Builder::new(GzEncoder::new(Vec::new(), Compression::fast()));
    for (name, content, size) in members {
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Regular);
        header.set_mode(0o644);
        header.set_size(*size);
        let raw = &mut header
            .as_gnu_mut()
            .expect("a GNU header must render as one")
            .name;
        raw[..name.len()].copy_from_slice(name.as_bytes());
        header.set_cksum();
        builder
            .append(&header, *content)
            .expect("appending an archive member must succeed");
    }
    builder
        .into_inner()
        .expect("finishing the tar stream must succeed")
        .finish()
        .expect("finishing the gzip stream must succeed")
}

/// A package whose `MANIFEST` names `filename` as the driver library.
fn package_naming(filename: &str, library: &[u8]) -> Vec<u8> {
    let manifest = format!("[Files]\ndriver = \"{filename}\"\n");
    archive(&[
        ("LICENSE", b"Apache-2.0".as_slice()),
        ("MANIFEST", manifest.as_bytes()),
        (filename, library),
    ])
}

const PACKAGE_URL: &str = "https://registry.invalid/mysql/v0.6.0/mysql_linux_amd64_v0.6.0.tar.gz";

#[test]
fn selects_the_highest_in_range_versions_platform_package() {
    let resolved = select_package(FIXTURE_INDEX, &Registry::public(Driver::MySql))
        .expect("the fixture index publishes two mysql versions inside the declared range");

    assert_eq!(
        resolved.url,
        format!("https://dbc-cdn.columnar.tech/mysql/0.6.1/mysql_{PLATFORM}_0.6.1.tar.gz")
    );
}

/// The two drivers' ranges are independent data, not one rule applied to both.
///
/// MySQL sits below 1.0, where a minor bump is breaking by SemVer convention,
/// so its declared range stops at the very next minor: `v0.7.0` is published
/// and excluded. A rule derived from that same "stop at the next minor"
/// shape would forbid PostgreSQL's own `1.13.0` just as it forbids MySQL's
/// `v0.7.0` — but PostgreSQL's declared range instead admits every `1.x`
/// through `<2.0.0`, so `1.13.0` resolves. That only holds because each
/// driver's range is stated as its own fact in its own `drivers/*.toml`,
/// never derived from the other's.
#[test]
fn ranges_are_independent_rather_than_one_shared_minor_bump_rule() {
    let mysql = select_package(FIXTURE_INDEX, &Registry::public(Driver::MySql))
        .expect("the fixture index publishes an in-range mysql version");
    assert_eq!(
        mysql.version.to_string(),
        "0.6.1",
        "MySQL's declared range excludes the published v0.7.0, the next minor bump"
    );

    let postgresql = select_package(FIXTURE_INDEX, &Registry::public(Driver::Postgres))
        .expect("the fixture index publishes an in-range postgresql version");
    assert_eq!(
        postgresql.version.to_string(),
        "1.13.0",
        "PostgreSQL's declared range admits a minor bump beyond 1.12.0 — the very kind \
         of bump MySQL's own range excludes — because the two ranges are independent \
         data rather than one shared rule"
    );
}

#[test]
fn unparseable_index_names_the_index_url() {
    let error = select_package("drivers: [", &Registry::public(Driver::MySql))
        .expect_err("a truncated document must not parse");

    assert!(
        error.to_string().contains("index.yaml"),
        "the error must name the document it could not read: {error}"
    );
}

#[test]
fn index_without_the_driver_names_the_driver_path() {
    let index = "drivers:\n  - path: postgresql\n    pkginfo: []\n";

    let error = select_package(index, &Registry::public(Driver::MySql))
        .expect_err("an index without the mysql driver must fail");

    assert!(
        error.to_string().contains("mysql"),
        "the error must name the driver it looked for: {error}"
    );
}

#[test]
fn unsatisfiable_range_names_driver_and_published_versions() {
    let index = concat!(
        "drivers:\n",
        "  - path: mysql\n",
        "    pkginfo:\n",
        "      - version: v0.5.0\n",
        "        packages: []\n",
        "      - version: v0.9.0\n",
        "        packages: []\n",
    );

    let error = select_package(index, &Registry::public(Driver::MySql))
        .expect_err("an index publishing nothing inside the range must fail");

    let message = error.to_string();
    assert!(
        message.contains("mysql"),
        "the error must name the driver it resolved: {message}"
    );
    assert!(
        message.contains(Driver::MySql.package().version_range),
        "the error must name the range that went unsatisfied: {message}"
    );
    assert!(
        message.contains("v0.5.0") && message.contains("v0.9.0"),
        "the error must name every version the index does publish: {message}"
    );
}

#[test]
fn an_index_publishing_no_version_at_all_still_reports_the_range() {
    let index = "drivers:\n  - path: mysql\n    pkginfo: []\n";

    let error = select_package(index, &Registry::public(Driver::MySql))
        .expect_err("an index publishing no version of the driver must fail");

    let message = error.to_string();
    assert!(
        message.contains(Driver::MySql.package().version_range),
        "the error must name the range even when there is nothing to compare it to: {message}"
    );
}

#[test]
fn version_without_the_platform_package_names_platform_and_version() {
    #[cfg(target_arch = "x86_64")]
    const OTHER_LINUX_PLATFORM: &str = "linux_arm64";
    #[cfg(target_arch = "aarch64")]
    const OTHER_LINUX_PLATFORM: &str = "linux_amd64";
    let index = format!(
        "drivers:\n  - path: mysql\n    pkginfo:\n      - version: v0.6.0\n        packages:\n\
         \x20         - platform: darwin_arm64\n            url: mysql/v0.6.0/mysql_darwin_arm64_v0.6.0.tar.gz\n\
         \x20         - platform: {OTHER_LINUX_PLATFORM}\n            url: mysql/v0.6.0/mysql_other_v0.6.0.tar.gz\n"
    );

    let error = select_package(&index, &Registry::public(Driver::MySql))
        .expect_err("a version without the compile target's package must fail");

    let message = error.to_string();
    assert!(
        message.contains(PLATFORM),
        "the error must name the platform it looked for: {message}"
    );
    assert!(
        message.contains("0.6.0"),
        "the error must name the version the range resolved to, which is where it \
         looked: {message}"
    );
}

#[test]
fn extraction_returns_the_library_the_manifest_names() {
    let package = package_naming("libadbc_driver_mysql.so", b"ELF");

    let driver = extract_driver(&package, PACKAGE_URL).expect("a well-formed package must extract");

    assert_eq!(driver.filename, "libadbc_driver_mysql.so");
    assert_eq!(
        std::fs::read(&driver.path).expect("the extracted library must be readable"),
        b"ELF"
    );
}

#[test]
fn extraction_ignores_a_member_the_manifest_does_not_name() {
    let package = archive(&[
        ("MANIFEST", b"[Files]\ndriver = \"lib.so\"\n".as_slice()),
        ("lib.so", b"ELF".as_slice()),
        ("lib.so.sig", b"signature".as_slice()),
    ]);

    let driver = extract_driver(&package, PACKAGE_URL).expect("a well-formed package must extract");

    assert_eq!(driver.filename, "lib.so");
}

#[test]
fn extraction_without_a_manifest_names_the_package_url() {
    let package = archive(&[("libadbc_driver_mysql.so", b"ELF".as_slice())]);

    let error = extract_driver(&package, PACKAGE_URL)
        .expect_err("a package without a MANIFEST must not extract");

    assert!(
        error.to_string().contains(PACKAGE_URL),
        "the error must name the package it read: {error}"
    );
}

#[test]
fn extraction_without_a_driver_key_reports_the_manifest() {
    let package = archive(&[("MANIFEST", b"[Files]\nlicense = \"LICENSE\"\n".as_slice())]);

    let error = extract_driver(&package, PACKAGE_URL)
        .expect_err("a MANIFEST without a driver key must not extract");

    assert!(
        error.to_string().contains("MANIFEST"),
        "the error must name the file it could not read: {error}"
    );
}

#[test]
fn extraction_without_the_named_library_names_it() {
    let package = archive(&[("MANIFEST", b"[Files]\ndriver = \"absent.so\"\n".as_slice())]);

    let error = extract_driver(&package, PACKAGE_URL)
        .expect_err("a MANIFEST naming an absent member must not extract");

    assert!(
        error.to_string().contains("absent.so"),
        "the error must name the library the archive lacks: {error}"
    );
}

/// Nothing verifies a driver package before it is unpacked — the registry
/// publishes no checksum — so what the unpacking step writes is bounded by
/// what the members declare, and the running total is what is compared:
/// the `MANIFEST` here is read and counted before the oversize member is
/// reached.
#[test]
fn a_member_past_the_unpacked_bound_names_the_limit_and_the_package() {
    let package = archive_declaring(&[
        (
            "MANIFEST",
            b"[Files]\ndriver = \"lib.so\"\n".as_slice(),
            "[Files]\ndriver = \"lib.so\"\n".len() as u64,
        ),
        ("lib.so", b"ELF".as_slice(), MAX_UNPACKED_BYTES + 1),
    ]);

    let error = extract_driver(&package, PACKAGE_URL)
        .expect_err("a package asking to write more than the bound must not be unpacked");

    let message = error.to_string();
    assert!(
        matches!(error, ProvisioningError::PackageTooLarge { .. }),
        "the refusal must be the size one rather than a truncated read: {message}"
    );
    assert!(
        message.contains(&MAX_UNPACKED_BYTES.to_string()),
        "the error must name the limit it enforced: {message}"
    );
    assert!(
        message.contains(PACKAGE_URL),
        "the error must name the package it refused: {message}"
    );
}

/// The `MANIFEST` is read into memory rather than written to disk, so it is
/// bounded by the same total: an archive declaring a gigabyte of TOML would
/// otherwise exhaust the sandbox's memory before anything looked at it.
#[test]
fn a_manifest_past_the_unpacked_bound_names_the_limit() {
    let package = archive_declaring(&[(
        "MANIFEST",
        b"[Files]\ndriver = \"lib.so\"\n".as_slice(),
        MAX_UNPACKED_BYTES + 1,
    )]);

    let error = extract_driver(&package, PACKAGE_URL)
        .expect_err("a MANIFEST larger than the bound must not be read");

    let message = error.to_string();
    assert!(
        matches!(error, ProvisioningError::PackageTooLarge { .. }),
        "the refusal must be the size one: {message}"
    );
    assert!(
        message.contains(&MAX_UNPACKED_BYTES.to_string()),
        "the error must name the limit it enforced: {message}"
    );
}

#[test]
fn extraction_writes_no_member_outside_its_own_directory() {
    let package = archive(&[
        ("MANIFEST", b"[Files]\ndriver = \"lib.so\"\n".as_slice()),
        ("lib.so", b"ELF".as_slice()),
        ("../escaped", b"escaped".as_slice()),
    ]);

    let driver = extract_driver(&package, PACKAGE_URL).expect("a well-formed package must extract");

    let extracted_into = driver
        .path
        .parent()
        .expect("the extracted library must have a directory");
    assert!(
        extracted_into.join("escaped").is_file(),
        "a member naming a parent directory must be written inside the extraction directory"
    );
    assert!(
        !extracted_into
            .parent()
            .expect("the extraction directory must have a parent")
            .join("escaped")
            .exists(),
        "a member naming a parent directory must not escape the extraction directory"
    );
}

#[test]
fn the_install_path_carries_the_driver_and_the_resolved_version() {
    assert_eq!(
        driver_install_path(
            Path::new("/buckets/bfsdefault/default"),
            Driver::MySql,
            &Version::new(0, 6, 1)
        ),
        Path::new("/buckets/bfsdefault/default/adbc-drivers/mysql/0.6.1/libadbc_driver_mysql.so")
    );
}

/// `driver-provisioning/driver-path-override`: the operator-owned location
/// carries the driver and no version, because nothing resolved one on the route
/// that reads it.
#[test]
fn the_default_location_carries_the_driver_and_no_version() {
    assert_eq!(
        default_driver_path(Path::new("/buckets/bfsdefault/default"), Driver::MySql),
        Path::new(
            "/buckets/bfsdefault/default/adbc-drivers/mysql/supplied/libadbc_driver_mysql.so"
        )
    );
}

/// Two technologies' pre-placed libraries never share a path, so a cluster
/// running both can pre-place both.
#[test]
fn the_default_location_is_distinct_per_driver() {
    let mount = Path::new("/buckets/bfsdefault/default");

    for (index, driver) in Driver::ALL.iter().enumerate() {
        for other in &Driver::ALL[index + 1..] {
            assert_ne!(
                default_driver_path(mount, *driver),
                default_driver_path(mount, *other),
                "`{}` and `{}` must pre-place their libraries at different paths",
                driver.as_str(),
                other.as_str()
            );
        }
    }
}

/// The property the whole last-resort design rests on: the operator-owned tree
/// and the adapter-owned one are disjoint by construction rather than by
/// convention, so a pre-placed library can never satisfy the versioned presence
/// check and self-install can never overwrite a pre-placed library.
///
/// Both paths are computed under one mount value, which is what makes the
/// comparison a statement about the layout rather than about two mounts. The
/// inequality over a sample of versions is the weaker half: it holds for any
/// sample. Only `supplied` failing to parse as a version holds for all of them.
#[test]
fn no_default_location_can_be_an_install_path() {
    let mount = Path::new("/buckets/bfsdefault/default");

    assert!(
        Version::parse(SUPPLIED_DIRECTORY).is_err(),
        "`{SUPPLIED_DIRECTORY}` must parse as no version at all, which is what makes the \
         two trees disjoint for every version the registry could ever publish"
    );

    for driver in Driver::ALL {
        let supplied = default_driver_path(mount, *driver);
        for version in [
            Version::new(0, 0, 0),
            Version::new(0, 6, 1),
            Version::new(1, 13, 0),
            Version::parse("1.0.0-rc.1").expect("a pre-release version parses"),
            Version::parse("0.6.1+build.5").expect("a build-metadata version parses"),
        ] {
            assert_ne!(
                supplied,
                driver_install_path(mount, *driver, &version),
                "the `{}` default location must not be the install path of {version}",
                driver.as_str()
            );
        }
    }
}

/// The index URL and the transport cause the one unreachable-registry fixture
/// is built with, so a test can assert the accessor hands back those two facts
/// rather than merely answering `Some`.
const UNREACHABLE_INDEX_URL: &str = "https://dbc-cdn.columnar.tech/index.yaml";
const UNREACHABLE_TRANSPORT_CAUSE: &str = "error trying to connect: dns error";

/// One constructed value of every [`ProvisioningError`] variant.
///
/// Both tests below read this rather than enumerating the taxonomy themselves,
/// so the enumeration has one owner and two copies cannot drift apart.
fn every_provisioning_error() -> Vec<ProvisioningError> {
    vec![
        ProvisioningError::RegistryUnreachable {
            url: UNREACHABLE_INDEX_URL.to_owned(),
            driver: "mysql".to_owned(),
            cause: UNREACHABLE_TRANSPORT_CAUSE.to_owned(),
        },
        ProvisioningError::PackageUnreachable {
            url: PACKAGE_URL.to_owned(),
            driver: "mysql".to_owned(),
            cause: "error trying to connect: connection refused".to_owned(),
        },
        ProvisioningError::TransferInterrupted {
            url: PACKAGE_URL.to_owned(),
            driver: "mysql".to_owned(),
            cause: "connection closed before message completed".to_owned(),
        },
        ProvisioningError::RegistryMisdirected {
            url: UNREACHABLE_INDEX_URL.to_owned(),
            driver: "mysql".to_owned(),
            cause: "too many redirects".to_owned(),
        },
        ProvisioningError::RegistryRejected {
            url: UNREACHABLE_INDEX_URL.to_owned(),
            status: 503,
            driver: "mysql".to_owned(),
        },
        ProvisioningError::ResponseTooLarge {
            url: UNREACHABLE_INDEX_URL.to_owned(),
            limit: MAX_INDEX_BYTES,
        },
        ProvisioningError::PackageTooLarge {
            url: PACKAGE_URL.to_owned(),
            limit: MAX_UNPACKED_BYTES,
        },
        ProvisioningError::IndexUnreadable {
            url: UNREACHABLE_INDEX_URL.to_owned(),
            cause: "invalid utf-8".to_owned(),
        },
        ProvisioningError::DriverNotPublished {
            driver: "mysql".to_owned(),
            url: UNREACHABLE_INDEX_URL.to_owned(),
        },
        ProvisioningError::RangeUnparseable {
            driver: "mysql".to_owned(),
            range: "not a range".to_owned(),
            cause: "unexpected character".to_owned(),
        },
        ProvisioningError::RangeNotSatisfied {
            driver: "mysql".to_owned(),
            range: ">=0.6.0, <0.7.0".to_owned(),
            published: vec!["v0.5.0".to_owned()],
            url: UNREACHABLE_INDEX_URL.to_owned(),
        },
        ProvisioningError::PlatformNotPublished {
            driver: "mysql".to_owned(),
            platform: PLATFORM.to_owned(),
            version: "0.6.1".to_owned(),
        },
        ProvisioningError::PackageUnreadable {
            url: PACKAGE_URL.to_owned(),
            cause: "not a gzip stream".to_owned(),
        },
        ProvisioningError::ManifestMissing {
            url: PACKAGE_URL.to_owned(),
        },
        ProvisioningError::ManifestUnreadable {
            url: PACKAGE_URL.to_owned(),
            cause: "missing field `driver`".to_owned(),
        },
        ProvisioningError::LibraryMissing {
            url: PACKAGE_URL.to_owned(),
            filename: "libadbc_driver_mysql.so".to_owned(),
        },
        ProvisioningError::LibraryFilenameDiverged {
            declared: "libadbc_driver_mysql_other.so".to_owned(),
            expected: "libadbc_driver_mysql.so".to_owned(),
        },
        ProvisioningError::BucketFsUnreachable {
            url: "https://exasol:2581/default".to_owned(),
            cause: "connection refused".to_owned(),
        },
        ProvisioningError::BucketFsRejected {
            url: "https://exasol:2581/default".to_owned(),
            status: 403,
        },
        ProvisioningError::DriverNotVisible {
            path: PathBuf::from(
                "/buckets/bfsdefault/default/adbc-drivers/mysql/0.6.1/libadbc_driver_mysql.so",
            ),
            deadline: Duration::from_secs(120),
        },
        ProvisioningError::LocalFailure {
            attempt: "creating a directory to unpack the driver package into".to_owned(),
            cause: "permission denied".to_owned(),
        },
        ProvisioningError::SuppliedDriverUnreadable {
            driver: "mysql".to_owned(),
            path: "/tmp/never-placed.so".to_owned(),
            cause: "no such file or directory".to_owned(),
        },
    ]
}

/// `driver-provisioning/driver-path-override`: only the index request that
/// reached no host at all is evidence of the air gap the last-resort route
/// serves. Every other failure means something answered, so it says nothing
/// about the cluster's egress and must not reach the default location.
#[test]
fn only_a_registry_reached_not_at_all_is_unreachable() {
    for error in every_provisioning_error() {
        match &error {
            ProvisioningError::RegistryUnreachable { .. } => assert_eq!(
                error.registry_unreachable(),
                Some((UNREACHABLE_INDEX_URL, UNREACHABLE_TRANSPORT_CAUSE)),
                "an index request that reached no host must hand back the index URL and \
                 the transport cause it was built with: {error:?}"
            ),
            answered => assert_eq!(
                error.registry_unreachable(),
                None,
                "a registry that answered says nothing about the cluster's egress: \
                 {answered:?}"
            ),
        }
    }
}

/// `driver-provisioning/driver-path-override`: no message on any route may deny
/// that the default driver location exists. An operator who followed
/// `README.md` to it must never be told it does not exist, whichever variant
/// answered and whichever route the statement took.
#[test]
fn no_provisioning_message_denies_the_default_location() {
    for error in every_provisioning_error() {
        let message = error.to_string();
        assert!(
            !message.contains("no path on the mount to pre-load"),
            "the default driver location exists on every cluster, so no message may deny \
             it: {message}"
        );
    }
}

/// `driver-provisioning/driver-self-install`: the leg × point-of-failure table
/// [`provisioning_error`] owns, asserted directly rather than through whichever
/// single leg a live test happens to exercise.
///
/// [`RegistryLeg`] exists for exactly one row of it — an index request that
/// reached no host — so a regression that dropped the argument and mapped both
/// legs back to [`ProvisioningError::RegistryUnreachable`] would leave the rest
/// of the suite green while re-enabling the last-resort fallback on a package
/// download that reached no host.
#[test]
fn each_download_failure_maps_to_one_variant_per_leg() {
    const CAUSE: &str = "error trying to connect: dns error";
    type IsExpected = fn(&ProvisioningError) -> bool;
    let rows: [(&str, DownloadFailure, RegistryLeg, IsExpected, bool); 10] = [
        (
            "an index request that reached no host",
            DownloadFailure::Unreachable(CAUSE.to_owned()),
            RegistryLeg::Index,
            |error| matches!(error, ProvisioningError::RegistryUnreachable { .. }),
            true,
        ),
        (
            "a package request that reached no host",
            DownloadFailure::Unreachable(CAUSE.to_owned()),
            RegistryLeg::Package,
            |error| matches!(error, ProvisioningError::PackageUnreachable { .. }),
            false,
        ),
        (
            "an index body that stopped after its status arrived",
            DownloadFailure::Interrupted(CAUSE.to_owned()),
            RegistryLeg::Index,
            |error| matches!(error, ProvisioningError::TransferInterrupted { .. }),
            false,
        ),
        (
            "a package body that stopped after its status arrived",
            DownloadFailure::Interrupted(CAUSE.to_owned()),
            RegistryLeg::Package,
            |error| matches!(error, ProvisioningError::TransferInterrupted { .. }),
            false,
        ),
        (
            "an index request the client could not follow through",
            DownloadFailure::Answered(CAUSE.to_owned()),
            RegistryLeg::Index,
            |error| matches!(error, ProvisioningError::RegistryMisdirected { .. }),
            false,
        ),
        (
            "a package request the client could not follow through",
            DownloadFailure::Answered(CAUSE.to_owned()),
            RegistryLeg::Package,
            |error| matches!(error, ProvisioningError::RegistryMisdirected { .. }),
            false,
        ),
        (
            "an index request answered with an HTTP status",
            DownloadFailure::Rejected(503),
            RegistryLeg::Index,
            |error| matches!(error, ProvisioningError::RegistryRejected { .. }),
            false,
        ),
        (
            "a package request answered with an HTTP status",
            DownloadFailure::Rejected(404),
            RegistryLeg::Package,
            |error| matches!(error, ProvisioningError::RegistryRejected { .. }),
            false,
        ),
        (
            "an index response past the size bound",
            DownloadFailure::TooLarge(MAX_INDEX_BYTES),
            RegistryLeg::Index,
            |error| matches!(error, ProvisioningError::ResponseTooLarge { .. }),
            false,
        ),
        (
            "a package response past the size bound",
            DownloadFailure::TooLarge(MAX_INDEX_BYTES),
            RegistryLeg::Package,
            |error| matches!(error, ProvisioningError::ResponseTooLarge { .. }),
            false,
        ),
    ];

    for (failure, download_failure, leg, is_expected_variant, is_air_gap_evidence) in rows {
        let error = provisioning_error(download_failure, leg, UNREACHABLE_INDEX_URL, Driver::MySql);
        assert!(
            is_expected_variant(&error),
            "{failure} must map to the one variant that reports it: {error:?}"
        );
        assert_eq!(
            error.registry_unreachable().is_some(),
            is_air_gap_evidence,
            "only an index request that reached no host is evidence of an air gap, so \
             {failure} must not answer otherwise: {error:?}"
        );
    }
}

#[test]
fn two_versions_of_one_driver_install_at_different_paths() {
    let mount = Path::new("/buckets/bfsdefault/default");

    assert_ne!(
        driver_install_path(mount, Driver::MySql, &Version::new(0, 6, 0)),
        driver_install_path(mount, Driver::MySql, &Version::new(0, 6, 1)),
        "a newer resolved version must not be installed over the version a running \
         query may still have mapped"
    );
}

/// `driver-provisioning/driver-path-override`: the path is used exactly as
/// given, neither re-rooted under the mount nor extended with the driver and
/// version segments self-install computes.
#[test]
fn a_supplied_driver_is_used_at_exactly_the_path_given() {
    let directory = TempDir::new().expect("a temporary directory must be creatable");
    let supplied = directory.path().join("operators-own-name.so");
    std::fs::write(&supplied, b"\x7fELF").expect("the fixture library must be writable");

    let path = supplied_driver(
        supplied.to_str().expect("the fixture path is UTF-8"),
        Driver::MySql,
    )
    .expect("a readable file at the path given must be used as it is");

    assert_eq!(path, supplied);
}

#[test]
fn a_supplied_path_holding_no_file_names_the_property_the_path_and_the_driver() {
    let directory = TempDir::new().expect("a temporary directory must be creatable");
    let absent = directory.path().join("never-placed.so");

    let error = supplied_driver(
        absent.to_str().expect("the fixture path is UTF-8"),
        Driver::Postgres,
    )
    .expect_err("a path holding no readable file must refuse the statement");

    let message = error.to_string();
    assert!(
        message.contains("DRIVER_PATH"),
        "the error must name the property that was set: {message}"
    );
    assert!(
        message.contains(absent.to_str().expect("the fixture path is UTF-8")),
        "the error must name the path it was given: {message}"
    );
    assert!(
        message.contains("postgresql"),
        "the error must name the driver it was looking for: {message}"
    );
}

/// A directory opens without error on Linux, so the check is the file kind
/// rather than the open alone — a `dlopen` of a directory fails far later and
/// names nothing an operator set.
#[test]
fn a_supplied_path_naming_a_directory_is_refused() {
    let directory = TempDir::new().expect("a temporary directory must be creatable");

    let error = supplied_driver(
        directory
            .path()
            .to_str()
            .expect("the fixture path is UTF-8"),
        Driver::MySql,
    )
    .expect_err("a directory is not a driver shared library");

    assert!(
        matches!(error, ProvisioningError::SuppliedDriverUnreadable { .. }),
        "the refusal must be the supplied-driver one: {error}"
    );
}

#[test]
fn redaction_replaces_every_occurrence_of_the_secret() {
    let redacted = redact(
        "PUT https://w:hunter2@bucketfs/ rejected hunter2",
        "hunter2",
    );

    assert!(
        !redacted.contains("hunter2"),
        "the secret must not survive redaction: {redacted}"
    );
}

#[test]
fn redaction_of_an_empty_secret_leaves_the_text_alone() {
    assert_eq!(redact("BucketFS answered 403", ""), "BucketFS answered 403");
}

#[test]
fn credentials_never_print_their_password() {
    let credentials = Credentials::new("w", "hunter2");

    let printed = format!("{credentials:?}");
    assert!(
        !printed.contains("hunter2"),
        "the password must not reach a debug rendering: {printed}"
    );
    assert!(
        printed.contains('w'),
        "the user is not a secret and stays diagnosable: {printed}"
    );
}

#[test]
fn a_bucket_reports_where_its_driver_is_installed() {
    let mount = "/buckets/bfsdefault/default";
    let version = Version::new(0, 6, 1);
    let bucket = Bucket::new(
        "https://exasol:2581/default",
        mount,
        Credentials::new("w", "hunter2"),
    )
    .visible_within(Duration::from_millis(1));

    assert_eq!(
        bucket.driver_path(Driver::MySql, &version),
        driver_install_path(Path::new(mount), Driver::MySql, &version),
        "the bucket must report exactly what the shared path-construction rule computes"
    );
}

#[test]
fn writing_a_package_that_is_not_gzip_names_the_package_url() {
    let mut not_gzip = Vec::new();
    not_gzip
        .write_all(b"<html>404</html>")
        .expect("writing to a vector must succeed");

    let error = extract_driver(&not_gzip, PACKAGE_URL)
        .expect_err("plain text must not extract as a package");

    assert!(
        error.to_string().contains(PACKAGE_URL),
        "the error must name the package it could not read: {error}"
    );
}
