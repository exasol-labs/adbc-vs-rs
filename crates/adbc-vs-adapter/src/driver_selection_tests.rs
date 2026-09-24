//! The rules that bind the driver identity's three tables.
//!
//! `vs-adapter/driver-selection` keys every per-driver fact on one identity so
//! a `match` over it is exhaustive and adding a variant fails the build at
//! each table that needs a new row. What a test can add to that is the half
//! the compiler cannot see: that the tables agree with the checked-in
//! declarations, that no two identities collide where a collision would make
//! one driver answer for another, and that a technology cannot be added by
//! data alone.

use adbc_vs_scan::driver::{Driver, DECLARED_DRIVERS};

use crate::adapter_notes::SourceQualifier;
use crate::metadata::{self, MetadataError};
use crate::sql_dialect::dialect_for;

/// The shape `metadata::source_for` must keep: it takes the qualifier the
/// `CONNECTION` address spells and answers with a source or with why that
/// qualifier is not one the driver declares.
type ResolveSource = fn(
    Driver,
    adbc_driver_manager::ManagedConnection,
    &SourceQualifier,
) -> Result<Box<dyn metadata::MetadataSource>, MetadataError>;

/// Checked by the compiler rather than at run time, because constructing what
/// `metadata::source_for` returns needs a live ADBC connection this tier has
/// none of by design.
const _: ResolveSource = metadata::source_for;

/// `vs-adapter/driver-selection`: each identity yields its own dialect and its
/// own package, and the two identities never share the values that would make
/// one driver's install or presence check answer for another's.
#[test]
fn each_driver_resolves_to_its_own_tables() {
    let mut libraries: Vec<&str> = Vec::new();
    let mut registry_paths: Vec<&str> = Vec::new();

    for driver in Driver::ALL {
        let package = driver.package();

        assert!(
            !libraries.contains(&package.library),
            "`{}` installs `{}`, which another identity already installs; the presence check \
             reads that filename, so one driver would answer for the other",
            driver.as_str(),
            package.library
        );
        assert!(
            !registry_paths.contains(&package.registry_path),
            "`{}` resolves its package under registry path `{}`, which another identity \
             already uses; both would resolve to the same published package",
            driver.as_str(),
            package.registry_path
        );

        libraries.push(package.library);
        registry_paths.push(package.registry_path);
    }
}

/// The value is operator-typed SQL text, and what is recorded is read back by
/// `getCapabilities` and `pushdown`, so the two must not depend on how it was
/// typed.
#[test]
fn driver_value_case_is_folded_to_canonical() {
    for driver in Driver::ALL {
        let canonical = driver.as_str();

        for spelling in [
            canonical.to_string(),
            canonical.to_ascii_uppercase(),
            fold_first_character(canonical),
        ] {
            let resolved = Driver::parse(&spelling)
                .unwrap_or_else(|error| panic!("`{spelling}` must resolve: {error}"));

            assert_eq!(resolved, *driver);
            assert_eq!(
                resolved.as_str(),
                canonical,
                "`{spelling}` must be recorded in the canonical spelling, not as it was typed"
            );
        }
    }
}

fn fold_first_character(value: &str) -> String {
    let mut characters = value.chars();
    match characters.next() {
        Some(first) => first.to_ascii_uppercase().to_string() + characters.as_str(),
        None => String::new(),
    }
}

/// `vs-adapter/driver-selection`: a `drivers/<name>.toml` file names a
/// technology, but a technology needs a dialect and a metadata source as well,
/// and neither is expressible as data. So the supported set is exactly the set
/// of identities compiled in, and dropping a file onto a cluster reaches
/// nothing — the declarations are read at build time and never at run time.
#[test]
fn supported_set_is_fixed_at_build_time() {
    let mut declared: Vec<&str> = DECLARED_DRIVERS.iter().map(|(name, _)| *name).collect();
    declared.sort_unstable();
    let mut supported: Vec<&str> = Driver::ALL.iter().map(|driver| driver.as_str()).collect();
    supported.sort_unstable();

    assert_eq!(
        declared, supported,
        "a declaration with no identity would be a technology nothing can render or discover \
         for, and an identity with no declaration would resolve no package"
    );

    for name in declared {
        Driver::parse(name)
            .unwrap_or_else(|error| panic!("a declared technology must be supported: {error}"));
    }
}

/// Every identity has a row in all three tables.
///
/// Two of the three are checked here. The metadata source is checked by the
/// compiler instead: `metadata::source_for` matches the identity with no
/// wildcard arm, so an identity without one does not build, and constructing
/// what it returns needs a live ADBC connection this tier has none of by
/// design. That its result answers that technology's own catalog is asserted
/// over the same `Driver::ALL` by
/// `metadata`'s `each_source_maps_its_own_catalog_row`.
#[test]
fn every_identity_yields_every_table() {
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
                "`{}` resolves a package whose `{field}` is empty, and every field of it is \
                 read without a fallback",
                driver.as_str()
            );
        }

        let capabilities = dialect_for(*driver).capabilities();
        assert!(
            !capabilities.is_empty(),
            "`{}` resolves a dialect advertising nothing",
            driver.as_str()
        );
    }
}
