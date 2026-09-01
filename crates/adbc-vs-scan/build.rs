//! Generates the compiled-in driver package table from `drivers/*.toml`.
//!
//! Adding a technology to adbc-vs is a reviewed code change, never an
//! operator-supplied file: the declaration is read here, at build time, and
//! reaches the binary as `&'static` data. A run-time parse would have moved
//! every mistake in it — a misspelled field, an empty library name — from the
//! build to the first `CREATE VIRTUAL SCHEMA` an operator ran, so this script
//! refuses to generate anything it cannot fully validate, naming the file and
//! the field that stopped it.
//!
//! It emits one `DriverPackage` constant and one advertised-capability list per
//! declaration, plus `DECLARED_DRIVERS` listing them by canonical name. Which of
//! them has a `Driver` variant is `driver.rs`'s decision, and the two are held
//! together by a test rather than by this script, which cannot see the enum.
//!
//! What it refuses is the `[package]` table's fields and nothing else. A
//! capability name is protocol vocabulary rather than a type this repo defines,
//! so this script has nothing to check one against, and giving it a list of
//! valid names would put a second capability list beside the declarations.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// One `drivers/<name>.toml` file. Unknown keys are refused rather than
/// ignored: a misspelled field would otherwise generate a row silently missing
/// the value it was meant to carry.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Declaration {
    package: Package,
    capabilities: Capabilities,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Package {
    registry_path: String,
    library: String,
    entry_point: String,
    version_range: String,
}

/// The capability set this technology's dialect advertises to Exasol.
///
/// The names are protocol vocabulary from `virtual-schema-common-java`, and
/// this script checks none of them against a list of its own: such a list would
/// be a second capability list beside the declarations, and the two could then
/// disagree about which names exist. A name no dialect can render reaches the
/// generated table and fails the invariant test that renders every advertised
/// capability, which is the one place that knows what rendering exists.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Capabilities {
    advertised: Vec<String>,
}

fn main() {
    let manifest_directory = PathBuf::from(required_var("CARGO_MANIFEST_DIR"));
    let declarations_directory = manifest_directory.join("drivers");
    let out_directory = PathBuf::from(required_var("OUT_DIR"));

    println!("cargo::rerun-if-changed=build.rs");
    println!(
        "cargo::rerun-if-changed={}",
        declarations_directory.display()
    );

    let declarations = read_declarations(&declarations_directory);

    let generated = out_directory.join("driver_packages.rs");
    fs::write(&generated, render(&declarations)).unwrap_or_else(|error| {
        fail(&format!("writing {}: {error}", generated.display()));
    });
}

/// Every declaration in `directory`, by canonical driver name.
///
/// Every file counts as a declaration, because the directory is the whole list
/// of technologies this release ships: a file this script skipped would drop a
/// driver from the table without saying so.
fn read_declarations(directory: &Path) -> BTreeMap<String, Declaration> {
    let entries = fs::read_dir(directory).unwrap_or_else(|error| {
        fail(&format!("reading {}: {error}", directory.display()));
    });

    let mut declarations = BTreeMap::new();
    for entry in entries {
        let path = entry
            .unwrap_or_else(|error| fail(&format!("reading {}: {error}", directory.display())))
            .path();
        if !path.is_file() {
            fail(&format!(
                "{} is not a driver declaration; drivers/ holds one <name>.toml per technology \
                 and nothing else",
                path.display()
            ));
        }
        let name = canonical_name(&path);
        declarations.insert(name, read_declaration(&path));
    }

    if declarations.is_empty() {
        fail(&format!(
            "{} declares no driver; adbc-vs loads a driver only if a declaration names it",
            directory.display()
        ));
    }
    declarations
}

/// The canonical driver name a declaration's file name states.
///
/// It is the value `DRIVER` takes and `adapterNotes.driver` records, so it is
/// restricted to what can be both a lower-case protocol value and part of a
/// Rust identifier.
fn canonical_name(path: &Path) -> String {
    let file_name = path.file_name().and_then(|name| name.to_str());
    let Some(file_name) = file_name else {
        fail(&format!("{} has no readable file name", path.display()));
    };
    let Some(stem) = file_name.strip_suffix(".toml") else {
        fail(&format!(
            "{} is not a driver declaration; every file in drivers/ is a <name>.toml",
            path.display()
        ));
    };
    let is_name = stem.starts_with(|character: char| character.is_ascii_lowercase())
        && stem
            .chars()
            .all(|character| character.is_ascii_lowercase() || character.is_ascii_digit());
    if !is_name {
        fail(&format!(
            "{} cannot name a driver: a driver name starts with a lower-case ASCII letter and \
             holds only lower-case ASCII letters and digits after it, because it is both the \
             DRIVER property value and part of a generated identifier",
            path.display()
        ));
    }
    stem.to_owned()
}

fn read_declaration(path: &Path) -> Declaration {
    let text = fs::read_to_string(path)
        .unwrap_or_else(|error| fail(&format!("reading {}: {error}", path.display())));
    let declaration: Declaration =
        toml::from_str(&text).unwrap_or_else(|error| fail(&format!("{}: {error}", path.display())));

    let package = &declaration.package;
    for (field, value) in [
        ("registry_path", &package.registry_path),
        ("library", &package.library),
        ("entry_point", &package.entry_point),
        ("version_range", &package.version_range),
    ] {
        if value.trim().is_empty() {
            fail(&format!(
                "{}: `[package] {field}` is empty, and every field of a driver package is read \
                 without a fallback",
                path.display()
            ));
        }
    }

    if declaration.capabilities.advertised.is_empty() {
        fail(&format!(
            "{}: `[capabilities] advertised` is empty, and a dialect advertising nothing \
             delegates nothing",
            path.display()
        ));
    }
    for capability in &declaration.capabilities.advertised {
        if capability.trim().is_empty() {
            fail(&format!(
                "{}: `[capabilities] advertised` holds a blank entry, and a blank capability \
                 name would reach the getCapabilities response",
                path.display()
            ));
        }
    }

    declaration
}

fn render(declarations: &BTreeMap<String, Declaration>) -> String {
    let mut generated = String::from(
        "// Generated by build.rs from drivers/*.toml. Do not edit.\n\
         //\n\
         // A package row and an advertised capability set per declaration, plus the\n\
         // list every declaration appears in.\n",
    );

    for (name, declaration) in declarations {
        let package = &declaration.package;
        generated.push_str(&format!(
            "\n/// The `{name}` driver package, from `drivers/{name}.toml`.\n\
             pub const {}: DriverPackage = DriverPackage {{\n\
             \x20   registry_path: {},\n\
             \x20   library: {},\n\
             \x20   entry_point: {},\n\
             \x20   version_range: {},\n\
             }};\n",
            package_constant_name(name),
            quote(&package.registry_path),
            quote(&package.library),
            quote(&package.entry_point),
            quote(&package.version_range),
        ));
        generated.push_str(&render_capabilities(name, &declaration.capabilities));
    }

    generated.push_str(
        "\n/// Every technology `drivers/` declares, by canonical name, in name order.\n\
         ///\n\
         /// Which of them this release can actually load is [`Driver`]'s decision; this\n\
         /// list is what holds the two together.\n\
         pub const DECLARED_DRIVERS: &[(&str, &DriverPackage)] = &[\n",
    );
    for name in declarations.keys() {
        generated.push_str(&format!(
            "    ({}, &{}),\n",
            quote(name),
            package_constant_name(name)
        ));
    }
    generated.push_str("];\n");

    generated
}

/// Renders one technology's advertised capability set, in the order its
/// declaration lists it.
///
/// Declaration order is kept rather than sorted, so the set an operator reads
/// in the TOML file is the set that reaches the `getCapabilities` response, and
/// so that response is byte-identical between calls. Exasol itself reads the
/// array as a set.
fn render_capabilities(name: &str, capabilities: &Capabilities) -> String {
    let mut generated = format!(
        "\n/// The capabilities the `{name}` dialect advertises, from\n\
         /// `drivers/{name}.toml`.\n\
         pub const {}: &[&str] = &[\n",
        capabilities_constant_name(name)
    );
    for capability in &capabilities.advertised {
        generated.push_str(&format!("    {},\n", quote(capability)));
    }
    generated.push_str("];\n");
    generated
}

fn package_constant_name(driver: &str) -> String {
    format!("{}_PACKAGE", driver.to_ascii_uppercase())
}

fn capabilities_constant_name(driver: &str) -> String {
    format!("{}_CAPABILITIES", driver.to_ascii_uppercase())
}

/// Renders a declared value as a Rust string literal.
///
/// A declaration is a reviewed file rather than operator input, but rendering
/// it unescaped would let a stray quote generate code that means something
/// other than what the file says.
fn quote(value: &str) -> String {
    format!("{value:?}")
}

fn required_var(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| fail(&format!("{name} is not set")))
}

fn fail(message: &str) -> ! {
    eprintln!("adbc-vs driver declarations: {message}");
    std::process::exit(1);
}
