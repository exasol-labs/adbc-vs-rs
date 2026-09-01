//! Asserts Makefile target behavior per CLAUDE.md ("Toolchain & build",
//! "Test tiers"),
//! through `make -n`/`make -q` introspection and direct parsing of the
//! Makefile's own rule text. No target's recipe is ever executed: these
//! tests touch no Docker daemon and no container stack, so they need no
//! live infrastructure.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

/// Serializes every test that depends on the real on-disk state (existence
/// and mtime) of the built `.so`, since `make -n`/`make -q cross-udf-build`
/// reads that state from the shared workspace filesystem.
static SO_LOCK: Mutex<()> = Mutex::new(());

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("adbc-vs-adapter crate must live two levels below the workspace root")
        .to_path_buf()
}

fn makefile_text() -> String {
    fs::read_to_string(workspace_root().join("Makefile"))
        .expect("Makefile must exist at the workspace root")
}

fn so_path() -> PathBuf {
    workspace_root().join("target/release/libadbc_vs_adapter.so")
}

fn run_make(args: &[&str]) -> Output {
    Command::new("make")
        .args(args)
        .current_dir(workspace_root())
        .output()
        .expect("`make` must be on PATH")
}

fn dry_run_stdout(target: &str) -> String {
    let output = run_make(&["-n", target]);
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn id_output(flag: &str) -> String {
    let output = Command::new("id")
        .arg(flag)
        .output()
        .expect("`id` must be on PATH");
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

struct Rule {
    prerequisites: Vec<String>,
    recipe: String,
}

fn rule(text: &str, target: &str) -> Rule {
    let header = format!("{target}:");
    let mut lines = text.lines();
    let header_line = lines
        .by_ref()
        .find(|line| line.starts_with(&header))
        .unwrap_or_else(|| panic!("Makefile must declare a `{target}:` rule"));
    let prerequisites = header_line[header.len()..]
        .split_whitespace()
        .map(str::to_string)
        .collect();
    let recipe = lines
        .take_while(|line| line.starts_with('\t'))
        .collect::<Vec<_>>()
        .join("\n");
    Rule {
        prerequisites,
        recipe,
    }
}

/// Backs up any `.so` already on disk for the duration of a test and
/// restores exactly the prior state (present or absent) on drop, so a
/// developer's own build artifact survives running these tests.
struct SoBackup {
    path: PathBuf,
    backup: Option<PathBuf>,
}

impl SoBackup {
    fn remove_existing() -> Self {
        let path = so_path();
        let backup = if path.exists() {
            let backup_path = path.with_extension("so.test-backup");
            fs::rename(&path, &backup_path).expect("failed to back up an existing .so");
            Some(backup_path)
        } else {
            None
        };
        SoBackup { path, backup }
    }

    fn create_with_mtime(self, mtime: SystemTime) -> Self {
        fs::create_dir_all(self.path.parent().unwrap()).expect("failed to create target/release");
        fs::write(&self.path, b"make-targets-test fixture").expect("failed to write fixture .so");
        fs::File::options()
            .write(true)
            .open(&self.path)
            .and_then(|f| f.set_modified(mtime))
            .expect("failed to set fixture .so mtime");
        self
    }
}

impl Drop for SoBackup {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
        if let Some(backup) = self.backup.take() {
            let _ = fs::rename(&backup, &self.path);
        }
    }
}

#[test]
fn cross_udf_build_runs_in_container_as_host_user() {
    let _guard = SO_LOCK.lock().unwrap();
    let _so = SoBackup::remove_existing();

    let dry_run = dry_run_stdout("cross-udf-build");

    let build_line = dry_run
        .lines()
        .find(|line| line.contains("cargo build --release"))
        .expect("dry run of a missing .so must show the cargo build recipe line");
    assert!(
        build_line.trim_start().starts_with("docker run"),
        "the build must run inside Docker, never bare on the host: {build_line}"
    );
    assert!(
        build_line.contains("rust:1.94-trixie"),
        "the build must use the rust:1.94-trixie image: {build_line}"
    );

    let user_flag = format!("-u {}:{}", id_output("-u"), id_output("-g"));
    assert!(
        build_line.contains(&user_flag),
        "the container must run as the invoking host UID:GID ({user_flag}): {build_line}"
    );

    assert!(
        dry_run.contains("-v adbc-vs-cargo-registry:/usr/local/cargo/registry"),
        "the cargo registry must be mounted from a persistent named volume"
    );
}

#[test]
fn cross_udf_build_validates_so() {
    let _guard = SO_LOCK.lock().unwrap();
    let _so = SoBackup::remove_existing();

    let dry_run = dry_run_stdout("cross-udf-build");

    let validate_line = dry_run
        .lines()
        .find(|line| line.contains("cargo exasol-udf validate"))
        .expect("dry run must show the validate step");
    assert!(
        validate_line.contains("target/release/libadbc_vs_adapter.so"),
        "the validate step must target the built .so: {validate_line}"
    );
    assert!(
        validate_line.trim_start().contains("docker run"),
        "validation must run inside the same container image: {validate_line}"
    );

    assert!(
        makefile_text().contains(".SHELLFLAGS := -eu -o pipefail -c"),
        "SHELLFLAGS must abort the target the moment `validate` reports a mismatch"
    );
}

#[test]
fn unchanged_sources_skip_build() {
    let _guard = SO_LOCK.lock().unwrap();
    let far_future = SystemTime::now() + Duration::from_secs(2 * 60 * 60);
    let _so = SoBackup::remove_existing().create_with_mtime(far_future);

    let query = run_make(&["-q", "cross-udf-build"]);
    assert_eq!(
        query.status.code(),
        Some(0),
        "a .so newer than every source must be reported up to date"
    );

    let dry_run = dry_run_stdout("cross-udf-build");
    assert!(
        !dry_run.contains("cargo build --release"),
        "Make must not start the Docker build for an unchanged workspace: {dry_run}"
    );
}

#[test]
fn touching_source_invalidates_so() {
    let _guard = SO_LOCK.lock().unwrap();
    let src = workspace_root().join("crates/adbc-vs-adapter/src/lib.rs");
    let source_mtime = fs::metadata(&src)
        .and_then(|metadata| metadata.modified())
        .expect("failed to read source file mtime");
    let _so =
        SoBackup::remove_existing().create_with_mtime(source_mtime - Duration::from_secs(60 * 60));

    let query = run_make(&["-q", "cross-udf-build"]);
    assert_eq!(
        query.status.code(),
        Some(1),
        "a source newer than the .so must be reported out of date"
    );
}

/// Restores a workspace file's modification time on drop, so a test that dates
/// a build input into the future leaves the tree exactly as it found it — a
/// file left dated forward would make every later `make` and `cargo` run treat
/// it as newer than every artifact built from it.
struct Mtime {
    path: PathBuf,
    original: SystemTime,
}

impl Mtime {
    fn dated_to(path: PathBuf, mtime: SystemTime) -> Self {
        let original = fs::metadata(&path)
            .and_then(|metadata| metadata.modified())
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
        let dated = Mtime { path, original };
        dated.set(mtime);
        dated
    }

    fn set(&self, mtime: SystemTime) {
        fs::File::options()
            .write(true)
            .open(&self.path)
            .and_then(|file| file.set_modified(mtime))
            .unwrap_or_else(|error| panic!("failed to date {}: {error}", self.path.display()));
    }
}

impl Drop for Mtime {
    fn drop(&mut self) {
        self.set(self.original);
    }
}

/// CLAUDE.md ("Toolchain & build"): no test ever runs against a stale binary.
///
/// `adbc-vs-scan`'s build script and its `drivers/*.toml` declarations sit
/// outside both crates' `src/` and are not manifests, so they reach the `.so`
/// only through a prerequisite of their own. Without one, editing a version
/// range or a capability list — the edit the declarations exist to make cheap —
/// leaves `make cross-udf-build` a no-op, and `test-artifact`, `test-e2e`, and
/// `bench` then certify the previous binary.
///
/// Each input is dated past a `.so` that is otherwise newer than the whole
/// workspace, so the only thing that can turn the target out of date is that
/// input: a prerequisite list that drops the declarations fails here rather
/// than returning a stale `.so` silently.
#[test]
fn touching_a_scan_build_input_invalidates_so() {
    let _guard = SO_LOCK.lock().unwrap();
    let newer_than_every_source = SystemTime::now() + Duration::from_secs(2 * 60 * 60);
    let _so = SoBackup::remove_existing().create_with_mtime(newer_than_every_source);

    for input in [
        "crates/adbc-vs-scan/build.rs",
        "crates/adbc-vs-scan/drivers/mysql.toml",
        "crates/adbc-vs-scan/drivers/postgresql.toml",
    ] {
        let up_to_date = run_make(&["-q", "cross-udf-build"]);
        assert_eq!(
            up_to_date.status.code(),
            Some(0),
            "a .so newer than every source must be reported up to date before {input} is dated \
             past it, or this test proves nothing about {input}"
        );

        let _dated = Mtime::dated_to(
            workspace_root().join(input),
            newer_than_every_source + Duration::from_secs(60 * 60),
        );

        let query = run_make(&["-q", "cross-udf-build"]);
        assert_eq!(
            query.status.code(),
            Some(1),
            "{input} newer than the .so must be reported out of date: it is a build input, so a \
             .so built before it is stale"
        );
    }
}

#[test]
fn lint_fmt_test_targets_use_expected_flags() {
    let fmt_check = dry_run_stdout("fmt-check");
    assert!(fmt_check.contains("cargo fmt --all -- --check"));

    let lint = dry_run_stdout("lint");
    assert!(lint.contains("cargo clippy --workspace --all-targets --all-features -- -D warnings"));

    let test = dry_run_stdout("test");
    assert!(test.contains("cargo test --workspace"));
    assert!(
        !test.contains("cargo test --workspace --lib"),
        "`make test` must run every non-feature-gated test in `tests/`, not just `--lib`: {test}"
    );
    assert!(
        !test.contains("docker"),
        "`make test` must not require Docker: {test}"
    );
}

#[test]
fn deny_target_runs_all_checks() {
    let deny = dry_run_stdout("deny");
    assert!(deny.contains("cargo deny check licenses advisories bans sources"));
    assert!(
        deny.contains("cargo test -p adbc-vs-adapter --features policy-check --test policy_test"),
        "`make deny` must also run the policy tier that needs cargo-deny on the host: {deny}"
    );
    assert!(
        makefile_text().contains(".SHELLFLAGS := -eu -o pipefail -c"),
        "SHELLFLAGS must propagate a failing `cargo deny` check's exit code"
    );
}

#[test]
fn stack_targets_start_expected_services() {
    let mysql_only = dry_run_stdout("stack-up-mysql");
    assert!(mysql_only.contains("docker compose up -d --wait mysql"));
    assert!(
        !mysql_only.to_lowercase().contains("exasol"),
        "stack-up-mysql must start MySQL alone: {mysql_only}"
    );

    let postgres_only = dry_run_stdout("stack-up-postgres");
    assert!(postgres_only.contains("docker compose up -d --wait postgres"));
    assert!(
        !postgres_only.to_lowercase().contains("exasol"),
        "stack-up-postgres must start PostgreSQL alone: {postgres_only}"
    );

    let both = dry_run_stdout("stack-up");
    assert!(both.contains("docker compose up -d --wait mysql"));
    assert!(both.contains("docker compose up -d --wait postgres"));
    assert!(both.contains("docker compose up -d exasol"));

    let down = dry_run_stdout("stack-down");
    assert!(down.contains("docker compose down"));
}

#[test]
fn container_tests_depend_on_tier_stack_target() {
    let text = makefile_text();

    let e2e = rule(&text, "test-e2e");
    assert!(
        e2e.prerequisites.iter().any(|p| p == "stack-up"),
        "test-e2e must declare stack-up as a prerequisite: {:?}",
        e2e.prerequisites
    );

    let it = rule(&text, "test-it");
    assert!(
        it.prerequisites.iter().any(|p| p == "stack-up-mysql"),
        "test-it must declare stack-up-mysql as a prerequisite: {:?}",
        it.prerequisites
    );
    assert!(
        !it.prerequisites.iter().any(|p| p == "stack-up"),
        "test-it must never start the Exasol-bearing stack: {:?}",
        it.prerequisites
    );

    let it_postgres = rule(&text, "test-it-postgres");
    assert!(
        it_postgres
            .prerequisites
            .iter()
            .any(|p| p == "stack-up-postgres"),
        "test-it-postgres must declare stack-up-postgres as a prerequisite: {:?}",
        it_postgres.prerequisites
    );
    assert!(
        !it_postgres.prerequisites.iter().any(|p| p == "stack-up"),
        "test-it-postgres must never start the Exasol-bearing stack: {:?}",
        it_postgres.prerequisites
    );
}

#[test]
fn test_artifact_depends_on_cross_udf_build() {
    let text = makefile_text();
    let artifact = rule(&text, "test-artifact");
    assert_eq!(artifact.prerequisites, vec!["cross-udf-build".to_string()]);
    assert!(artifact
        .recipe
        .contains("cargo test -p adbc-vs-adapter --features built-artifact --test artifact_test"));
}

#[test]
fn release_tarball_depends_on_cross_udf_build() {
    let text = makefile_text();
    let tarball = rule(&text, "release-tarball");
    assert_eq!(
        tarball.prerequisites,
        vec!["$(RELEASE_TARBALL)".to_string()]
    );

    let dist_rule = rule(&text, "$(RELEASE_TARBALL)");
    assert_eq!(dist_rule.prerequisites, vec!["$(SO_FILE)".to_string()]);
    assert!(dist_rule.recipe.contains("tar -czf"));
}

#[test]
fn test_it_target_needs_no_exasol() {
    let text = makefile_text();
    let it = rule(&text, "test-it");
    assert_eq!(it.prerequisites, vec!["stack-up-mysql".to_string()]);
    assert!(!it.prerequisites.iter().any(|p| p == "cross-udf-build"));
    assert!(it.recipe.contains("--features mysql-it"));
    assert!(it.recipe.contains("--test mysql_it_test"));
    assert!(
        !it.recipe.to_lowercase().contains("exasol"),
        "test-it drives the ADBC layer directly and must not mention Exasol: {}",
        it.recipe
    );
}

#[test]
fn test_it_postgres_target_needs_no_exasol() {
    let text = makefile_text();
    let it_postgres = rule(&text, "test-it-postgres");
    assert_eq!(
        it_postgres.prerequisites,
        vec!["stack-up-postgres".to_string()]
    );
    assert!(!it_postgres
        .prerequisites
        .iter()
        .any(|p| p == "cross-udf-build"));
    assert!(it_postgres.recipe.contains("--features postgres-it"));
    assert!(it_postgres.recipe.contains("--test postgres_it_test"));
    assert!(
        !it_postgres.recipe.to_lowercase().contains("exasol"),
        "test-it-postgres drives the ADBC layer directly and must not mention Exasol: {}",
        it_postgres.recipe
    );
}

#[test]
fn test_e2e_depends_on_cross_udf_build() {
    let text = makefile_text();
    let e2e = rule(&text, "test-e2e");
    assert!(e2e.prerequisites.iter().any(|p| p == "cross-udf-build"));
    assert!(e2e.recipe.contains("--features exasol-e2e"));
    assert!(e2e.recipe.contains("--test-threads=1"));
    assert!(e2e.recipe.contains("--test e2e_mysql_test"));
}

#[test]
fn test_e2e_postgres_depends_on_cross_udf_build_and_stack_up() {
    let text = makefile_text();
    let e2e_postgres = rule(&text, "test-e2e-postgres");
    assert!(
        e2e_postgres
            .prerequisites
            .iter()
            .any(|p| p == "cross-udf-build"),
        "test-e2e-postgres must declare cross-udf-build as a prerequisite: {:?}",
        e2e_postgres.prerequisites
    );
    assert!(
        e2e_postgres.prerequisites.iter().any(|p| p == "stack-up"),
        "test-e2e-postgres must declare stack-up as a prerequisite: {:?}",
        e2e_postgres.prerequisites
    );
    assert!(e2e_postgres.recipe.contains("--features exasol-e2e"));
    assert!(e2e_postgres.recipe.contains("--test-threads=1"));
    assert!(e2e_postgres.recipe.contains("--test e2e_postgres_test"));
    assert!(
        !e2e_postgres.recipe.contains("e2e_mysql_test"),
        "test-e2e-postgres must run its own binary, not the MySQL one: {}",
        e2e_postgres.recipe
    );
}

#[test]
fn bench_target_has_expected_prerequisites_and_recipe() {
    let text = makefile_text();
    let bench = rule(&text, "bench");
    assert_eq!(
        bench.prerequisites,
        vec!["cross-udf-build".to_string(), "stack-up".to_string()]
    );
    assert!(bench
        .recipe
        .contains("cargo test -p adbc-vs-adapter --features bench --test bench_mysql_test"));
    assert!(bench.recipe.contains("--test-threads=1"));
    assert!(bench.recipe.contains("--nocapture"));
}

/// CLAUDE.md ("Test tiers"): every non-unit tier sits behind a default-off
/// cargo feature and is reached through a target of its own.
///
/// This one also has to start nothing: it reads a public HTTPS endpoint, so a
/// container prerequisite would make the slowest part of the check the part
/// that has nothing to do with what it measures.
#[test]
fn test_registry_live_target_needs_no_stack() {
    let text = makefile_text();
    let registry_live = rule(&text, "test-registry-live");

    assert!(
        registry_live.prerequisites.is_empty(),
        "test-registry-live reads the public registry and must start no container: {:?}",
        registry_live.prerequisites
    );
    assert!(registry_live.recipe.contains("--features registry-live"));
    assert!(registry_live
        .recipe
        .contains("--test provisioning_test -- registry_live"));
}
