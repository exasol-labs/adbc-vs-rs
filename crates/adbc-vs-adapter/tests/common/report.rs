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

use super::bench_data::Source;
use super::bench_stack::JdbcBaseline;

/// Which virtual-schema path produced a [`Sample`].
///
/// `Abc` is this repo's ADBC-based scan UDF; `Jdbc` is Exasol's official
/// JDBC-based virtual schema for the source (`mysql-virtual-schema` for
/// MySQL). Named after the technology each path is built on,
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
///
/// `jdbc` is `None` for an ADBC-only run, which the report then renders with
/// no JDBC rows, no ratio column, and a framing note saying so.
pub struct Provenance {
    pub source: Source,
    pub exasol_image: String,
    pub source_image: String,
    pub jdbc: Option<JdbcBaseline>,
    pub so_size_bytes: u64,
    pub so_mtime: String,
    /// Source-specific tuning rows, as (label, rendered value).
    pub source_settings: Vec<(String, String)>,
    pub host_cpu_count: usize,
    pub host_load_average: String,
    pub docker_version: String,
    pub generated_at_utc: String,
}

/// Renders one CSV row per sample, header included, each row naming
/// `source` so CSVs from several sources concatenate unambiguously.
pub fn render_csv(source: Source, samples: &[Sample]) -> String {
    let mut csv = String::from(
        "source,tier,adapter,run_index,warm_up,rows,bytes,duration_seconds,rows_per_sec,mib_per_sec\n",
    );
    for sample in samples {
        csv.push_str(&format!(
            "{},{},{},{},{},{},{},{},{},{}\n",
            source.name(),
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

    markdown.push_str(&format!(
        "# adbc-vs Benchmark Report: {}\n\n",
        provenance.source.display_name()
    ));
    markdown.push_str(&render_provenance_table(provenance));
    markdown.push_str(&framing_note(provenance));
    markdown.push_str(CLUSTER_CAVEAT);
    markdown.push_str(&render_summary_table(samples, provenance.jdbc.is_some()));

    markdown
}

fn framing_note(provenance: &Provenance) -> String {
    match &provenance.jdbc {
        Some(jdbc) => format!(
            "\
## What this compares

This benchmark measures Exasol's native ExaLoader bulk-transfer path \
(`IMPORT ... FROM JDBC`, reached through the `{}` \
adapter's pushdown planning) against this repo's Rust scan UDF, which \
streams Arrow batches over ADBC. It is **not** a \"Java vs Rust\" comparison \
of adapter code: for this benchmark's query shape, the JDBC adapter's Java \
script only performs metadata and pushdown planning, and the actual bulk \
data transfer runs through Exasol's native ExaLoader, entirely outside the \
Java UDF sandbox.

",
            jdbc.vs_jar_name
        ),
        None => format!(
            "\
## What this measures

ADBC-only run: no JDBC baseline exists for {} in this benchmark, so these \
are absolute throughput figures for this repo's Rust scan UDF streaming \
Arrow batches over ADBC, with no comparison ratio.

",
            provenance.source.display_name()
        ),
    }
}

const CLUSTER_CAVEAT: &str = "\
## Caveat

These results come from a single-node `docker-db` container and do not \
extrapolate to a real multi-node Exasol cluster. Both ExaLoader and the scan \
UDF parallelize across nodes in a cluster deployment — a dimension this \
single-node benchmark cannot exercise.

";

fn render_provenance_table(provenance: &Provenance) -> String {
    let mut table = String::from("## Provenance\n\n| Field | Value |\n|---|---|\n");
    let source_name = provenance.source.display_name();
    let mut rows: Vec<(String, String)> = vec![
        ("Source".to_string(), provenance.source.name().to_string()),
        ("Exasol image".to_string(), provenance.exasol_image.clone()),
        (
            format!("{source_name} image"),
            provenance.source_image.clone(),
        ),
    ];
    match &provenance.jdbc {
        Some(jdbc) => rows.extend([
            (
                format!("{} jar version", jdbc.vs_jar_name),
                jdbc.vs_jar_version.to_string(),
            ),
            (
                format!("{} version", jdbc.driver_name),
                jdbc.driver_version.to_string(),
            ),
            (
                "JDBC driver FETCHSIZE".to_string(),
                jdbc.fetchsize.to_string(),
            ),
        ]),
        None => rows.push((
            "JDBC baseline".to_string(),
            "none (ADBC-only run)".to_string(),
        )),
    }
    rows.extend([
        (
            "adbc-vs .so size (bytes)".to_string(),
            provenance.so_size_bytes.to_string(),
        ),
        ("adbc-vs .so mtime".to_string(), provenance.so_mtime.clone()),
    ]);
    rows.extend(provenance.source_settings.iter().cloned());
    rows.extend([
        (
            "Host CPU count".to_string(),
            provenance.host_cpu_count.to_string(),
        ),
        (
            "Host load average (1/5/15 min)".to_string(),
            provenance.host_load_average.clone(),
        ),
        (
            "Docker version".to_string(),
            provenance.docker_version.clone(),
        ),
        (
            "Generated at (UTC)".to_string(),
            provenance.generated_at_utc.clone(),
        ),
    ]);
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

/// The per-(tier, adapter) summary. `with_ratio` is false for an ADBC-only
/// run, which then carries no ratio column at all rather than a column of
/// `n/a`.
fn render_summary_table(samples: &[Sample], with_ratio: bool) -> String {
    let groups = group_rows_per_sec(samples);
    let mut table = String::from(if with_ratio {
        "## Summary\n\n\
         | Tier | Adapter | Median rows/sec | Median MiB/sec | Min rows/sec | adbc \u{f7} jdbc rows/sec ratio |\n\
         |---|---|---|---|---|---|\n"
    } else {
        "## Summary\n\n\
         | Tier | Adapter | Median rows/sec | Median MiB/sec | Min rows/sec |\n\
         |---|---|---|---|---|\n"
    });

    for ((tier, adapter), rows_per_sec) in &groups {
        let mut sorted = rows_per_sec.clone();
        let median_rows = median(&mut sorted);
        let min_rows = sorted.first().copied().unwrap_or(f64::NAN);
        let median_mib = median_mib_per_sec(samples, tier, *adapter);
        table.push_str(&format!(
            "| {tier} | {adapter} | {median_rows:.0} | {median_mib:.2} | {min_rows:.0} |"
        ));
        if with_ratio {
            let ratio = match (
                find_median(&groups, tier, Adapter::Abc),
                find_median(&groups, tier, Adapter::Jdbc),
            ) {
                (Some(abc), Some(jdbc)) => format!("{:.3}", abc / jdbc),
                _ => "n/a".to_string(),
            };
            table.push_str(&format!(" {ratio} |"));
        }
        table.push('\n');
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
/// `provenance.source` and `provenance.generated_at_utc`
/// (`results-<source>-<stamp>.csv`, `report-<source>-<stamp>.md`), and
/// returns their paths as (csv, markdown).
pub fn write_report(dir: &Path, samples: &[Sample], provenance: &Provenance) -> (PathBuf, PathBuf) {
    fs::create_dir_all(dir).unwrap_or_else(|error| panic!("creating {}: {error}", dir.display()));

    let source = provenance.source.name();
    let stamp = filename_safe(&provenance.generated_at_utc);
    let csv_path = dir.join(format!("results-{source}-{stamp}.csv"));
    let markdown_path = dir.join(format!("report-{source}-{stamp}.md"));

    fs::write(&csv_path, render_csv(provenance.source, samples))
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
            source: Source::Mysql,
            exasol_image: "exasol/docker-db:2025.2.1".to_string(),
            source_image: "mysql:8.4".to_string(),
            jdbc: Some(super::super::bench_stack::MYSQL_JDBC_BASELINE),
            so_size_bytes: 12_345_678,
            so_mtime: "2026-08-28T00:00:00Z".to_string(),
            source_settings: vec![(
                "MySQL innodb_buffer_pool_size".to_string(),
                "128M".to_string(),
            )],
            host_cpu_count: 8,
            host_load_average: "0.52 / 0.58 / 0.59".to_string(),
            docker_version: "27.3.1".to_string(),
            generated_at_utc: "2026-08-28T12:34:56Z".to_string(),
        }
    }

    fn adbc_only_provenance() -> Provenance {
        Provenance {
            source: Source::Trino,
            source_image: "trinodb/trino:483".to_string(),
            jdbc: None,
            source_settings: Vec::new(),
            ..provenance()
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
        let csv = render_csv(Source::Mysql, &[]);
        assert_eq!(
            csv,
            "source,tier,adapter,run_index,warm_up,rows,bytes,duration_seconds,rows_per_sec,mib_per_sec\n"
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
        let csv = render_csv(Source::Postgres, &samples);
        let data_line = csv.lines().nth(1).expect("one data row");
        assert_eq!(
            data_line,
            "postgres,narrow_small,adbc,0,true,100000,10485760,2,50000,5"
        );
    }

    #[test]
    fn render_csv_has_one_line_per_sample_plus_header() {
        let samples = vec![
            sample("narrow_small", Adapter::Abc, 0, false, 1, 1, 1.0),
            sample("narrow_small", Adapter::Jdbc, 0, false, 1, 1, 1.0),
        ];
        assert_eq!(render_csv(Source::Mysql, &samples).lines().count(), 3);
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
        assert!(markdown.contains("| Source | mysql |"));
        assert!(markdown.contains("| MySQL image | mysql:8.4 |"));
        assert!(markdown.contains("| mysql-virtual-schema jar version | 6.0.3 |"));
        assert!(markdown.contains("| MySQL Connector/J version | 9.6.0 |"));
        assert!(markdown.contains("| MySQL innodb_buffer_pool_size | 128M |"));
        assert!(markdown.contains("0.52 / 0.58 / 0.59"));
    }

    #[test]
    fn render_markdown_without_a_jdbc_baseline_says_so_and_drops_the_ratio_column() {
        let samples = vec![sample("wide", Adapter::Abc, 0, false, 1, 0, 1.0)];
        let markdown = render_markdown(&samples, &adbc_only_provenance());
        assert!(markdown.contains("# adbc-vs Benchmark Report: Trino"));
        assert!(markdown.contains("| Trino image | trinodb/trino:483 |"));
        assert!(markdown.contains("| JDBC baseline | none (ADBC-only run) |"));
        assert!(markdown.contains("ADBC-only run"));
        assert!(!markdown.contains("FETCHSIZE"), "{markdown}");
        assert!(!markdown.contains("rows/sec ratio"), "{markdown}");
        assert!(
            markdown.contains("| wide | adbc | 1 | 0.00 | 1 |\n"),
            "{markdown}"
        );
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

        assert_eq!(csv_path, dir.join("results-mysql-2026-08-28T12-34-56Z.csv"));
        assert_eq!(
            markdown_path,
            dir.join("report-mysql-2026-08-28T12-34-56Z.md")
        );
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
