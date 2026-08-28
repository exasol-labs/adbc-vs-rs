//! Parses the `dbc` driver registry's `index.yaml` and looks up one
//! driver's package for one version and platform.
//!
//! This module does no networking. It only turns the registry document
//! into structured data and answers lookups against it; fetching the
//! document and the package it points to belongs to `provisioning`.

use serde::Deserialize;

/// The registry index document: one entry per driver.
#[derive(Debug, Deserialize)]
struct Index {
    drivers: Vec<Driver>,
}

/// One driver's registry entry: its lookup path and every published
/// version.
#[derive(Debug, Deserialize)]
struct Driver {
    path: String,
    pkginfo: Vec<VersionBlock>,
}

/// One published version of a driver, with its per-platform packages.
#[derive(Debug, Deserialize)]
struct VersionBlock {
    version: String,
    packages: Vec<Package>,
}

/// One platform's package within a version block. `url` is relative to
/// the registry base URL.
#[derive(Debug, Deserialize)]
struct Package {
    platform: String,
    url: String,
}

/// Why a registry index document did not yield a package URL.
#[derive(Debug)]
pub enum LookupError {
    Unparseable(String),
    NoDriver,
    NoVersion,
    NoPlatform,
}

/// Resolves the absolute URL of one driver's package for one version and
/// platform, from a registry index document.
pub fn package_url(
    yaml: &str,
    driver: &str,
    version: &str,
    platform: &str,
    base_url: &str,
) -> Result<String, LookupError> {
    let index: Index = serde_yaml_ng::from_str(yaml)
        .map_err(|error| LookupError::Unparseable(error.to_string()))?;
    let driver = index
        .drivers
        .iter()
        .find(|entry| entry.path == driver)
        .ok_or(LookupError::NoDriver)?;
    let version_block = driver
        .pkginfo
        .iter()
        .find(|block| block.version == version)
        .ok_or(LookupError::NoVersion)?;
    let package = version_block
        .packages
        .iter()
        .find(|package| package.platform == platform)
        .ok_or(LookupError::NoPlatform)?;

    Ok(resolve_url(base_url, &package.url))
}

/// Resolves a package's relative `url` against the registry base URL.
pub fn resolve_url(base: &str, relative_url: &str) -> String {
    format!(
        "{}/{}",
        base.trim_end_matches('/'),
        relative_url.trim_start_matches('/')
    )
}

#[cfg(test)]
#[path = "registry_index_tests.rs"]
mod tests;
