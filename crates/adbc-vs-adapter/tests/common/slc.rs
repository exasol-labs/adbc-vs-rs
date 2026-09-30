//! Making the `RUST` script language available in the stack's Exasol.
//!
//! A Script Language Container is a tarball that BucketFS extracts and that a
//! `SCRIPT_LANGUAGES` alias then points at. This module owns that whole
//! sequence, so callers only ever learn that the language is usable.

use std::fs;
use std::path::PathBuf;
use std::process;

use super::exasol::{self, Exasol};
use super::run;

/// The alias `CREATE ... RUST ... SCRIPT` statements name.
pub const LANGUAGE_ALIAS: &str = "RUST";

/// The container release to install.
///
/// A UDF shared object only loads into the container built from the same
/// `exasol-udf-sdk`, so this MUST stay equal to the `exasol-udf-sdk` version
/// the crates depend on.
pub const SLC_VERSION: &str = "0.23.1";

/// Where the tarball lands in the bucket, and — once BucketFS extracts it —
/// the directory the alias points at.
const SLC_NAME: &str = "rustslc";

/// The parameter whose value lists every registered script language.
const SCRIPT_LANGUAGES: &str = "SCRIPT_LANGUAGES";

/// Installs the Script Language Container and registers its alias.
///
/// Registration is `ALTER SYSTEM`, not `ALTER SESSION`: the harness reaches
/// Exasol through short-lived sessions, and every later statement — creating a
/// script, creating a virtual schema, querying one — has to find the language
/// in a session this call never sees. The existing value is merged rather than
/// replaced, so the built-in languages survive.
///
/// Repeat calls are safe. The call returns only once the archive is extracted
/// and the script client the alias names is readable: BucketFS acknowledges
/// the upload well before it finishes unpacking a container of this size, and
/// a script created against a half-extracted one fails at run time.
pub fn install(exasol: &Exasol) {
    exasol.put_file(&cached_tarball(), &format!("slc/{SLC_NAME}.tar.gz"));
    exasol.await_bucket_path(&script_client_path());

    let current = exasol
        .query_scalar(&format!(
            "SELECT SYSTEM_VALUE FROM EXA_PARAMETERS \
             WHERE PARAMETER_NAME = '{SCRIPT_LANGUAGES}'"
        ))
        .unwrap_or_default();
    let merged = merged_script_languages(&current, &language_definition());
    assert!(
        !merged.contains('\''),
        "{SCRIPT_LANGUAGES} would carry a quote that breaks the statement: {merged}"
    );

    exasol.execute(&format!("ALTER SYSTEM SET {SCRIPT_LANGUAGES} = '{merged}'"));
}

/// The `RUST=...` entry for `SCRIPT_LANGUAGES`.
///
/// The URL names the directory BucketFS extracts the archive into, and the
/// fragment names the script client inside it.
fn language_definition() -> String {
    let directory = exasol::bucketfs_path(&format!("slc/{SLC_NAME}"));
    format!(
        "{LANGUAGE_ALIAS}=localzmq+protobuf:///{directory}?lang=rust\
         #buckets/{directory}/{SCRIPT_CLIENT}"
    )
}

/// The script client inside the extracted container, relative to it. Its
/// presence is what proves the extraction finished.
const SCRIPT_CLIENT: &str = "exaudf/exaudfclient";

/// The bucket-relative path of that script client.
fn script_client_path() -> String {
    format!("slc/{SLC_NAME}/{SCRIPT_CLIENT}")
}

/// Adds `definition` to `current`, dropping any entry that already claims the
/// same alias.
///
/// Entries are space-separated `ALIAS=url` pairs whose URLs carry `=` of their
/// own, so an alias is only ever the text before the first one.
fn merged_script_languages(current: &str, definition: &str) -> String {
    let alias = alias_of(definition);
    let mut entries: Vec<&str> = current
        .split_whitespace()
        .filter(|entry| !alias_of(entry).eq_ignore_ascii_case(alias))
        .collect();
    entries.push(definition);
    entries.join(" ")
}

/// The language alias an entry registers.
fn alias_of(entry: &str) -> &str {
    match entry.split_once('=') {
        Some((alias, _)) => alias,
        None => entry,
    }
}

/// The release asset holding the container for `version`.
fn release_url(version: &str) -> String {
    format!(
        "https://github.com/exasol-labs/language-container-rs/releases/download/\
         v{version}/lc-rust-{version}.tar.gz"
    )
}

/// The downloaded tarball, fetching it on first use.
///
/// The download lands on a private path and is renamed into place only once
/// `curl` reports success, so an interrupted run never leaves a truncated
/// archive that later runs would upload as if it were whole.
fn cached_tarball() -> PathBuf {
    let cache = exasol::workspace_root().join("target").join("slc");
    let tarball = cache.join(format!("lc-rust-{SLC_VERSION}.tar.gz"));
    if tarball.metadata().is_ok_and(|file| file.len() > 0) {
        return tarball;
    }

    fs::create_dir_all(&cache)
        .unwrap_or_else(|error| panic!("creating {}: {error}", cache.display()));

    let url = release_url(SLC_VERSION);
    let partial = cache.join(format!("lc-rust-{SLC_VERSION}.tar.gz.{}", process::id()));
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

    #[test]
    fn language_definition_points_at_the_extracted_directory() {
        let definition = language_definition();
        assert!(
            !definition.contains(".tar.gz"),
            "alias must name the extracted directory, not the archive: {definition}"
        );
        assert!(
            definition.starts_with("RUST=localzmq+protobuf:///bfsdefault/default/slc/rustslc?"),
            "unexpected alias URL: {definition}"
        );
    }

    #[test]
    fn language_definition_names_the_script_client() {
        assert!(
            language_definition()
                .ends_with("#buckets/bfsdefault/default/slc/rustslc/exaudf/exaudfclient"),
            "alias must name the script client inside the container: {}",
            language_definition()
        );
    }

    #[test]
    fn merged_script_languages_keeps_the_built_in_languages() {
        let merged = merged_script_languages("PYTHON3=builtin_python3 JAVA=builtin_java", "RUST=x");
        assert_eq!(merged, "PYTHON3=builtin_python3 JAVA=builtin_java RUST=x");
    }

    #[test]
    fn merged_script_languages_replaces_an_earlier_entry_for_the_same_alias() {
        let merged = merged_script_languages("PYTHON3=p RUST=old JAVA=j", "RUST=new");
        assert_eq!(merged, "PYTHON3=p JAVA=j RUST=new");
    }

    #[test]
    fn merged_script_languages_drops_every_earlier_entry_for_the_same_alias() {
        let merged = merged_script_languages("RUST=one PYTHON3=p RUST=two", "RUST=new");
        assert_eq!(merged, "PYTHON3=p RUST=new");
    }

    #[test]
    fn merged_script_languages_keeps_an_alias_that_merely_starts_with_the_same_letters() {
        let merged = merged_script_languages("RUSTY=r", "RUST=new");
        assert_eq!(merged, "RUSTY=r RUST=new");
    }

    #[test]
    fn merged_script_languages_ignores_the_equals_signs_inside_a_url() {
        let merged = merged_script_languages("RUST=localzmq+protobuf:///a?lang=rust#b", "RUST=new");
        assert_eq!(merged, "RUST=new");
    }

    #[test]
    fn merged_script_languages_from_an_empty_value_carries_no_leading_space() {
        assert_eq!(merged_script_languages("", "RUST=new"), "RUST=new");
    }

    #[test]
    fn release_url_names_the_asset_of_the_pinned_version() {
        assert_eq!(
            release_url("0.23.0"),
            "https://github.com/exasol-labs/language-container-rs/releases/download/v0.23.0/lc-rust-0.23.0.tar.gz"
        );
    }
}
