//! Runs the full PostgreSQL transmission-throughput benchmark against the
//! Docker Compose stack: every tier `bench_data::Tier::all()` declares,
//! alternating ADBC and JDBC (`postgresql-virtual-schema`) samples, then
//! writes the CSV and Markdown report under `dist/bench/`.
//!
//! The twin of `bench_mysql_test.rs`. There is no skip path: [`bench::run`]
//! panics on any fairness or verification failure rather than returning a
//! result for a run that did not actually work.
//!
//! One shared stack provisions both virtual schemas and the `BENCH` sink
//! schema once, which is why this runs with `--test-threads=1`.
#![cfg(feature = "bench")]

mod common;

use common::{bench, bench_stack, exasol, report};

#[test]
fn benchmark() {
    let stack = bench_stack::shared_postgres();

    let samples = bench::run(stack);
    let provenance = bench::gather_provenance(stack);

    // Anchored at `workspace_root()` for the same reason as in
    // `bench_mysql_test.rs`: `cargo test` runs with the crate directory as the
    // working directory, and only the root `/dist` is gitignored.
    let report_dir = exasol::workspace_root().join("dist/bench");
    let (csv_path, markdown_path) = report::write_report(&report_dir, &samples, &provenance);

    println!("wrote {}", csv_path.display());
    println!("wrote {}", markdown_path.display());
}
