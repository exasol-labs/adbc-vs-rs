//! Fetches a driver package from the real registry, exactly the way
//! `adbc-vs-adapter::provisioning` resolves and installs one, and caches its
//! extracted library on the local filesystem.
//!
//! This is the only route an integration tier has to a driver library outside
//! BucketFS, so it has to agree with production about where a package lives —
//! and it agrees by calling production's own code rather than by restating
//! it. `registry_index::resolve_package` resolves the driver's declared range
//! against the same `index.yaml`, `provisioning::declared_library` reads the
//! library's file name out of the package's `MANIFEST`, and
//! `provisioning::verify_library_filename` refuses a package declaring
//! anything but the compiled-in name. Nothing here holds a second definition
//! of either document's shape, so a registry that changes one cannot leave
//! this tier certifying against the old one.
//!
//! Reaching the adapter from here is a dev-dependency and only a
//! dev-dependency: the shipped crates run `adbc-vs-adapter -> adbc-vs-scan`,
//! and cargo resolves dev-dependencies separately, so the two directions
//! together are not a cycle it refuses.
//!
//! Takes a [`Driver`] rather than naming MySQL, so a PostgreSQL integration
//! tier calls the same function this one does.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use adbc_vs_adapter::provisioning::{declared_library, verify_library_filename};
use adbc_vs_adapter::registry_index;
use adbc_vs_scan::driver::{Driver, PLATFORM, REGISTRY_BASE_URL};

const REGISTRY_INDEX_FILE: &str = "index.yaml";
const MANIFEST_MEMBER: &str = "MANIFEST";

/// Fetches, resolves, and caches `driver`'s ADBC library from the real
/// registry, and returns the path to the cached, extracted library.
///
/// Caches under the resolved version rather than a compiled-in one, so a
/// second call in the same run — or a later run once the registry has
/// published a new version in range — never serves a stale cache entry as if
/// it were the version that was actually resolved.
pub fn fetch_driver_library(driver: Driver) -> PathBuf {
    let package = driver.package();
    let index_url = registry_index::resolve_url(REGISTRY_BASE_URL, REGISTRY_INDEX_FILE);
    let index = String::from_utf8(download(&index_url))
        .unwrap_or_else(|error| panic!("the registry index at {index_url} is not UTF-8: {error}"));
    let resolved = registry_index::resolve_package(
        &index,
        package.registry_path,
        package.version_range,
        PLATFORM,
        REGISTRY_BASE_URL,
    )
    .unwrap_or_else(|error| {
        panic!(
            "the registry index at {index_url} must publish a `{PLATFORM}` package of `{}` \
             satisfying `{}`: {error:?}",
            driver.as_str(),
            package.version_range
        )
    });

    let directory = std::env::temp_dir().join(format!(
        "adbc-vs-driver-cache/{}-{}",
        driver.as_str(),
        resolved.version
    ));
    let installed = directory.join(package.library);
    if installed.is_file() {
        return installed;
    }
    fs::create_dir_all(&directory).expect("the driver cache directory is creatable");

    let archive = download(&resolved.url);
    let library = extract_declared_library(&archive, &resolved.url, driver);

    let staged = directory.join(format!("{}.{}", package.library, std::process::id()));
    fs::write(&staged, library).expect("the extracted driver is writable");
    fs::rename(&staged, &installed).expect("the extracted driver moves into the cache");
    installed
}

/// Extracts the shared library a downloaded package's `MANIFEST` names, rather
/// than a file matching the compiled-in name by convention, and fails loudly
/// if the two diverge.
///
/// Both halves are the product's own: reading the declaration and refusing a
/// divergent one are what `provisioning` does before it uploads a driver, and
/// a tier that applied its own rule here could load a library production would
/// have rejected.
fn extract_declared_library(package: &[u8], url: &str, driver: Driver) -> Vec<u8> {
    let manifest = find_member(package, MANIFEST_MEMBER)
        .unwrap_or_else(|| panic!("the driver package at {url} declares no {MANIFEST_MEMBER}"));
    let manifest = String::from_utf8(manifest).unwrap_or_else(|error| {
        panic!("the {MANIFEST_MEMBER} of the driver package at {url} is not UTF-8: {error}")
    });

    let declared = declared_library(&manifest, url).unwrap_or_else(|error| panic!("{error}"));
    verify_library_filename(&declared, driver).unwrap_or_else(|error| {
        panic!(
            "the driver package at {url} cannot serve the {} tier: {error}",
            driver.as_str()
        )
    });

    find_member(package, &declared).unwrap_or_else(|| {
        panic!(
            "the {MANIFEST_MEMBER} of the driver package at {url} names `{declared}`, but the \
             archive holds no such file"
        )
    })
}

/// One archive member's raw bytes, matched by its exact path within the tar
/// archive.
fn find_member(package: &[u8], name: &str) -> Option<Vec<u8>> {
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(package));
    let entries = archive
        .entries()
        .expect("the driver package is a readable tar archive");
    for entry in entries {
        let mut entry = entry.expect("every driver package member is readable");
        let path = entry.path().expect("every member has a path").into_owned();
        if path == Path::new(name) {
            let mut bytes = Vec::new();
            entry
                .read_to_end(&mut bytes)
                .expect("the member is readable");
            return Some(bytes);
        }
    }
    None
}

fn download(url: &str) -> Vec<u8> {
    let response =
        reqwest::blocking::get(url).unwrap_or_else(|error| panic!("fetching {url}: {error}"));
    let response = response
        .error_for_status()
        .unwrap_or_else(|error| panic!("fetching {url}: {error}"));
    response
        .bytes()
        .unwrap_or_else(|error| panic!("reading {url}: {error}"))
        .to_vec()
}
