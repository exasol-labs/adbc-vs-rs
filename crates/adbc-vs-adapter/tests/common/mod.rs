//! Shared harness for the tests that drive the Docker Compose stack.
//!
//! The stack is fixed by `docker-compose.yml` at the workspace root, so the
//! harness auto-detects everything it needs from it rather than taking
//! configuration: a test that reaches a different Exasol is a test that is
//! silently not testing this stack.
//!
//! Cargo builds a `tests/` subdirectory only through a test target that
//! declares it, so `tests/harness_test.rs` exists to keep this module
//! compiling and linted before a live-stack test consumes it.

#![allow(dead_code)]

pub mod bench;
pub mod bench_data;
pub mod bench_stack;
pub mod exasol;
pub mod items;
pub mod jdbc_vs;
pub mod mirror;
pub mod mysql;
pub mod probe;
pub mod readiness;
pub mod report;
pub mod schema;
pub mod script;
pub mod slc;
pub mod stack;
pub mod udf;

use std::process::Command;

/// Runs an external tool to completion and returns its standard output.
///
/// The harness drives the stack through the tools the project already
/// requires — `docker`, `curl`, and `exapump` — so every failure has to become
/// a test failure rather than a silently ignored exit status. `description`
/// names the attempt in the panic message; it never carries the argument list,
/// which would put credentials into test output.
pub fn run(description: &str, program: &str, args: &[&str]) -> String {
    try_run(description, program, args).unwrap_or_else(|failure| panic!("{failure}"))
}

/// Runs an external tool to completion, reporting a non-zero exit as an error
/// rather than as a failed test.
///
/// Only for the calls whose failure is the thing under test — a statement that
/// must be rejected, a file that may legitimately not exist yet. Everything
/// else goes through [`run`], where an ignored exit status cannot hide.
pub fn try_run(description: &str, program: &str, args: &[&str]) -> Result<String, String> {
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|error| format!("{description}: could not run `{program}`: {error}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "{description}: `{program}` exited with {}\n{}",
            output.status,
            stderr.trim_end()
        ));
    }

    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}
