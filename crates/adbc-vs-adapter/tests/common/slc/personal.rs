//! Installing the container into Exasol Personal, whose launcher installs a
//! custom container from a local tarball and restarts the database to load it.
//!
//! What the deployment already holds under [`LANGUAGE_ALIAS`] decides the
//! step, because the harness replaces only a container it could have
//! installed itself, and never one that `exasol slc install rust` installed.

use serde_json::Value;

use super::super::exasol::{launcher, Exasol, PERSONAL_DEPLOYMENT};
use super::{asset_name, cached_tarball, LANGUAGE_ALIAS, SLC_VERSION};

/// The custom-client language identifier language-container-rs implements.
const LANGUAGE: &str = "rust";

/// Where `exasol slc install rust` downloads its container from: the latest
/// language-container-rs release, which it records as the container's source.
const RELEASE_DOWNLOADS: &str = "https://github.com/exasol-labs/language-container-rs/releases/";

/// Makes the pinned container of the host's architecture the deployment's
/// [`LANGUAGE_ALIAS`], and returns once Exasol answers SQL again.
///
/// Repeat calls are safe: a deployment that already runs the pinned asset is
/// left alone, which also spares it the restart an install costs.
pub fn install(exasol: &Exasol) {
    let listing = launcher(
        "listing the script language containers of the Exasol Personal deployment",
        &["slc", "list", "--json"],
    );
    let action = match container_step(&listing, &asset_name(SLC_VERSION)) {
        ContainerStep::Keep => return,
        ContainerStep::Install => "install",
        ContainerStep::Update => "update",
        ContainerStep::TracksLatestRelease(source) => panic!("{}", foreign_container(&source)),
    };

    let tarball = cached_tarball();
    let source = tarball.to_string_lossy().into_owned();
    launcher(
        &format!(
            "running `exasol slc custom {action}` for the {LANGUAGE_ALIAS} container {source}"
        ),
        &[
            "slc",
            "custom",
            action,
            "--source",
            &source,
            "--alias",
            LANGUAGE_ALIAS,
            "--language",
            LANGUAGE,
            "--auto-approve",
        ],
    );
    exasol.await_sql();
}

/// What the deployment's `RUST` container calls for.
#[derive(Debug, PartialEq)]
enum ContainerStep {
    /// No container claims the alias.
    Install,
    /// The container is the pinned asset of the host's architecture.
    Keep,
    /// A custom container from another source, such as an earlier pinned
    /// version, holds the alias.
    Update,
    /// `exasol slc install rust` installed the container from this source,
    /// and it follows the latest release rather than the pinned one.
    TracksLatestRelease(String),
}

/// Classifies the `RUST` entry of an `exasol slc list --json` listing
/// against `pinned_asset`, the release asset file name the harness installs.
fn container_step(listing: &str, pinned_asset: &str) -> ContainerStep {
    let containers: Vec<Value> = serde_json::from_str(listing)
        .unwrap_or_else(|error| panic!("parsing `exasol slc list --json`: {error}\n{listing}"));
    let Some(rust) = containers.iter().find(|container| {
        container["type"] == "custom"
            && container["alias"]
                .as_str()
                .is_some_and(|alias| alias.eq_ignore_ascii_case(LANGUAGE_ALIAS))
    }) else {
        return ContainerStep::Install;
    };
    let source = rust["source"].as_str().unwrap_or_else(|| {
        panic!("`exasol slc list --json` reports no source for the {LANGUAGE_ALIAS} container")
    });

    let asset = source.rsplit_once('/').map_or(source, |(_, asset)| asset);
    if asset == pinned_asset {
        ContainerStep::Keep
    } else if source.starts_with(RELEASE_DOWNLOADS) {
        ContainerStep::TracksLatestRelease(source.to_owned())
    } else {
        ContainerStep::Update
    }
}

/// The failure for a container the harness must not replace.
fn foreign_container(source: &str) -> String {
    format!(
        "the {LANGUAGE_ALIAS} container of Exasol Personal deployment `{PERSONAL_DEPLOYMENT}` \
         comes from {source}, which `exasol slc install rust` installs and which follows the \
         latest language-container-rs release rather than the pinned {SLC_VERSION}. The harness \
         does not replace a container it did not install; remove it with `exasol slc remove \
         {LANGUAGE_ALIAS} --deployment {PERSONAL_DEPLOYMENT}` and run again"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const PINNED: &str = "lc-rust-0.23.1.tar.gz";

    /// An `exasol slc list --json` listing of launcher 2.3.0: the official
    /// catalog, plus `custom` when a custom container is installed.
    fn listing(custom: &str) -> String {
        format!(
            r#"[
  {{"type": "official", "language": "java", "flavor": "java-17", "version": "11.2.0", "aliases": ["JAVA", "JAVA17"], "installed": false}},
  {{"type": "official", "language": "python", "flavor": "python-3.12", "version": "11.2.0", "aliases": ["PYTHON3", "PYTHON312"], "installed": false}}{custom}
]"#
        )
    }

    fn rust_container_from(source: &str) -> String {
        listing(&format!(
            r#",
  {{"type": "custom", "language": "rust", "alias": "RUST", "source": "{source}", "installed": true, "available": true}}"#
        ))
    }

    #[test]
    fn a_deployment_with_no_rust_container_gets_the_pinned_one_installed() {
        assert_eq!(container_step(&listing(""), PINNED), ContainerStep::Install);
    }

    #[test]
    fn a_custom_container_of_another_alias_leaves_rust_to_be_installed() {
        let other = listing(
            r#",
  {"type": "custom", "language": "python", "alias": "MYPY3", "source": "/w/my-python.tar.gz", "installed": true, "available": true}"#,
        );

        assert_eq!(container_step(&other, PINNED), ContainerStep::Install);
    }

    #[test]
    fn the_pinned_asset_installed_from_its_cached_tarball_is_kept() {
        assert_eq!(
            container_step(
                &rust_container_from("/w/target/slc/lc-rust-0.23.1.tar.gz"),
                PINNED
            ),
            ContainerStep::Keep
        );
    }

    #[test]
    fn the_pinned_asset_installed_from_its_release_url_is_kept() {
        assert_eq!(
            container_step(
                &rust_container_from(
                    "https://github.com/exasol-labs/language-container-rs/releases/download/v0.23.1/lc-rust-0.23.1.tar.gz"
                ),
                PINNED
            ),
            ContainerStep::Keep
        );
    }

    #[test]
    fn a_container_from_an_earlier_pinned_tarball_is_updated() {
        assert_eq!(
            container_step(
                &rust_container_from("/w/target/slc/lc-rust-0.23.0.tar.gz"),
                PINNED
            ),
            ContainerStep::Update
        );
    }

    #[test]
    fn the_pinned_version_of_another_architecture_is_updated() {
        assert_eq!(
            container_step(
                &rust_container_from("/w/target/slc/lc-rust-0.23.1-aarch64.tar.gz"),
                PINNED
            ),
            ContainerStep::Update
        );
    }

    #[test]
    fn a_container_slc_install_rust_installed_is_not_replaced() {
        let latest = "https://github.com/exasol-labs/language-container-rs/releases/download/v0.24.0/lc-rust-0.24.0.tar.gz";

        assert_eq!(
            container_step(&rust_container_from(latest), PINNED),
            ContainerStep::TracksLatestRelease(latest.to_string())
        );
    }

    #[test]
    fn the_refusal_names_the_source_and_how_to_remove_the_container() {
        let message = foreign_container("https://github.com/x/lc-rust-0.24.0.tar.gz");

        assert!(
            message.contains("https://github.com/x/lc-rust-0.24.0.tar.gz")
                && message.contains("exasol slc remove RUST --deployment adbc-vs")
                && message.contains("0.23.1"),
            "{message}"
        );
    }
}
