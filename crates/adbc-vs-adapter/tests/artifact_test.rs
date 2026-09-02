//! Artifact scenarios (`specs/vs-adapter/adapter-request-dispatch`,
//! `specs/scan-udf/arrow-scan`, and `specs/driver-provisioning/driver-self-install`).
//!
//! These tests need the real SLC-compatible shared object at
//! `target/release/libadbc_vs_adapter.so`, built inside `rust:1.94-trixie` by
//! `make cross-udf-build`. That is why they sit behind the `built-artifact`
//! feature: `make test-artifact` declares the build target as its prerequisite
//! and enables the feature, while a plain `cargo test --workspace` compiles
//! this file away rather than failing on a machine with no Docker. Enable the
//! feature without the build and it fails with a clear message instead of
//! silently skipping.
#![cfg(feature = "built-artifact")]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The published ADBC MySQL driver package's size in bytes, as the registry
/// served it for `v0.6.0`. Which version a cluster installs is now the range
/// resolved at run time, so this is an order-of-magnitude reference point
/// rather than a number to keep in step. `adbc-vs-adapter` never bundles
/// this driver (decision-log; CLAUDE.md's licensing scope rule); an adapter
/// `.so` anywhere near this size would mean it had been statically linked or
/// embedded in instead of being fetched into BucketFS at run time.
const DRIVER_PACKAGE_BYTES: u64 = 46_408_368;

const ADAPTER_ENTRY_SYMBOL: &str = "__exa_udf_entry_ADBC_ADAPTER";
const SCAN_ENTRY_SYMBOL: &str = "__exa_udf_entry_ADBC_SCAN";

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/adbc-vs-adapter has two parent directories up to the workspace root")
        .to_path_buf()
}

fn built_so_path() -> PathBuf {
    workspace_root().join("target/release/libadbc_vs_adapter.so")
}

fn require_built_so() -> PathBuf {
    let path = built_so_path();
    assert!(
        path.is_file(),
        "{} must exist — run `make cross-udf-build` (or `make test-artifact`, which depends \
         on it) before this test",
        path.display()
    );
    path
}

/// The adapter `.so`'s defined dynamic symbols, one per line, as `nm -D
/// --defined-only` reports them. Panics rather than skipping if `nm` itself
/// is unavailable or fails, per this project's no-silent-skip rule.
fn defined_dynamic_symbols(so_path: &Path) -> String {
    let output = Command::new("nm")
        .args(["-D", "--defined-only"])
        .arg(so_path)
        .output()
        .unwrap_or_else(|error| {
            panic!(
                "could not run `nm -D --defined-only {}`: {error}",
                so_path.display()
            )
        });
    assert!(
        output.status.success(),
        "`nm -D --defined-only {}` exited with {}\nstderr:\n{}",
        so_path.display(),
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn exports_symbol(so_path: &Path, symbol: &str) -> bool {
    defined_dynamic_symbols(so_path)
        .lines()
        .any(|line| line.split_whitespace().last() == Some(symbol))
}

/// No driver this release supports is in the shipped artifact, by either route
/// it could get there: statically linked into the `.so`, or committed to the
/// repository.
///
/// Enumerated from [`Driver::ALL`] rather than hand-listed, because both facts
/// a driver is checked against — its entry-point symbol and its library file
/// name — are already on the identity, so a technology added to it inherits
/// the assertion instead of needing a second spelling here that a maintainer
/// could forget.
#[test]
fn no_bundled_driver_binary() {
    let root = workspace_root();

    let tracked_files = Command::new("git")
        .args(["ls-files"])
        .current_dir(&root)
        .output()
        .expect("could not run `git ls-files`");
    assert!(
        tracked_files.status.success(),
        "`git ls-files` must exit 0\nstderr:\n{}",
        String::from_utf8_lossy(&tracked_files.stderr)
    );
    let tracked = String::from_utf8_lossy(&tracked_files.stdout);

    let so_path = require_built_so();
    let so_size = fs::metadata(&so_path)
        .unwrap_or_else(|error| panic!("could not stat {}: {error}", so_path.display()))
        .len();
    assert!(
        so_size < DRIVER_PACKAGE_BYTES / 2,
        "{} is {so_size} bytes, within range of the {DRIVER_PACKAGE_BYTES}-byte driver package \
         — it must never statically link or embed the driver",
        so_path.display()
    );

    for driver in adbc_vs_scan::driver::Driver::ALL {
        let library = driver.package().library;
        let tracked_driver_files: Vec<&str> = tracked
            .lines()
            .filter(|path| *path == library || path.ends_with(&format!("/{library}")))
            .collect();
        assert!(
            tracked_driver_files.is_empty(),
            "no tracked file may be the ADBC {} driver binary {library}, found: {:?}",
            driver.as_str(),
            tracked_driver_files
        );

        let entry_point = driver.package().entry_point;
        assert!(
            !exports_symbol(&so_path, entry_point),
            "{} must not export the {} driver's own {entry_point} symbol — the driver is \
             fetched into BucketFS at run time, never present in the built artifact",
            so_path.display(),
            driver.as_str()
        );
    }
}

#[test]
fn exports_adapter_entry_symbol() {
    let so_path = require_built_so();
    assert!(
        exports_symbol(&so_path, ADAPTER_ENTRY_SYMBOL),
        "{} must export {ADAPTER_ENTRY_SYMBOL}, the ADAPTER SCRIPT entry point",
        so_path.display()
    );
}

#[test]
fn exports_scan_entry_symbol() {
    let so_path = require_built_so();
    assert!(
        exports_symbol(&so_path, SCAN_ENTRY_SYMBOL),
        "{} must export {SCAN_ENTRY_SYMBOL}, the scan SCALAR ... EMITS entry point",
        so_path.display()
    );
}
