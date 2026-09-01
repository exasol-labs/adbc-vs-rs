use super::*;

const FIXTURE_INDEX: &str = include_str!("../tests/fixtures/index.yaml");
const REGISTRY_BASE: &str = "https://dbc-cdn.columnar.tech";
const LINUX: &str = "linux_amd64";

/// The ranges the two shipped declarations state, spelled out rather than read
/// through the driver identity: these tests exercise the resolution rule, and
/// stating the premise here is what lets them read as one claim each.
const MYSQL_RANGE: &str = ">=0.6.0, <0.7.0";
const POSTGRESQL_RANGE: &str = ">=1.12.0, <2.0.0";

#[test]
fn range_resolves_to_highest_published_match() {
    let resolved = resolve_package(FIXTURE_INDEX, "mysql", MYSQL_RANGE, LINUX, REGISTRY_BASE)
        .expect("the fixture publishes two mysql versions inside the range");

    assert_eq!(
        resolved.version,
        Version::new(0, 6, 1),
        "0.6.1 is the highest mysql version the fixture publishes inside the range"
    );
    assert_eq!(
        resolved.url, "https://dbc-cdn.columnar.tech/mysql/0.6.1/mysql_linux_amd64_0.6.1.tar.gz",
        "the url must come from the resolved block and resolve against the base"
    );
}

#[test]
fn each_drivers_range_resolves_against_its_own_published_versions() {
    let resolved = resolve_package(
        FIXTURE_INDEX,
        "postgresql",
        POSTGRESQL_RANGE,
        LINUX,
        REGISTRY_BASE,
    )
    .expect("the fixture publishes two postgresql versions inside the range");

    assert_eq!(
        resolved.version,
        Version::new(1, 13, 0),
        "postgresql resolves against its own entry and its own range, not mysql's"
    );
    assert_eq!(
        resolved.url,
        "https://dbc-cdn.columnar.tech/postgresql/1.13.0/postgresql_linux_amd64_1.13.0.tar.gz"
    );
}

#[test]
fn a_v_prefixed_published_version_resolves_like_a_bare_one() {
    let prefixed = resolve_package(FIXTURE_INDEX, "postgresql", "=1.12.0", LINUX, REGISTRY_BASE)
        .expect("the fixture publishes postgresql 1.12.0, spelled `v1.12.0`");
    let bare = resolve_package(FIXTURE_INDEX, "mysql", "=0.6.1", LINUX, REGISTRY_BASE)
        .expect("the fixture publishes mysql 0.6.1, spelled without a prefix");

    assert_eq!(prefixed.version, Version::new(1, 12, 0));
    assert_eq!(bare.version, Version::new(0, 6, 1));
}

#[test]
fn a_published_version_above_the_range_is_not_selected() {
    let resolved = resolve_package(
        FIXTURE_INDEX,
        "mysql",
        ">=0.5.0, <0.6.0",
        LINUX,
        REGISTRY_BASE,
    )
    .expect("the fixture publishes mysql v0.5.0");

    assert_eq!(
        resolved.version,
        Version::new(0, 5, 0),
        "three higher mysql versions are published and none of them satisfies this range"
    );
}

#[test]
fn an_unparseable_published_version_is_skipped() {
    let index = concat!(
        "drivers:\n",
        "  - path: mysql\n",
        "    pkginfo:\n",
        "      - version: v0.6.0\n",
        "        packages:\n",
        "          - platform: linux_amd64\n",
        "            url: mysql/v0.6.0/mysql_linux_amd64_v0.6.0.tar.gz\n",
        "      - version: nightly\n",
        "        packages:\n",
        "          - platform: linux_amd64\n",
        "            url: mysql/nightly/mysql_linux_amd64_nightly.tar.gz\n",
    );

    let resolved = resolve_package(index, "mysql", MYSQL_RANGE, LINUX, REGISTRY_BASE)
        .expect("one scalar that is no version must not make the driver unresolvable");

    assert_eq!(resolved.version, Version::new(0, 6, 0));
}

#[test]
fn unsatisfiable_range_reports_every_published_version() {
    let error = resolve_package(FIXTURE_INDEX, "mysql", ">=9.0.0", LINUX, REGISTRY_BASE)
        .expect_err("the fixture publishes no mysql version this high");

    let LookupError::NoVersionInRange { published } = error else {
        panic!("a range no published version satisfies must report the published versions");
    };
    assert_eq!(
        published,
        vec!["v0.6.0", "nightly", "0.6.1", "v0.7.0", "v0.5.0"],
        "every published scalar must be reported as spelled, so an operator can see \
         what the range missed"
    );
}

#[test]
fn unknown_driver_is_reported() {
    let result = resolve_package(
        FIXTURE_INDEX,
        "does-not-exist",
        MYSQL_RANGE,
        LINUX,
        REGISTRY_BASE,
    );

    assert!(matches!(result, Err(LookupError::NoDriver)));
}

#[test]
fn unknown_platform_reports_the_resolved_version() {
    let error = resolve_package(
        FIXTURE_INDEX,
        "mysql",
        MYSQL_RANGE,
        "windows_amd64",
        REGISTRY_BASE,
    )
    .expect_err("the fixture publishes no windows package");

    let LookupError::NoPlatform { resolved } = error else {
        panic!("a resolved version publishing no such platform must report that version");
    };
    assert_eq!(
        resolved,
        Version::new(0, 6, 1),
        "the platform was looked for in the version the range resolved to"
    );
}

#[test]
fn unparseable_range_is_reported() {
    let result = resolve_package(
        FIXTURE_INDEX,
        "mysql",
        "newest please",
        LINUX,
        REGISTRY_BASE,
    );

    assert!(matches!(result, Err(LookupError::UnparseableRange(_))));
}

#[test]
fn malformed_yaml_fails_to_parse() {
    let result = resolve_package(
        "not: [valid, yaml: structure",
        "mysql",
        MYSQL_RANGE,
        LINUX,
        REGISTRY_BASE,
    );

    assert!(matches!(result, Err(LookupError::Unparseable(_))));
}

#[test]
fn a_published_prerelease_is_not_selected() {
    let index = concat!(
        "drivers:\n",
        "  - path: mysql\n",
        "    pkginfo:\n",
        "      - version: v0.6.0\n",
        "        packages:\n",
        "          - platform: linux_amd64\n",
        "            url: mysql/v0.6.0/mysql_linux_amd64_v0.6.0.tar.gz\n",
        "      - version: v0.6.9-rc.1\n",
        "        packages:\n",
        "          - platform: linux_amd64\n",
        "            url: mysql/v0.6.9-rc.1/mysql_linux_amd64_v0.6.9-rc.1.tar.gz\n",
    );

    let resolved = resolve_package(index, "mysql", MYSQL_RANGE, LINUX, REGISTRY_BASE)
        .expect("the release version inside the range must still resolve");

    assert_eq!(
        resolved.version,
        Version::new(0, 6, 0),
        "a release candidate is numerically higher and inside the range's bounds, and a \
         range naming no prerelease must still not install one"
    );
}

#[test]
fn a_published_version_that_is_no_string_is_skipped() {
    let index = concat!(
        "drivers:\n",
        "  - path: mysql\n",
        "    pkginfo:\n",
        "      - version: 1.8\n",
        "        packages: []\n",
        "      - version: v0.6.0\n",
        "        packages:\n",
        "          - platform: linux_amd64\n",
        "            url: mysql/v0.6.0/mysql_linux_amd64_v0.6.0.tar.gz\n",
    );

    let resolved = resolve_package(index, "mysql", MYSQL_RANGE, LINUX, REGISTRY_BASE).expect(
        "a scalar YAML reads as a number rather than a version string must be skipped like \
         any other unparseable one, not fail the whole document",
    );

    assert_eq!(resolved.version, Version::new(0, 6, 0));
}
