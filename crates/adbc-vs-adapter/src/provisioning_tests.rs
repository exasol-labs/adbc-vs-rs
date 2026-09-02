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
        "https://dbc-cdn.columnar.tech/mysql/0.6.1/mysql_linux_amd64_0.6.1.tar.gz"
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
    let index = concat!(
        "drivers:\n",
        "  - path: mysql\n",
        "    pkginfo:\n",
        "      - version: v0.6.0\n",
        "        packages:\n",
        "          - platform: darwin_arm64\n",
        "            url: mysql/v0.6.0/mysql_darwin_arm64_v0.6.0.tar.gz\n",
    );

    let error = select_package(index, &Registry::public(Driver::MySql))
        .expect_err("a version without a linux_amd64 package must fail");

    let message = error.to_string();
    assert!(
        message.contains("linux_amd64"),
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
