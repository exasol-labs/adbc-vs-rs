//! Runs the full transmission-throughput benchmark against the Docker
//! Compose stack: every tier `bench_data::Tier::all()` declares, alternating
//! ADBC and JDBC samples, then writes the CSV and Markdown report under
//! `dist/bench/`.
//!
//! There is no skip path. [`bench::run`] panics on any fairness or
//! verification failure — a dropped row, a mismatched column set, an
//! unreachable stack — rather than returning a result for a run that did not
//! actually work, so this test's only job is orchestration: bring up the
//! shared stack, run the samples, and write the report.
//!
//! One shared stack provisions both virtual schemas and the `BENCH` sink
//! schema once, which is why `make bench` runs this with
//! `--test-threads=1`.
#![cfg(feature = "bench")]

mod common;

use common::{bench, bench_stack, exasol, report};

#[test]
fn benchmark() {
    let stack = bench_stack::shared();

    let samples = bench::run(stack);
    let provenance = bench::gather_provenance(stack);

    // `cargo test` runs a test binary with its crate directory as the
    // working directory, not the workspace root, so the report must be
    // anchored at `workspace_root()` rather than a bare relative path — a
    // bare "dist/bench" would land under
    // `crates/adbc-vs-adapter/dist/bench`, which the workspace's root-only
    // `/dist` gitignore entry does not cover.
    let report_dir = exasol::workspace_root().join("dist/bench");
    let (csv_path, markdown_path) = report::write_report(&report_dir, &samples, &provenance);

    println!("wrote {}", csv_path.display());
    println!("wrote {}", markdown_path.display());
}
