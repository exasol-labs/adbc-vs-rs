//! Parses the `dbc` driver registry's `index.yaml` and resolves one driver's
//! declared version range to one published package.
//!
//! This module does no networking. It only turns the registry document into
//! structured data and answers lookups against it; fetching the document and
//! the package it points to belongs to `provisioning`.
//!
//! Resolution is semantic rather than textual, and that is the module's whole
//! reason to exist as its own unit. The registry's version spellings are not
//! uniform — the MySQL entry publishes `v0.6.0` where the PostgreSQL entry
//! publishes `1.12.0` — so a caller comparing strings would need a
//! per-technology rule. Parsing every published scalar, normalizing one
//! optional leading `v`, and taking the highest that satisfies the range makes
//! one rule serve every driver, and lets a driver state which versions it
//! accepts rather than which single version it was released against.
//!
//! A scalar that is no version at all is skipped rather than surfaced. The
//! registry is a third party's document and adbc-vs has no say in what it
//! publishes, so one malformed entry beside the wanted one must not make a
//! driver uninstallable. A published prerelease is skipped for the same
//! reason a declaration would have to opt into one explicitly: it is inside
//! the range numerically and outside what the range was written to accept.

use semver::{Version, VersionReq};
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

/// The one published package a driver's range resolved to.
///
/// The version travels with the URL because every record made downstream is
/// keyed on it: the BucketFS install path carries it, the presence check tests
/// for it, and `adapterNotes` reports it. Deriving it back out of the URL would
/// put the same decision in two places.
#[derive(Debug)]
pub struct ResolvedPackage {
    pub version: Version,
    pub url: String,
}

/// Why a registry index document did not yield a package.
///
/// Each variant carries what the caller could not already know — the parse
/// cause, what the index publishes, which version the range resolved to — and
/// nothing it handed in. Rendering an operator-facing message is
/// `provisioning`'s, because the index URL that message needs is its fact.
#[derive(Debug)]
pub enum LookupError {
    Unparseable(String),
    NoDriver,
    UnparseableRange(String),
    NoVersionInRange { published: Vec<String> },
    NoPlatform { resolved: Version },
}

/// Resolves a driver's version range against a registry index document to the
/// highest published version satisfying it, and that version's package for one
/// platform.
pub fn resolve_package(
    yaml: &str,
    driver: &str,
    version_range: &str,
    platform: &str,
    base_url: &str,
) -> Result<ResolvedPackage, LookupError> {
    let accepted = VersionReq::parse(version_range)
        .map_err(|error| LookupError::UnparseableRange(error.to_string()))?;
    let index: Index = serde_yaml_ng::from_str(yaml)
        .map_err(|error| LookupError::Unparseable(error.to_string()))?;
    let driver = index
        .drivers
        .iter()
        .find(|entry| entry.path == driver)
        .ok_or(LookupError::NoDriver)?;

    let resolved = driver
        .pkginfo
        .iter()
        .filter_map(|block| Some((published_version(&block.version)?, block)))
        .filter(|(version, _)| accepted.matches(version))
        .max_by(|(one, _), (other, _)| one.cmp(other));

    let Some((version, block)) = resolved else {
        return Err(LookupError::NoVersionInRange {
            published: driver
                .pkginfo
                .iter()
                .map(|block| block.version.clone())
                .collect(),
        });
    };

    let package = block
        .packages
        .iter()
        .find(|package| package.platform == platform)
        .ok_or(LookupError::NoPlatform {
            resolved: version.clone(),
        })?;

    Ok(ResolvedPackage {
        version,
        url: resolve_url(base_url, &package.url),
    })
}

/// One published `version` scalar as a comparable version, or nothing at all
/// if the registry published something that is not one.
///
/// Exactly one leading `v` is normalized away, because that is the whole
/// difference between the two spellings the registry actually uses. A scalar
/// carrying anything else in front stays unparseable rather than being
/// massaged into a version it never claimed to be.
fn published_version(published: &str) -> Option<Version> {
    Version::parse(published.strip_prefix('v').unwrap_or(published)).ok()
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
