//! Runs the Trino transmission-throughput benchmark against the Docker
//! Compose stack: every tier `bench_data::Tier::all()` declares, ADBC samples
//! only, then writes the CSV and Markdown report under `dist/bench/`.
//!
//! ADBC-only by necessity, not by choice: Exasol publishes no JDBC-based
//! virtual schema for Trino as a source, so there is no baseline to
//! alternate against and the report carries absolute ADBC throughput
//! (rows/s, MiB/s) per tier rather than a speedup.
//!
//! There is no skip path. [`bench::run`] panics on any verification failure
//! — a dropped row, a mismatched column set, an unreachable stack — rather
//! than returning a result for a run that did not actually work, so this
//! test's only job is orchestration: bring up the shared stack, run the
//! samples, and write the report.
//!
//! One shared stack provisions the virtual schema and the `BENCH` sink schema
//! once, which is why this binary runs with `--test-threads=1`.
#![cfg(feature = "bench")]

mod common;

use common::{bench, bench_stack, exasol, report};

#[test]
fn benchmark() {
    let stack = bench_stack::shared_trino();

    let samples = bench::run(stack);
    let provenance = bench::gather_provenance(stack);

    // Anchored at `workspace_root()` for the reason `bench_mysql_test.rs`
    // gives: `cargo test` runs with the crate directory as the working
    // directory, which the root-only `/dist` gitignore entry does not cover.
    let report_dir = exasol::workspace_root().join("dist/bench");
    let (csv_path, markdown_path) = report::write_report(&report_dir, &samples, &provenance);

    println!("wrote {}", csv_path.display());
    println!("wrote {}", markdown_path.display());
}
