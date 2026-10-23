//! Asserts `.github/workflows/ci.yml` per CLAUDE.md ("CI"):
//! that it parses as YAML, and that every `make <target>` a job step invokes
//! names a target the `Makefile` actually declares.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde_yaml_ng::Value;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("adbc-vs-adapter crate must live two levels below the workspace root")
        .to_path_buf()
}

fn workflow_text() -> String {
    fs::read_to_string(workspace_root().join(".github/workflows/ci.yml"))
        .expect(".github/workflows/ci.yml must exist")
}

fn workflow() -> Value {
    serde_yaml_ng::from_str(&workflow_text()).expect(".github/workflows/ci.yml must parse as YAML")
}

fn makefile_targets() -> HashSet<String> {
    let text = fs::read_to_string(workspace_root().join("Makefile")).expect("Makefile must exist");
    let mut targets = HashSet::new();
    for line in text.lines() {
        if line.starts_with('\t') || line.starts_with('#') || line.starts_with('.') {
            continue;
        }
        let Some(colon_idx) = line.find(':') else {
            continue;
        };
        if line[colon_idx..].starts_with(":=") {
            continue;
        }
        let name = line[..colon_idx].trim();
        if name.is_empty() || name.contains('$') || name.contains(' ') {
            continue;
        }
        targets.insert(name.to_string());
    }
    targets
}

fn jobs(workflow: &Value) -> &serde_yaml_ng::Mapping {
    workflow
        .get("jobs")
        .and_then(Value::as_mapping)
        .expect("workflow must declare a `jobs` mapping")
}

fn job<'a>(workflow: &'a Value, name: &str) -> &'a Value {
    jobs(workflow)
        .get(name)
        .unwrap_or_else(|| panic!("workflow must declare a `{name}` job"))
}

fn step_run_strings(job: &Value) -> Vec<String> {
    job.get("steps")
        .and_then(Value::as_sequence)
        .unwrap_or_else(|| panic!("job must declare a `steps` sequence"))
        .iter()
        .filter_map(|step| step.get("run"))
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect()
}

/// Every `make <target>` token across a job's `run:` steps, in the order
/// each step contributed them.
fn make_targets_invoked(job: &Value) -> Vec<String> {
    let mut invoked = Vec::new();
    for run in step_run_strings(job) {
        let tokens: Vec<&str> = run.split_whitespace().collect();
        for (i, token) in tokens.iter().enumerate() {
            if *token == "make" {
                if let Some(target) = tokens.get(i + 1) {
                    invoked.push((*target).to_string());
                }
            }
        }
    }
    invoked
}

fn job_names(workflow: &Value) -> Vec<String> {
    jobs(workflow)
        .keys()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect()
}

const X86_RUNNER: &str = "ubuntu-latest";
const ARM_RUNNER: &str = "ubuntu-24.04-arm";
const MATRIX_RUNNER: &str = "${{ matrix.runner }}";
const RUST_ACTION: &str = "dtolnay/rust-toolchain@1.94";

const E2E_JOBS: [&str; 3] = ["e2e", "e2e-postgres", "e2e-trino"];

/// The jobs a release is gated on, which `package` needs one by one.
const GATING_JOBS: [&str; 10] = [
    "build-so",
    "lint",
    "unit-tests",
    "deny",
    "integration-tests",
    "integration-tests-postgres",
    "integration-tests-trino",
    "e2e",
    "e2e-postgres",
    "e2e-trino",
];

/// Jobs that run once on each architecture.
const BOTH_ARCHITECTURE_JOBS: [&str; 7] = [
    "build-so",
    "lint",
    "unit-tests",
    "integration-tests",
    "integration-tests-postgres",
    "integration-tests-trino",
    "package",
];

/// Jobs that run cargo on the host, so the toolchain is theirs to install.
const HOST_CARGO_JOBS: [&str; 10] = [
    "build-so",
    "lint",
    "unit-tests",
    "integration-tests",
    "integration-tests-postgres",
    "integration-tests-trino",
    "e2e",
    "e2e-postgres",
    "e2e-trino",
    "package",
];

fn matrix_of<'a>(job: &'a Value, name: &str) -> &'a Value {
    job.get("strategy")
        .and_then(|strategy| strategy.get("matrix"))
        .unwrap_or_else(|| panic!("job `{name}` must declare `strategy.matrix`"))
}

/// Every runner a job's matrix names: a plain `runner` list, or the `runner` of
/// each `include` row.
fn matrix_runners(job: &Value, name: &str) -> Vec<String> {
    let matrix = matrix_of(job, name);
    let values: Vec<&Value> = if let Some(runners) = matrix.get("runner") {
        runners
            .as_sequence()
            .unwrap_or_else(|| panic!("`{name}` matrix.runner must be a sequence"))
            .iter()
            .collect()
    } else {
        matrix
            .get("include")
            .and_then(Value::as_sequence)
            .unwrap_or_else(|| panic!("`{name}` matrix must list runners or `include` rows"))
            .iter()
            .map(|row| {
                row.get("runner")
                    .unwrap_or_else(|| panic!("every `{name}` include row must name a runner"))
            })
            .collect()
    };
    values
        .into_iter()
        .map(|runner| {
            runner
                .as_str()
                .unwrap_or_else(|| panic!("`{name}` runner must be a string"))
                .to_string()
        })
        .collect()
}

fn both_runners() -> HashSet<String> {
    [X86_RUNNER, ARM_RUNNER].map(str::to_string).into()
}

fn step_uses(job: &Value) -> Vec<String> {
    job.get("steps")
        .and_then(Value::as_sequence)
        .unwrap_or_else(|| panic!("job must declare a `steps` sequence"))
        .iter()
        .filter_map(|step| step.get("uses"))
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect()
}

fn needs_of(job: &Value, name: &str) -> Vec<String> {
    match job.get("needs") {
        Some(Value::Sequence(needs)) => needs
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
        Some(Value::String(need)) => vec![need.clone()],
        _ => panic!("job `{name}` must declare `needs`"),
    }
}

#[test]
fn workflow_make_targets_exist() {
    let workflow = workflow();
    let declared = makefile_targets();

    let mut invoked_anywhere = Vec::new();
    for name in job_names(&workflow) {
        invoked_anywhere.extend(make_targets_invoked(job(&workflow, &name)));
    }

    assert!(
        !invoked_anywhere.is_empty(),
        "the workflow must invoke at least one `make` target"
    );
    for target in &invoked_anywhere {
        assert!(
            declared.contains(target),
            "workflow invokes `make {target}`, which the Makefile does not declare"
        );
    }
}

#[test]
fn workflow_triggers_and_concurrency() {
    let workflow = workflow();
    let on = workflow
        .get("on")
        .expect("workflow must declare an `on` block");

    let push_branches = on
        .get("push")
        .and_then(|p| p.get("branches"))
        .and_then(Value::as_sequence)
        .expect("`on.push.branches` must be a sequence");
    assert!(push_branches.iter().any(|b| b.as_str() == Some("main")));

    let pr_branches = on
        .get("pull_request")
        .and_then(|p| p.get("branches"))
        .and_then(Value::as_sequence)
        .expect("`on.pull_request.branches` must be a sequence");
    assert!(pr_branches.iter().any(|b| b.as_str() == Some("main")));

    assert!(
        on.get("merge_group").is_some(),
        "workflow must trigger on merge_group"
    );
    assert!(
        on.get("workflow_dispatch").is_some(),
        "workflow must trigger on workflow_dispatch"
    );

    let concurrency = workflow
        .get("concurrency")
        .expect("workflow must declare a `concurrency` block");
    let group = concurrency
        .get("group")
        .and_then(Value::as_str)
        .expect("concurrency.group must be a string");
    assert!(group.contains("github.workflow") && group.contains("github.ref"));

    let cancel = concurrency
        .get("cancel-in-progress")
        .and_then(Value::as_str)
        .expect("concurrency.cancel-in-progress must be a string expression");
    assert!(
        cancel.contains("refs/heads/main"),
        "cancel-in-progress must special-case the default branch: {cancel}"
    );
    assert!(
        cancel.to_lowercase().contains("tags"),
        "cancel-in-progress must special-case tag refs: {cancel}"
    );
}

#[test]
fn workflow_declares_required_jobs() {
    let workflow = workflow();

    let build_so = make_targets_invoked(job(&workflow, "build-so"));
    assert!(
        build_so.iter().any(|t| t == "cross-udf-build"),
        "build-so must call `make cross-udf-build`"
    );
    assert!(
        build_so.iter().any(|t| t == "test-artifact"),
        "build-so must also call `make test-artifact`"
    );

    let lint = make_targets_invoked(job(&workflow, "lint"));
    assert!(
        lint.iter().any(|t| t == "fmt-check"),
        "lint must call `make fmt-check`"
    );
    assert!(
        lint.iter().any(|t| t == "lint"),
        "lint must call `make lint`"
    );

    let unit_tests = make_targets_invoked(job(&workflow, "unit-tests"));
    assert!(
        unit_tests.iter().any(|t| t == "test"),
        "unit-tests must call `make test`"
    );
}

#[test]
fn policy_and_container_jobs_call_make() {
    let workflow = workflow();

    let deny = make_targets_invoked(job(&workflow, "deny"));
    assert!(
        deny.iter().any(|t| t == "deny"),
        "deny must call `make deny`"
    );

    for (name, target) in [
        ("integration-tests", "test-it"),
        ("integration-tests-postgres", "test-it-postgres"),
        ("integration-tests-trino", "test-it-trino"),
        ("e2e", "test-e2e"),
        ("e2e-postgres", "test-e2e-postgres"),
        ("e2e-trino", "test-e2e-trino"),
    ] {
        let invoked = make_targets_invoked(job(&workflow, name));
        assert!(
            invoked.iter().any(|t| t == target),
            "the `{name}` job must call `make {target}`, and no other job may: a tier that \
             shares a job with another runs only while that other one passed, so a red MySQL \
             tier would leave PostgreSQL with no signal at the moment it is most needed"
        );

        for (other, other_target) in [
            ("integration-tests", "test-it"),
            ("integration-tests-postgres", "test-it-postgres"),
            ("integration-tests-trino", "test-it-trino"),
            ("e2e", "test-e2e"),
            ("e2e-postgres", "test-e2e-postgres"),
            ("e2e-trino", "test-e2e-trino"),
        ] {
            if other == name {
                continue;
            }
            assert!(
                !make_targets_invoked(job(&workflow, other))
                    .iter()
                    .any(|t| t == target),
                "`{other}` runs `{other_target}` and must not also run `{target}`"
            );
        }
    }

    for name in job_names(&workflow) {
        let job = job(&workflow, &name);
        let runs_on = job
            .get("runs-on")
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("job `{name}` must declare `runs-on`"));
        assert!(
            [X86_RUNNER, ARM_RUNNER, MATRIX_RUNNER].contains(&runs_on),
            "job `{name}` must run on {X86_RUNNER}, {ARM_RUNNER}, or {MATRIX_RUNNER}, got: {runs_on}"
        );
        if runs_on == MATRIX_RUNNER {
            for runner in matrix_runners(job, &name) {
                assert!(
                    [X86_RUNNER, ARM_RUNNER].contains(&runner.as_str()),
                    "job `{name}` matrix runner must be {X86_RUNNER} or {ARM_RUNNER}, got: {runner}"
                );
            }
        }
    }
}

#[test]
fn workflow_has_no_cloud_job() {
    let text = workflow_text().to_lowercase();
    for forbidden in [
        "azure",
        "aws",
        "databricks",
        "snowflake",
        "unity catalog",
        "unity-catalog",
    ] {
        assert!(
            !text.contains(forbidden),
            "workflow must not reference {forbidden}"
        );
    }

    let workflows_dir = workspace_root().join(".github/workflows");
    let workflow_files: Vec<_> = fs::read_dir(&workflows_dir)
        .expect(".github/workflows must exist")
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().extension().and_then(|e| e.to_str()) == Some("yml"))
        .collect();
    assert_eq!(
        workflow_files.len(),
        1,
        "the repository must not contain a second workflow that sweeps cloud resources"
    );
}

#[test]
fn release_job_is_gated_and_packages_artifacts() {
    let workflow = workflow();
    let package = job(&workflow, "package");
    let release = job(&workflow, "release");

    assert_eq!(
        matrix_runners(package, "package")
            .into_iter()
            .collect::<HashSet<_>>(),
        both_runners(),
        "package must run on both runners, so each architecture packs its own tarball"
    );

    let package_needs = needs_of(package, "package");
    for required in GATING_JOBS {
        assert!(
            package_needs.iter().any(|n| n == required),
            "package must need `{required}` so a red job blocks it"
        );
    }

    let release_needs = needs_of(release, "release");
    assert!(
        release_needs.iter().any(|n| n == "package"),
        "release must need `package`"
    );

    for (name, gated) in [("package", package), ("release", release)] {
        let condition = gated
            .get("if")
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("`{name}` job must declare an `if` condition"));
        assert!(
            condition.contains("refs/heads/main"),
            "`{name}` must only run on push to main: {condition}"
        );
    }

    let package_targets = make_targets_invoked(package);
    for target in ["release-tarball", "third-party-licenses"] {
        assert!(
            package_targets.iter().any(|t| t == target),
            "package must call `make {target}`"
        );
    }
    let release_targets = make_targets_invoked(release);
    for target in ["release-tarball", "third-party-licenses"] {
        assert!(
            !release_targets.iter().any(|t| t == target),
            "release only publishes, so it must not call `make {target}`"
        );
    }

    let build_so_artifact = artifact_names(job(&workflow, "build-so"), "actions/upload-artifact");
    assert!(
        build_so_artifact
            .iter()
            .any(|name| name.contains("runner.arch")),
        "build-so must name its .so artifact after `runner.arch`: {build_so_artifact:?}"
    );
    assert_eq!(
        artifact_names(package, "actions/download-artifact")
            .into_iter()
            .next(),
        build_so_artifact.into_iter().next(),
        "package must download the .so of its own architecture"
    );

    let release_files = release
        .get("steps")
        .and_then(Value::as_sequence)
        .expect("release job must declare steps")
        .iter()
        .filter(|step| {
            step.get("uses")
                .and_then(Value::as_str)
                .is_some_and(|u| u.starts_with("softprops/action-gh-release"))
        })
        .filter_map(|step| step.get("with").and_then(|with| with.get("files")))
        .filter_map(Value::as_str)
        .collect::<Vec<_>>()
        .join("\n");
    for asset in [
        "dist/adbc-vs.tar.gz",
        "dist/adbc-vs-linux-arm64.tar.gz",
        "dist/THIRD-PARTY-LICENSES.md",
    ] {
        assert!(
            release_files.lines().any(|line| line.trim() == asset),
            "release must publish `{asset}`: {release_files}"
        );
    }

    let release_steps = release
        .get("steps")
        .and_then(Value::as_sequence)
        .expect("release job must declare steps");
    let publishes_strictly = release_steps
        .iter()
        .filter(|step| {
            step.get("uses")
                .and_then(Value::as_str)
                .is_some_and(|u| u.starts_with("softprops/action-gh-release"))
        })
        .all(|step| {
            step.get("with")
                .and_then(|with| with.get("fail_on_unmatched_files"))
                .and_then(Value::as_bool)
                == Some(true)
        });
    assert!(
        publishes_strictly,
        "the release step must set `fail_on_unmatched_files: true`, so a missing asset fails the job"
    );

    let run_of = |step: &Value| {
        step.get("run")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let verify_at = release_steps.iter().position(|step| {
        let run = run_of(step);
        [
            "dist/adbc-vs.tar.gz",
            "dist/adbc-vs-linux-arm64.tar.gz",
            "dist/THIRD-PARTY-LICENSES.md",
        ]
        .iter()
        .all(|asset| run.contains(asset))
    });
    let push_at = release_steps
        .iter()
        .position(|step| run_of(step).contains("git push origin"))
        .expect("release must push the tag");
    assert!(
        verify_at.is_some_and(|verify| verify < push_at),
        "a step naming all three assets must run before the tag is pushed: verify at {verify_at:?}, push at {push_at}"
    );
}

/// The `name:` input of every step in `job` whose action starts with `action`.
fn artifact_names(job: &Value, action: &str) -> Vec<String> {
    job.get("steps")
        .and_then(Value::as_sequence)
        .unwrap_or_else(|| panic!("job must declare a `steps` sequence"))
        .iter()
        .filter(|step| {
            step.get("uses")
                .and_then(Value::as_str)
                .is_some_and(|u| u.starts_with(action))
        })
        .filter_map(|step| step.get("with").and_then(|with| with.get("name")))
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect()
}

#[test]
fn library_jobs_cover_both_architectures() {
    let workflow = workflow();

    for name in BOTH_ARCHITECTURE_JOBS {
        let runners = matrix_runners(job(&workflow, name), name);
        assert_eq!(
            runners.iter().collect::<HashSet<_>>().len(),
            runners.len(),
            "`{name}` must not list a runner twice"
        );
        assert_eq!(
            runners.into_iter().collect::<HashSet<_>>(),
            both_runners(),
            "`{name}` must run on both ubuntu-latest and ubuntu-24.04-arm, so the aarch64 \
             branches are compiled and tested, not only written"
        );
    }
}

#[test]
fn every_job_that_runs_cargo_on_the_host_installs_rust_1_94() {
    let workflow = workflow();

    for name in HOST_CARGO_JOBS {
        assert!(
            step_uses(job(&workflow, name))
                .iter()
                .any(|u| u == RUST_ACTION),
            "`{name}` runs cargo on the host and must install Rust with `{RUST_ACTION}`: no \
             source confirms that the ubuntu-24.04-arm image ships rustup"
        );
    }
}

/// The four Exasol rows each E2E job runs: two Docker images on x86_64, and
/// Exasol Personal on both architectures, because no Docker Exasol image exists
/// for arm64.
#[test]
fn every_e2e_job_runs_the_four_exasol_rows() {
    let workflow = workflow();
    let expected: [(&str, &str, &str, Option<&str>); 4] = [
        (
            "docker-2025",
            X86_RUNNER,
            "docker",
            Some("exasol/docker-db:latest-2025.2"),
        ),
        (
            "docker-2026",
            X86_RUNNER,
            "docker",
            Some("exasol/docker-db:latest-2026.1"),
        ),
        ("personal-x86_64", X86_RUNNER, "personal", None),
        ("personal-arm64", ARM_RUNNER, "personal", None),
    ];

    for name in E2E_JOBS {
        let e2e = job(&workflow, name);
        let strategy = e2e
            .get("strategy")
            .expect("e2e job must declare a strategy");
        assert_eq!(
            strategy.get("fail-fast").and_then(Value::as_bool),
            Some(false),
            "`{name}` must not cancel the other rows when one fails"
        );

        let rows = matrix_of(e2e, name)
            .get("include")
            .and_then(Value::as_sequence)
            .unwrap_or_else(|| panic!("`{name}` matrix must list its rows under `include`"));
        assert_eq!(rows.len(), expected.len(), "`{name}` must have four rows");
        for (row, (row_name, runner, target, image)) in rows.iter().zip(expected) {
            let field = |key: &str| row.get(key).and_then(Value::as_str);
            assert_eq!(field("name"), Some(row_name), "`{name}` row name");
            assert_eq!(
                field("runner"),
                Some(runner),
                "`{name}` row {row_name} runner"
            );
            assert_eq!(
                field("target"),
                Some(target),
                "`{name}` row {row_name} target"
            );
            assert_eq!(field("image"), image, "`{name}` row {row_name} image");
        }

        let target_step = e2e
            .get("steps")
            .and_then(Value::as_sequence)
            .expect("e2e job must declare steps")
            .iter()
            .find(|step| {
                step.get("run")
                    .and_then(Value::as_str)
                    .is_some_and(|r| r.starts_with("make test-e2e"))
            })
            .unwrap_or_else(|| panic!("`{name}` must call an end-to-end make target"));
        let env = |key: &str| {
            target_step
                .get("env")
                .and_then(|env| env.get(key))
                .and_then(Value::as_str)
        };
        assert_eq!(env("EXASOL_TARGET"), Some("${{ matrix.target }}"));
        assert_eq!(env("EXASOL_IMAGE"), Some("${{ matrix.image }}"));
    }
}

/// The launcher pulls the `exasol/nano` image it pins, so a nano tag named in
/// the workflow would be a second pin that can disagree with it.
#[test]
fn personal_rows_install_podman_and_the_launcher_without_naming_a_nano_tag() {
    let workflow = workflow();

    assert!(
        !workflow_text().to_lowercase().contains("nano"),
        "the workflow must not name an exasol/nano tag"
    );

    for name in E2E_JOBS {
        let steps = job(&workflow, name)
            .get("steps")
            .and_then(Value::as_sequence)
            .expect("e2e job must declare steps");
        let personal_runs: Vec<&str> = steps
            .iter()
            .filter(|step| {
                step.get("if").and_then(Value::as_str) == Some("matrix.target == 'personal'")
            })
            .filter_map(|step| step.get("run").and_then(Value::as_str))
            .collect();
        assert!(
            personal_runs.iter().any(|r| r.contains("podman")),
            "`{name}` must install Podman on a Personal row when it is absent"
        );
        assert!(
            personal_runs
                .iter()
                .any(|r| r.contains("https://downloads.exasol.com/exasol-personal/installer.sh")),
            "`{name}` must install the Exasol Personal launcher on a Personal row"
        );
    }
}

/// Every job that starts the Exasol container carries the same runner
/// preparation, whichever source's end-to-end binary it runs: the sandbox
/// needs an unprivileged user namespace, and the images do not fit a
/// GitHub-hosted runner's default free space.
#[test]
fn every_e2e_job_relaxes_apparmor_and_frees_disk() {
    let workflow = workflow();

    for name in ["e2e", "e2e-postgres", "e2e-trino"] {
        let e2e = job(&workflow, name);
        let runs = step_run_strings(e2e);

        let e2e_index = runs
            .iter()
            .position(|r| r.contains("make test-e2e"))
            .unwrap_or_else(|| panic!("the `{name}` job must call an end-to-end make target"));

        let apparmor_index = runs
            .iter()
            .position(|r| r.contains("kernel.apparmor_restrict_unprivileged_userns=0"))
            .unwrap_or_else(|| {
                panic!("the `{name}` job must relax AppArmor for the Exasol UDF sandbox")
            });
        assert!(
            apparmor_index < e2e_index,
            "`{name}` must relax AppArmor before the Exasol container starts"
        );

        let disk_free_index = runs
            .iter()
            .position(|r| r.contains("rm -rf") || r.contains("docker image prune"))
            .unwrap_or_else(|| panic!("the `{name}` job must free runner disk space"));
        assert!(
            disk_free_index < e2e_index,
            "`{name}` must free disk space before pulling images"
        );

        let steps = e2e
            .get("steps")
            .and_then(Value::as_sequence)
            .unwrap_or_else(|| panic!("the `{name}` job must declare steps"));
        let uploads_log_on_failure = steps.iter().any(|step| {
            step.get("uses")
                .and_then(Value::as_str)
                .is_some_and(|u| u.starts_with("actions/upload-artifact"))
                && step.get("if").and_then(Value::as_str) == Some("failure()")
        });
        assert!(
            uploads_log_on_failure,
            "the `{name}` job must upload the Exasol container log as an artifact when it fails"
        );
    }
}
