//! The benchmark's data tiers: what each one is, how it is seeded into each
//! [`Source`], and the gate that proves every compared adapter sees it
//! identically before anything is timed.
//!
//! Two halves, deliberately kept apart. Everything above [`ensure_seeded`] is
//! pure — it builds SQL text and computes byte widths, touching no source, no
//! Docker, and no live row count, which is what puts the exact statements this
//! benchmark issues under `make test` with no stack running. [`ensure_seeded`]
//! and [`assert_matching_columns`] are the only two functions that reach a live
//! stack, and each delegates every judgement it makes to a pure function
//! beside them, so the reasoning stays testable and only the I/O does not.
//!
//! # One logical tier, three dialects
//!
//! A [`Tier`] declares its columns once, as a [`ColumnType`] per column.
//! [`ddl_type`] and [`value_expression`] translate that declaration into each
//! source's own spelling with a plain `match` on ([`Source`], [`ColumnType`]),
//! so every source holds the same rows: contiguous ids `1..=target_rows`, and
//! every other column a deterministic function of the row's id (the id itself,
//! its decimal string, `2020-01-01` plus that many days, or
//! `2020-01-01 00:00:00` plus that many minutes). The sink checksum
//! `bench.rs` verifies therefore has the same closed form for every source.
//!
//! [`ColumnType::Char`] is declared `CHAR(n)` on MySQL only. PostgreSQL and
//! Trino blank-pad `CHAR`, so this repo's adapter leaves such a column
//! unmapped for both (see `vs-adapter/postgresql-type-mapping` and
//! `vs-adapter/trino-type-mapping`); declaring it `VARCHAR(n)` there keeps the
//! column — and its value — in the comparison.
//!
//! # Seeding per source
//!
//! - **MySQL** — a small deterministic base batch ([`seed_insert`]), grown by
//!   repeated `INSERT INTO t SELECT ... FROM t` steps ([`doubling_step`]) that
//!   double the row count, then trimmed to the exact target
//!   ([`seeding_plan`]'s trim statement). MySQL 8.4 has no row generator, so
//!   this keeps seeding to a handful of constant-size statements per tier
//!   instead of streaming millions of literal `INSERT` values from Rust.
//! - **PostgreSQL** — one `INSERT ... SELECT ... FROM generate_series(1, N)`.
//! - **Trino** — one `CREATE TABLE ... AS SELECT` into the `memory` catalog,
//!   over two cross-joined `UNNEST(sequence(...))` blocks, since Trino caps one
//!   `sequence` at 10,000 elements and its `memory` connector supports no
//!   `DELETE` a trim would need.
//!
//! [`seed_statements`] is the one entry point: the full ordered statement list
//! a rebuild runs for a (source, tier) pair.
//!
//! # Nominal byte width
//!
//! [`nominal_row_width`] sums one fixed byte estimate per column, for the
//! report's MiB/sec figure. It is deliberately not byte-perfect — assumptions
//! (documented on [`nominal_column_width`]) mirror common fixed-width
//! estimates for these types, nothing more precise — and it is the same for
//! every source, so MiB/sec figures stay comparable across sources.

use std::collections::BTreeSet;

use super::bench_stack::BenchStack;
use super::exasol::{cell, Exasol};
use super::mysql::MySql;
use super::postgres::Postgres;
use super::trino::{self, Trino};

/// The source database a benchmark run seeds and reads through its virtual
/// schemas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Mysql,
    Postgres,
    Trino,
}

impl Source {
    /// The lowercase name every rendered output and report filename uses —
    /// also the `docker-compose.yml` service that hosts the source.
    pub fn name(self) -> &'static str {
        match self {
            Source::Mysql => "mysql",
            Source::Postgres => "postgres",
            Source::Trino => "trino",
        }
    }

    /// The product name a human-facing report line uses.
    pub fn display_name(self) -> &'static str {
        match self {
            Source::Mysql => "MySQL",
            Source::Postgres => "PostgreSQL",
            Source::Trino => "Trino",
        }
    }
}

/// A live connection to one benchmark source, owning the harness client that
/// reaches it. Every method dispatches with a plain `match`.
pub enum SourceDb {
    Mysql(MySql),
    Postgres(Postgres),
    Trino(Trino),
}

impl SourceDb {
    pub fn source(&self) -> Source {
        match self {
            SourceDb::Mysql(_) => Source::Mysql,
            SourceDb::Postgres(_) => Source::Postgres,
            SourceDb::Trino(_) => Source::Trino,
        }
    }

    /// Runs one statement; a rejected statement panics.
    pub fn execute(&self, sql: &str) {
        match self {
            SourceDb::Mysql(db) => db.execute(sql),
            SourceDb::Postgres(db) => db.execute(sql),
            SourceDb::Trino(db) => db.execute(sql),
        }
    }

    /// Runs a one-row, one-column query and returns its rendered value; a
    /// rejected query panics.
    pub fn query_scalar(&self, sql: &str) -> String {
        match self {
            SourceDb::Mysql(db) => db.query_scalar(sql),
            SourceDb::Postgres(db) => db.query_scalar(sql),
            SourceDb::Trino(db) => db.query_scalar(sql),
        }
    }
}

/// A tier column's logical type, translated into each source's DDL by
/// [`ddl_type`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnType {
    Int,
    BigInt,
    Decimal(u32, u32),
    Varchar(u32),
    Char(u32),
    Date,
    Timestamp,
    Double,
}

/// One data tier the benchmark seeds and scans.
///
/// `name` is both the tier's identifier and the source table name it seeds
/// into. `columns` is ordered, and by convention its first entry is the
/// row-number-driving primary key column (named `id` in every tier below) —
/// every builder in this module keys off `columns[0]` for that role rather
/// than hard-coding the name `"id"`.
#[derive(Debug, Clone, Copy)]
pub struct Tier {
    pub name: &'static str,
    pub target_rows: u64,
    pub columns: &'static [(&'static str, ColumnType)],
}

/// The 8-column, ~100B/row shape shared by `narrow_small` and `narrow_large`
/// — matches the MySQL type set `adbc-vs-scan`'s coercion already proves it
/// maps (see `tests/common/mysql.rs`'s `orders` fixture): `INT`, `BIGINT`,
/// `DECIMAL`, `VARCHAR`, `CHAR`, `DATE`, `DATETIME`, `DOUBLE`.
const NARROW_COLUMNS: &[(&str, ColumnType)] = &[
    ("id", ColumnType::Int),
    ("big_id", ColumnType::BigInt),
    ("amount", ColumnType::Decimal(10, 2)),
    ("name", ColumnType::Varchar(40)),
    ("code", ColumnType::Char(8)),
    ("order_date", ColumnType::Date),
    ("created_at", ColumnType::Timestamp),
    ("price", ColumnType::Double),
];

/// The ~1KB/row shape for `wide`: the same proven type set as
/// [`NARROW_COLUMNS`], widened with more `VARCHAR(200)` and `DECIMAL` columns
/// rather than any new type.
const WIDE_COLUMNS: &[(&str, ColumnType)] = &[
    ("id", ColumnType::Int),
    ("big_id", ColumnType::BigInt),
    ("amount_a", ColumnType::Decimal(12, 4)),
    ("amount_b", ColumnType::Decimal(12, 4)),
    ("amount_c", ColumnType::Decimal(12, 4)),
    ("description_a", ColumnType::Varchar(200)),
    ("description_b", ColumnType::Varchar(200)),
    ("description_c", ColumnType::Varchar(200)),
    ("description_d", ColumnType::Varchar(200)),
    ("code", ColumnType::Char(8)),
    ("order_date", ColumnType::Date),
    ("created_at", ColumnType::Timestamp),
    ("price", ColumnType::Double),
];

/// The size of the deterministic base batch every MySQL tier is seeded from
/// before any doubling step runs.
const BASE_SEED_ROWS: u64 = 4;

/// The size of one Trino `sequence` block: Trino rejects a `sequence` longer
/// than 10,000 elements, so a tier's row numbers are built as
/// `block * TRINO_BLOCK_ROWS + offset` over two cross-joined sequences.
const TRINO_BLOCK_ROWS: u64 = 10_000;

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
    pub fn id_column(&self) -> &'static str {
        self.columns[0].0
    }
}

/// The table name `source` addresses `tier` by in every seeding statement.
///
/// Trino's CLI session names no catalog or schema, so its tables are
/// qualified by the `memory` catalog and schema `docker/trino/init.sql`
/// creates. MySQL and PostgreSQL sessions already default to the database
/// (and `public` schema) the virtual schemas read.
pub fn source_table(source: Source, tier: &Tier) -> String {
    match source {
        Source::Mysql | Source::Postgres => tier.name.to_string(),
        Source::Trino => format!("{}.{}.{}", trino::CATALOG, trino::SCHEMA, tier.name),
    }
}

/// `column_type`'s DDL spelling in `source`.
pub fn ddl_type(source: Source, column_type: ColumnType) -> String {
    use ColumnType::*;
    match (source, column_type) {
        (Source::Mysql, Int) => "INT".to_string(),
        (Source::Postgres | Source::Trino, Int) => "INTEGER".to_string(),
        (_, BigInt) => "BIGINT".to_string(),
        (Source::Postgres, Decimal(precision, scale)) => format!("NUMERIC({precision},{scale})"),
        (Source::Mysql | Source::Trino, Decimal(precision, scale)) => {
            format!("DECIMAL({precision},{scale})")
        }
        (_, Varchar(length)) => format!("VARCHAR({length})"),
        (Source::Mysql, Char(length)) => format!("CHAR({length})"),
        (Source::Postgres | Source::Trino, Char(length)) => format!("VARCHAR({length})"),
        (_, Date) => "DATE".to_string(),
        (Source::Mysql, Timestamp) => "DATETIME".to_string(),
        (Source::Postgres, Timestamp) => "TIMESTAMP".to_string(),
        (Source::Trino, Timestamp) => "TIMESTAMP(3)".to_string(),
        (Source::Mysql | Source::Trino, Double) => "DOUBLE".to_string(),
        (Source::Postgres, Double) => "DOUBLE PRECISION".to_string(),
    }
}

/// The full ordered statement list that rebuilds `tier` from nothing in
/// `source`: drop, create, fill, and (MySQL only) trim onto the exact target.
pub fn seed_statements(source: Source, tier: &Tier) -> Vec<String> {
    let mut statements = vec![drop_table_statement(source, tier)];
    match source {
        Source::Mysql => {
            let plan = seeding_plan(tier);
            statements.push(create_table_ddl(source, tier));
            statements.push(seed_insert(tier));
            statements.extend(
                plan.doubling_offsets
                    .into_iter()
                    .map(|offset| doubling_step(tier, offset)),
            );
            statements.extend(plan.trim_statement);
        }
        Source::Postgres => {
            statements.push(create_table_ddl(source, tier));
            statements.push(postgres_generate_insert(tier));
        }
        Source::Trino => statements.push(trino_generate_ctas(tier)),
    }
    statements
}

/// Builds the `CREATE TABLE IF NOT EXISTS` DDL for `tier` in `source`, with
/// its id column as the primary key. Trino tables are created by
/// [`trino_generate_ctas`] instead: its `memory` connector has no primary key.
pub fn create_table_ddl(source: Source, tier: &Tier) -> String {
    let column_defs: Vec<String> = tier
        .columns
        .iter()
        .map(|(name, column_type)| format!("{name} {} NOT NULL", ddl_type(source, *column_type)))
        .collect();
    format!(
        "CREATE TABLE IF NOT EXISTS {} ({}, PRIMARY KEY ({}))",
        source_table(source, tier),
        column_defs.join(", "),
        tier.id_column()
    )
}

/// The `SELECT` list computing every column of `tier` from `row_number_expr`
/// in `source`'s dialect.
fn select_list(source: Source, tier: &Tier, row_number_expr: &str) -> String {
    tier.columns
        .iter()
        .map(|(_, column_type)| value_expression(source, *column_type, row_number_expr))
        .collect::<Vec<_>>()
        .join(", ")
}

fn column_names(tier: &Tier) -> String {
    tier.columns
        .iter()
        .map(|(name, _)| *name)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Builds the MySQL base-seed `INSERT` statement: [`BASE_SEED_ROWS`]
/// deterministic rows, row `n`'s columns each derived from the literal row
/// number `n` by [`value_expression`].
pub fn seed_insert(tier: &Tier) -> String {
    let rows: Vec<String> = (1..=BASE_SEED_ROWS)
        .map(|row_number| {
            format!(
                "({})",
                select_list(Source::Mysql, tier, &row_number.to_string())
            )
        })
        .collect();
    format!(
        "INSERT INTO {} ({}) VALUES {}",
        tier.name,
        column_names(tier),
        rows.join(", ")
    )
}

/// Builds one MySQL doubling step: `INSERT INTO <table> SELECT ... FROM
/// <table>`, where every column is recomputed from `(id + offset)` by
/// [`value_expression`] — the same formula [`seed_insert`] applies to a
/// literal row number. `offset` must be the table's current row count, so
/// every new id lands strictly above every existing one and no doubling step
/// can mint a duplicate primary key.
pub fn doubling_step(tier: &Tier, offset: u64) -> String {
    let row_number_expr = format!("({} + {offset})", tier.id_column());
    format!(
        "INSERT INTO {} SELECT {} FROM {}",
        tier.name,
        select_list(Source::Mysql, tier, &row_number_expr),
        tier.name
    )
}

/// The PostgreSQL fill: every row `1..=target_rows` in one statement, each
/// column computed from `generate_series`'s `n`.
fn postgres_generate_insert(tier: &Tier) -> String {
    format!(
        "INSERT INTO {} ({}) SELECT {} FROM generate_series(1, {}) AS g(n)",
        source_table(Source::Postgres, tier),
        column_names(tier),
        select_list(Source::Postgres, tier, "n"),
        tier.target_rows
    )
}

/// The Trino create-and-fill: one CTAS whose row numbers `1..=target_rows`
/// come from `block * TRINO_BLOCK_ROWS + offset` over two cross-joined
/// `sequence`s, each within Trino's 10,000-element `sequence` limit for every
/// tier below 100,000,000 rows. Every column is `CAST` to its exact
/// [`ddl_type`], since a CTAS takes its column types from the expressions.
fn trino_generate_ctas(tier: &Tier) -> String {
    let last_block = tier
        .target_rows
        .div_ceil(TRINO_BLOCK_ROWS)
        .saturating_sub(1);
    let named_columns: Vec<String> = tier
        .columns
        .iter()
        .map(|(name, column_type)| {
            format!(
                "{} AS {name}",
                value_expression(Source::Trino, *column_type, "n")
            )
        })
        .collect();
    format!(
        "CREATE TABLE {} AS SELECT {} FROM (SELECT block_index * {TRINO_BLOCK_ROWS} + block_row AS n \
         FROM UNNEST(sequence(0, {last_block})) AS blocks(block_index) \
         CROSS JOIN UNNEST(sequence(1, {TRINO_BLOCK_ROWS})) AS block_rows(block_row)) AS g \
         WHERE n <= {}",
        source_table(Source::Trino, tier),
        named_columns.join(", "),
        tier.target_rows
    )
}

/// The ordered doubling steps and optional trim needed to take a MySQL `tier`
/// from [`BASE_SEED_ROWS`] to exactly `tier.target_rows`.
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

/// Builds the `DELETE` statement that trims `tier`'s MySQL table down to
/// exactly `tier.target_rows`, by deleting every row whose id exceeds the
/// target. Sound because every id [`doubling_step`] mints is offset above the
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
        .map(|(_, column_type)| nominal_column_width(*column_type))
        .sum()
}

/// A defensible, fixed byte estimate for one column type — not byte-perfect,
/// just consistent enough to make MiB/sec figures comparable across tiers,
/// sources, and re-runs:
///
/// - `Int` = 4, `BigInt` = 8: native fixed-width integers.
/// - `Decimal(p,s)` ≈ `p / 2 + 1`: a packed-decimal-style estimate (roughly
///   two digits per byte) plus one byte for sign, independent of scale.
/// - `Varchar(n)` = `n`, `Char(n)` = `n`: worst-case (`VARCHAR`) or exact
///   (`CHAR`) character count, one byte per character.
/// - `Date` = 3, `Timestamp` = 8, `Double` = 8: native fixed-width temporal
///   and floating-point encodings.
fn nominal_column_width(column_type: ColumnType) -> u32 {
    match column_type {
        ColumnType::Int => 4,
        ColumnType::BigInt | ColumnType::Timestamp | ColumnType::Double => 8,
        ColumnType::Date => 3,
        ColumnType::Decimal(precision, _) => precision / 2 + 1,
        ColumnType::Varchar(length) | ColumnType::Char(length) => length,
    }
}

/// Builds the SQL expression that computes one column's deterministic value
/// from `row_number_expr` in `source`'s dialect — a literal row index or an
/// `(id + offset)` expression for MySQL, the generator's `n` for PostgreSQL
/// and Trino. Every source computes the same logical value:
///
/// - `Int`/`BigInt`/`Decimal`/`Double`: the row number itself. MySQL's `CAST`
///   has no `INT` or `BIGINT` target type; `SIGNED` is its 64-bit signed
///   integer cast, which narrows on assignment into an `INT` column.
/// - `Varchar`/`Char`: the row number's decimal-string form, always well
///   within these tiers' declared widths at their target row counts.
/// - `Date`: `2020-01-01` plus the row number in days.
/// - `Timestamp`: `2020-01-01 00:00:00` plus the row number in minutes.
///
/// Trino casts every value to its exact [`ddl_type`], since its tables are
/// created by CTAS and take their column types from these expressions.
fn value_expression(source: Source, column_type: ColumnType, row_number_expr: &str) -> String {
    use ColumnType::*;
    let n = row_number_expr;
    match (source, column_type) {
        (Source::Mysql, Timestamp) => {
            format!("DATE_ADD('2020-01-01 00:00:00', INTERVAL {n} MINUTE)")
        }
        (Source::Mysql, Date) => format!("DATE_ADD('2020-01-01', INTERVAL {n} DAY)"),
        (Source::Mysql, Varchar(_) | Char(_)) => format!("CAST({n} AS CHAR)"),
        (Source::Mysql, Int | BigInt) => format!("CAST({n} AS SIGNED)"),
        (Source::Mysql, Double | Decimal(..)) => {
            format!("CAST({n} AS {})", ddl_type(source, column_type))
        }

        (Source::Postgres, Timestamp) => {
            format!("TIMESTAMP '2020-01-01 00:00:00' + {n} * INTERVAL '1 minute'")
        }
        (Source::Postgres, Date) => format!("DATE '2020-01-01' + CAST({n} AS INTEGER)"),
        (Source::Postgres, Varchar(_) | Char(_)) => format!("CAST({n} AS VARCHAR)"),
        (Source::Postgres, Int | BigInt | Decimal(..) | Double) => {
            format!("CAST({n} AS {})", ddl_type(source, column_type))
        }

        (Source::Trino, Timestamp) => format!(
            "CAST(date_add('minute', {n}, TIMESTAMP '2020-01-01 00:00:00') AS {})",
            ddl_type(source, column_type)
        ),
        (Source::Trino, Date) => format!("date_add('day', {n}, DATE '2020-01-01')"),
        (Source::Trino, Varchar(_) | Char(_)) => format!(
            "CAST(CAST({n} AS VARCHAR) AS {})",
            ddl_type(source, column_type)
        ),
        (Source::Trino, Int | BigInt | Decimal(..) | Double) => {
            format!("CAST({n} AS {})", ddl_type(source, column_type))
        }
    }
}

/// The statement that removes `tier`'s table before it is rebuilt, tolerating
/// a table that was never there.
fn drop_table_statement(source: Source, tier: &Tier) -> String {
    format!("DROP TABLE IF EXISTS {}", source_table(source, tier))
}

/// The one-value query counting whether `tier`'s table exists in `source`,
/// answered from the source's own `information_schema`.
fn table_exists_query(source: Source, tier: &Tier) -> String {
    let table = tier.name;
    match source {
        Source::Mysql => format!(
            "SELECT COUNT(*) FROM information_schema.tables \
             WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = '{table}'"
        ),
        Source::Postgres => format!(
            "SELECT COUNT(*) FROM information_schema.tables \
             WHERE table_schema = 'public' AND table_name = '{table}'"
        ),
        Source::Trino => format!(
            "SELECT count(*) FROM {}.information_schema.tables \
             WHERE table_schema = '{}' AND table_name = '{table}'",
            trino::CATALOG,
            trino::SCHEMA
        ),
    }
}

/// Brings `tier`'s source table to exactly `tier.target_rows` rows, and
/// proves it got there before returning.
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
pub fn ensure_seeded(db: &SourceDb, tier: &Tier) {
    let source = db.source();
    if seeded_rows(db, tier) == Some(tier.target_rows) {
        return;
    }

    for statement in seed_statements(source, tier) {
        db.execute(&statement);
    }

    let seeded = seeded_rows(db, tier).unwrap_or_else(|| {
        panic!(
            "seeding tier `{}` left no table `{}` in {}",
            tier.name,
            source_table(source, tier),
            source.display_name()
        )
    });
    assert_eq!(
        seeded,
        tier.target_rows,
        "seeding tier `{}` left {seeded} row(s) in {} table `{}`, not the {} it targets",
        tier.name,
        source.display_name(),
        source_table(source, tier),
        tier.target_rows
    );
}

/// The row count of `tier`'s table, or `None` when the table is not there.
///
/// Absence is answered from `information_schema` rather than from a failed
/// `COUNT(*)`, so a source that cannot answer at all fails the run instead of
/// being read as an empty tier that needs reseeding.
fn seeded_rows(db: &SourceDb, tier: &Tier) -> Option<u64> {
    let source = db.source();
    let table = source_table(source, tier);
    let exists = row_count_of(&db.query_scalar(&table_exists_query(source, tier)), &table) > 0;
    exists.then(|| {
        row_count_of(
            &db.query_scalar(&format!("SELECT COUNT(*) FROM {table}")),
            &table,
        )
    })
}

/// Reads the count a source rendered for `table`, failing the run on anything
/// that is not one.
fn row_count_of(rendered: &str, table: &str) -> u64 {
    rendered.parse().unwrap_or_else(|error| {
        panic!("the source answered a count over `{table}` with `{rendered}`, not a row count: {error}")
    })
}

/// Fails the run unless every virtual schema the benchmark compares exposes
/// exactly the columns `tier` declares.
///
/// This is the benchmark's fairness gate, and it belongs before a tier's first
/// timed sample rather than after it: an adapter that quietly omitted a column
/// it could not map would move less data for the same query and report the
/// difference as speed. Holding every side to the tier's own declaration
/// rather than only to each other also catches the case where both narrow
/// identically — still a fair race, but one whose MiB/sec figure would be
/// computed from bytes nothing transferred. An ADBC-only run (no JDBC
/// baseline) still runs the gate over its one schema, for the second reason.
///
/// Every schema is refreshed first. They cache their source's metadata at
/// creation, so a column set read without refreshing is a snapshot from before
/// this tier's table existed; republishing it is part of reading the live
/// answer, not a step a caller could sensibly skip.
///
/// Column names are compared case-sensitively. Every virtual schema must
/// expose the source identifiers in lower case, since every benchmark query
/// double-quotes them, so `"id"` and `"ID"` are different identifiers to
/// Exasol — a casing difference between the sides is one a query built for
/// either side could not survive against the other.
pub fn assert_matching_columns(stack: &BenchStack, tier: &Tier) {
    let sides: Vec<ExposedColumns> = stack
        .virtual_schemas()
        .into_iter()
        .map(|schema| {
            stack.exasol.execute(&refresh_statement(schema));
            ExposedColumns {
                schema: schema.to_owned(),
                columns: exposed_columns(&stack.exasol, schema, tier.name),
            }
        })
        .collect();

    if let Some(mismatch) = column_set_mismatch(stack.source.source(), tier, &sides) {
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
fn column_set_mismatch(source: Source, tier: &Tier, sides: &[ExposedColumns]) -> Option<String> {
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
        "tier `{}`: the virtual schemas do not all expose exactly the {} column(s) \
         declared for {} table `{}`, so timing a query over them would not measure the \
         declared work:\n{}",
        tier.name,
        declared.len(),
        source.display_name(),
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

    const ALL_SOURCES: [Source; 3] = [Source::Mysql, Source::Postgres, Source::Trino];

    fn narrow_small() -> Tier {
        Tier::all()[0]
    }

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
        assert_eq!(tiers[2].columns.len(), 13);
    }

    #[test]
    fn source_names_match_their_compose_services() {
        assert_eq!(Source::Mysql.name(), "mysql");
        assert_eq!(Source::Postgres.name(), "postgres");
        assert_eq!(Source::Trino.name(), "trino");
    }

    #[test]
    fn mysql_ddl_types_keep_the_original_spellings() {
        let spelled: Vec<String> = NARROW_COLUMNS
            .iter()
            .map(|(_, column_type)| ddl_type(Source::Mysql, *column_type))
            .collect();
        assert_eq!(
            spelled,
            [
                "INT",
                "BIGINT",
                "DECIMAL(10,2)",
                "VARCHAR(40)",
                "CHAR(8)",
                "DATE",
                "DATETIME",
                "DOUBLE"
            ]
        );
    }

    #[test]
    fn postgres_and_trino_declare_char_as_varchar_since_their_char_is_unmapped() {
        assert_eq!(
            ddl_type(Source::Postgres, ColumnType::Char(8)),
            "VARCHAR(8)"
        );
        assert_eq!(ddl_type(Source::Trino, ColumnType::Char(8)), "VARCHAR(8)");
    }

    #[test]
    fn every_source_declares_only_types_its_adbc_mapping_accepts() {
        let postgres: Vec<String> = WIDE_COLUMNS
            .iter()
            .map(|(_, column_type)| ddl_type(Source::Postgres, *column_type))
            .collect();
        for spelled in &postgres {
            assert!(
                [
                    "INTEGER",
                    "BIGINT",
                    "NUMERIC(",
                    "VARCHAR(",
                    "DATE",
                    "TIMESTAMP",
                    "DOUBLE PRECISION"
                ]
                .iter()
                .any(|prefix| spelled.starts_with(prefix)),
                "PostgreSQL type `{spelled}` is outside postgresql-type-mapping"
            );
            assert!(!spelled.starts_with("CHAR("), "{spelled}");
        }
        for (_, column_type) in WIDE_COLUMNS {
            let spelled = ddl_type(Source::Trino, *column_type);
            assert!(
                [
                    "INTEGER",
                    "BIGINT",
                    "DECIMAL(",
                    "VARCHAR(",
                    "DATE",
                    "TIMESTAMP(",
                    "DOUBLE"
                ]
                .iter()
                .any(|prefix| spelled.starts_with(prefix)),
                "Trino type `{spelled}` is outside trino-type-mapping"
            );
        }
    }

    #[test]
    fn create_table_ddl_declares_every_column_not_null_with_the_id_primary_key() {
        assert_eq!(
            create_table_ddl(Source::Mysql, &narrow_small()),
            "CREATE TABLE IF NOT EXISTS narrow_small (id INT NOT NULL, \
             big_id BIGINT NOT NULL, amount DECIMAL(10,2) NOT NULL, \
             name VARCHAR(40) NOT NULL, code CHAR(8) NOT NULL, \
             order_date DATE NOT NULL, created_at DATETIME NOT NULL, \
             price DOUBLE NOT NULL, PRIMARY KEY (id))"
        );
    }

    #[test]
    fn postgres_create_table_ddl_uses_postgres_spellings() {
        assert_eq!(
            create_table_ddl(Source::Postgres, &narrow_small()),
            "CREATE TABLE IF NOT EXISTS narrow_small (id INTEGER NOT NULL, \
             big_id BIGINT NOT NULL, amount NUMERIC(10,2) NOT NULL, \
             name VARCHAR(40) NOT NULL, code VARCHAR(8) NOT NULL, \
             order_date DATE NOT NULL, created_at TIMESTAMP NOT NULL, \
             price DOUBLE PRECISION NOT NULL, PRIMARY KEY (id))"
        );
    }

    #[test]
    fn seed_insert_names_every_column_and_seeds_base_seed_rows_row_tuples() {
        let sql = seed_insert(&narrow_small());
        assert!(sql.starts_with(
            "INSERT INTO narrow_small (id, big_id, amount, name, code, order_date, \
             created_at, price) VALUES "
        ));
        assert_eq!(sql.matches("), (").count() + 1, BASE_SEED_ROWS as usize);
    }

    #[test]
    fn seed_insert_derives_every_column_from_the_literal_row_number() {
        let sql = seed_insert(&narrow_small());
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
        let sql = doubling_step(&Tier::all()[2], 500);
        assert!(sql.starts_with("INSERT INTO wide SELECT "));
        assert!(sql.ends_with(" FROM wide"));
    }

    #[test]
    fn doubling_step_offsets_every_column_by_the_current_row_count() {
        let sql = doubling_step(&narrow_small(), 500);
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

    #[test]
    fn mysql_seed_statements_keep_the_doubling_shape() {
        let tier = Tier::all()[1];
        let plan = seeding_plan(&tier);
        let statements = seed_statements(Source::Mysql, &tier);
        assert_eq!(statements[0], "DROP TABLE IF EXISTS narrow_large");
        assert_eq!(statements[1], create_table_ddl(Source::Mysql, &tier));
        assert_eq!(statements[2], seed_insert(&tier));
        assert_eq!(statements.len(), 3 + plan.doubling_offsets.len() + 1);
        assert_eq!(
            statements.last().map(String::as_str),
            Some("DELETE FROM narrow_large WHERE id > 1000000")
        );
    }

    #[test]
    fn postgres_seed_statements_fill_from_generate_series_in_one_insert() {
        let statements = seed_statements(Source::Postgres, &narrow_small());
        assert_eq!(statements.len(), 3);
        assert_eq!(statements[0], "DROP TABLE IF EXISTS narrow_small");
        let fill = &statements[2];
        assert!(
            fill.starts_with(
                "INSERT INTO narrow_small (id, big_id, amount, name, code, order_date, \
                 created_at, price) SELECT CAST(n AS INTEGER), CAST(n AS BIGINT), \
                 CAST(n AS NUMERIC(10,2)), CAST(n AS VARCHAR), CAST(n AS VARCHAR), \
                 DATE '2020-01-01' + CAST(n AS INTEGER), \
                 TIMESTAMP '2020-01-01 00:00:00' + n * INTERVAL '1 minute', \
                 CAST(n AS DOUBLE PRECISION)"
            ),
            "{fill}"
        );
        assert!(
            fill.ends_with(" FROM generate_series(1, 100000) AS g(n)"),
            "{fill}"
        );
    }

    #[test]
    fn trino_seed_statements_drop_then_ctas_into_the_memory_catalog() {
        let statements = seed_statements(Source::Trino, &narrow_small());
        assert_eq!(
            statements,
            [
                "DROP TABLE IF EXISTS memory.adbc_vs.narrow_small".to_string(),
                "CREATE TABLE memory.adbc_vs.narrow_small AS SELECT \
                 CAST(n AS INTEGER) AS id, CAST(n AS BIGINT) AS big_id, \
                 CAST(n AS DECIMAL(10,2)) AS amount, \
                 CAST(CAST(n AS VARCHAR) AS VARCHAR(40)) AS name, \
                 CAST(CAST(n AS VARCHAR) AS VARCHAR(8)) AS code, \
                 date_add('day', n, DATE '2020-01-01') AS order_date, \
                 CAST(date_add('minute', n, TIMESTAMP '2020-01-01 00:00:00') AS TIMESTAMP(3)) \
                 AS created_at, CAST(n AS DOUBLE) AS price \
                 FROM (SELECT block_index * 10000 + block_row AS n \
                 FROM UNNEST(sequence(0, 9)) AS blocks(block_index) \
                 CROSS JOIN UNNEST(sequence(1, 10000)) AS block_rows(block_row)) AS g \
                 WHERE n <= 100000"
                    .to_string(),
            ]
        );
    }

    #[test]
    fn trino_ctas_blocks_cover_every_real_tier_exactly_once() {
        for tier in Tier::all() {
            let last_block = tier.target_rows.div_ceil(TRINO_BLOCK_ROWS) - 1;
            assert!(
                (last_block + 1) * TRINO_BLOCK_ROWS >= tier.target_rows,
                "tier {} generates too few row numbers",
                tier.name
            );
            assert!(
                last_block < TRINO_BLOCK_ROWS,
                "tier {} needs a block sequence over Trino's 10,000-element limit",
                tier.name
            );
            assert!(trino_generate_ctas(&tier).contains(&format!("sequence(0, {last_block})")));
        }
    }

    #[test]
    fn every_source_seeds_every_column_of_every_tier() {
        for source in ALL_SOURCES {
            for tier in Tier::all() {
                let expected: Vec<String> = tier
                    .columns
                    .iter()
                    .map(|(_, column_type)| value_expression(source, *column_type, "n"))
                    .collect();
                assert_eq!(select_list(source, &tier, "n"), expected.join(", "));
                assert_eq!(expected.len(), tier.columns.len());
            }
        }
    }

    #[test]
    fn table_exists_query_reads_each_sources_own_catalog() {
        let tier = narrow_small();
        assert!(table_exists_query(Source::Mysql, &tier).contains("TABLE_SCHEMA = DATABASE()"));
        assert!(table_exists_query(Source::Postgres, &tier).contains("table_schema = 'public'"));
        assert!(table_exists_query(Source::Trino, &tier)
            .contains("FROM memory.information_schema.tables WHERE table_schema = 'adbc_vs'"));
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
        let plan = seeding_plan(&Tier::all()[1]);
        assert_eq!(
            plan.trim_statement,
            Some("DELETE FROM narrow_large WHERE id > 1000000".to_string())
        );
    }

    #[test]
    fn nominal_row_width_sums_the_documented_fixed_estimate_per_column() {
        // id(4) + big_id(8) + amount DECIMAL(10,2)(10/2+1=6) + name VARCHAR(40)(40)
        // + code CHAR(8)(8) + order_date DATE(3) + created_at DATETIME(8) + price DOUBLE(8)
        assert_eq!(
            nominal_row_width(&narrow_small()),
            4 + 8 + 6 + 40 + 8 + 3 + 8 + 8
        );
        // id(4) + big_id(8) + 3×DECIMAL(12,4)(12/2+1=7 each) + 4×VARCHAR(200)(200 each)
        // + code CHAR(8)(8) + order_date DATE(3) + created_at DATETIME(8) + price DOUBLE(8)
        assert_eq!(
            nominal_row_width(&Tier::all()[2]),
            4 + 8 + 7 * 3 + 200 * 4 + 8 + 3 + 8 + 8
        );
    }

    #[test]
    fn narrow_row_width_lands_near_the_planned_hundred_bytes_per_row() {
        let width = nominal_row_width(&narrow_small());
        assert!(
            (50..=150).contains(&width),
            "narrow_small's nominal width {width} is far from the ~100B/row the plan expects"
        );
    }

    #[test]
    fn wide_row_width_lands_near_the_planned_kilobyte_per_row() {
        let width = nominal_row_width(&Tier::all()[2]);
        assert!(
            (500..=1500).contains(&width),
            "wide's nominal width {width} is far from the ~1KB/row the plan expects"
        );
    }

    #[test]
    fn drop_table_statement_names_the_tiers_table_and_tolerates_its_absence() {
        assert_eq!(
            drop_table_statement(Source::Mysql, &narrow_small()),
            "DROP TABLE IF EXISTS narrow_small"
        );
        assert_eq!(
            drop_table_statement(Source::Trino, &narrow_small()),
            "DROP TABLE IF EXISTS memory.adbc_vs.narrow_small"
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
    fn row_count_of_reads_the_number_the_source_rendered() {
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
        let names = narrow_column_names();
        let sides = [
            exposing("MYSQLVS_ADBC", &names),
            exposing("MYSQLVS_JDBC", &names),
        ];

        assert_eq!(
            column_set_mismatch(Source::Mysql, &narrow_small(), &sides),
            None
        );
    }

    #[test]
    fn no_mismatch_for_a_single_adbc_only_side_exposing_the_declared_columns() {
        let sides = [exposing("TRINOVS_ADBC", &narrow_column_names())];
        assert_eq!(
            column_set_mismatch(Source::Trino, &narrow_small(), &sides),
            None
        );
    }

    #[test]
    fn no_mismatch_when_a_side_exposes_the_declared_columns_in_another_order() {
        let mut reordered = narrow_column_names();
        reordered.reverse();
        let sides = [
            exposing("MYSQLVS_ADBC", &reordered),
            exposing("MYSQLVS_JDBC", &narrow_column_names()),
        ];

        assert_eq!(
            column_set_mismatch(Source::Mysql, &narrow_small(), &sides),
            None,
            "ordinal position is not part of the column set the benchmark compares"
        );
    }

    #[test]
    fn a_side_omitting_a_declared_column_is_named_with_the_column_it_dropped() {
        let narrowed: Vec<&str> = narrow_column_names()
            .into_iter()
            .filter(|name| *name != "code")
            .collect();
        let sides = [
            exposing("PGVS_ADBC", &narrowed),
            exposing("PGVS_JDBC", &narrow_column_names()),
        ];

        let mismatch = column_set_mismatch(Source::Postgres, &narrow_small(), &sides)
            .expect("a narrower side is not a fair comparison");
        assert!(
            mismatch.contains("PGVS_ADBC") && mismatch.contains("missing: code"),
            "the report must name the side and the column it dropped: {mismatch}"
        );
        assert!(
            mismatch.contains("narrow_small") && mismatch.contains("PostgreSQL"),
            "the report must name the tier and source it fails: {mismatch}"
        );
    }

    #[test]
    fn a_side_exposing_a_column_beyond_the_declaration_is_named_with_it() {
        let mut widened = narrow_column_names();
        widened.push("stowaway");
        let sides = [
            exposing("MYSQLVS_ADBC", &narrow_column_names()),
            exposing("MYSQLVS_JDBC", &widened),
        ];

        let mismatch = column_set_mismatch(Source::Mysql, &narrow_small(), &sides)
            .expect("a wider side is not a fair comparison");
        assert!(
            mismatch.contains("MYSQLVS_JDBC") && mismatch.contains("unexpected: stowaway"),
            "the report must name the side and the column it added: {mismatch}"
        );
    }

    #[test]
    fn a_report_accounts_for_every_side_even_when_only_one_diverges() {
        let sides = [
            exposing("MYSQLVS_ADBC", &[]),
            exposing("MYSQLVS_JDBC", &narrow_column_names()),
        ];

        let mismatch = column_set_mismatch(Source::Mysql, &narrow_small(), &sides)
            .expect("an empty side is a divergence");
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

        let mismatch = column_set_mismatch(Source::Mysql, &narrow_small(), &sides).expect(
            "the benchmark double-quotes every column name, so `\"ID\"` and `\"id\"` are \
             different identifiers and a query built for one side would fail on the other",
        );
        assert!(mismatch.contains("MYSQLVS_ADBC"), "{mismatch}");
    }
}
