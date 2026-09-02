use super::driver::*;

#[test]
fn parse_accepts_every_canonical_name() {
    for driver in Driver::ALL {
        assert_eq!(
            Driver::parse(driver.as_str()).expect("a canonical name must parse"),
            *driver
        );
    }
}

#[test]
fn parse_is_case_insensitive() {
    for driver in Driver::ALL {
        let shouted = driver.as_str().to_ascii_uppercase();
        assert_eq!(
            Driver::parse(&shouted)
                .unwrap_or_else(|error| panic!("`{shouted}` must parse: {error}")),
            *driver
        );
    }
}

#[test]
fn as_str_is_the_canonical_lower_case_name() {
    for driver in Driver::ALL {
        let name = driver.as_str();
        assert_eq!(
            name,
            name.to_ascii_lowercase(),
            "the canonical name is what `adapterNotes.driver` records and `parse` compares \
             against, so it is spelled in one case only"
        );
        assert!(!name.is_empty());
    }
}

/// The refusal reaches an operator as the whole diagnosis of a failed
/// `CREATE VIRTUAL SCHEMA`, so it names the value it rejected and every value
/// it would have accepted — read off the identities rather than restated.
#[test]
fn parse_rejects_an_unknown_driver_naming_every_supported_value() {
    let error = Driver::parse("sybase").expect_err("an unknown driver must not parse");

    let message = error.to_string();
    assert!(
        message.contains("sybase"),
        "the error must name the value it rejected: {message}"
    );
    for driver in Driver::ALL {
        assert!(
            message.contains(driver.as_str()),
            "the error must name supported driver `{}`: {message}",
            driver.as_str()
        );
    }
}

#[test]
fn parse_rejects_an_empty_driver_value() {
    Driver::parse("").expect_err("an empty DRIVER value names no driver");
}

/// Every identity resolves to a package with all four facts present. Nothing
/// downstream re-checks them: provisioning fetches `registry_path`, installs
/// `library`, and the scan loads `entry_point`, each without a fallback.
#[test]
fn every_identity_declares_a_complete_package() {
    for driver in Driver::ALL {
        let package = driver.package();
        for (field, value) in [
            ("registry_path", package.registry_path),
            ("library", package.library),
            ("entry_point", package.entry_point),
            ("version_range", package.version_range),
        ] {
            assert!(
                !value.is_empty(),
                "driver `{}` declares an empty `{field}`",
                driver.as_str()
            );
        }
    }
}

/// The `drivers/` directory is the whole list of technologies this release
/// ships, so a file with no identity — or an identity with no file — is a
/// half-added driver rather than a working one.
#[test]
fn the_declared_files_are_exactly_the_identities() {
    let mut declared: Vec<&str> = DECLARED_DRIVERS.iter().map(|(name, _)| *name).collect();
    declared.sort_unstable();
    let mut identities: Vec<&str> = Driver::ALL.iter().map(|driver| driver.as_str()).collect();
    identities.sort_unstable();

    assert_eq!(
        declared, identities,
        "every drivers/<name>.toml must have a `Driver` variant and every variant a file"
    );
}

/// Release 0.2 installed exactly this package. The row is what carries those
/// facts now, so it is pinned here rather than trusted to have been copied
/// across correctly.
#[test]
fn the_mysql_row_carries_the_identity_release_0_2_installed() {
    let package = Driver::MySql.package();

    assert_eq!(package.registry_path, "mysql");
    assert_eq!(package.library, "libadbc_driver_mysql.so");
    assert_eq!(package.entry_point, "AdbcDriverMysqlInit");
    assert_eq!(package.version_range, ">=0.6.0, <0.7.0");
}

/// The PostgreSQL package facts were measured against the published registry
/// and the published archive rather than derived from the driver's name: the
/// registry path, the archive's own `MANIFEST` entry, and the exported entry
/// symbol are each a fact about what Columnar publishes. Pinned here so a row
/// edited to a plausible-looking spelling fails a test rather than a download.
#[test]
fn the_postgresql_row_carries_the_measured_package_facts() {
    let package = Driver::Postgres.package();

    assert_eq!(package.registry_path, "postgresql");
    assert_eq!(package.library, "libadbc_driver_postgresql.so");
    assert_eq!(package.entry_point, "AdbcDriverPostgresqlInit");
    assert_eq!(package.version_range, ">=1.12.0, <2.0.0");
}

/// The generated tables are what the checked-in declaration says, field for
/// field and capability for capability.
///
/// Read back out of `drivers/<name>.toml` rather than compared against
/// literals: a literal here would be a second copy of the declaration, and
/// the two could then disagree in exactly the direction this asserts against.
///
/// It reads the generated side against the file, so it catches a value the
/// build invented, mangled, or read from the wrong declaration. That the file
/// declares nothing the generated side dropped is a per-technology claim about
/// a known set, asserted where that set is known: for MySQL, by
/// `capabilities_tests`' release-0.2 set.
#[test]
fn generated_tables_match_the_checked_in_toml() {
    for driver in Driver::ALL {
        let path = format!(
            "{}/drivers/{}.toml",
            env!("CARGO_MANIFEST_DIR"),
            driver.as_str()
        );
        let declaration = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("every identity has a declaration at {path}: {error}"));

        let package = driver.package();
        for (field, value) in [
            ("registry_path", package.registry_path),
            ("library", package.library),
            ("entry_point", package.entry_point),
            ("version_range", package.version_range),
        ] {
            assert!(
                declaration.contains(&format!("{field} = {value:?}")),
                "the generated `{field}` of `{}` is {value:?}, which {path} does not declare",
                driver.as_str()
            );
        }

        let capabilities = driver.capabilities();
        assert!(
            !capabilities.is_empty(),
            "`{}` advertises nothing, so {path} declares a technology that would delegate no \
             fragment at all",
            driver.as_str()
        );
        for capability in capabilities {
            assert!(
                declaration.contains(&format!("{capability:?}")),
                "the generated set of `{}` advertises {capability:?}, which {path} does not \
                 declare",
                driver.as_str()
            );
        }
    }
}
