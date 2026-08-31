//! The benchmark's MySQL data tiers: what each one is, how it is seeded, and
//! the gate that proves both adapters see it identically before anything is
//! timed.
//!
//! Two halves, deliberately kept apart. Everything above [`ensure_seeded`] is
//! pure — it builds SQL text and computes byte widths, touching no MySQL, no
//! Docker, and no live row count, which is what puts the exact statements this
//! benchmark issues under `make test` with no stack running. [`ensure_seeded`]
//! and [`assert_matching_columns`] are the only two functions that reach a live
//! stack, and each delegates every judgement it makes to a pure function
//! beside them, so the reasoning stays testable and only the I/O does not.
//!
//! # Doubling-insert seeding
//!
//! Each [`Tier`] is seeded by a small deterministic base batch
//! ([`seed_insert`]), then grown by repeated `INSERT INTO t SELECT ... FROM
//! t` steps ([`doubling_step`]) that double the row count, and finally
//! trimmed to the exact target ([`seeding_plan`]'s trim statement). This
//! keeps seeding to a handful of constant-size statements per tier instead of
//! streaming millions of literal `INSERT` values from Rust.
//!
//! Every column's value in both the base batch and every doubling step is
//! derived from the same per-type formula ([`value_expression`]), applied to
//! a single "row number" input: a literal row index for the base batch, or
//! `(id + offset)` — the pre-doubling id, offset by the row count so far —
//! for a doubling step. Reusing one formula for both means a doubling step
//! never needs column-specific logic, and every column stays a deterministic
//! function of the row's `id`, so re-running seeding from scratch reproduces
//! identical data and no doubling step can mint a duplicate primary key.
//!
//! # Nominal byte width
//!
//! [`nominal_row_width`] sums one fixed byte estimate per column, for the
//! report's MiB/sec figure. It is deliberately not byte-perfect — assumptions
//! (documented on [`nominal_column_width`]) mirror common fixed-width
//! estimates for these MySQL types, nothing more precise.

use std::collections::BTreeSet;

use super::bench_stack::{ADBC_VIRTUAL_SCHEMA, JDBC_VIRTUAL_SCHEMA};
use super::exasol::{cell, Exasol};
use super::mysql::MySql;

/// One data tier the benchmark seeds and scans.
///
/// `name` is both the tier's identifier and the MySQL table name it seeds
/// into. `columns` is ordered, and by convention its first entry is the
/// row-number-driving primary key column (named `id` in every tier below) —
/// every builder in this module keys off `columns[0]` for that role rather
/// than hard-coding the name `"id"`.
#[derive(Debug, Clone, Copy)]
pub struct Tier {
    pub name: &'static str,
    pub target_rows: u64,
    pub columns: &'static [(&'static str, &'static str)],
}

/// The 8-column, ~100B/row shape shared by `narrow_small` and `narrow_large`
/// — matches the MySQL type set `adbc-vs-scan`'s coercion already proves it
/// maps (see `tests/common/mysql.rs`'s `orders` fixture): `INT`, `BIGINT`,
/// `DECIMAL`, `VARCHAR`, `CHAR`, `DATE`, `DATETIME`, `DOUBLE`.
const NARROW_COLUMNS: &[(&str, &str)] = &[
    ("id", "INT"),
    ("big_id", "BIGINT"),
    ("amount", "DECIMAL(10,2)"),
    ("name", "VARCHAR(40)"),
    ("code", "CHAR(8)"),
    ("order_date", "DATE"),
    ("created_at", "DATETIME"),
    ("price", "DOUBLE"),
];

/// The ~1KB/row shape for `wide`: the same proven type set as
/// [`NARROW_COLUMNS`], widened with more `VARCHAR(200)` and `DECIMAL` columns
/// rather than any new type.
const WIDE_COLUMNS: &[(&str, &str)] = &[
    ("id", "INT"),
    ("big_id", "BIGINT"),
    ("amount_a", "DECIMAL(12,4)"),
    ("amount_b", "DECIMAL(12,4)"),
    ("amount_c", "DECIMAL(12,4)"),
    ("description_a", "VARCHAR(200)"),
    ("description_b", "VARCHAR(200)"),
    ("description_c", "VARCHAR(200)"),
    ("description_d", "VARCHAR(200)"),
    ("code", "CHAR(8)"),
    ("order_date", "DATE"),
    ("created_at", "DATETIME"),
    ("price", "DOUBLE"),
];

/// The size of the deterministic base batch every tier is seeded from before
/// any doubling step runs.
const BASE_SEED_ROWS: u64 = 4;

impl Tier {
    /// The three medium-scale tiers the benchmark runs, in the order the
    /// report presents them.
    pub fn all() -> [Tier; 3] {
        [
            Tier {
                name: "narrow_small",
                target_rows: 100_000,
                columns: NARROW_COLUMNS,
            },
            Tier {
                name: "narrow_large",
                target_rows: 1_000_000,
                columns: NARROW_COLUMNS,
            },
            Tier {
                name: "wide",
                target_rows: 200_000,
                columns: WIDE_COLUMNS,
            },
        ]
    }

    /// The row-number-driving primary key column's name — `columns[0]` by
    /// convention.
    fn id_column(&self) -> &'static str {
        self.columns[0].0
    }
}

/// Builds the `CREATE TABLE IF NOT EXISTS` DDL for `tier`, with its id column
/// as the primary key.
pub fn create_table_ddl(tier: &Tier) -> String {
    let column_defs: Vec<String> = tier
        .columns
        .iter()
        .map(|(name, ddl_type)| format!("{name} {ddl_type} NOT NULL"))
        .collect();
    format!(
        "CREATE TABLE IF NOT EXISTS {} ({}, PRIMARY KEY ({}))",
        tier.name,
        column_defs.join(", "),
        tier.id_column()
    )
}

/// Builds the base-seed `INSERT` statement: [`BASE_SEED_ROWS`] deterministic
/// rows, row `n`'s columns each derived from the literal row number `n` by
/// [`value_expression`].
pub fn seed_insert(tier: &Tier) -> String {
    let column_names: Vec<&str> = tier.columns.iter().map(|(name, _)| *name).collect();
    let rows: Vec<String> = (1..=BASE_SEED_ROWS)
        .map(|row_number| {
            let values: Vec<String> = tier
                .columns
                .iter()
                .map(|(_, ddl_type)| value_expression(ddl_type, &row_number.to_string()))
                .collect();
            format!("({})", values.join(", "))
        })
        .collect();
    format!(
        "INSERT INTO {} ({}) VALUES {}",
        tier.name,
        column_names.join(", "),
        rows.join(", ")
    )
}

/// Builds one doubling step: `INSERT INTO <table> SELECT ... FROM <table>`,
/// where every column is recomputed from `(id + offset)` by
/// [`value_expression`] — the same formula [`seed_insert`] applies to a
/// literal row number. `offset` must be the table's current row count, so
/// every new id lands strictly above every existing one and no doubling step
/// can mint a duplicate primary key.
pub fn doubling_step(tier: &Tier, offset: u64) -> String {
    let row_number_expr = format!("({} + {offset})", tier.id_column());
    let select_list: Vec<String> = tier
        .columns
        .iter()
        .map(|(_, ddl_type)| value_expression(ddl_type, &row_number_expr))
        .collect();
    format!(
        "INSERT INTO {} SELECT {} FROM {}",
        tier.name,
        select_list.join(", "),
        tier.name
    )
}

/// The ordered doubling steps and optional trim needed to take `tier` from
/// [`BASE_SEED_ROWS`] to exactly `tier.target_rows`.
pub struct SeedingPlan {
    /// The row-count offset to pass to [`doubling_step`], one per step, in
    /// the order they must run.
    pub doubling_offsets: Vec<u64>,
    /// `Some` when doubling overshoots the target, trimming the surplus rows
    /// above it. `None` when the target is an exact power-of-two multiple of
    /// [`BASE_SEED_ROWS`] and doubling lands on it exactly.
    pub trim_statement: Option<String>,
}

/// Computes [`SeedingPlan`] for `tier`: doubles from [`BASE_SEED_ROWS`] until
/// the row count reaches or exceeds `tier.target_rows`, then trims any
/// overshoot so the final count is exact, not merely "at least".
pub fn seeding_plan(tier: &Tier) -> SeedingPlan {
    let mut row_count = BASE_SEED_ROWS;
    let mut doubling_offsets = Vec::new();
    while row_count < tier.target_rows {
        doubling_offsets.push(row_count);
        row_count *= 2;
    }
    let trim_statement = (row_count > tier.target_rows).then(|| trim_statement_for(tier));
    SeedingPlan {
        doubling_offsets,
        trim_statement,
    }
}

/// Builds the `DELETE` statement that trims `tier`'s table down to exactly
/// `tier.target_rows`, by deleting every row whose id exceeds the target.
/// Sound because every id [`doubling_step`] mints is offset above the
/// previous row count, so ids stay in seed order with no gaps: the rows below
/// the target are exactly the rows to keep.
fn trim_statement_for(tier: &Tier) -> String {
    format!(
        "DELETE FROM {} WHERE {} > {}",
        tier.name,
        tier.id_column(),
        tier.target_rows
    )
}

/// The nominal per-row byte width of `tier`: the sum of [`nominal_column_width`]
/// over its columns, for the report's MiB/sec figure.
pub fn nominal_row_width(tier: &Tier) -> u32 {
    tier.columns
        .iter()
        .map(|(_, ddl_type)| nominal_column_width(ddl_type))
        .sum()
}

/// A defensible, fixed byte estimate for one MySQL column type — not
/// byte-perfect, just consistent enough to make MiB/sec figures comparable
/// across tiers and across a re-run:
///
/// - `INT` = 4, `BIGINT` = 8: native fixed-width integers.
/// - `DECIMAL(p,s)` ≈ `p / 2 + 1`: a packed-decimal-style estimate (roughly
///   two digits per byte) plus one byte for sign, independent of scale.
/// - `VARCHAR(n)` = `n`, `CHAR(n)` = `n`: worst-case (`VARCHAR`) or exact
///   (`CHAR`) character count, one byte per character.
/// - `DATE` = 3, `DATETIME` = 8, `DOUBLE` = 8: native fixed-width temporal
///   and floating-point encodings.
fn nominal_column_width(ddl_type: &str) -> u32 {
    let upper = ddl_type.to_ascii_uppercase();
    if upper.starts_with("BIGINT") {
        8
    } else if upper.starts_with("INT") {
        4
    } else if upper.starts_with("DATETIME") {
        8
    } else if upper.starts_with("DATE") {
        3
    } else if upper.starts_with("DOUBLE") {
        8
    } else if upper.starts_with("DECIMAL") {
        let precision = paren_number(&upper)
            .unwrap_or_else(|| panic!("DECIMAL type `{ddl_type}` declares no precision"));
        precision / 2 + 1
    } else if upper.starts_with("VARCHAR") || upper.starts_with("CHAR") {
        paren_number(&upper)
            .unwrap_or_else(|| panic!("{ddl_type} type `{ddl_type}` declares no length"))
    } else {
        panic!("no nominal byte width rule for MySQL type `{ddl_type}`")
    }
}

/// Builds the SQL expression that computes one column's deterministic value
/// from `row_number_expr` — a literal row index from [`seed_insert`], or an
/// `(id + offset)` expression from [`doubling_step`] — so both call sites
/// share one per-type formula instead of each column needing bespoke
/// transform logic:
///
/// - `DATETIME`/`DATE`: an absolute date offset by `row_number_expr` via
///   `DATE_ADD`, since neither is a valid `CAST` target in MySQL.
/// - `VARCHAR`/`CHAR`: the row number's decimal-string form via
///   `CAST(... AS CHAR)`, always well within these tiers' declared widths at
///   their target row counts.
/// - `INT`/`BIGINT`: `CAST(... AS SIGNED)` — MySQL's `CAST` has no `INT` or
///   `BIGINT` target type; `SIGNED` is the 64-bit signed integer cast, which
///   narrows on assignment into an `INT` column exactly as a literal would.
/// - `DOUBLE`/`DECIMAL(p,s)`: `CAST(... AS <ddl_type>)` directly — both are
///   valid `CAST` target types in MySQL 8.0.17+.
fn value_expression(ddl_type: &str, row_number_expr: &str) -> String {
    let upper = ddl_type.to_ascii_uppercase();
    if upper.starts_with("DATETIME") {
        format!("DATE_ADD('2020-01-01 00:00:00', INTERVAL {row_number_expr} MINUTE)")
    } else if upper.starts_with("DATE") {
        format!("DATE_ADD('2020-01-01', INTERVAL {row_number_expr} DAY)")
    } else if upper.starts_with("VARCHAR") || upper.starts_with("CHAR") {
        format!("CAST({row_number_expr} AS CHAR)")
    } else if upper.starts_with("INT") || upper.starts_with("BIGINT") {
        format!("CAST({row_number_expr} AS SIGNED)")
    } else if upper.starts_with("DOUBLE") || upper.starts_with("DECIMAL") {
        format!("CAST({row_number_expr} AS {ddl_type})")
    } else {
        panic!("no value-expression rule for MySQL type `{ddl_type}`")
    }
}

/// Parses the first number inside a type's parentheses, e.g. `12` out of
/// `"DECIMAL(12,4)"` or `"VARCHAR(12)"`.
fn paren_number(ddl_type: &str) -> Option<u32> {
    let after_open = ddl_type.split_once('(')?.1;
    let digits = after_open.split([',', ')']).next()?;
    digits.trim().parse().ok()
}

/// The statement that removes `tier`'s table before it is rebuilt, tolerating
/// a table that was never there.
fn drop_table_statement(tier: &Tier) -> String {
    format!("DROP TABLE IF EXISTS {}", tier.name)
}

/// Brings `tier`'s MySQL table to exactly `tier.target_rows` rows, and proves
/// it got there before returning.
///
/// A table already holding exactly the target count is left untouched, so a
/// benchmark re-run against a stack an earlier one seeded starts timing
/// immediately instead of regenerating a million rows it already has. Anything
/// else — no table, a short one, one an interrupted run left half-grown — is
/// dropped and rebuilt from scratch rather than topped up, because the tier's
/// data is a deterministic function of its row number and a partial table
/// carries no record of how far the generator got.
///
/// The count is re-read after the rebuild and disagreement fails the run.
/// Seeding that silently produced the wrong number of rows would not surface
/// as an error later: it would surface as a throughput figure, which is worse.
pub fn ensure_seeded(mysql: &MySql, tier: &Tier) {
    if seeded_rows(mysql, tier) == Some(tier.target_rows) {
        return;
    }

    rebuild(mysql, tier);

    let seeded = seeded_rows(mysql, tier).unwrap_or_else(|| {
        panic!(
            "seeding tier `{}` left no table `{}` in MySQL",
            tier.name, tier.name
        )
    });
    assert_eq!(
        seeded, tier.target_rows,
        "seeding tier `{}` left {seeded} row(s) in MySQL table `{}`, not the {} it targets",
        tier.name, tier.name, tier.target_rows
    );
}

/// The row count of `tier`'s table, or `None` when the table is not there.
///
/// Absence is answered from `information_schema` rather than from a failed
/// `COUNT(*)`, so a MySQL that cannot answer at all fails the run instead of
/// being read as an empty tier that needs reseeding.
fn seeded_rows(mysql: &MySql, tier: &Tier) -> Option<u64> {
    table_exists(mysql, tier.name).then(|| {
        row_count_of(
            &mysql.query_scalar(&format!("SELECT COUNT(*) FROM {}", tier.name)),
            tier.name,
        )
    })
}

fn table_exists(mysql: &MySql, table: &str) -> bool {
    let rendered = mysql.query_scalar(&format!(
        "SELECT COUNT(*) FROM information_schema.tables \
         WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = '{table}'"
    ));
    row_count_of(&rendered, table) > 0
}

/// Reads the count MySQL rendered for `table`, failing the run on anything
/// that is not one.
fn row_count_of(rendered: &str, table: &str) -> u64 {
    rendered.parse().unwrap_or_else(|error| {
        panic!("MySQL answered a count over `{table}` with `{rendered}`, not a row count: {error}")
    })
}

/// Regenerates `tier`'s table from nothing: the base batch, every doubling
/// step [`seeding_plan`] calls for, then the trim onto the exact target.
fn rebuild(mysql: &MySql, tier: &Tier) {
    let plan = seeding_plan(tier);

    mysql.execute(&drop_table_statement(tier));
    mysql.execute(&create_table_ddl(tier));
    mysql.execute(&seed_insert(tier));
    for offset in plan.doubling_offsets {
        mysql.execute(&doubling_step(tier, offset));
    }
    if let Some(trim) = &plan.trim_statement {
        mysql.execute(trim);
    }
}

/// Fails the run unless both virtual schemas the benchmark compares expose
/// exactly the columns `tier` declares.
///
/// This is the benchmark's fairness gate, and it belongs before a tier's first
/// timed sample rather than after it: an adapter that quietly omitted a column
/// it could not map would move less data for the same query and report the
/// difference as speed. Holding both sides to the tier's own declaration
/// rather than only to each other also catches the case where both narrow
/// identically — still a fair race, but one whose MiB/sec figure would be
/// computed from bytes nothing transferred.
///
/// Both schemas are refreshed first. They cache their source's metadata at
/// creation, so a column set read without refreshing is a snapshot from before
/// this tier's table existed; republishing it is part of reading the live
/// answer, not a step a caller could sensibly skip.
///
/// Column names are compared case-sensitively. Both virtual schemas expose
/// MySQL identifiers in lower case and every benchmark query therefore
/// double-quotes them, so `"id"` and `"ID"` are different identifiers to
/// Exasol — a casing difference between the sides is one a query built for
/// either side could not survive against the other.
pub fn assert_matching_columns(exasol: &Exasol, tier: &Tier) {
    let sides: Vec<ExposedColumns> = [ADBC_VIRTUAL_SCHEMA, JDBC_VIRTUAL_SCHEMA]
        .into_iter()
        .map(|schema| {
            exasol.execute(&refresh_statement(schema));
            ExposedColumns {
                schema: schema.to_owned(),
                columns: exposed_columns(exasol, schema, tier.name),
            }
        })
        .collect();

    if let Some(mismatch) = column_set_mismatch(tier, &sides) {
        panic!("{mismatch}");
    }
}

/// The statement that republishes `schema`'s metadata from its source.
fn refresh_statement(schema: &str) -> String {
    format!("ALTER VIRTUAL SCHEMA {schema} REFRESH")
}

/// The column names `schema` exposes for `table`, empty when it exposes no
/// such table at all.
fn exposed_columns(exasol: &Exasol, schema: &str, table: &str) -> Vec<String> {
    exasol
        .query_rows(&format!(
            "SELECT COLUMN_NAME FROM SYS.EXA_ALL_COLUMNS \
             WHERE COLUMN_SCHEMA = '{schema}' AND COLUMN_TABLE = '{table}' \
             ORDER BY COLUMN_ORDINAL_POSITION"
        ))
        .iter()
        .filter_map(|row| cell(row, "COLUMN_NAME"))
        .collect()
}

/// One virtual schema's live answer for a tier's table: the schema's own name,
/// so a report can say which side is wrong, and the columns it exposes.
struct ExposedColumns {
    schema: String,
    columns: Vec<String>,
}

/// How one side's exposed columns differ from a tier's declaration.
struct ColumnDivergence {
    missing: Vec<String>,
    unexpected: Vec<String>,
}

impl ColumnDivergence {
    fn is_empty(&self) -> bool {
        self.missing.is_empty() && self.unexpected.is_empty()
    }
}

/// Reports how `sides` fail to expose exactly what `tier` declares, or `None`
/// when every one of them does.
///
/// Pure, so the gate's own diffing is covered without a container: only
/// [`assert_matching_columns`] talks to a live stack, and it does nothing with
/// the answer but hand it here.
fn column_set_mismatch(tier: &Tier, sides: &[ExposedColumns]) -> Option<String> {
    let declared: BTreeSet<&str> = tier.columns.iter().map(|(name, _)| *name).collect();
    let divergences: Vec<(&ExposedColumns, ColumnDivergence)> = sides
        .iter()
        .map(|side| (side, divergence_from(&declared, side)))
        .collect();

    if divergences
        .iter()
        .all(|(_, divergence)| divergence.is_empty())
    {
        return None;
    }

    let report: Vec<String> = divergences
        .iter()
        .map(|(side, divergence)| side_line(side, divergence))
        .collect();
    Some(format!(
        "tier `{}`: the two virtual schemas do not both expose exactly the {} column(s) \
         declared for MySQL table `{}`, so timing a query over them would not compare the \
         same work:\n{}",
        tier.name,
        declared.len(),
        tier.name,
        report.join("\n")
    ))
}

fn divergence_from(declared: &BTreeSet<&str>, side: &ExposedColumns) -> ColumnDivergence {
    let exposed: BTreeSet<&str> = side.columns.iter().map(String::as_str).collect();
    ColumnDivergence {
        missing: declared
            .difference(&exposed)
            .map(|name| (*name).to_owned())
            .collect(),
        unexpected: exposed
            .difference(declared)
            .map(|name| (*name).to_owned())
            .collect(),
    }
}

/// One line of the report, written for every side rather than only the
/// diverging ones: a reader comparing two schemas has to be able to tell a
/// side that matched from one the report simply left out.
fn side_line(side: &ExposedColumns, divergence: &ColumnDivergence) -> String {
    if divergence.is_empty() {
        return format!("  {} exposes exactly the declared columns", side.schema);
    }
    let mut notes = Vec::new();
    if !divergence.missing.is_empty() {
        notes.push(format!("missing: {}", divergence.missing.join(", ")));
    }
    if !divergence.unexpected.is_empty() {
        notes.push(format!("unexpected: {}", divergence.unexpected.join(", ")));
    }
    format!("  {} — {}", side.schema, notes.join("; "))
}

#[cfg(test)]
mod tests {
    use std::panic;

    use super::*;

    #[test]
    fn all_tiers_match_the_medium_scale_names_and_targets() {
        let tiers = Tier::all();
        assert_eq!(tiers[0].name, "narrow_small");
        assert_eq!(tiers[0].target_rows, 100_000);
        assert_eq!(tiers[1].name, "narrow_large");
        assert_eq!(tiers[1].target_rows, 1_000_000);
        assert_eq!(tiers[2].name, "wide");
        assert_eq!(tiers[2].target_rows, 200_000);
    }

    #[test]
    fn narrow_small_and_narrow_large_share_the_same_eight_columns() {
        let tiers = Tier::all();
        assert_eq!(tiers[0].columns.len(), 8);
        assert_eq!(tiers[0].columns, tiers[1].columns);
    }

    #[test]
    fn every_tier_restricts_columns_to_the_proven_type_set() {
        let allowed_prefixes = [
            "INT", "BIGINT", "DECIMAL", "VARCHAR", "CHAR", "DATE", "DATETIME", "DOUBLE",
        ];
        for tier in Tier::all() {
            for (name, ddl_type) in tier.columns {
                let upper = ddl_type.to_ascii_uppercase();
                assert!(
                    allowed_prefixes
                        .iter()
                        .any(|prefix| upper.starts_with(prefix)),
                    "{}.{name} declares `{ddl_type}`, outside the proven type set",
                    tier.name
                );
            }
        }
    }

    #[test]
    fn create_table_ddl_declares_every_column_not_null_with_the_id_primary_key() {
        let tier = Tier {
            name: "narrow_small",
            target_rows: 100_000,
            columns: NARROW_COLUMNS,
        };
        assert_eq!(
            create_table_ddl(&tier),
            "CREATE TABLE IF NOT EXISTS narrow_small (id INT NOT NULL, \
             big_id BIGINT NOT NULL, amount DECIMAL(10,2) NOT NULL, \
             name VARCHAR(40) NOT NULL, code CHAR(8) NOT NULL, \
             order_date DATE NOT NULL, created_at DATETIME NOT NULL, \
             price DOUBLE NOT NULL, PRIMARY KEY (id))"
        );
    }

    #[test]
    fn create_table_ddl_is_idempotent_via_if_not_exists() {
        let tier = Tier::all()[2];
        assert!(create_table_ddl(&tier).starts_with("CREATE TABLE IF NOT EXISTS wide "));
    }

    #[test]
    fn seed_insert_names_every_column_and_seeds_base_seed_rows_row_tuples() {
        let tier = Tier {
            name: "narrow_small",
            target_rows: 100_000,
            columns: NARROW_COLUMNS,
        };
        let sql = seed_insert(&tier);
        assert!(sql.starts_with(
            "INSERT INTO narrow_small (id, big_id, amount, name, code, order_date, \
             created_at, price) VALUES "
        ));
        assert_eq!(sql.matches("), (").count() + 1, BASE_SEED_ROWS as usize);
    }

    #[test]
    fn seed_insert_derives_every_column_from_the_literal_row_number() {
        let tier = Tier {
            name: "narrow_small",
            target_rows: 100_000,
            columns: NARROW_COLUMNS,
        };
        let sql = seed_insert(&tier);
        assert!(sql.contains("CAST(1 AS SIGNED)"), "id/big_id row 1: {sql}");
        assert!(
            sql.contains("CAST(1 AS DECIMAL(10,2))"),
            "amount row 1: {sql}"
        );
        assert!(sql.contains("CAST(1 AS CHAR)"), "name/code row 1: {sql}");
        assert!(
            sql.contains("DATE_ADD('2020-01-01', INTERVAL 1 DAY)"),
            "order_date row 1: {sql}"
        );
        assert!(
            sql.contains("DATE_ADD('2020-01-01 00:00:00', INTERVAL 1 MINUTE)"),
            "created_at row 1: {sql}"
        );
        assert!(sql.contains("CAST(1 AS DOUBLE)"), "price row 1: {sql}");
        assert!(sql.contains("CAST(4 AS SIGNED)"), "last seeded row: {sql}");
    }

    #[test]
    fn doubling_step_selects_from_and_into_the_same_table() {
        let tier = Tier {
            name: "wide",
            target_rows: 200_000,
            columns: WIDE_COLUMNS,
        };
        let sql = doubling_step(&tier, 500);
        assert!(sql.starts_with("INSERT INTO wide SELECT "));
        assert!(sql.ends_with(" FROM wide"));
    }

    #[test]
    fn doubling_step_offsets_every_column_by_the_current_row_count() {
        let tier = Tier {
            name: "narrow_small",
            target_rows: 100_000,
            columns: NARROW_COLUMNS,
        };
        let sql = doubling_step(&tier, 500);
        assert!(
            sql.contains("CAST((id + 500) AS SIGNED)"),
            "id/big_id offset: {sql}"
        );
        assert!(
            sql.contains("CAST((id + 500) AS DECIMAL(10,2))"),
            "amount offset: {sql}"
        );
        assert!(
            sql.contains("DATE_ADD('2020-01-01', INTERVAL (id + 500) DAY)"),
            "order_date offset: {sql}"
        );
    }

    #[test]
    fn doubling_step_produces_the_right_number_of_select_expressions() {
        let tier = Tier::all()[2];
        let sql = doubling_step(&tier, 1);
        let select_list = sql
            .strip_prefix("INSERT INTO wide SELECT ")
            .and_then(|rest| rest.strip_suffix(" FROM wide"))
            .expect("doubling_step's fixed shape");
        assert_eq!(
            select_list.matches("CAST(").count() + select_list.matches("DATE_ADD(").count(),
            tier.columns.len()
        );
    }

    fn simulate_final_row_count(tier: &Tier, plan: &SeedingPlan) -> u64 {
        let mut row_count = BASE_SEED_ROWS;
        for _ in &plan.doubling_offsets {
            row_count *= 2;
        }
        if plan.trim_statement.is_some() {
            row_count = tier.target_rows;
        }
        row_count
    }

    #[test]
    fn seeding_plan_reaches_the_exact_target_for_every_real_tier() {
        for tier in Tier::all() {
            let plan = seeding_plan(&tier);
            assert_eq!(
                simulate_final_row_count(&tier, &plan),
                tier.target_rows,
                "tier {} did not converge on its target",
                tier.name
            );
            assert!(
                plan.doubling_offsets.len() as f64
                    <= (tier.target_rows as f64 / BASE_SEED_ROWS as f64)
                        .log2()
                        .ceil()
                        + 1.0,
                "tier {} took an implausible number of doubling steps: {}",
                tier.name,
                plan.doubling_offsets.len()
            );
        }
    }

    #[test]
    fn seeding_plan_never_produces_more_than_one_trim_statement() {
        for tier in Tier::all() {
            let plan = seeding_plan(&tier);
            assert!(plan.trim_statement.iter().count() <= 1);
        }
    }

    #[test]
    fn seeding_plan_trims_when_the_target_is_already_below_the_base_seed_size() {
        let tier = Tier {
            name: "tiny",
            target_rows: 2,
            columns: NARROW_COLUMNS,
        };
        let plan = seeding_plan(&tier);
        assert!(
            plan.doubling_offsets.is_empty(),
            "a target below the base seed needs no doubling: {:?}",
            plan.doubling_offsets
        );
        assert_eq!(
            plan.trim_statement,
            Some("DELETE FROM tiny WHERE id > 2".to_string())
        );
    }

    #[test]
    fn seeding_plan_needs_no_trim_when_the_target_is_an_exact_power_of_two_multiple_of_base() {
        let tier = Tier {
            name: "exact",
            target_rows: 64,
            columns: NARROW_COLUMNS,
        };
        let plan = seeding_plan(&tier);
        assert_eq!(plan.doubling_offsets, vec![4, 8, 16, 32]);
        assert_eq!(plan.trim_statement, None);
        assert_eq!(simulate_final_row_count(&tier, &plan), 64);
    }

    #[test]
    fn trim_statement_deletes_by_the_tiers_own_id_column_and_target() {
        let tier = Tier {
            name: "narrow_large",
            target_rows: 1_000_000,
            columns: NARROW_COLUMNS,
        };
        let plan = seeding_plan(&tier);
        assert_eq!(
            plan.trim_statement,
            Some("DELETE FROM narrow_large WHERE id > 1000000".to_string())
        );
    }

    #[test]
    fn nominal_row_width_sums_the_documented_fixed_estimate_per_column() {
        let narrow = Tier {
            name: "narrow_small",
            target_rows: 100_000,
            columns: NARROW_COLUMNS,
        };
        // id(4) + big_id(8) + amount DECIMAL(10,2)(10/2+1=6) + name VARCHAR(40)(40)
        // + code CHAR(8)(8) + order_date DATE(3) + created_at DATETIME(8) + price DOUBLE(8)
        assert_eq!(nominal_row_width(&narrow), 4 + 8 + 6 + 40 + 8 + 3 + 8 + 8);

        let wide = Tier {
            name: "wide",
            target_rows: 200_000,
            columns: WIDE_COLUMNS,
        };
        // id(4) + big_id(8) + 3×DECIMAL(12,4)(12/2+1=7 each) + 4×VARCHAR(200)(200 each)
        // + code CHAR(8)(8) + order_date DATE(3) + created_at DATETIME(8) + price DOUBLE(8)
        assert_eq!(
            nominal_row_width(&wide),
            4 + 8 + 7 * 3 + 200 * 4 + 8 + 3 + 8 + 8
        );
    }

    #[test]
    fn narrow_row_width_lands_near_the_planned_hundred_bytes_per_row() {
        let tier = Tier::all()[0];
        let width = nominal_row_width(&tier);
        assert!(
            (50..=150).contains(&width),
            "narrow_small's nominal width {width} is far from the ~100B/row the plan expects"
        );
    }

    #[test]
    fn wide_row_width_lands_near_the_planned_kilobyte_per_row() {
        let tier = Tier::all()[2];
        let width = nominal_row_width(&tier);
        assert!(
            (500..=1500).contains(&width),
            "wide's nominal width {width} is far from the ~1KB/row the plan expects"
        );
    }

    #[test]
    fn paren_number_reads_the_first_number_inside_parentheses() {
        assert_eq!(paren_number("DECIMAL(12,4)"), Some(12));
        assert_eq!(paren_number("VARCHAR(200)"), Some(200));
        assert_eq!(paren_number("INT"), None);
    }

    #[test]
    fn drop_table_statement_names_the_tiers_table_and_tolerates_its_absence() {
        assert_eq!(
            drop_table_statement(&Tier::all()[0]),
            "DROP TABLE IF EXISTS narrow_small"
        );
    }

    #[test]
    fn refresh_statement_republishes_the_named_virtual_schemas_metadata() {
        assert_eq!(
            refresh_statement("MYSQLVS_ADBC"),
            "ALTER VIRTUAL SCHEMA MYSQLVS_ADBC REFRESH"
        );
    }

    #[test]
    fn row_count_of_reads_the_number_mysql_rendered() {
        assert_eq!(row_count_of("100000", "narrow_small"), 100_000);
    }

    #[test]
    fn row_count_of_rejects_an_answer_that_is_not_a_count() {
        let payload = panic::catch_unwind(|| row_count_of("", "narrow_small"))
            .expect_err("an empty answer is not a row count");
        let message = payload
            .downcast_ref::<String>()
            .cloned()
            .expect("the failure carries a message");
        assert!(
            message.contains("narrow_small"),
            "the failure must name the table it could not count: {message}"
        );
    }

    fn exposing(schema: &str, columns: &[&str]) -> ExposedColumns {
        ExposedColumns {
            schema: schema.to_string(),
            columns: columns.iter().map(|name| name.to_string()).collect(),
        }
    }

    fn narrow_column_names() -> Vec<&'static str> {
        NARROW_COLUMNS.iter().map(|(name, _)| *name).collect()
    }

    #[test]
    fn no_mismatch_when_every_side_exposes_exactly_the_declared_columns() {
        let tier = Tier::all()[0];
        let names = narrow_column_names();
        let sides = [
            exposing("MYSQLVS_ADBC", &names),
            exposing("MYSQLVS_JDBC", &names),
        ];

        assert_eq!(column_set_mismatch(&tier, &sides), None);
    }

    #[test]
    fn no_mismatch_when_a_side_exposes_the_declared_columns_in_another_order() {
        let tier = Tier::all()[0];
        let mut reordered = narrow_column_names();
        reordered.reverse();
        let sides = [
            exposing("MYSQLVS_ADBC", &reordered),
            exposing("MYSQLVS_JDBC", &narrow_column_names()),
        ];

        assert_eq!(
            column_set_mismatch(&tier, &sides),
            None,
            "ordinal position is not part of the column set the benchmark compares"
        );
    }

    #[test]
    fn a_side_omitting_a_declared_column_is_named_with_the_column_it_dropped() {
        let tier = Tier::all()[0];
        let narrowed: Vec<&str> = narrow_column_names()
            .into_iter()
            .filter(|name| *name != "code")
            .collect();
        let sides = [
            exposing("MYSQLVS_ADBC", &narrowed),
            exposing("MYSQLVS_JDBC", &narrow_column_names()),
        ];

        let mismatch =
            column_set_mismatch(&tier, &sides).expect("a narrower side is not a fair comparison");
        assert!(
            mismatch.contains("MYSQLVS_ADBC") && mismatch.contains("missing: code"),
            "the report must name the side and the column it dropped: {mismatch}"
        );
        assert!(
            mismatch.contains("narrow_small"),
            "the report must name the tier it fails: {mismatch}"
        );
    }

    #[test]
    fn a_side_exposing_a_column_beyond_the_declaration_is_named_with_it() {
        let tier = Tier::all()[0];
        let mut widened = narrow_column_names();
        widened.push("stowaway");
        let sides = [
            exposing("MYSQLVS_ADBC", &narrow_column_names()),
            exposing("MYSQLVS_JDBC", &widened),
        ];

        let mismatch =
            column_set_mismatch(&tier, &sides).expect("a wider side is not a fair comparison");
        assert!(
            mismatch.contains("MYSQLVS_JDBC") && mismatch.contains("unexpected: stowaway"),
            "the report must name the side and the column it added: {mismatch}"
        );
    }

    #[test]
    fn a_report_accounts_for_every_side_even_when_only_one_diverges() {
        let tier = Tier::all()[0];
        let sides = [
            exposing("MYSQLVS_ADBC", &[]),
            exposing("MYSQLVS_JDBC", &narrow_column_names()),
        ];

        let mismatch = column_set_mismatch(&tier, &sides).expect("an empty side is a divergence");
        assert!(
            mismatch.contains("MYSQLVS_JDBC"),
            "a reader must be able to tell the matching side apart from an unreported one: \
             {mismatch}"
        );
        for name in narrow_column_names() {
            assert!(
                mismatch.contains(name),
                "a side exposing the table not at all is missing every declared column, \
                 including {name}: {mismatch}"
            );
        }
    }

    #[test]
    fn a_case_difference_between_the_sides_is_a_divergence() {
        let tier = Tier::all()[0];
        let upper_cased: Vec<String> = narrow_column_names()
            .iter()
            .map(|name| name.to_ascii_uppercase())
            .collect();
        let sides = [
            ExposedColumns {
                schema: "MYSQLVS_ADBC".to_string(),
                columns: upper_cased,
            },
            exposing("MYSQLVS_JDBC", &narrow_column_names()),
        ];

        let mismatch = column_set_mismatch(&tier, &sides).expect(
            "the benchmark double-quotes every column name, so `\"ID\"` and `\"id\"` are \
             different identifiers and a query built for one side would fail on the other",
        );
        assert!(mismatch.contains("MYSQLVS_ADBC"), "{mismatch}");
    }
}
