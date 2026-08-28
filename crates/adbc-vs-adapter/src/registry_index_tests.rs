use super::*;

const FIXTURE_INDEX: &str = include_str!("../tests/fixtures/index.yaml");
const REGISTRY_BASE: &str = "https://dbc-cdn.columnar.tech";

#[test]
fn finds_driver_version_and_platform() {
    let result = package_url(
        FIXTURE_INDEX,
        "mysql",
        "v0.6.0",
        "linux_amd64",
        REGISTRY_BASE,
    );

    assert!(result.is_ok(), "mysql v0.6.0 linux_amd64 must resolve");
}

#[test]
fn unknown_driver_is_reported() {
    let result = package_url(
        FIXTURE_INDEX,
        "does-not-exist",
        "v0.6.0",
        "linux_amd64",
        REGISTRY_BASE,
    );

    assert!(matches!(result, Err(LookupError::NoDriver)));
}

#[test]
fn unknown_version_is_reported() {
    let result = package_url(
        FIXTURE_INDEX,
        "mysql",
        "v9.9.9",
        "linux_amd64",
        REGISTRY_BASE,
    );

    assert!(matches!(result, Err(LookupError::NoVersion)));
}

#[test]
fn unknown_platform_is_reported() {
    let result = package_url(
        FIXTURE_INDEX,
        "mysql",
        "v0.6.0",
        "windows_amd64",
        REGISTRY_BASE,
    );

    assert!(matches!(result, Err(LookupError::NoPlatform)));
}

#[test]
fn resolves_relative_url_against_base() {
    let url = package_url(
        FIXTURE_INDEX,
        "mysql",
        "v0.6.0",
        "linux_amd64",
        REGISTRY_BASE,
    )
    .expect("mysql v0.6.0 linux_amd64 must resolve");

    assert_eq!(
        url,
        "https://dbc-cdn.columnar.tech/mysql/v0.6.0/mysql_linux_amd64_v0.6.0.tar.gz"
    );
}

#[test]
fn malformed_yaml_fails_to_parse() {
    let result = package_url(
        "not: [valid, yaml: structure",
        "mysql",
        "v0.6.0",
        "linux_amd64",
        REGISTRY_BASE,
    );

    assert!(matches!(result, Err(LookupError::Unparseable(_))));
}
