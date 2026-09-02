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
        ("e2e", "test-e2e"),
        ("e2e-postgres", "test-e2e-postgres"),
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
            ("e2e", "test-e2e"),
            ("e2e-postgres", "test-e2e-postgres"),
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
        let runs_on = job(&workflow, &name)
            .get("runs-on")
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("job `{name}` must declare `runs-on`"));
        assert_eq!(
            runs_on, "ubuntu-latest",
            "job `{name}` must run on ubuntu-latest, got: {runs_on}"
        );
    }
}

#[test]
fn workflow_has_no_cloud_or_arm_job() {
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
    for forbidden in ["arm64", "aarch64", "ubuntu-24.04-arm", "runs-on: arm"] {
        assert!(
            !text.contains(forbidden),
            "workflow must not declare an ARM runner ({forbidden})"
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
    let release = job(&workflow, "release");

    let needs = release
        .get("needs")
        .and_then(Value::as_sequence)
        .expect("release job must declare `needs`");
    for required in [
        "build-so",
        "lint",
        "unit-tests",
        "deny",
        "integration-tests",
        "integration-tests-postgres",
        "e2e",
        "e2e-postgres",
    ] {
        assert!(
            needs.iter().any(|n| n.as_str() == Some(required)),
            "release must need `{required}` so a red job blocks it"
        );
    }

    let condition = release
        .get("if")
        .and_then(Value::as_str)
        .expect("release job must declare an `if` condition");
    assert!(
        condition.contains("refs/heads/main"),
        "release must only run on push to main: {condition}"
    );

    let targets = make_targets_invoked(release);
    assert!(
        targets.iter().any(|t| t == "release-tarball"),
        "release must call `make release-tarball`"
    );
    assert!(
        targets.iter().any(|t| t == "third-party-licenses"),
        "release must call `make third-party-licenses`"
    );

    let build_so_uploads = job(&workflow, "build-so")
        .get("steps")
        .and_then(Value::as_sequence)
        .expect("build-so job must declare steps")
        .iter()
        .any(|step| {
            step.get("uses")
                .and_then(Value::as_str)
                .is_some_and(|u| u.starts_with("actions/upload-artifact"))
        });
    assert!(
        build_so_uploads,
        "build-so must upload the .so artifact for the release job to download"
    );
}

/// Every job that starts the Exasol container carries the same runner
/// preparation, whichever source's end-to-end binary it runs: the sandbox
/// needs an unprivileged user namespace, and the images do not fit a
/// GitHub-hosted runner's default free space.
#[test]
fn every_e2e_job_relaxes_apparmor_and_frees_disk() {
    let workflow = workflow();

    for name in ["e2e", "e2e-postgres"] {
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
