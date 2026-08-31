//! Rendering the benchmark's output: CSV, a provenance-carrying Markdown
//! report, and a one-line stdout echo for `--nocapture` progress.
//!
//! Every function here is pure — it turns already-measured data into text —
//! except [`write_report`], whose only side effect is placing that text on
//! disk. The measuring itself (driving the live stack, timing a CTAS) is
//! `bench.rs`'s job, not this module's.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Which virtual-schema path produced a [`Sample`].
///
/// `Abc` is this repo's ADBC-based scan UDF; `Jdbc` is Exasol's official
/// `mysql-virtual-schema`. Named after the technology each path is built on,
/// not the language of its adapter code — the benchmark's whole point is that
/// "JDBC vs ADBC" and "Java vs Rust" are not the same comparison here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Adapter {
    Abc,
    Jdbc,
}

impl Adapter {
    /// The lowercase form every rendered output uses.
    fn label(self) -> &'static str {
        match self {
            Adapter::Abc => "adbc",
            Adapter::Jdbc => "jdbc",
        }
    }
}

impl fmt::Display for Adapter {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.label())
    }
}

/// One timed run of one (tier, adapter) pair.
///
/// `duration` is a plain [`Duration`], never an `Option` — a run that did not
/// happen must not exist as a `Sample` at all, so "no data" and "zero
/// elapsed time" can never be confused with each other.
#[derive(Debug, Clone)]
pub struct Sample {
    pub tier: String,
    pub adapter: Adapter,
    pub run_index: u32,
    pub warm_up: bool,
    pub rows: u64,
    pub bytes: u64,
    pub duration: Duration,
}

impl Sample {
    /// Throughput in rows per second, derived from `rows` and `duration`.
    pub fn rows_per_sec(&self) -> f64 {
        self.rows as f64 / self.duration.as_secs_f64()
    }

    /// Throughput in MiB per second, derived from `bytes` and `duration`.
    pub fn mib_per_sec(&self) -> f64 {
        (self.bytes as f64 / (1024.0 * 1024.0)) / self.duration.as_secs_f64()
    }
}

/// Everything the report's header needs to let a reader independently check
/// what environment a number came from, gathered by the caller (`bench.rs`)
/// from the live stack, `docker-compose.yml`, and the built `.so`.
pub struct Provenance {
    pub exasol_image: String,
    pub mysql_image: String,
    pub vs_jar_version: String,
    pub connector_j_version: String,
    pub jdbc_fetchsize: String,
    pub so_size_bytes: u64,
    pub so_mtime: String,
    pub mysql_innodb_buffer_pool_size: String,
    pub host_cpu_count: usize,
    pub docker_version: String,
    pub generated_at_utc: String,
}

/// Renders one CSV row per sample, header included.
pub fn render_csv(samples: &[Sample]) -> String {
    let mut csv = String::from(
        "tier,adapter,run_index,warm_up,rows,bytes,duration_seconds,rows_per_sec,mib_per_sec\n",
    );
    for sample in samples {
        csv.push_str(&format!(
            "{},{},{},{},{},{},{},{},{}\n",
            sample.tier,
            sample.adapter,
            sample.run_index,
            sample.warm_up,
            sample.rows,
            sample.bytes,
            sample.duration.as_secs_f64(),
            sample.rows_per_sec(),
            sample.mib_per_sec(),
        ));
    }
    csv
}

/// Renders the Markdown report: provenance header, the framing note and
/// cluster caveat this benchmark's numbers require, then a per-(tier,
/// adapter) summary table.
pub fn render_markdown(samples: &[Sample], provenance: &Provenance) -> String {
    let mut markdown = String::new();

    markdown.push_str("# adbc-vs Benchmark Report\n\n");
    markdown.push_str(&render_provenance_table(provenance));
    markdown.push_str(FRAMING_NOTE);
    markdown.push_str(CLUSTER_CAVEAT);
    markdown.push_str(&render_summary_table(samples));

    markdown
}

const FRAMING_NOTE: &str = "\
## What this compares

This benchmark measures Exasol's native ExaLoader bulk-transfer path \
(`IMPORT ... FROM JDBC`, reached through the `mysql-virtual-schema` \
adapter's pushdown planning) against this repo's Rust scan UDF, which \
streams Arrow batches over ADBC. It is **not** a \"Java vs Rust\" comparison \
of adapter code: for this benchmark's query shape, the JDBC adapter's Java \
script only performs metadata and pushdown planning, and the actual bulk \
data transfer runs through Exasol's native ExaLoader, entirely outside the \
Java UDF sandbox.

";

const CLUSTER_CAVEAT: &str = "\
## Caveat

These results come from a single-node `docker-db` container and do not \
extrapolate to a real multi-node Exasol cluster. Both ExaLoader and the scan \
UDF parallelize across nodes in a cluster deployment — a dimension this \
single-node benchmark cannot exercise.

";

fn render_provenance_table(provenance: &Provenance) -> String {
    let mut table = String::from("## Provenance\n\n| Field | Value |\n|---|---|\n");
    let rows: [(&str, String); 11] = [
        ("Exasol image", provenance.exasol_image.clone()),
        ("MySQL image", provenance.mysql_image.clone()),
        (
            "mysql-virtual-schema jar version",
            provenance.vs_jar_version.clone(),
        ),
        (
            "MySQL Connector/J version",
            provenance.connector_j_version.clone(),
        ),
        ("JDBC driver FETCHSIZE", provenance.jdbc_fetchsize.clone()),
        (
            "adbc-vs .so size (bytes)",
            provenance.so_size_bytes.to_string(),
        ),
        ("adbc-vs .so mtime", provenance.so_mtime.clone()),
        (
            "MySQL innodb_buffer_pool_size",
            provenance.mysql_innodb_buffer_pool_size.clone(),
        ),
        ("Host CPU count", provenance.host_cpu_count.to_string()),
        ("Docker version", provenance.docker_version.clone()),
        ("Generated at (UTC)", provenance.generated_at_utc.clone()),
    ];
    for (label, value) in rows {
        table.push_str(&format!("| {label} | {value} |\n"));
    }
    table.push('\n');
    table
}

/// A (tier, adapter) pair's non-warm-up samples, in first-seen order.
type Group = ((String, Adapter), Vec<f64>);

/// Groups the non-warm-up samples' rows/sec by (tier, adapter), preserving
/// the order each pair first appears in. The sample counts here are always
/// small (three tiers times two adapters times a handful of runs), so a
/// linear scan per sample costs nothing worth optimizing away.
fn group_rows_per_sec(samples: &[Sample]) -> Vec<Group> {
    let mut groups: Vec<Group> = Vec::new();
    for sample in samples.iter().filter(|sample| !sample.warm_up) {
        let key = (sample.tier.clone(), sample.adapter);
        match groups.iter_mut().find(|(existing, _)| *existing == key) {
            Some((_, values)) => values.push(sample.rows_per_sec()),
            None => groups.push((key, vec![sample.rows_per_sec()])),
        }
    }
    groups
}

fn median_mib_per_sec(samples: &[Sample], tier: &str, adapter: Adapter) -> f64 {
    let mut values: Vec<f64> = samples
        .iter()
        .filter(|sample| !sample.warm_up && sample.tier == tier && sample.adapter == adapter)
        .map(Sample::mib_per_sec)
        .collect();
    median(&mut values)
}

fn render_summary_table(samples: &[Sample]) -> String {
    let groups = group_rows_per_sec(samples);
    let mut table = String::from(
        "## Summary\n\n\
         | Tier | Adapter | Median rows/sec | Median MiB/sec | Min rows/sec | adbc \u{f7} jdbc rows/sec ratio |\n\
         |---|---|---|---|---|---|\n",
    );

    for ((tier, adapter), rows_per_sec) in &groups {
        let mut sorted = rows_per_sec.clone();
        let median_rows = median(&mut sorted);
        let min_rows = sorted.first().copied().unwrap_or(f64::NAN);
        let median_mib = median_mib_per_sec(samples, tier, *adapter);
        let ratio = match (
            find_median(&groups, tier, Adapter::Abc),
            find_median(&groups, tier, Adapter::Jdbc),
        ) {
            (Some(abc), Some(jdbc)) => format!("{:.3}", abc / jdbc),
            _ => "n/a".to_string(),
        };
        table.push_str(&format!(
            "| {tier} | {adapter} | {median_rows:.0} | {median_mib:.2} | {min_rows:.0} | {ratio} |\n"
        ));
    }

    table.push('\n');
    table
}

fn find_median(groups: &[Group], tier: &str, adapter: Adapter) -> Option<f64> {
    groups
        .iter()
        .find(|((group_tier, group_adapter), _)| group_tier == tier && *group_adapter == adapter)
        .map(|(_, values)| median(&mut values.clone()))
}

/// The middle value once `values` is sorted. An even count averages the two
/// middle values; the benchmark's own design always produces an odd count
/// (three measured runs per pair), so that branch exists only so the
/// function is total rather than because the benchmark relies on it.
fn median(values: &mut [f64]) -> f64 {
    assert!(!values.is_empty(), "median of zero samples is undefined");
    values.sort_by(|a, b| a.partial_cmp(b).expect("benchmark durations are never NaN"));
    let mid = values.len() / 2;
    if values.len().is_multiple_of(2) {
        (values[mid - 1] + values[mid]) / 2.0
    } else {
        values[mid]
    }
}

/// Prints one line of live progress for a completed sample, for the
/// Makefile's `--nocapture` run to surface.
pub fn print_sample(sample: &Sample) {
    let warm_up = if sample.warm_up { " (warm-up)" } else { "" };
    println!(
        "{} {} run {}{warm_up}: {:.0} rows/sec",
        sample.tier,
        sample.adapter,
        sample.run_index,
        sample.rows_per_sec()
    );
}

/// Writes both report files into `dir` (created if missing), named from
/// `provenance.generated_at_utc`, and returns their paths as (csv, markdown).
pub fn write_report(dir: &Path, samples: &[Sample], provenance: &Provenance) -> (PathBuf, PathBuf) {
    fs::create_dir_all(dir).unwrap_or_else(|error| panic!("creating {}: {error}", dir.display()));

    let stamp = filename_safe(&provenance.generated_at_utc);
    let csv_path = dir.join(format!("results-{stamp}.csv"));
    let markdown_path = dir.join(format!("report-{stamp}.md"));

    fs::write(&csv_path, render_csv(samples))
        .unwrap_or_else(|error| panic!("writing {}: {error}", csv_path.display()));
    fs::write(&markdown_path, render_markdown(samples, provenance))
        .unwrap_or_else(|error| panic!("writing {}: {error}", markdown_path.display()));

    (csv_path, markdown_path)
}

/// Replaces every character a filename can't safely carry (an RFC 3339
/// timestamp's `:` above all) with `-`.
fn filename_safe(text: &str) -> String {
    text.chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                '-'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(
        tier: &str,
        adapter: Adapter,
        run_index: u32,
        warm_up: bool,
        rows: u64,
        bytes: u64,
        seconds: f64,
    ) -> Sample {
        Sample {
            tier: tier.to_string(),
            adapter,
            run_index,
            warm_up,
            rows,
            bytes,
            duration: Duration::from_secs_f64(seconds),
        }
    }

    fn provenance() -> Provenance {
        Provenance {
            exasol_image: "exasol/docker-db:2025.2.1".to_string(),
            mysql_image: "mysql:8.4".to_string(),
            vs_jar_version: "6.0.3".to_string(),
            connector_j_version: "9.6.0".to_string(),
            jdbc_fetchsize: "100000".to_string(),
            so_size_bytes: 12_345_678,
            so_mtime: "2026-08-28T00:00:00Z".to_string(),
            mysql_innodb_buffer_pool_size: "128M".to_string(),
            host_cpu_count: 8,
            docker_version: "27.3.1".to_string(),
            generated_at_utc: "2026-08-28T12:34:56Z".to_string(),
        }
    }

    #[test]
    fn adapter_displays_as_lowercase_technology_name() {
        assert_eq!(Adapter::Abc.to_string(), "adbc");
        assert_eq!(Adapter::Jdbc.to_string(), "jdbc");
    }

    #[test]
    fn sample_derives_rows_per_sec_from_rows_and_duration() {
        let sample = sample(
            "narrow_small",
            Adapter::Abc,
            0,
            false,
            100_000,
            10_000_000,
            2.0,
        );
        assert_eq!(sample.rows_per_sec(), 50_000.0);
    }

    #[test]
    fn sample_derives_mib_per_sec_from_bytes_and_duration() {
        let sample = sample(
            "narrow_small",
            Adapter::Abc,
            0,
            false,
            100_000,
            2 * 1024 * 1024,
            2.0,
        );
        assert_eq!(sample.mib_per_sec(), 1.0);
    }

    #[test]
    fn median_of_odd_count_is_the_middle_value() {
        let mut values = vec![3.0, 1.0, 2.0];
        assert_eq!(median(&mut values), 2.0);
    }

    #[test]
    fn median_of_even_count_averages_the_two_middle_values() {
        let mut values = vec![1.0, 2.0, 3.0, 4.0];
        assert_eq!(median(&mut values), 2.5);
    }

    #[test]
    fn median_of_single_value_is_that_value() {
        let mut values = vec![7.0];
        assert_eq!(median(&mut values), 7.0);
    }

    #[test]
    #[should_panic(expected = "median of zero samples is undefined")]
    fn median_of_no_values_panics() {
        median(&mut []);
    }

    #[test]
    fn render_csv_includes_a_header_row() {
        let csv = render_csv(&[]);
        assert_eq!(
            csv,
            "tier,adapter,run_index,warm_up,rows,bytes,duration_seconds,rows_per_sec,mib_per_sec\n"
        );
    }

    #[test]
    fn render_csv_includes_one_row_per_sample_with_derived_columns() {
        let samples = vec![sample(
            "narrow_small",
            Adapter::Abc,
            0,
            true,
            100_000,
            10_485_760,
            2.0,
        )];
        let csv = render_csv(&samples);
        let data_line = csv.lines().nth(1).expect("one data row");
        assert_eq!(
            data_line,
            "narrow_small,adbc,0,true,100000,10485760,2,50000,5"
        );
    }

    #[test]
    fn render_csv_has_one_line_per_sample_plus_header() {
        let samples = vec![
            sample("narrow_small", Adapter::Abc, 0, false, 1, 1, 1.0),
            sample("narrow_small", Adapter::Jdbc, 0, false, 1, 1, 1.0),
        ];
        assert_eq!(render_csv(&samples).lines().count(), 3);
    }

    #[test]
    fn render_markdown_includes_every_provenance_field() {
        let markdown = render_markdown(&[], &provenance());
        assert!(markdown.contains("exasol/docker-db:2025.2.1"));
        assert!(markdown.contains("mysql:8.4"));
        assert!(markdown.contains("6.0.3"));
        assert!(markdown.contains("9.6.0"));
        assert!(markdown.contains("100000"));
        assert!(markdown.contains("12345678"));
        assert!(markdown.contains("2026-08-28T00:00:00Z"));
        assert!(markdown.contains("128M"));
        assert!(markdown.contains(" 8 "));
        assert!(markdown.contains("27.3.1"));
        assert!(markdown.contains("2026-08-28T12:34:56Z"));
    }

    #[test]
    fn render_markdown_states_the_exaloader_vs_arrow_udf_framing() {
        let markdown = render_markdown(&[], &provenance());
        assert!(markdown.contains("ExaLoader"));
        assert!(markdown.contains("not"));
        assert!(markdown.contains("Java vs Rust"));
    }

    #[test]
    fn render_markdown_states_the_single_node_cluster_caveat() {
        let markdown = render_markdown(&[], &provenance());
        assert!(markdown.contains("single-node"));
        assert!(markdown.contains("cluster"));
    }

    #[test]
    fn render_markdown_summary_reports_the_median_across_measured_runs_and_excludes_warm_up() {
        let samples = vec![
            sample("narrow_small", Adapter::Abc, 0, true, 100_000, 0, 100.0),
            sample("narrow_small", Adapter::Abc, 0, false, 100_000, 0, 2.0),
            sample("narrow_small", Adapter::Abc, 1, false, 100_000, 0, 1.0),
            sample("narrow_small", Adapter::Abc, 2, false, 100_000, 0, 4.0),
        ];
        let markdown = render_markdown(&samples, &provenance());
        assert!(
            markdown.contains("| narrow_small | adbc | 50000 |"),
            "expected the median (100000/2s=50000 rows/sec) in:\n{markdown}"
        );
    }

    #[test]
    fn render_markdown_summary_computes_the_adbc_over_jdbc_ratio_per_tier() {
        let samples = vec![
            sample("narrow_small", Adapter::Abc, 0, false, 100_000, 0, 1.0),
            sample("narrow_small", Adapter::Jdbc, 0, false, 100_000, 0, 2.0),
        ];
        let markdown = render_markdown(&samples, &provenance());
        assert!(
            markdown.contains("| narrow_small | adbc | 100000 | 0.00 | 100000 | 2.000 |"),
            "expected a 2x ratio (100000 vs 50000 rows/sec) in:\n{markdown}"
        );
        assert!(
            markdown.contains("| narrow_small | jdbc | 50000 | 0.00 | 50000 | 2.000 |"),
            "expected the same ratio on the jdbc row in:\n{markdown}"
        );
    }

    #[test]
    fn render_markdown_summary_reports_ratio_as_not_available_without_both_adapters() {
        let samples = vec![sample("wide", Adapter::Abc, 0, false, 1, 0, 1.0)];
        let markdown = render_markdown(&samples, &provenance());
        assert!(markdown.contains("| wide | adbc | 1 | 0.00 | 1 | n/a |"));
    }

    #[test]
    fn write_report_creates_the_directory_and_both_named_files() {
        let dir = std::env::temp_dir().join(format!("adbc-vs-report-test-{}", std::process::id()));
        let samples = vec![sample("narrow_small", Adapter::Abc, 0, false, 1, 1, 1.0)];
        let provenance = provenance();

        let (csv_path, markdown_path) = write_report(&dir, &samples, &provenance);

        assert_eq!(csv_path, dir.join("results-2026-08-28T12-34-56Z.csv"));
        assert_eq!(markdown_path, dir.join("report-2026-08-28T12-34-56Z.md"));
        assert!(fs::metadata(&csv_path).is_ok_and(|metadata| metadata.len() > 0));
        assert!(fs::metadata(&markdown_path).is_ok_and(|metadata| metadata.len() > 0));

        fs::remove_dir_all(&dir)
            .unwrap_or_else(|error| panic!("cleaning up {}: {error}", dir.display()));
    }

    #[test]
    fn filename_safe_replaces_every_non_alphanumeric_character() {
        assert_eq!(
            filename_safe("2026-08-28T12:34:56Z"),
            "2026-08-28T12-34-56Z"
        );
    }
}
