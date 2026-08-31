//! Driving the benchmark itself: the alternating warm-up/measured loop over
//! both adapters, the per-sample verification that turns a fast run into a
//! failed one the moment it looks suspicious, and the provenance a report
//! needs to be independently checkable.
//!
//! Split the same way [`super::bench_data`] is: everything that only builds
//! SQL text or does arithmetic is pure and lives under `make test` with no
//! Docker running; [`run`] and [`gather_provenance`] are the only two
//! functions that reach a live stack, and both delegate every judgement they
//! make to a pure function beside them.
//!
//! # Alternation
//!
//! Each [`Tier`] runs [`ROUNDS`] rounds — one discarded warm-up followed by
//! [`MEASURED_ROUNDS`] measured rounds — and every round runs both adapters,
//! one after the other. [`round_order`] swaps which adapter goes first on
//! every round (ADBC, JDBC, ADBC, JDBC becomes JDBC-first on the odd rounds),
//! so a systematic effect that favors "whichever adapter runs first" — cache
//! state, thermal drift, background load easing off — lands on both adapters
//! equally across the run rather than always advantaging one of them.
//!
//! # Checksum
//!
//! [`bench_data`] seeds every tier with contiguous ids `1..=target_rows` (see
//! its own module doc), so the sum of a tier's id column has a closed form —
//! [`expected_checksum`] — that needs no extra round trip to MySQL to compute
//! and no dependency on which adapter answered it.
//!
//! # Warm-up samples
//!
//! The warm-up run is verified exactly like a measured one — a warm-up that
//! silently dropped rows is still a benchmark that never worked — but it is
//! still pushed into the returned samples with `warm_up: true` rather than
//! discarded. [`super::report`]'s summary table already filters warm-up
//! samples out of the median it reports; keeping them in the CSV instead
//! means the report's raw data carries every sample the run actually made,
//! not a subset chosen before anyone got to look at it.

use std::fs;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use super::bench_data::{self, Tier};
use super::bench_stack::{self, BenchStack};
use super::exasol::{self, Exasol};
use super::report::{Adapter, Provenance, Sample};

/// One discarded warm-up round followed by [`MEASURED_ROUNDS`] measured ones.
const WARM_UP_ROUNDS: u32 = 1;
const MEASURED_ROUNDS: u32 = 3;
const ROUNDS: u32 = WARM_UP_ROUNDS + MEASURED_ROUNDS;

/// The schema-qualified sink table every sample's CTAS creates and a later
/// sample replaces.
const SINK_TABLE: &str = "SINK";

/// The `mysql-virtual-schema` jar version and MySQL Connector/J version
/// [`super::jdbc_vs`] provisions, and the `FETCHSIZE` its `settings.cfg`
/// registers.
///
/// Mirrored here rather than imported: `jdbc_vs`'s jar constants and the
/// `FETCHSIZE` literal inside its generated `settings.cfg` are private to
/// that module, existing only to build BucketFS upload paths and driver
/// registration text. Reporting a human-readable provenance string is this
/// module's own concern, not a reason to expose a new public item there —
/// but it does mean the two modules must be bumped together if either one's
/// version or `FETCHSIZE` ever changes.
const VS_JAR_VERSION: &str = "6.0.3";
const CONNECTOR_J_VERSION: &str = "9.6.0";
const JDBC_FETCHSIZE: &str = "100000";

/// The shared object `make cross-udf-build` produces, relative to the
/// workspace root — mirrored from [`super::udf`], whose own copy of this
/// path is private to its BucketFS upload.
const BUILT_LIBRARY: &str = "target/release/libadbc_vs_adapter.so";

/// Runs every [`Tier`] the benchmark defines: seeds it, gates it on the
/// fairness column-set check, then times [`ROUNDS`] alternating rounds of
/// both adapters, verifying and echoing every sample as it completes.
///
/// A failure anywhere — an unseeded tier, a column mismatch, a dropped row, a
/// wrong checksum — panics rather than being recorded, so nothing this
/// function returns can be a benchmark result for a run that did not actually
/// work.
pub fn run(stack: &BenchStack) -> Vec<Sample> {
    let mut samples = Vec::new();

    for tier in Tier::all() {
        bench_data::ensure_seeded(&stack.mysql, &tier);
        bench_data::assert_matching_columns(&stack.exasol, &tier);

        let expected_checksum = expected_checksum(tier.target_rows);

        for round_index in 0..ROUNDS {
            let warm_up = is_warm_up_round(round_index);
            let run_index = round_index.saturating_sub(WARM_UP_ROUNDS);

            for adapter in round_order(round_index) {
                let sample =
                    time_sample(stack, &tier, adapter, run_index, warm_up, expected_checksum);
                super::report::print_sample(&sample);
                samples.push(sample);
            }
        }
    }

    samples
}

/// Times one CTAS, verifies its result, and returns the [`Sample`] it
/// produced.
fn time_sample(
    stack: &BenchStack,
    tier: &Tier,
    adapter: Adapter,
    run_index: u32,
    warm_up: bool,
    expected_checksum: u64,
) -> Sample {
    let statement = sink_ctas_statement(tier, virtual_schema_for(adapter));

    let started = Instant::now();
    stack.exasol.execute(&statement);
    let duration = started.elapsed();

    verify_sink(&stack.exasol, tier, expected_checksum);

    Sample {
        tier: tier.name.to_string(),
        adapter,
        run_index,
        warm_up,
        rows: tier.target_rows,
        bytes: tier.target_rows * u64::from(bench_data::nominal_row_width(tier)),
        duration,
    }
}

/// Fails the run unless the sink CTAS just produced exactly `tier`'s seeded
/// row count and the checksum contiguous ids `1..=tier.target_rows` predict.
///
/// Both checks run on every sample, warm-up included: a warm-up that quietly
/// moved the wrong data would still leave every measured sample after it
/// looking like a valid comparison.
fn verify_sink(exasol: &Exasol, tier: &Tier, expected_checksum: u64) {
    let sink = sink_table();

    let row_count = parse_row_count(
        &exasol
            .query_scalar(&format!("SELECT COUNT(*) FROM {sink}"))
            .unwrap_or_else(|| {
                panic!(
                    "tier `{}`: the sink row count query answered no value",
                    tier.name
                )
            }),
        tier.name,
    );
    assert_eq!(
        row_count, tier.target_rows,
        "tier `{}` sink held {row_count} row(s) after its CTAS, not the {} rows seeded — a \
         fast run that dropped rows must fail, not report a throughput figure",
        tier.name, tier.target_rows
    );

    let column = id_column(tier);
    let checksum = parse_checksum(
        &exasol
            .query_scalar(&format!("SELECT SUM(\"{column}\") FROM {sink}"))
            .unwrap_or_else(|| {
                panic!(
                    "tier `{}`: the sink checksum query answered no value",
                    tier.name
                )
            }),
        column,
        tier.name,
    );
    assert_eq!(
        checksum, expected_checksum,
        "tier `{}` sink's SUM(\"{column}\") was {checksum}, not the {expected_checksum} \
         expected for the contiguous ids 1..={}",
        tier.name, tier.target_rows
    );
}

/// The row-number-driving primary key column's name — `columns[0]` by the
/// convention [`Tier`] documents.
fn id_column(tier: &Tier) -> &'static str {
    tier.columns[0].0
}

/// The sum of every id from 1 to `target_rows`, the checksum a tier's
/// contiguously-seeded id column must equal.
fn expected_checksum(target_rows: u64) -> u64 {
    target_rows
        .checked_mul(target_rows + 1)
        .and_then(|product| product.checked_div(2))
        .unwrap_or_else(|| panic!("target_rows {target_rows} overflows the checksum formula"))
}

/// The `CREATE OR REPLACE TABLE ... AS SELECT` this benchmark times: every
/// column of `tier`, lowercase double-quoted, from `virtual_schema`'s
/// lowercase double-quoted table.
///
/// Deliberately carries no `ORDER BY` and no `LIMIT` — the query shape this
/// benchmark's fairness invariant requires, since the JDBC side pushes both
/// down and the ADBC side pushes neither, and including either would stop
/// comparing the same work.
fn sink_ctas_statement(tier: &Tier, virtual_schema: &str) -> String {
    format!(
        "CREATE OR REPLACE TABLE {} AS SELECT {} FROM {virtual_schema}.\"{}\"",
        sink_table(),
        quoted_column_list(tier),
        tier.name
    )
}

fn quoted_column_list(tier: &Tier) -> String {
    tier.columns
        .iter()
        .map(|(name, _)| format!("\"{name}\""))
        .collect::<Vec<_>>()
        .join(", ")
}

fn sink_table() -> String {
    format!("{}.{SINK_TABLE}", bench_stack::SINK_SCHEMA)
}

fn virtual_schema_for(adapter: Adapter) -> &'static str {
    match adapter {
        Adapter::Abc => bench_stack::ADBC_VIRTUAL_SCHEMA,
        Adapter::Jdbc => bench_stack::JDBC_VIRTUAL_SCHEMA,
    }
}

fn is_warm_up_round(round_index: u32) -> bool {
    round_index < WARM_UP_ROUNDS
}

/// Which adapter runs first, and second, in round `round_index`: ADBC first
/// on an even round, JDBC first on an odd one.
///
/// A round's two samples always run back-to-back regardless of order — this
/// only decides which one goes first, so whatever the machine is doing at the
/// start of a round (caches cold, a background process winding down) hits
/// both adapters across the run instead of always the same one.
fn round_order(round_index: u32) -> [Adapter; 2] {
    if round_index.is_multiple_of(2) {
        [Adapter::Abc, Adapter::Jdbc]
    } else {
        [Adapter::Jdbc, Adapter::Abc]
    }
}

/// Gathers everything [`Provenance`] needs from the live stack, the built
/// `.so`, and `docker-compose.yml`, so the report's header can be checked
/// against its sources rather than trusted.
pub fn gather_provenance(stack: &BenchStack) -> Provenance {
    let workspace_root = exasol::workspace_root();

    let compose_text = fs::read_to_string(workspace_root.join("docker-compose.yml"))
        .unwrap_or_else(|error| panic!("reading docker-compose.yml: {error}"));

    let so_path = workspace_root.join(BUILT_LIBRARY);
    let so_metadata = fs::metadata(&so_path)
        .unwrap_or_else(|error| panic!("reading metadata for {}: {error}", so_path.display()));
    let so_modified = so_metadata.modified().unwrap_or_else(|error| {
        panic!(
            "reading the modified time of {}: {error}",
            so_path.display()
        )
    });

    let host_cpu_count = std::thread::available_parallelism()
        .unwrap_or_else(|error| panic!("reading the host CPU count: {error}"))
        .get();

    let docker_version = parse_docker_version(&super::run(
        "reading the docker version",
        "docker",
        &["--version"],
    ));

    Provenance {
        exasol_image: parse_service_image(&compose_text, "exasol"),
        mysql_image: parse_service_image(&compose_text, "mysql"),
        vs_jar_version: VS_JAR_VERSION.to_string(),
        connector_j_version: CONNECTOR_J_VERSION.to_string(),
        jdbc_fetchsize: JDBC_FETCHSIZE.to_string(),
        so_size_bytes: so_metadata.len(),
        so_mtime: format_utc(so_modified),
        mysql_innodb_buffer_pool_size: stack.mysql.query_scalar("SELECT @@innodb_buffer_pool_size"),
        host_cpu_count,
        docker_version,
        generated_at_utc: format_utc(SystemTime::now()),
    }
}

/// The image tag `docker-compose.yml` declares for `service`, read out of its
/// raw text rather than a YAML parser: the file's shape is stable and simple
/// enough — a two-space-indented service name, its properties indented
/// further underneath — that matching that shape directly is simpler than
/// bringing in a parser to do it.
fn parse_service_image(compose_text: &str, service: &str) -> String {
    let header = format!("  {service}:");
    let mut lines = compose_text.lines().skip_while(|line| *line != "services:");
    assert!(
        lines.next().is_some(),
        "docker-compose.yml declares no `services:` section"
    );

    let mut in_service = false;
    for line in lines {
        if line == header {
            in_service = true;
            continue;
        }
        if !in_service {
            continue;
        }
        if let Some(value) = line.trim().strip_prefix("image:") {
            return value.trim().to_string();
        }
        if line.starts_with("  ") && !line.starts_with("    ") {
            break;
        }
    }

    panic!("docker-compose.yml declares no `image:` under service `{service}`")
}

/// The version number out of `docker --version`'s `Docker version X, build Y`
/// text.
fn parse_docker_version(raw: &str) -> String {
    let trimmed = raw.trim();
    let after_prefix = trimmed.strip_prefix("Docker version ").unwrap_or_else(|| {
        panic!(
            "`docker --version` answered `{trimmed}`, not the expected `Docker version <x>` shape"
        )
    });
    after_prefix
        .split(',')
        .next()
        .unwrap_or(after_prefix)
        .trim()
        .to_string()
}

fn parse_row_count(rendered: &str, tier_name: &str) -> u64 {
    rendered.parse().unwrap_or_else(|error| {
        panic!("tier `{tier_name}`: sink row count `{rendered}` did not parse: {error}")
    })
}

/// Parses a `SUM(...)` result as a checksum, accepting either an integer
/// rendering or a decimal one — Exasol's `SUM` over an `INT` column can
/// render as either, depending on the type it settles on internally.
fn parse_checksum(rendered: &str, id_column: &str, tier_name: &str) -> u64 {
    rendered.parse::<u64>().unwrap_or_else(|_| {
        rendered
            .parse::<f64>()
            .map(|value| value.round() as u64)
            .unwrap_or_else(|error| {
                panic!(
                "tier `{tier_name}`: sink SUM(\"{id_column}\") `{rendered}` did not parse as a \
                 checksum: {error}"
            )
            })
    })
}

const SECONDS_PER_DAY: u64 = 24 * 60 * 60;

/// Formats `time` as an RFC 3339 UTC timestamp (`YYYY-MM-DDThh:mm:ssZ`).
///
/// Hand-rolled rather than pulled from a date/time crate: this module has
/// exactly two timestamps to render, and [`civil_from_days`] is a small,
/// well-known, dependency-free way to do the one nontrivial part — Gregorian
/// calendar math — without adding a dependency for it.
fn format_utc(time: SystemTime) -> String {
    let elapsed = time
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|error| panic!("a benchmark timestamp predates the Unix epoch: {error}"));

    let total_seconds = elapsed.as_secs();
    let days = (total_seconds / SECONDS_PER_DAY) as i64;
    let time_of_day = total_seconds % SECONDS_PER_DAY;
    let (year, month, day) = civil_from_days(days);

    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        time_of_day / 3600,
        (time_of_day % 3600) / 60,
        time_of_day % 60,
    )
}

/// Converts a day count since the Unix epoch (1970-01-01) into a Gregorian
/// (year, month, day), by Howard Hinnant's public-domain `civil_from_days`
/// algorithm (<http://howardhinnant.github.io/date_algorithms.html>).
fn civil_from_days(days_since_epoch: i64) -> (i64, u32, u32) {
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = (z - era * 146_097) as u64;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era as i64 + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_index + 2) / 5 + 1) as u32;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    } as u32;
    let year = if month <= 2 { year + 1 } else { year };
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoted_column_list_double_quotes_every_column_lowercase() {
        let tier = Tier::all()[0];
        assert_eq!(
            quoted_column_list(&tier),
            "\"id\", \"big_id\", \"amount\", \"name\", \"code\", \"order_date\", \
             \"created_at\", \"price\""
        );
    }

    #[test]
    fn sink_table_names_the_fixed_schema_and_table() {
        assert_eq!(sink_table(), "BENCH.SINK");
    }

    #[test]
    fn sink_ctas_statement_selects_the_tiers_columns_from_the_named_virtual_schema() {
        let tier = Tier::all()[0];
        assert_eq!(
            sink_ctas_statement(&tier, "MYSQLVS_ADBC"),
            "CREATE OR REPLACE TABLE BENCH.SINK AS SELECT \"id\", \"big_id\", \"amount\", \
             \"name\", \"code\", \"order_date\", \"created_at\", \"price\" FROM \
             MYSQLVS_ADBC.\"narrow_small\""
        );
    }

    #[test]
    fn sink_ctas_statement_never_includes_order_by_or_limit() {
        for tier in Tier::all() {
            for schema in ["MYSQLVS_ADBC", "MYSQLVS_JDBC"] {
                let statement = sink_ctas_statement(&tier, schema).to_uppercase();
                assert!(
                    !statement.contains("ORDER BY") && !statement.contains("LIMIT"),
                    "the benchmark query must push neither ORDER BY nor LIMIT, or the two \
                     adapters would stop being compared over the same work: {statement}"
                );
            }
        }
    }

    #[test]
    fn virtual_schema_for_maps_each_adapter_to_its_own_virtual_schema() {
        assert_eq!(virtual_schema_for(Adapter::Abc), "MYSQLVS_ADBC");
        assert_eq!(virtual_schema_for(Adapter::Jdbc), "MYSQLVS_JDBC");
    }

    #[test]
    fn id_column_reads_the_tiers_first_declared_column() {
        for tier in Tier::all() {
            assert_eq!(id_column(&tier), "id");
        }
    }

    #[test]
    fn expected_checksum_sums_the_contiguous_ids_from_one_to_target_rows() {
        assert_eq!(expected_checksum(4), 10);
        assert_eq!(expected_checksum(100_000), 5_000_050_000);
        assert_eq!(expected_checksum(1_000_000), 500_000_500_000);
        assert_eq!(expected_checksum(200_000), 20_000_100_000);
    }

    #[test]
    fn is_warm_up_round_is_true_for_only_the_first_round() {
        assert!(is_warm_up_round(0));
        for round in WARM_UP_ROUNDS..ROUNDS {
            assert!(
                !is_warm_up_round(round),
                "round {round} must not be a warm-up"
            );
        }
    }

    #[test]
    fn rounds_run_one_warm_up_and_three_measured_rounds() {
        assert_eq!(WARM_UP_ROUNDS, 1);
        assert_eq!(MEASURED_ROUNDS, 3);
        assert_eq!(ROUNDS, 4);
    }

    #[test]
    fn round_order_alternates_which_adapter_goes_first() {
        assert_eq!(round_order(0), [Adapter::Abc, Adapter::Jdbc]);
        assert_eq!(round_order(1), [Adapter::Jdbc, Adapter::Abc]);
        assert_eq!(round_order(2), [Adapter::Abc, Adapter::Jdbc]);
        assert_eq!(round_order(3), [Adapter::Jdbc, Adapter::Abc]);
    }

    #[test]
    fn round_order_never_runs_the_same_adapter_first_on_every_round() {
        let first_adapters: Vec<Adapter> = (0..ROUNDS).map(|round| round_order(round)[0]).collect();
        assert!(
            first_adapters.contains(&Adapter::Abc) && first_adapters.contains(&Adapter::Jdbc),
            "every round starting with the same adapter would let a systematic effect favor \
             one side: {first_adapters:?}"
        );
    }

    #[test]
    fn parse_row_count_reads_the_number_exasol_rendered() {
        assert_eq!(parse_row_count("100000", "narrow_small"), 100_000);
    }

    #[test]
    fn parse_row_count_rejects_an_answer_that_is_not_a_count() {
        let payload = std::panic::catch_unwind(|| parse_row_count("not-a-number", "narrow_small"))
            .expect_err("a non-numeric answer is not a row count");
        let message = payload
            .downcast_ref::<String>()
            .cloned()
            .expect("a message");
        assert!(message.contains("narrow_small"), "{message}");
    }

    #[test]
    fn parse_checksum_reads_a_plain_integer_rendering() {
        assert_eq!(
            parse_checksum("5000050000", "id", "narrow_small"),
            5_000_050_000
        );
    }

    #[test]
    fn parse_checksum_reads_a_decimal_rendering() {
        assert_eq!(
            parse_checksum("5000050000.0", "id", "narrow_small"),
            5_000_050_000
        );
    }

    #[test]
    fn parse_checksum_rejects_an_answer_that_is_neither() {
        let payload = std::panic::catch_unwind(|| parse_checksum("n/a", "id", "narrow_small"))
            .expect_err("a non-numeric answer is not a checksum");
        let message = payload
            .downcast_ref::<String>()
            .cloned()
            .expect("a message");
        assert!(
            message.contains("narrow_small") && message.contains("id"),
            "{message}"
        );
    }

    #[test]
    fn parse_docker_version_reads_the_version_out_of_the_full_banner() {
        assert_eq!(
            parse_docker_version("Docker version 27.3.1, build ce12230\n"),
            "27.3.1"
        );
    }

    #[test]
    fn parse_docker_version_rejects_an_answer_of_the_wrong_shape() {
        let payload = std::panic::catch_unwind(|| parse_docker_version("not docker at all"))
            .expect_err("an answer with no version prefix is not a docker version string");
        let message = payload
            .downcast_ref::<String>()
            .cloned()
            .expect("a message");
        assert!(message.contains("not docker at all"), "{message}");
    }

    const COMPOSE_FIXTURE: &str = "\
networks:
  adbc-vs:
    driver: bridge

volumes:
  exasol-data:

services:
  exasol:
    image: exasol/docker-db:2025.2.1
    privileged: true
    ports:
      - \"127.0.0.1:8563:8563\"

  mysql:
    image: mysql:8.4
    ports:
      - \"127.0.0.1:3306:3306\"
";

    #[test]
    fn parse_service_image_reads_the_first_services_image() {
        assert_eq!(
            parse_service_image(COMPOSE_FIXTURE, "exasol"),
            "exasol/docker-db:2025.2.1"
        );
    }

    #[test]
    fn parse_service_image_reads_the_second_services_image() {
        assert_eq!(parse_service_image(COMPOSE_FIXTURE, "mysql"), "mysql:8.4");
    }

    #[test]
    fn parse_service_image_fails_the_run_for_an_undeclared_service() {
        let payload = std::panic::catch_unwind(|| parse_service_image(COMPOSE_FIXTURE, "postgres"))
            .expect_err("a service the compose file does not declare cannot have an image");
        let message = payload
            .downcast_ref::<String>()
            .cloned()
            .expect("a message");
        assert!(message.contains("postgres"), "{message}");
    }

    #[test]
    fn civil_from_days_reads_the_unix_epoch() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
    }

    #[test]
    fn civil_from_days_crosses_a_thirty_one_day_month() {
        assert_eq!(civil_from_days(31), (1970, 2, 1));
    }

    #[test]
    fn civil_from_days_crosses_a_leap_year() {
        assert_eq!(civil_from_days(730), (1972, 1, 1));
    }

    #[test]
    fn format_utc_renders_the_unix_epoch() {
        assert_eq!(format_utc(UNIX_EPOCH), "1970-01-01T00:00:00Z");
    }

    #[test]
    fn format_utc_zero_pads_a_single_digit_second() {
        assert_eq!(
            format_utc(UNIX_EPOCH + std::time::Duration::from_secs(5)),
            "1970-01-01T00:00:05Z"
        );
    }

    #[test]
    fn format_utc_carries_the_time_of_day_across_a_day_boundary() {
        assert_eq!(
            format_utc(UNIX_EPOCH + std::time::Duration::from_secs(90_000)),
            "1970-01-02T01:00:00Z"
        );
    }
}
