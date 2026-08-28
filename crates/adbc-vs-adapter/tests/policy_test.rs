//! Licensing-policy checks (CLAUDE.md, "Licensing & dependency policy").
//!
//! Scope: this policy governs the Rust crate dependency tree only. The
//! `adbc-drivers/mysql` driver fetched at run time is Apache-2.0 and is
//! never bundled, so it falls outside these checks (decision-log [15]).

use std::fs;
use std::path::{Path, PathBuf};
#[cfg(feature = "policy-check")]
use std::process::Command;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/adbc-vs-adapter has two parent directories up to the workspace root")
        .to_path_buf()
}

#[test]
fn workspace_declares_mit_licensing() {
    let root = workspace_root();

    let cargo_toml = fs::read_to_string(root.join("Cargo.toml"))
        .expect("workspace root Cargo.toml must be readable");
    let manifest: toml::Value =
        toml::from_str(&cargo_toml).expect("workspace root Cargo.toml must be valid TOML");
    let license = manifest
        .get("workspace")
        .and_then(|workspace| workspace.get("package"))
        .and_then(|package| package.get("license"))
        .and_then(|license| license.as_str())
        .expect("[workspace.package].license must be set");
    assert_eq!(license, "MIT");

    let license_text =
        fs::read_to_string(root.join("LICENSE")).expect("LICENSE file must exist and be readable");
    assert!(
        license_text.contains("MIT License"),
        "LICENSE must contain the MIT license text"
    );
    assert!(
        license_text.contains("Copyright (c) 2026 Exasol AG"),
        "LICENSE must carry the copyright line \"Copyright (c) 2026 Exasol AG\""
    );

    for member in ["adbc-vs-adapter", "adbc-vs-scan"] {
        let manifest_path = root.join("crates").join(member).join("Cargo.toml");
        let manifest_text = fs::read_to_string(&manifest_path).unwrap_or_else(|error| {
            panic!("{} must be readable: {error}", manifest_path.display())
        });
        let manifest: toml::Value = toml::from_str(&manifest_text).unwrap_or_else(|error| {
            panic!("{} must be valid TOML: {error}", manifest_path.display())
        });
        let uses_workspace_license = manifest
            .get("package")
            .and_then(|package| package.get("license"))
            .and_then(|license| license.get("workspace"))
            .and_then(|workspace| workspace.as_bool())
            .unwrap_or(false);
        assert!(
            uses_workspace_license,
            "{member}'s Cargo.toml must set [package] license.workspace = true"
        );
    }
}

/// Behind `policy-check` because it needs `cargo-deny` on the host, which the
/// fast tier of `cargo test --workspace` does not require; `make deny`
/// installs nothing but enables the feature.
#[cfg(feature = "policy-check")]
#[test]
fn cargo_deny_check_passes() {
    let root = workspace_root();

    let output = Command::new("cargo")
        .args(["deny", "check", "licenses", "advisories", "bans", "sources"])
        .current_dir(&root)
        .output()
        .expect("could not run `cargo deny check` — is cargo-deny installed?");

    assert!(
        output.status.success(),
        "cargo deny check must exit 0\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn deny_toml() -> toml::Value {
    let root = workspace_root();
    let text =
        fs::read_to_string(root.join("deny.toml")).expect("workspace root deny.toml must exist");
    toml::from_str(&text).expect("deny.toml must be valid TOML")
}

#[test]
fn allow_list_excludes_copyleft() {
    let config = deny_toml();

    let allow = config
        .get("licenses")
        .and_then(|licenses| licenses.get("allow"))
        .and_then(|allow| allow.as_array())
        .expect("[licenses].allow must be an array");
    assert!(!allow.is_empty(), "[licenses].allow must not be empty");

    let copyleft_markers = ["GPL", "LGPL", "AGPL", "MPL", "OpenSSL"];
    for entry in allow {
        let identifier = entry
            .as_str()
            .expect("each [licenses].allow entry must be a string license identifier");
        for marker in copyleft_markers {
            assert!(
                !identifier.contains(marker),
                "[licenses].allow must not contain a copyleft or OpenSSL identifier, found {identifier:?}"
            );
        }
    }

    let exceptions = config
        .get("licenses")
        .and_then(|licenses| licenses.get("exceptions"))
        .and_then(|exceptions| exceptions.as_array())
        .expect("[licenses].exceptions must be an array");
    assert!(
        exceptions.is_empty(),
        "deny.toml must declare no [[licenses.exceptions]] entry unless a dependency actually \
         needs one, found {exceptions:?}"
    );
}

/// A line "looks dated" when it is a TOML comment (`#`) containing an
/// ISO-8601 date (`YYYY-MM-DD`), checked without a regex dependency.
fn is_dated_comment(line: &str) -> bool {
    let trimmed = line.trim_start();
    if !trimmed.starts_with('#') {
        return false;
    }

    let bytes = trimmed.as_bytes();
    if bytes.len() < 10 {
        return false;
    }
    (0..=bytes.len() - 10).any(|start| {
        let window = &trimmed[start..start + 10];
        let window_bytes = window.as_bytes();
        window_bytes[0..4].iter().all(u8::is_ascii_digit)
            && window_bytes[4] == b'-'
            && window_bytes[5..7].iter().all(u8::is_ascii_digit)
            && window_bytes[7] == b'-'
            && window_bytes[8..10].iter().all(u8::is_ascii_digit)
    })
}

/// A line is a real `[advisories].ignore` array entry (a quoted id or an
/// inline table) rather than blank, a comment, or the array's closing
/// bracket.
fn is_ignore_entry(trimmed: &str) -> bool {
    !trimmed.starts_with('#') && trimmed != "]" && !trimmed.is_empty()
}

/// Every `[advisories].ignore` entry in `deny_toml_text` not immediately
/// preceded by a dated comment.
fn undated_ignore_entries(deny_toml_text: &str) -> Vec<String> {
    let lines: Vec<&str> = deny_toml_text.lines().collect();

    let advisories_start = lines
        .iter()
        .position(|line| line.trim() == "[advisories]")
        .expect("deny.toml must have an [advisories] section");
    let advisories_end = lines[advisories_start + 1..]
        .iter()
        .position(|line| line.trim_start().starts_with('['))
        .map(|offset| advisories_start + 1 + offset)
        .unwrap_or(lines.len());
    let section = &lines[advisories_start..advisories_end];

    let ignore_start = section
        .iter()
        .position(|line| line.trim_start().starts_with("ignore"))
        .expect("[advisories] must have an `ignore` key");
    let array_end = section[ignore_start..]
        .iter()
        .position(|line| line.trim_end().ends_with(']'))
        .map(|offset| ignore_start + offset)
        .expect("[advisories].ignore array must be closed with `]`");

    let mut last_non_blank_was_dated_comment = false;
    let mut undated_entries = Vec::new();
    for line in &section[ignore_start + 1..=array_end] {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if is_dated_comment(line) {
            last_non_blank_was_dated_comment = true;
            continue;
        }
        if trimmed.starts_with('#') {
            last_non_blank_was_dated_comment = false;
            continue;
        }
        if is_ignore_entry(trimmed) && !last_non_blank_was_dated_comment {
            undated_entries.push(trimmed.to_string());
        }
        last_non_blank_was_dated_comment = false;
    }

    undated_entries
}

#[test]
fn advisory_ignores_are_dated() {
    let root = workspace_root();
    let raw = fs::read_to_string(root.join("deny.toml")).expect("deny.toml must be readable");

    let undated_entries = undated_ignore_entries(&raw);

    assert!(
        undated_entries.is_empty(),
        "each [advisories].ignore entry must be immediately preceded by a comment stating the \
         ISO-8601 date the exception was granted; entries missing one: {undated_entries:?}"
    );
}

#[test]
fn a_dated_comment_immediately_above_an_entry_needs_no_further_justification() {
    let text = "[advisories]\nignore = [\n    # 2026-08-26 justified by decision-log [7]\n    \"RUSTSEC-2020-0001\",\n]\n";

    assert!(undated_ignore_entries(text).is_empty());
}

#[test]
fn an_undated_comment_above_an_entry_leaves_it_reported() {
    let text =
        "[advisories]\nignore = [\n    # justified, but not dated\n    \"RUSTSEC-2020-0001\",\n]\n";

    let undated = undated_ignore_entries(text);
    assert_eq!(undated.len(), 1);
    assert!(undated[0].contains("RUSTSEC-2020-0001"));
}

#[test]
fn an_entry_with_no_comment_at_all_is_reported() {
    let text = "[advisories]\nignore = [\n    \"RUSTSEC-2020-0001\",\n]\n";

    let undated = undated_ignore_entries(text);
    assert_eq!(undated.len(), 1);
    assert!(undated[0].contains("RUSTSEC-2020-0001"));
}

#[test]
fn is_dated_comment_recognizes_an_iso_8601_date() {
    assert!(is_dated_comment(
        "# 2026-08-26 justified by decision-log [7]"
    ));
}

#[test]
fn is_dated_comment_rejects_a_comment_with_no_date() {
    assert!(!is_dated_comment("# justified, but not dated"));
}
