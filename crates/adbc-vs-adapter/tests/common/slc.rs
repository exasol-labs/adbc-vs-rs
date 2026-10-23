//! Making the `RUST` script language available in the stack's Exasol.
//!
//! The container is the pinned language-container-rs release asset of the
//! host's architecture. How it is installed is the Exasol backend's: Docker
//! Exasol extracts it from BucketFS behind a `SCRIPT_LANGUAGES` alias, and
//! Exasol Personal installs it through its launcher. This module owns both
//! sequences, so callers only ever learn that the language is usable.

use std::fs;
use std::path::PathBuf;
use std::process;

use super::exasol;
use super::run;

#[cfg(not(feature = "exasol-personal"))]
mod docker;
#[cfg(feature = "exasol-personal")]
mod personal;

#[cfg(not(feature = "exasol-personal"))]
pub use docker::install;
#[cfg(feature = "exasol-personal")]
pub use personal::install;

/// The alias `CREATE ... RUST ... SCRIPT` statements name.
pub const LANGUAGE_ALIAS: &str = "RUST";

/// The container release to install.
///
/// A UDF shared object only loads into the container built from the same
/// `exasol-udf-sdk`, so this MUST stay equal to the `exasol-udf-sdk` version
/// the crates depend on.
pub const SLC_VERSION: &str = "0.23.1";

/// The suffix language-container-rs gives the release asset of the
/// architecture this harness runs on.
#[cfg(target_arch = "x86_64")]
const ASSET_ARCHITECTURE_SUFFIX: &str = "";
#[cfg(target_arch = "aarch64")]
const ASSET_ARCHITECTURE_SUFFIX: &str = "-aarch64";

/// The file name of the release asset holding the container for `version`.
fn asset_name(version: &str) -> String {
    format!("lc-rust-{version}{ASSET_ARCHITECTURE_SUFFIX}.tar.gz")
}

/// The release asset holding the container for `version`.
fn release_url(version: &str) -> String {
    format!(
        "https://github.com/exasol-labs/language-container-rs/releases/download/v{version}/{}",
        asset_name(version)
    )
}

/// The downloaded tarball, fetching it on first use.
///
/// The download lands on a private path and is renamed into place only once
/// `curl` reports success, so an interrupted run never leaves a truncated
/// archive that later runs would upload as if it were whole.
fn cached_tarball() -> PathBuf {
    let cache = exasol::workspace_root().join("target").join("slc");
    let tarball = cache.join(asset_name(SLC_VERSION));
    if tarball.metadata().is_ok_and(|file| file.len() > 0) {
        return tarball;
    }

    fs::create_dir_all(&cache)
        .unwrap_or_else(|error| panic!("creating {}: {error}", cache.display()));

    let url = release_url(SLC_VERSION);
    let partial = cache.join(format!("{}.{}", asset_name(SLC_VERSION), process::id()));
    let destination = partial.to_string_lossy().into_owned();
    run(
        &format!("downloading the Rust script language container from {url}"),
        "curl",
        &[
            "--fail",
            "--silent",
            "--show-error",
            "--location",
            "--proto",
            "=https",
            "--max-time",
            "600",
            "--output",
            &destination,
            &url,
        ],
    );

    fs::rename(&partial, &tarball).unwrap_or_else(|error| {
        panic!(
            "moving the downloaded container to {}: {error}",
            tarball.display()
        )
    });
    tarball
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The release publishes one asset per architecture, and only the one
    /// built for the host's architecture loads its UDFs.
    #[cfg(target_arch = "x86_64")]
    #[test]
    fn release_url_names_the_asset_of_the_pinned_version() {
        assert_eq!(
            release_url("0.23.0"),
            "https://github.com/exasol-labs/language-container-rs/releases/download/v0.23.0/lc-rust-0.23.0.tar.gz"
        );
    }

    /// The release publishes one asset per architecture, and only the one
    /// built for the host's architecture loads its UDFs.
    #[cfg(target_arch = "aarch64")]
    #[test]
    fn release_url_names_the_asset_of_the_pinned_version() {
        assert_eq!(
            release_url("0.23.0"),
            "https://github.com/exasol-labs/language-container-rs/releases/download/v0.23.0/lc-rust-0.23.0-aarch64.tar.gz"
        );
    }
}
