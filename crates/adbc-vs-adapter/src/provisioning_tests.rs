use std::io::Write;
use std::path::Path;
use std::time::Duration;

use flate2::write::GzEncoder;
use flate2::Compression;

use super::*;

const FIXTURE_INDEX: &str = include_str!("../tests/fixtures/index.yaml");

/// The SHA-256 of the empty input, from the algorithm's own test vectors.
const EMPTY_DIGEST: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

/// Builds a gzipped tar archive of `(name, content)` members, the shape the
/// driver registry publishes packages in.
///
/// Member names are written into the header directly, because the builder
/// refuses to write the `..` an archive would have to carry to attempt a path
/// traversal.
fn archive(members: &[(&str, &[u8])]) -> Vec<u8> {
    let mut builder = tar::Builder::new(GzEncoder::new(Vec::new(), Compression::fast()));
    for (name, content) in members {
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Regular);
        header.set_mode(0o644);
        header.set_size(content.len() as u64);
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
fn selects_the_pinned_platform_package_of_the_pinned_version() {
    let url = select_package_url(FIXTURE_INDEX, &Registry::pinned())
        .expect("the fixture index publishes the pinned package");

    assert_eq!(
        url,
        "https://dbc-cdn.columnar.tech/mysql/v0.6.0/mysql_linux_amd64_v0.6.0.tar.gz"
    );
}

#[test]
fn unparseable_index_names_the_index_url() {
    let error = select_package_url("drivers: [", &Registry::pinned())
        .expect_err("a truncated document must not parse");

    assert!(
        error.to_string().contains("index.yaml"),
        "the error must name the document it could not read: {error}"
    );
}

#[test]
fn index_without_the_driver_names_the_driver_path() {
    let index = "drivers:\n  - path: postgresql\n    pkginfo: []\n";

    let error = select_package_url(index, &Registry::pinned())
        .expect_err("an index without the mysql driver must fail");

    assert!(
        error.to_string().contains("mysql"),
        "the error must name the driver it looked for: {error}"
    );
}

#[test]
fn index_without_the_pinned_version_names_that_version() {
    let index =
        "drivers:\n  - path: mysql\n    pkginfo:\n      - version: v0.5.0\n        packages: []\n";

    let error = select_package_url(index, &Registry::pinned())
        .expect_err("an index without the pinned version must fail");

    let message = error.to_string();
    assert!(
        message.contains("v0.6.0"),
        "the error must name the version it looked for: {message}"
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

    let error = select_package_url(index, &Registry::pinned())
        .expect_err("a version without a linux_amd64 package must fail");

    let message = error.to_string();
    assert!(
        message.contains("linux_amd64"),
        "the error must name the platform it looked for: {message}"
    );
    assert!(
        message.contains("v0.6.0"),
        "the error must name the version it looked in: {message}"
    );
}

#[test]
fn digest_of_no_bytes_is_the_published_empty_digest() {
    assert_eq!(digest_of(&[]), EMPTY_DIGEST);
}

#[test]
fn matching_digest_verifies() {
    verify_digest(
        b"driver package",
        &digest_of(b"driver package"),
        PACKAGE_URL,
    )
    .expect("bytes must verify against their own digest");
}

#[test]
fn digest_mismatch_reports_both_digests() {
    let error = verify_digest(b"tampered", EMPTY_DIGEST, PACKAGE_URL)
        .expect_err("bytes that hash differently must not verify");

    let message = error.to_string();
    assert!(
        message.contains(EMPTY_DIGEST),
        "the error must report the expected digest: {message}"
    );
    assert!(
        message.contains(&digest_of(b"tampered")),
        "the error must report the computed digest: {message}"
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
fn the_manifest_filename_must_equal_the_compiled_in_filename() {
    verify_library_filename(DRIVER_LIBRARY).expect("the compiled-in filename must verify");
}

#[test]
fn a_diverging_manifest_filename_names_both_filenames() {
    let error = verify_library_filename("libadbc_driver_mysql_v2.so")
        .expect_err("a filename other than the compiled-in one must not verify");

    let message = error.to_string();
    assert!(
        message.contains("libadbc_driver_mysql_v2.so"),
        "the error must name the filename the archive declared: {message}"
    );
    assert!(
        message.contains(DRIVER_LIBRARY),
        "the error must name the compiled-in filename: {message}"
    );
}

#[test]
fn the_install_path_is_the_driver_directory_of_the_mount() {
    assert_eq!(
        driver_install_path(Path::new("/buckets/bfsdefault/default")),
        Path::new("/buckets/bfsdefault/default/adbc-drivers/libadbc_driver_mysql.so")
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
    let bucket = Bucket::new(
        "https://exasol:2581/default",
        "/buckets/bfsdefault/default",
        Credentials::new("w", "hunter2"),
    )
    .visible_within(Duration::from_millis(1));

    assert_eq!(
        bucket.driver_path(),
        Path::new("/buckets/bfsdefault/default/adbc-drivers/libadbc_driver_mysql.so")
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
