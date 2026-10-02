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

    let trino_only = dry_run_stdout("stack-up-trino");
    assert!(trino_only.contains("docker compose up -d trino"));
    assert!(
        !trino_only.to_lowercase().contains("exasol"),
        "stack-up-trino must start Trino alone: {trino_only}"
    );

    let exasol_only = dry_run_stdout("stack-up-exasol");
    assert!(exasol_only.contains("docker compose up -d exasol"));
    assert!(
        !exasol_only.to_lowercase().contains("mysql")
            && !exasol_only.to_lowercase().contains("postgres")
            && !exasol_only.to_lowercase().contains("trino"),
        "stack-up-exasol must start Exasol alone: {exasol_only}"
    );

    let everything = dry_run_stdout("stack-up");
    assert!(everything.contains("docker compose up -d --wait mysql"));
    assert!(everything.contains("docker compose up -d --wait postgres"));
    assert!(everything.contains("docker compose up -d trino"));
    assert!(everything.contains("docker compose up -d exasol"));

    let down = dry_run_stdout("stack-down");
    assert!(down.contains("docker compose down"));
}

#[test]
fn container_tests_depend_on_tier_stack_target() {
    let text = makefile_text();

    let e2e = rule(&text, "test-e2e");
    assert!(
        e2e.prerequisites.iter().any(|p| p == "stack-up-exasol"),
        "test-e2e must declare stack-up-exasol as a prerequisite: {:?}",
        e2e.prerequisites
    );

    let it = rule(&text, "test-it");
    assert!(
        it.prerequisites.iter().any(|p| p == "stack-up-mysql"),
        "test-it must declare stack-up-mysql as a prerequisite: {:?}",
        it.prerequisites
    );
    assert!(
        !it.prerequisites.iter().any(|p| p == "stack-up-exasol"),
        "test-it must never start Exasol: {:?}",
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
        !it_postgres
            .prerequisites
            .iter()
            .any(|p| p == "stack-up-exasol"),
        "test-it-postgres must never start Exasol: {:?}",
        it_postgres.prerequisites
    );

    let it_trino = rule(&text, "test-it-trino");
    assert!(
        it_trino.prerequisites.iter().any(|p| p == "stack-up-trino"),
        "test-it-trino must declare stack-up-trino as a prerequisite: {:?}",
        it_trino.prerequisites
    );
    assert!(
        !it_trino
            .prerequisites
            .iter()
            .any(|p| p == "stack-up-exasol"),
        "test-it-trino must never start Exasol: {:?}",
        it_trino.prerequisites
    );

    for it_target in ["test-it", "test-it-postgres", "test-it-trino"] {
        let it_rule = rule(&text, it_target);
        assert!(
            !it_rule.recipe.to_lowercase().contains("exasol"),
            "{it_target} drives the ADBC layer directly and must not mention Exasol: {}",
            it_rule.recipe
        );
    }
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
    assert_eq!(
        e2e.prerequisites,
        vec![
            "cross-udf-build".to_string(),
            "stack-up-exasol".to_string(),
            "stack-up-mysql".to_string(),
        ],
        "test-e2e must depend on exactly the sources e2e_mysql_test queries: {:?}",
        e2e.prerequisites
    );
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
    for stack_target in ["stack-up-exasol", "stack-up-postgres", "stack-up-mysql"] {
        assert!(
            e2e_postgres.prerequisites.iter().any(|p| p == stack_target),
            "test-e2e-postgres must declare {stack_target} as a prerequisite: {:?}",
            e2e_postgres.prerequisites
        );
    }
    assert!(
        !e2e_postgres.prerequisites.iter().any(|p| p == "stack-up"),
        "test-e2e-postgres must name each source's own stack-up-* target, not the \
         everything-up alias: {:?}",
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

const BENCH_SOURCES: [&str; 3] = ["mysql", "postgres", "trino"];

/// CLAUDE.md ("Benchmarking"): each source's bench depends on Exasol plus
/// only its own source, and runs only its own binary.
#[test]
fn each_bench_source_target_has_expected_prerequisites_and_recipe() {
    let text = makefile_text();
    for source in BENCH_SOURCES {
        let target = format!("bench-{source}");
        let bench = rule(&text, &target);
        assert_eq!(
            bench.prerequisites,
            vec![
                "cross-udf-build".to_string(),
                "stack-up-exasol".to_string(),
                format!("stack-up-{source}"),
            ],
            "{target} must depend on exactly the stack its own binary queries"
        );
        assert!(
            bench.recipe.contains(&format!(
                "cargo test -p adbc-vs-adapter --features bench --test bench_{source}_test"
            )),
            "{target} must run bench_{source}_test: {}",
            bench.recipe
        );
        for other in BENCH_SOURCES.iter().filter(|other| **other != source) {
            assert!(
                !bench.recipe.contains(&format!("bench_{other}_test")),
                "{target} must run only its own binary: {}",
                bench.recipe
            );
        }
        assert!(bench.recipe.contains("--test-threads=1"));
        assert!(bench.recipe.contains("--nocapture"));
    }
}

/// `make bench` runs every source one after another and the summary last.
/// It has no prerequisites of its own, so `make -j bench` cannot schedule two
/// sources' timings onto the same cores at once.
#[test]
fn bench_runs_every_source_sequentially_then_the_summary() {
    let text = makefile_text();
    let bench = rule(&text, "bench");
    assert!(
        bench.prerequisites.is_empty(),
        "bench must sequence its sources in its recipe, not as prerequisites make -j \
         could run in parallel: {:?}",
        bench.prerequisites
    );

    let dry_run = dry_run_stdout("bench");
    let mut steps: Vec<String> = BENCH_SOURCES
        .iter()
        .map(|source| format!("--test bench_{source}_test"))
        .collect();
    steps.push("scripts/bench_summary.py".to_string());
    let positions: Vec<usize> = steps
        .iter()
        .map(|step| {
            dry_run
                .find(step.as_str())
                .unwrap_or_else(|| panic!("`make -n bench` must reach `{step}`:\n{dry_run}"))
        })
        .collect();
    assert!(
        positions.windows(2).all(|pair| pair[0] < pair[1]),
        "`make bench` must run {steps:?} in that order:\n{dry_run}"
    );
}

/// Actually runs `bench-summary` (Python, no container) against fixture CSVs:
/// the newest CSV per source wins, warm-up samples are excluded from the
/// median, and an ADBC-only source shows `n/a` instead of a ratio.
#[test]
fn bench_summary_combines_the_newest_csv_per_source() {
    let dir = std::env::temp_dir().join(format!("adbc-vs-bench-summary-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("failed to create the fixture directory");
    let header =
        "source,tier,adapter,run_index,warm_up,rows,bytes,duration_seconds,rows_per_sec,mib_per_sec\n";
    let write = |name: &str, rows: &str| {
        fs::write(dir.join(name), format!("{header}{rows}")).expect("failed to write fixture CSV");
    };
    write(
        "results-mysql-2026-01-01T00-00-00Z.csv",
        "mysql,narrow_small,adbc,0,false,1,1,1,1,1\nmysql,narrow_small,jdbc,0,false,1,1,1,1,1\n",
    );
    write(
        "results-mysql-2026-02-01T00-00-00Z.csv",
        "mysql,narrow_small,adbc,0,true,1,1,1,999999,1\n\
         mysql,narrow_small,adbc,0,false,1,1,1,300,1\n\
         mysql,narrow_small,adbc,1,false,1,1,1,400,1\n\
         mysql,narrow_small,adbc,2,false,1,1,1,500,1\n\
         mysql,narrow_small,jdbc,0,false,1,1,1,100,1\n\
         mysql,narrow_small,jdbc,1,false,1,1,1,200,1\n\
         mysql,narrow_small,jdbc,2,false,1,1,1,50,1\n",
    );
    write(
        "results-trino-2026-02-01T00-00-00Z.csv",
        "trino,wide,adbc,0,false,1,1,1,700,1\n",
    );

    let output = run_make(&["bench-summary", &format!("BENCH_DIR={}", dir.display())]);
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let summaries: Vec<_> = fs::read_dir(&dir)
        .expect("fixture directory must still exist")
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("summary-"))
        .collect();
    let _ = fs::remove_dir_all(&dir);

    assert!(output.status.success(), "bench-summary failed: {stderr}");
    assert!(
        stdout.contains("| mysql | narrow_small | 400 | 100 | 4.000 |"),
        "the newest MySQL CSV's measured medians must be summarized: {stdout}"
    );
    assert!(
        stdout.contains("| trino | wide | 700 | n/a | n/a |"),
        "an ADBC-only source must show n/a for JDBC and the ratio: {stdout}"
    );
    assert!(
        !stdout.contains("2026-01-01"),
        "an older CSV must not be an input: {stdout}"
    );
    assert!(
        stderr.contains("postgres left out"),
        "a missing source must be named: {stderr}"
    );
    assert_eq!(
        summaries.len(),
        1,
        "exactly one summary file must be written"
    );
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

#[test]
fn test_it_trino_target_needs_no_exasol() {
    let text = makefile_text();
    let it_trino = rule(&text, "test-it-trino");
    assert_eq!(it_trino.prerequisites, vec!["stack-up-trino".to_string()]);
    assert!(!it_trino
        .prerequisites
        .iter()
        .any(|p| p == "cross-udf-build"));
    assert!(it_trino.recipe.contains("--features trino-it"));
    assert!(it_trino.recipe.contains("--test trino_it_test"));
    assert!(
        !it_trino.recipe.to_lowercase().contains("exasol"),
        "test-it-trino drives the ADBC layer directly and must not mention Exasol: {}",
        it_trino.recipe
    );
}

#[test]
fn test_e2e_trino_depends_on_expected_targets() {
    let text = makefile_text();
    let e2e_trino = rule(&text, "test-e2e-trino");
    assert_eq!(
        e2e_trino.prerequisites,
        vec![
            "cross-udf-build".to_string(),
            "stack-up-exasol".to_string(),
            "stack-up-trino".to_string(),
            "stack-up-mysql".to_string(),
        ]
    );
    assert!(e2e_trino.recipe.contains("--features exasol-e2e"));
    assert!(e2e_trino.recipe.contains("--test-threads=1"));
    assert!(e2e_trino.recipe.contains("--test e2e_trino_test"));
    assert!(
        !e2e_trino.recipe.contains("e2e_mysql_test")
            && !e2e_trino.recipe.contains("e2e_postgres_test"),
        "test-e2e-trino must run its own binary: {}",
        e2e_trino.recipe
    );
}

/// A container that silently drops password authentication would still pass
/// every other assertion in this file: `make -n` never executes a recipe, so
/// nothing here can observe the live server. What it can observe is whether
/// the recipe text itself asks for authentication and checks for its
/// absence -- a `stack-up-trino` that lost the `--user`/`--password` flags,
/// fell back to `--wait`, or dropped the negative probe would fail exactly
/// one of the assertions below.
#[test]
fn stack_up_trino_polls_for_readiness_and_probes_authentication() {
    let text = makefile_text();
    let stack_up_trino = rule(&text, "stack-up-trino");

    assert!(
        !stack_up_trino.recipe.contains("--wait"),
        "stack-up-trino must not rely on the image's own HEALTHCHECK via --wait: {}",
        stack_up_trino.recipe
    );
    assert!(
        stack_up_trino.recipe.contains("until "),
        "stack-up-trino must poll for readiness the way stack-up-exasol does: {}",
        stack_up_trino.recipe
    );
    assert!(
        stack_up_trino.recipe.contains("TRINO_WAIT_TIMEOUT"),
        "stack-up-trino must fail with a named, bounded deadline: {}",
        stack_up_trino.recipe
    );

    let cli_lines: Vec<&str> = stack_up_trino
        .recipe
        .lines()
        .filter(|line| line.contains("trino --server"))
        .collect();
    assert!(
        !cli_lines.is_empty(),
        "stack-up-trino must invoke the trino CLI at least once: {}",
        stack_up_trino.recipe
    );
    for line in &cli_lines {
        assert!(
            line.contains("--server https://"),
            "every CLI invocation must use TLS, never plain HTTP: {line}"
        );
        assert!(
            !line.contains("http://localhost:8080"),
            "no CLI invocation may target the insecure port: {line}"
        );
    }

    assert!(
        stack_up_trino.recipe.contains("--user adbc_vs"),
        "the readiness/seeding invocation must authenticate as a named user: {}",
        stack_up_trino.recipe
    );
    assert!(
        stack_up_trino.recipe.contains("--password"),
        "the readiness/seeding invocation must pass --password: {}",
        stack_up_trino.recipe
    );

    let negative_probe = stack_up_trino
        .recipe
        .lines()
        .find(|line| line.contains("--execute \"SELECT 1\""))
        .expect("stack-up-trino must carry a credential-free negative probe");
    assert!(
        !negative_probe.contains("--user") && !negative_probe.contains("--password"),
        "the negative probe must carry no credential, or it cannot prove authentication is \
         enforced: {negative_probe}"
    );
}
