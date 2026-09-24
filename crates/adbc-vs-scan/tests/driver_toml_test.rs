//! Asserts that a malformed driver declaration fails the build.
//!
//! Adding a technology is a build-time edit, so the build script is the only
//! thing standing between a mistake in `drivers/*.toml` and a driver table
//! silently missing a field. Its refusal is driven by running the real
//! compiled build script over deliberately broken declarations, rather than
//! by reading it: the generator's whole interface is its exit status and the
//! message it leaves on stderr.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// The compiled build script that produced this test's own `OUT_DIR`.
///
/// Running the real executable is what makes these assertions about the build
/// rather than about a reimplementation of it: the generator's only interface
/// is the exit status and the message it leaves on stderr.
///
/// Cargo runs a build script's *compilation* and its *execution* (the one that
/// produced `OUT_DIR`) as two differently fingerprinted units, so
/// `build-script-build` never sits beside this test's own `OUT_DIR` and no
/// file records which one produced it. But cargo must compile a build script
/// before it can run it, so the binary this `OUT_DIR` came from is the newest
/// `build-script-build` whose own modification time is no later than the
/// `invoked.timestamp` cargo drops beside `OUT_DIR` when it ran that binary.
/// Picking the newest one over the whole build directory instead — with no
/// reference to this `OUT_DIR` at all — can return a binary compiled for an
/// unrelated feature set that merely finished building more recently.
fn build_script() -> PathBuf {
    let out_dir = PathBuf::from(env!("OUT_DIR"));
    let run_dir = out_dir
        .parent()
        .unwrap_or_else(|| panic!("OUT_DIR {} has no run directory", out_dir.display()));
    let build_root = run_dir
        .parent()
        .unwrap_or_else(|| panic!("OUT_DIR {} has no build directory", out_dir.display()));
    let invoked_at = fs::metadata(run_dir.join("invoked.timestamp"))
        .and_then(|metadata| metadata.modified())
        .unwrap_or_else(|error| {
            panic!("reading {}'s invoked.timestamp: {error}", run_dir.display())
        });

    fs::read_dir(build_root)
        .unwrap_or_else(|error| panic!("reading {}: {error}", build_root.display()))
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path().join("build-script-build"))
        .filter(|path| path.is_file())
        .filter(|path| {
            path.parent()
                .and_then(Path::file_name)
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("adbc-vs-scan-"))
        })
        .filter_map(|path| {
            let built = fs::metadata(&path).and_then(|metadata| metadata.modified());
            built.ok().map(|built| (built, path))
        })
        .filter(|(built, _)| *built <= invoked_at)
        .max_by_key(|(built, _)| *built)
        .map(|(_, path)| path)
        .unwrap_or_else(|| {
            panic!(
                "no adbc-vs-scan build script under {} was compiled before {} ran it",
                build_root.display(),
                run_dir.display()
            )
        })
}

/// A throwaway crate root holding `drivers/` with the given files, deleted
/// before any assertion runs so a failing assertion leaves nothing behind.
struct DeclarationFixture {
    root: PathBuf,
}

impl DeclarationFixture {
    fn holding(label: &str, files: &[(&str, &str)]) -> Self {
        let root = std::env::temp_dir().join(format!(
            "adbc-vs-driver-declarations/{}-{label}",
            std::process::id()
        ));
        let drivers = root.join("drivers");
        fs::create_dir_all(&drivers).expect("the fixture drivers directory is creatable");
        fs::create_dir_all(root.join("out")).expect("the fixture output directory is creatable");
        for (name, content) in files {
            fs::write(drivers.join(name), content).expect("a fixture declaration is writable");
        }
        Self { root }
    }

    /// Runs the build script over this fixture and removes it, so the caller
    /// asserts against the captured result rather than against a live
    /// directory.
    fn generate(self) -> Output {
        let output = Command::new(build_script())
            .env("CARGO_MANIFEST_DIR", &self.root)
            .env("OUT_DIR", self.root.join("out"))
            .output()
            .expect("the compiled build script is executable");
        let _ = fs::remove_dir_all(&self.root);
        output
    }
}

fn refusal(label: &str, files: &[(&str, &str)]) -> String {
    let output = DeclarationFixture::holding(label, files).generate();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        !output.status.success(),
        "a malformed driver declaration must fail the build, not generate a table: {stderr}"
    );
    stderr
}

const COMPLETE_DECLARATION: &str = "[package]\n\
     registry_path = \"mysql\"\n\
     library = \"libadbc_driver_mysql.so\"\n\
     entry_point = \"AdbcDriverMysqlInit\"\n\
     version_range = \">=0.6.0, <0.7.0\"\n\
     [qualifier]\n\
     segment_count = 1\n\
     [capabilities]\n\
     advertised = [\"FILTER_EXPRESSIONS\"]\n";

/// `package` plus the two tables a declaration also carries, so a case about
/// the `[package]` table declares a complete file in every other respect and
/// the refusal it draws names the defect it is about.
fn with_the_rest_of_a_declaration(package: &str) -> String {
    format!(
        "{package}[qualifier]\nsegment_count = 1\n\
         [capabilities]\nadvertised = [\"FILTER_EXPRESSIONS\"]\n"
    )
}

#[test]
fn a_complete_declaration_generates_its_table() {
    let output =
        DeclarationFixture::holding("complete", &[("mysql.toml", COMPLETE_DECLARATION)]).generate();

    assert!(
        output.status.success(),
        "a complete declaration must generate: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// The segment count has one owner, and it is this file: the check that a
/// `CONNECTION` address's path spells the qualifier and the check that the
/// metadata source produced it both read it, and neither has a fallback. A file
/// omitting it would leave two modules with nothing to agree on, so the build
/// refuses it rather than generating a table with a count invented for it.
#[test]
fn a_declaration_with_no_qualifier_table_names_the_file_and_the_field() {
    let without_qualifier = "[package]\n\
         registry_path = \"mysql\"\n\
         library = \"libadbc_driver_mysql.so\"\n\
         entry_point = \"AdbcDriverMysqlInit\"\n\
         version_range = \">=0.6.0, <0.7.0\"\n\
         [capabilities]\n\
         advertised = [\"FILTER_EXPRESSIONS\"]\n";

    let message = refusal("no-qualifier", &[("mysql.toml", without_qualifier)]);

    assert!(
        message.contains("mysql.toml") && message.contains("qualifier"),
        "the refusal must name the file and the qualifier declaration it omits: {message}"
    );
}

/// A table qualified by no name at all is one no dialect can render, and the
/// address check would accept every address for it, so zero is refused at the
/// build rather than reaching either reader.
#[test]
fn a_declaration_qualifying_with_no_segment_names_the_file_and_the_field() {
    let no_segments = "[package]\n\
         registry_path = \"mysql\"\n\
         library = \"libadbc_driver_mysql.so\"\n\
         entry_point = \"AdbcDriverMysqlInit\"\n\
         version_range = \">=0.6.0, <0.7.0\"\n\
         [qualifier]\n\
         segment_count = 0\n\
         [capabilities]\n\
         advertised = [\"FILTER_EXPRESSIONS\"]\n";

    let message = refusal("no-segments", &[("mysql.toml", no_segments)]);

    assert!(
        message.contains("mysql.toml") && message.contains("segment_count"),
        "the refusal must name the file and the count that qualifies nothing: {message}"
    );
}

#[test]
fn a_declaration_missing_a_field_names_the_file_and_the_field() {
    let incomplete = "[package]\n\
         registry_path = \"mysql\"\n\
         library = \"libadbc_driver_mysql.so\"\n\
         version_range = \">=0.6.0, <0.7.0\"\n";

    let message = refusal(
        "missing-field",
        &[("mysql.toml", &with_the_rest_of_a_declaration(incomplete))],
    );

    assert!(
        message.contains("mysql.toml"),
        "the refusal must name the file it read: {message}"
    );
    assert!(
        message.contains("entry_point"),
        "the refusal must name the field the file omits: {message}"
    );
}

#[test]
fn a_declaration_with_an_empty_field_names_the_file_and_the_field() {
    let empty = "[package]\n\
         registry_path = \"mysql\"\n\
         library = \"\"\n\
         entry_point = \"AdbcDriverMysqlInit\"\n\
         version_range = \">=0.6.0, <0.7.0\"\n";

    let message = refusal(
        "empty-field",
        &[("mysql.toml", &with_the_rest_of_a_declaration(empty))],
    );

    assert!(
        message.contains("mysql.toml") && message.contains("library"),
        "the refusal must name the file and the empty field: {message}"
    );
}

#[test]
fn a_declaration_with_an_unknown_field_names_the_file_and_the_field() {
    let misspelled = "[package]\n\
         registry_paths = \"mysql\"\n\
         library = \"libadbc_driver_mysql.so\"\n\
         entry_point = \"AdbcDriverMysqlInit\"\n\
         version_range = \">=0.6.0, <0.7.0\"\n";

    let message = refusal(
        "unknown-field",
        &[("mysql.toml", &with_the_rest_of_a_declaration(misspelled))],
    );

    assert!(
        message.contains("mysql.toml") && message.contains("registry_paths"),
        "the refusal must name the file and the field it does not recognize: {message}"
    );
}

#[test]
fn a_declaration_with_no_package_table_names_the_file() {
    let message = refusal("no-table", &[("mysql.toml", "library = \"lib.so\"\n")]);

    assert!(
        message.contains("mysql.toml"),
        "the refusal must name the file it read: {message}"
    );
}

/// A declaration carries the capability list its dialect advertises as well as
/// its package row, and the dialect returns that list without a fallback, so a
/// file omitting it declares a technology that would advertise nothing.
///
/// The build refuses only an empty list and a blank entry, because either
/// would put a dialect advertising nothing, or a blank name, straight into the
/// `getCapabilities` response. Whether a name matches a capability a dialect
/// actually renders is not checked here and is not checked by the build at
/// all: a capability name is protocol vocabulary this repo defines no type for,
/// and a list of valid names inside the build script would be the second
/// capability list `vs-adapter/pushdown-dialect` forbids. A name no dialect
/// renders is caught by the invariant test that renders every advertised
/// capability, not by the build.
#[test]
fn a_declaration_with_no_capability_list_names_the_file_and_the_field() {
    let package_only = "[package]\n\
         registry_path = \"mysql\"\n\
         library = \"libadbc_driver_mysql.so\"\n\
         entry_point = \"AdbcDriverMysqlInit\"\n\
         version_range = \">=0.6.0, <0.7.0\"\n\
         [qualifier]\n\
         segment_count = 1\n";

    let message = refusal("no-capabilities", &[("mysql.toml", package_only)]);

    assert!(
        message.contains("mysql.toml") && message.contains("capabilities"),
        "the refusal must name the file and the capability list it omits: {message}"
    );
}

/// Every file in `drivers/` declares a technology, so one that is not a
/// declaration at all is a mistake to report rather than a file to skip —
/// skipping it would drop a driver from the table without saying so.
#[test]
fn a_file_that_is_not_a_declaration_names_the_file() {
    let message = refusal(
        "not-a-declaration",
        &[("mysql.toml", COMPLETE_DECLARATION), ("notes.txt", "hello")],
    );

    assert!(
        message.contains("notes.txt"),
        "the refusal must name the file it cannot read as a declaration: {message}"
    );
}

#[test]
fn a_declaration_whose_name_is_not_a_driver_name_names_the_file() {
    let message = refusal("bad-stem", &[("My SQL.toml", COMPLETE_DECLARATION)]);

    assert!(
        message.contains("My SQL.toml"),
        "the refusal must name the file whose name cannot be a driver name: {message}"
    );
}

/// A leading digit passes the old "lower-case letters and digits" check but
/// is not a valid Rust identifier's first character, so `package_constant_name`
/// would emit an unparseable constant name — a compile error inside
/// `OUT_DIR/driver_packages.rs` instead of this named refusal.
#[test]
fn a_declaration_whose_name_starts_with_a_digit_names_the_file() {
    let message = refusal("leading-digit", &[("9db.toml", COMPLETE_DECLARATION)]);

    assert!(
        message.contains("9db.toml"),
        "the refusal must name the file whose name cannot be a driver name: {message}"
    );
}

#[test]
fn a_declaration_with_an_empty_capability_list_names_the_file() {
    let empty_capability_list = "[package]\n\
         registry_path = \"mysql\"\n\
         library = \"libadbc_driver_mysql.so\"\n\
         entry_point = \"AdbcDriverMysqlInit\"\n\
         version_range = \">=0.6.0, <0.7.0\"\n\
         [qualifier]\n\
         segment_count = 1\n\
         [capabilities]\n\
         advertised = []\n";

    let message = refusal(
        "empty-capability-list",
        &[("mysql.toml", empty_capability_list)],
    );

    assert!(
        message.contains("mysql.toml"),
        "the refusal must name the file whose capability list advertises nothing: {message}"
    );
}

#[test]
fn a_declaration_with_a_blank_capability_entry_names_the_file() {
    let blank_capability_entry = "[package]\n\
         registry_path = \"mysql\"\n\
         library = \"libadbc_driver_mysql.so\"\n\
         entry_point = \"AdbcDriverMysqlInit\"\n\
         version_range = \">=0.6.0, <0.7.0\"\n\
         [qualifier]\n\
         segment_count = 1\n\
         [capabilities]\n\
         advertised = [\"\"]\n";

    let message = refusal(
        "blank-capability-entry",
        &[("mysql.toml", blank_capability_entry)],
    );

    assert!(
        message.contains("mysql.toml"),
        "the refusal must name the file whose capability list carries a blank entry: {message}"
    );
}
