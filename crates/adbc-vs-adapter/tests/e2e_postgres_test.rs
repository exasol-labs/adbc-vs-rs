//! Drives adbc-vs end to end against the Docker Compose stack over PostgreSQL:
//! an Exasol virtual schema over the `postgres` container, with the ADBC driver
//! fetched from the public registry into BucketFS on the way, beside a second
//! virtual schema over MySQL that the two-driver claims need.
//!
//! There is no skip path. A stack that is down, a driver that will not install,
//! or a query that returns the wrong rows fails the run, because a tier that
//! quietly skips itself proves nothing.
//!
//! Every delegated fragment is verified twice, which is the pattern this tier
//! exists for. `EXPLAIN VIRTUAL` proves the fragment reached the statement the
//! adapter generated for PostgreSQL, and a row comparison against
//! [`common::postgres_mirror`] — a local Exasol table holding byte-identical
//! values — proves it produced Exasol's own answer. Either alone is blind: a
//! silently dropped fragment still returns rows, and a right answer says
//! nothing about where it was computed.
//!
//! The dialect renders no collation, so a delegated character comparison runs
//! under the collation PostgreSQL applies to the column. PostgreSQL's default
//! `en_US.utf8` locale orders `B`, `a`, `C` differently from Exasol's
//! code-point order, which is why two tables carry the same rows: `items`
//! keeps the default and its tests assert PostgreSQL's own answer, while
//! `items_c` declares `COLLATE "C"` on every character column and its tests
//! compare with the mirror, which holds Exasol's answer.
//!
//! Every test shares one stack, two virtual schemas, and the drivers installed
//! into one BucketFS, which is why this binary runs with `--test-threads=1`.
#![cfg(feature = "exasol-e2e")]

mod common;

use adbc_vs_adapter::pushdown::SCAN_SCRIPT_NAME;
use adbc_vs_adapter::scan_spec;
use adbc_vs_scan::driver::Driver;
use common::exasol::cell;
use common::postgres_mirror;
use common::postgres_stack::{self as stack, POSTGRES_VIRTUAL_SCHEMA as VIRTUAL_SCHEMA};
use common::{postgres, schema};

/// How many rows `docker/postgres/init.sql` seeds into `items`.
const ITEM_ROWS: usize = 6;

/// How many distinct `category` values those rows hold under Exasol's
/// comparison rules: `widget`, `Widget`, `cafe`, and `café`. PostgreSQL's own
/// equality is deterministic and agrees, which is why the dialect renders no
/// collation on a grouping key.
const CATEGORY_GROUPS: usize = 4;

/// The `public`-qualified `items` table, as the dialect writes it. `public` is
/// the namespace `adapterNotes` records for a PostgreSQL source, never the
/// database the connection address names.
const QUALIFIED_ITEMS: &str = "\"public\".\"items\"";

/// The table holding the rows under the database default collation, and the
/// table holding the same rows with every character column declared
/// `COLLATE "C"`. The mirror's `items` is the answer both are compared with
/// where the collation agrees with Exasol's.
const ITEMS: &str = "items";
const ITEMS_C: &str = "items_c";

/// The sum and the mean of the seeded `id` values, 1 through [`ITEM_ROWS`],
/// worked out by hand: what Exasol has to answer for the two aggregates this
/// release delegates to no source.
const SEEDED_ID_TOTAL: &str = "21";
const SEEDED_ID_MEAN: f64 = 3.5;

/// How far a returned mean may sit from [`SEEDED_ID_MEAN`]. Exasol declares
/// `AVG` `DOUBLE` for every numeric operand, so the value arrives as a
/// floating-point number and is compared as one.
const MEAN_TOLERANCE: f64 = 1e-9;

// --- the mirror is a valid oracle -------------------------------------------

/// Every differential test below rests on the mirror answering under Exasol's
/// own rules rather than PostgreSQL's, so this test establishes that before any
/// comparison relies on it. Without it, a delegated query and a mirror that
/// both ordered by locale weight would agree and prove nothing.
#[test]
fn mirror_answers_under_exasols_own_comparison_and_ordering_rules() {
    let stack = stack::shared();

    let rows = stack.exasol.query_rows(&format!(
        "SELECT COUNT(*) AS N, COUNT(DISTINCT \"category\") AS CATEGORIES, \
         MIN(\"sort_key\") AS MINKEY, MAX(\"sort_key\") AS MAXKEY, \
         MIN(\"tag\") AS MINTAG, MAX(\"tag\") AS MAXTAG FROM {}.\"items\"",
        postgres_mirror::SCHEMA
    ));
    let answers = &rows[0];

    assert_eq!(cell(answers, "N"), Some(ITEM_ROWS.to_string()));
    assert_eq!(
        cell(answers, "CATEGORIES"),
        Some(CATEGORY_GROUPS.to_string()),
        "Exasol compares character values by exact value, so widget, Widget, cafe, and café are \
         four distinct values"
    );
    assert_eq!(
        cell(answers, "MINKEY"),
        Some("B".to_string()),
        "Exasol orders by code point, so 'B' precedes 'C' precedes 'a'. PostgreSQL's own \
         en_US.utf8 locale answers 'a' here, the divergence `items_c` closes at the source"
    );
    assert_eq!(
        cell(answers, "MAXKEY"),
        Some("a".to_string()),
        "code-point order puts 'a' last, where the source's locale collation puts 'C'"
    );
    assert_eq!(
        cell(answers, "MINTAG"),
        Some("ab".to_string()),
        "Exasol pads nothing, so MIN over 'ab ', 'ab', 'ab!' is 'ab'"
    );
    assert_eq!(cell(answers, "MAXTAG"), Some("ab!".to_string()));
}

/// A differential comparison is only meaningful while both sides declare the
/// same columns under the same Exasol types: a mirror declaring a wider
/// `VARCHAR` would change an aggregate's declared type for a reason that is not
/// a pushdown defect.
#[test]
fn mirror_declares_the_same_columns_and_types_as_the_virtual_table() {
    for table in ["items", "item_notes"] {
        let mirrored = declared_columns(postgres_mirror::SCHEMA, table);
        let virtual_table = declared_columns(VIRTUAL_SCHEMA, table);

        assert!(
            !mirrored.is_empty(),
            "the mirror must declare table {table}"
        );
        assert_eq!(
            mirrored, virtual_table,
            "the mirror of {table} must declare the same column names in the same order with the \
             same Exasol types as the virtual table, or a differential comparison compares two \
             different things"
        );
    }

    assert_eq!(
        declared_columns(postgres_mirror::SCHEMA, ITEMS),
        declared_columns(VIRTUAL_SCHEMA, ITEMS_C),
        "`items_c` is compared with the mirror's `items`, so the two must declare the same column \
         names in the same order with the same Exasol types"
    );
}

// --- projection, filter, aggregation, join ----------------------------------

#[test]
fn projection_reaches_postgresql_and_matches_the_mirror() {
    let query = |schema: &str, table: &str| {
        format!("SELECT \"id\", \"category\" FROM {schema}.\"{table}\" ORDER BY \"id\"")
    };

    assert_eq!(
        generated_statement(&query(VIRTUAL_SCHEMA, ITEMS)),
        format!("SELECT \"id\", \"category\" FROM {QUALIFIED_ITEMS}"),
        "the statement sent to PostgreSQL must project the two named columns and no other, from \
         the `public`-qualified table"
    );

    let rows = assert_matches_mirror(ITEMS, query);
    assert_eq!(rows.len(), ITEM_ROWS);
    assert_eq!(rows[0].len(), 2, "only the two named columns come back");
    assert_eq!(cell(&rows[0], "category"), Some("widget".to_string()));
}

/// Two filters, because the two answer different questions. The first proves
/// that equality, `LIKE`, and `BETWEEN` over numbers reach PostgreSQL and select
/// the rows Exasol selects. The second runs over `items_c`, whose character
/// columns declare `COLLATE "C"`, and proves that an ordering comparison and a
/// character `BETWEEN` reach PostgreSQL bare and still select the rows Exasol
/// selects: `sort_key < 'C'` returns two rows under code-point order, and
/// `sort_key BETWEEN 'C' AND 'a'` returns four where the `en_US.utf8` locale
/// would return none.
#[test]
fn filter_reaches_postgresql_and_matches_the_mirror() {
    let query = |schema: &str, table: &str| {
        format!(
            "SELECT \"id\" FROM {schema}.\"{table}\" WHERE (\"category\" = 'Widget' OR \
             \"tag\" LIKE 'ab!%') AND \"id\" BETWEEN 1 AND 4 ORDER BY \"id\""
        )
    };

    let statement = generated_statement(&query(VIRTUAL_SCHEMA, ITEMS_C));
    for fragment in ["WHERE", " OR ", " AND ", "LIKE", "BETWEEN"] {
        assert!(
            statement.contains(fragment),
            "the generated statement must carry {fragment}: {statement}"
        );
    }
    assert!(
        !statement.contains("COLLATE"),
        "the dialect renders no collation: {statement}"
    );

    let rows = assert_matches_mirror(ITEMS_C, query);
    assert_eq!(
        column_values(&rows, "id"),
        vec!["2".to_string(), "3".to_string(), "4".to_string()],
        "row 2 is a Widget, row 3 carries tag 'ab!', row 4 is both, and row 1's tag 'ab ' matches \
         neither branch: {rows:?}"
    );

    let ordering_query = |schema: &str, table: &str| {
        format!("SELECT \"id\" FROM {schema}.\"{table}\" WHERE \"sort_key\" < 'C' ORDER BY \"id\"")
    };

    let ordering_statement = generated_statement(&ordering_query(VIRTUAL_SCHEMA, ITEMS_C));
    assert!(
        ordering_statement.contains("(\"sort_key\" < E'C')")
            && !ordering_statement.contains("COLLATE"),
        "the row comparison below only tells a bare comparison from a dropped one while the \
         predicate actually reached PostgreSQL: a filter Exasol computed itself would return the \
         same rows: {ordering_statement}"
    );

    let ordering_rows = assert_matches_mirror(ITEMS_C, ordering_query);
    assert_eq!(
        column_values(&ordering_rows, "id"),
        vec!["1".to_string(), "4".to_string()],
        "only the two 'B' rows precede 'C' in code-point order, which `items_c`'s own collation \
         supplies: {ordering_rows:?}"
    );

    let between_query = |schema: &str, table: &str| {
        format!(
            "SELECT \"id\" FROM {schema}.\"{table}\" WHERE \"sort_key\" BETWEEN 'C' AND 'a' \
             ORDER BY \"id\""
        )
    };

    let between_statement = generated_statement(&between_query(VIRTUAL_SCHEMA, ITEMS_C));
    assert!(
        between_statement.contains("BETWEEN E'C' AND E'a'")
            && !between_statement.contains("COLLATE"),
        "the character BETWEEN must reach PostgreSQL bare: {between_statement}"
    );

    let between_rows = assert_matches_mirror(ITEMS_C, between_query);
    assert_eq!(
        column_values(&between_rows, "id"),
        vec![
            "2".to_string(),
            "3".to_string(),
            "5".to_string(),
            "6".to_string()
        ],
        "'C' and 'a' bound the range in code-point order: {between_rows:?}"
    );
}

/// Runs over `items_c`, whose character columns declare `COLLATE "C"`: the
/// delegated `MIN`, `MAX`, `GROUP BY`, and `COUNT(DISTINCT)` reach PostgreSQL
/// bare and return Exasol's code-point answers because the column's own
/// collation supplies them.
#[test]
fn aggregation_reaches_postgresql_and_matches_the_mirror() {
    let single_group_query = |schema: &str, table: &str| {
        format!(
            "SELECT COUNT(*) AS N, MIN(\"sort_key\") AS MINKEY, MAX(\"sort_key\") AS MAXKEY, \
             MIN(\"tag\") AS MINTAG, MAX(\"tag\") AS MAXTAG FROM {schema}.\"{table}\""
        )
    };

    let statement = generated_statement(&single_group_query(VIRTUAL_SCHEMA, ITEMS_C));
    for fragment in ["COUNT(*)", "MIN(", "MAX("] {
        assert!(
            statement.contains(fragment),
            "the generated statement must carry {fragment}: {statement}"
        );
    }
    assert!(
        !statement.contains("GROUP BY") && !statement.contains("COLLATE"),
        "a single-group aggregation must render no GROUP BY clause and no collation: {statement}"
    );

    let rows = assert_matches_mirror(ITEMS_C, single_group_query);
    let answers = &rows[0];
    assert_eq!(cell(answers, "N"), Some(ITEM_ROWS.to_string()));
    assert_eq!(
        cell(answers, "MINKEY"),
        Some("B".to_string()),
        "the delegated MIN over a `COLLATE \"C\"` column must return Exasol's code-point \
         minimum: {answers:?}"
    );
    assert_eq!(cell(answers, "MAXKEY"), Some("a".to_string()));
    assert_eq!(
        cell(answers, "MINTAG"),
        Some("ab".to_string()),
        "the delegated MIN must not pad the trailing-space value: {answers:?}"
    );
    assert_eq!(cell(answers, "MAXTAG"), Some("ab!".to_string()));

    let grouped_query = |schema: &str, table: &str| {
        format!(
            "SELECT \"category\" AS CATEGORY, COUNT(*) AS N FROM {schema}.\"{table}\" \
             GROUP BY \"category\" ORDER BY \"category\""
        )
    };

    let grouped_statement = generated_statement(&grouped_query(VIRTUAL_SCHEMA, ITEMS_C));
    assert!(
        grouped_statement.contains("GROUP BY") && grouped_statement.contains("COUNT(*)"),
        "a grouped aggregation must render both COUNT and GROUP BY: {grouped_statement}"
    );

    let grouped_rows = assert_matches_mirror(ITEMS_C, grouped_query);
    assert_eq!(grouped_rows.len(), CATEGORY_GROUPS);

    let distinct_query = |schema: &str, table: &str| {
        format!("SELECT COUNT(DISTINCT \"sort_key\") AS N FROM {schema}.\"{table}\"")
    };

    let distinct_statement = generated_statement(&distinct_query(VIRTUAL_SCHEMA, ITEMS_C));
    assert!(
        distinct_statement.contains("COUNT(DISTINCT \"sort_key\")")
            && !distinct_statement.contains("COLLATE"),
        "the distinct count must reach PostgreSQL bare: {distinct_statement}"
    );

    let distinct_rows = assert_matches_mirror(ITEMS_C, distinct_query);
    assert_eq!(
        cell(&distinct_rows[0], "N"),
        Some("3".to_string()),
        "'B', 'a', and 'C' are three distinct values: {distinct_rows:?}"
    );
}

#[test]
fn join_reaches_postgresql_and_matches_the_mirror() {
    let query = |schema: &str, table: &str| {
        format!(
            "SELECT i.\"id\" AS ID, n.\"note\" AS NOTE FROM {schema}.\"{table}\" i INNER JOIN \
             {schema}.\"item_notes\" n ON i.\"id\" = n.\"item_id\" ORDER BY i.\"id\""
        )
    };

    let statement = generated_statement(&query(VIRTUAL_SCHEMA, ITEMS));
    assert!(
        statement.contains("INNER JOIN"),
        "the join must reach PostgreSQL as one statement rather than being withdrawn into two \
         scans: {statement}"
    );

    let rows = assert_matches_mirror(ITEMS, query);
    assert_eq!(
        rows.len(),
        5,
        "the one items row with no matching note and the one note matching no item are both \
         excluded: {rows:?}"
    );
}

/// `FULL OUTER JOIN` is the one capability the PostgreSQL dialect advertises
/// and the MySQL one does not, so this is the test that would fail if the
/// advertised set or the rendering of the keyword regressed. Both unmatched
/// rows have to come back, because a join silently rendered as an inner one
/// returns the five matched rows and looks correct until they are counted.
#[test]
fn full_outer_join_is_delegated_and_returns_both_unmatched_rows() {
    let query = |schema: &str, table: &str| {
        format!(
            "SELECT i.\"id\" AS ID, n.\"note\" AS NOTE FROM {schema}.\"{table}\" i FULL OUTER JOIN \
             {schema}.\"item_notes\" n ON i.\"id\" = n.\"item_id\" ORDER BY i.\"id\", n.\"note\""
        )
    };

    let statement = generated_statement(&query(VIRTUAL_SCHEMA, ITEMS));
    assert!(
        statement.contains("FULL OUTER JOIN"),
        "the full outer join must be delegated whole: {statement}"
    );
    assert!(
        !statement.contains("COLLATE"),
        "the dialect renders no collation on a join condition: {statement}"
    );

    let rows = assert_matches_mirror(ITEMS, query);
    assert_eq!(
        rows.len(),
        7,
        "five matched rows plus the unmatched row of each side: {rows:?}"
    );
    let unmatched_item = rows
        .iter()
        .find(|row| cell(row, "ID") == Some("1".to_string()))
        .expect("item 1 carries no note and must still come back");
    assert_eq!(
        cell(unmatched_item, "NOTE"),
        None,
        "the unmatched items row must return a null note, even though PostgreSQL declares that \
         column NOT NULL: {unmatched_item:?}"
    );
    let orphaned_note = rows
        .iter()
        .find(|row| cell(row, "NOTE") == Some("orphan".to_string()))
        .expect("the note matching no item must still come back");
    assert_eq!(
        cell(orphaned_note, "ID"),
        None,
        "the unmatched item_notes row must return null item columns: {orphaned_note:?}"
    );
}

// --- collation passthrough --------------------------------------------------

/// Over `items`, whose character columns take the database default
/// `en_US.utf8`, a delegated ordering comparison, `MIN`, and `MAX` return
/// PostgreSQL's own answer, because the dialect renders no collation and the
/// column's collation decides. The values are `B`, `a`, `C` in the `sort_key`
/// column: the locale orders them `a`, `B`, `C`, where Exasol's code-point
/// order puts `B` first and `a` last.
///
/// `EXPLAIN VIRTUAL` proves the fragments reached PostgreSQL bare, and the
/// rows prove the answer is the source's. The answer is asserted directly,
/// not compared with the mirror, because the mirror holds Exasol's answer and
/// the two are expected to differ.
#[test]
fn ordering_and_min_max_follow_the_columns_own_collation() {
    let comparison = format!(
        "SELECT \"id\" FROM {VIRTUAL_SCHEMA}.\"{ITEMS}\" WHERE \"sort_key\" < 'B' ORDER BY \"id\""
    );
    let comparison_statement = generated_statement(&comparison);
    assert!(
        comparison_statement.contains("(\"sort_key\" < E'B')")
            && !comparison_statement.contains("COLLATE"),
        "the ordering comparison must reach PostgreSQL bare: {comparison_statement}"
    );
    let rows = stack::shared().exasol.query_rows(&comparison);
    assert_eq!(
        column_values(&rows, "id"),
        vec!["2".to_string(), "5".to_string()],
        "the en_US.utf8 locale orders 'a' before 'B', so the two rows holding 'a' come back, \
         where Exasol's code-point order returns none: {rows:?}"
    );

    let aggregates = format!(
        "SELECT MIN(\"sort_key\") AS LO, MAX(\"sort_key\") AS HI FROM {VIRTUAL_SCHEMA}.\"{ITEMS}\""
    );
    assert_eq!(
        generated_statement(&aggregates),
        format!("SELECT MIN(\"sort_key\"), MAX(\"sort_key\") FROM {QUALIFIED_ITEMS}"),
        "MIN and MAX must reach PostgreSQL bare"
    );
    let rows = stack::shared().exasol.query_rows(&aggregates);
    assert_eq!(cell(&rows[0], "LO"), Some("a".to_string()), "{rows:?}");
    assert_eq!(cell(&rows[0], "HI"), Some("C".to_string()), "{rows:?}");
}

/// The whole rule, read off the statements the deployed adapter actually
/// generated: no operand in any position carries a collation. Absence is
/// asserted as the absence of the word anywhere in the statement, not of one
/// spelling, over `items` and over `items_c` alike, since the declared
/// collation of a column must not change what the dialect renders.
#[test]
fn no_generated_statement_carries_a_collation_keyword() {
    for table in [ITEMS, ITEMS_C] {
        for query in [
            format!("SELECT \"id\" FROM {VIRTUAL_SCHEMA}.\"{table}\" WHERE \"sort_key\" < 'C'"),
            format!("SELECT \"id\" FROM {VIRTUAL_SCHEMA}.\"{table}\" WHERE 'C' < \"sort_key\""),
            format!(
                "SELECT \"id\" FROM {VIRTUAL_SCHEMA}.\"{table}\" WHERE \"sort_key\" BETWEEN 'B' AND 'C'"
            ),
            format!("SELECT \"id\" FROM {VIRTUAL_SCHEMA}.\"{table}\" WHERE \"category\" = 'Widget'"),
            format!("SELECT \"id\" FROM {VIRTUAL_SCHEMA}.\"{table}\" WHERE \"tag\" LIKE 'ab%'"),
            format!(
                "SELECT \"id\" FROM {VIRTUAL_SCHEMA}.\"{table}\" WHERE \"category\" IN ('cafe', 'Widget')"
            ),
            format!(
                "SELECT \"category\", COUNT(*) FROM {VIRTUAL_SCHEMA}.\"{table}\" GROUP BY \"category\""
            ),
            format!("SELECT COUNT(DISTINCT \"category\") FROM {VIRTUAL_SCHEMA}.\"{table}\""),
            format!(
                "SELECT MIN(\"sort_key\"), MAX(\"tag\") FROM {VIRTUAL_SCHEMA}.\"{table}\""
            ),
            format!(
                "SELECT i.\"id\" FROM {VIRTUAL_SCHEMA}.\"{table}\" i INNER JOIN \
                 {VIRTUAL_SCHEMA}.\"item_notes\" n ON i.\"id\" = n.\"item_id\" \
                 WHERE i.\"sort_key\" < 'C'"
            ),
        ] {
            for statement in generated_statements(&query) {
                assert!(
                    !statement.contains("COLLATE"),
                    "no statement may carry a collation keyword, whatever the column declares; \
                     `{query}` generated: {statement}"
                );
            }
        }
    }
}

/// `"sort_key" || ''` is an expression the dialect advertises no capability
/// for, so Exasol keeps the comparison and the `MIN` and evaluates them
/// itself over the rows PostgreSQL returns, under its own code-point order.
/// That is the remedy for a column whose source collation is not what the
/// user wants, and it needs no change at the source.
///
/// `EXPLAIN VIRTUAL` proves the fragments stayed in Exasol, and the rows
/// prove the answer is Exasol's: no row holds `a` for the comparison, and
/// `MIN` is `B`, where the same queries over the bare column return PostgreSQL's
/// locale answer.
#[test]
fn an_ordering_comparison_over_a_wrapped_operand_is_evaluated_by_exasol() {
    let comparison = |schema: &str, table: &str| {
        format!(
            "SELECT \"id\", \"sort_key\" FROM {schema}.\"{table}\" \
             WHERE \"sort_key\" || '' < 'B' ORDER BY \"id\""
        )
    };
    let comparison_statement = generated_statement(&comparison(VIRTUAL_SCHEMA, ITEMS));
    assert!(
        !comparison_statement.contains('<') && !comparison_statement.contains("WHERE"),
        "the comparison over the wrapped operand must not reach PostgreSQL: \
         {comparison_statement}"
    );
    let rows = assert_matches_mirror(ITEMS, comparison);
    assert!(
        column_values(&rows, "sort_key")
            .iter()
            .all(|value| value != "a"),
        "Exasol's code-point order puts no 'a' before 'B': {rows:?}"
    );

    let minimum = |schema: &str, table: &str| {
        format!("SELECT MIN(\"sort_key\" || '') AS LO FROM {schema}.\"{table}\"")
    };
    let minimum_statement = generated_statement(&minimum(VIRTUAL_SCHEMA, ITEMS));
    assert!(
        !minimum_statement.contains("MIN("),
        "the MIN over the wrapped operand must not reach PostgreSQL: {minimum_statement}"
    );
    let rows = assert_matches_mirror(ITEMS, minimum);
    assert_eq!(cell(&rows[0], "LO"), Some("B".to_string()), "{rows:?}");
}

/// The answer the bare grouping key and the bare `DISTINCT` operand have to
/// produce: the mirror's own group count. Rendering nothing is only correct
/// while PostgreSQL's uncollated grouping agrees with Exasol's, and this is
/// what measures that agreement against the deployed stack rather than assuming
/// it.
#[test]
fn uncollated_grouping_and_distinct_return_the_mirrors_group_count() {
    let grouped_query = |schema: &str, table: &str| {
        format!(
            "SELECT \"category\" AS CATEGORY, COUNT(*) AS N FROM {schema}.\"{table}\" \
             GROUP BY \"category\" ORDER BY \"category\""
        )
    };

    let grouped_statement = generated_statement(&grouped_query(VIRTUAL_SCHEMA, ITEMS));
    assert!(
        grouped_statement.contains("GROUP BY \"category\"")
            && !grouped_statement.contains("COLLATE"),
        "the group count below measures PostgreSQL's own uncollated grouping only while the \
         GROUP BY reached PostgreSQL bare; one Exasol computed itself would agree with the \
         mirror whatever the dialect rendered: {grouped_statement}"
    );

    let rows = assert_matches_mirror(ITEMS, grouped_query);
    assert_eq!(
        rows.len(),
        CATEGORY_GROUPS,
        "widget, Widget, cafe, and café must stay four groups: a collation folding case or \
         accent returns fewer, and Exasol would then disagree: {rows:?}"
    );
    for (category, expected) in [
        ("widget", "2"),
        ("Widget", "2"),
        ("cafe", "1"),
        ("café", "1"),
    ] {
        let count = rows
            .iter()
            .find(|row| cell(row, "CATEGORY") == Some(category.to_string()))
            .and_then(|row| cell(row, "N"));
        assert_eq!(
            count,
            Some(expected.to_string()),
            "group {category} must hold {expected} row(s), which is what a group folded together \
             with its case or accent variant would not: {rows:?}"
        );
    }

    let distinct_query = |schema: &str, table: &str| {
        format!("SELECT COUNT(DISTINCT \"category\") AS N FROM {schema}.\"{table}\"")
    };

    let distinct_statement = generated_statement(&distinct_query(VIRTUAL_SCHEMA, ITEMS));
    assert!(
        distinct_statement.contains("COUNT(DISTINCT \"category\")")
            && !distinct_statement.contains("COLLATE"),
        "the distinct count below measures PostgreSQL's own reduction only while the DISTINCT \
         reached it, and its operand must render bare: {distinct_statement}"
    );

    let distinct_rows = assert_matches_mirror(ITEMS, distinct_query);
    assert_eq!(
        cell(&distinct_rows[0], "N"),
        Some(CATEGORY_GROUPS.to_string())
    );
}

/// A select-list predicate reading the grouped character column is delegated
/// whole. The grouping key and the predicate's operand render as the same bare
/// column, which is what satisfies PostgreSQL's full-group-by rule; over
/// `items_c` the predicate's answer is Exasol's.
///
/// A failure here reporting PostgreSQL's own "must appear in the GROUP BY
/// clause" is not a fixture problem: it means the dialect rendered the grouping
/// key and its select-list occurrence as two different expressions.
#[test]
fn select_list_predicate_over_the_grouped_character_column_delegates_and_matches_the_mirror() {
    let query = |schema: &str, table: &str| {
        format!(
            "SELECT \"category\" AS CATEGORY, \"category\" < 'm' AS UNDER_M FROM \
             {schema}.\"{table}\" GROUP BY \"category\" ORDER BY \"category\""
        )
    };

    let statement = generated_statement(&query(VIRTUAL_SCHEMA, ITEMS_C));
    assert!(
        statement.contains("GROUP BY \"category\"")
            && statement.contains("(\"category\" < E'm')")
            && !statement.contains("COLLATE"),
        "the grouping key and the select-list predicate must render bare: {statement}"
    );

    let rows = assert_matches_mirror(ITEMS_C, query);
    assert_eq!(rows.len(), CATEGORY_GROUPS);
    let under_m = |category: &str| {
        rows.iter()
            .find(|row| cell(row, "CATEGORY") == Some(category.to_string()))
            .and_then(|row| cell(row, "UNDER_M"))
    };
    assert_eq!(under_m("Widget"), Some("true".to_string()), "{rows:?}");
    assert_eq!(under_m("cafe"), Some("true".to_string()), "{rows:?}");
    assert_eq!(under_m("café"), Some("true".to_string()), "{rows:?}");
    assert_eq!(
        under_m("widget"),
        Some("false".to_string()),
        "'w' follows 'm' in code-point order: {rows:?}"
    );
}

// --- what is deliberately not delegated -------------------------------------

/// `postgresql.toml` withholds `FN_AGG_AVG`, `FN_AGG_AVG_DISTINCT`,
/// `FN_AGG_SUM`, and `FN_AGG_SUM_DISTINCT` whole — a framework decision rather
/// than a PostgreSQL one — so Exasol delegates neither and computes both itself
/// over whatever the pushed-down statement projects. That withholding costs no
/// answer, which is what this measures.
///
/// The answers are pinned against values worked out by hand rather than against
/// the mirror alone. Exasol computes a withheld aggregate on both sides, so a
/// differential comparison of one is two runs of the same computation and could
/// not disagree; only a value stated independently of it can fail.
#[test]
fn avg_and_sum_are_withdrawn_and_match_the_mirror() {
    for function in ["AVG", "SUM"] {
        let query = |schema: &str, table: &str| {
            format!("SELECT {function}(\"id\") AS VAL FROM {schema}.\"{table}\"")
        };

        for statement in generated_statements(&query(VIRTUAL_SCHEMA, ITEMS)) {
            assert!(
                !statement.to_uppercase().contains(function),
                "no generated statement may carry a delegated {function}, since this release \
                 withholds it and Exasol computes it itself: {statement}"
            );
        }

        let rows = assert_matches_mirror(ITEMS, query);
        assert_eq!(
            rows.len(),
            1,
            "a single-group aggregation returns exactly one row: {rows:?}"
        );
    }

    let rows = stack::shared().exasol.query_rows(&format!(
        "SELECT SUM(\"id\") AS TOTAL, AVG(\"id\") AS MEAN FROM {VIRTUAL_SCHEMA}.\"items\""
    ));
    assert_eq!(
        cell(&rows[0], "TOTAL"),
        Some(SEEDED_ID_TOTAL.to_string()),
        "the seeded ids 1 through {ITEM_ROWS} sum to {SEEDED_ID_TOTAL}, and the withheld SUM \
         must still answer with it rather than failing: {rows:?}"
    );
    let mean: f64 = cell(&rows[0], "MEAN")
        .expect("the withheld AVG must answer with a value")
        .parse()
        .expect("Exasol returns an average as a number");
    assert!(
        (mean - SEEDED_ID_MEAN).abs() < MEAN_TOLERANCE,
        "the withheld AVG must answer with the mean of the seeded ids, {SEEDED_ID_MEAN}, not \
         {mean}"
    );
}

#[test]
fn two_column_group_by_is_not_delegated() {
    let query = |schema: &str, table: &str| {
        format!(
            "SELECT \"category\" AS CATEGORY, \"sort_key\" AS SORTKEY, COUNT(*) AS N FROM \
             {schema}.\"{table}\" GROUP BY \"category\", \"sort_key\" \
             ORDER BY \"category\", \"sort_key\""
        )
    };

    for statement in generated_statements(&query(VIRTUAL_SCHEMA, ITEMS)) {
        assert!(
            !statement.contains("GROUP BY") && !statement.to_uppercase().contains("COUNT"),
            "a two-column GROUP BY must be withdrawn whole, because this release advertises no \
             AGGREGATE_GROUP_BY_TUPLE: {statement}"
        );
    }

    let rows = assert_matches_mirror(ITEMS, query);
    assert_eq!(
        rows.len(),
        ITEM_ROWS,
        "every row carries its own category/sort_key pair, so grouping by both yields one group \
         per row: {rows:?}"
    );
}

#[test]
fn no_generated_statement_carries_order_limit_or_offset() {
    for suffix in ["ORDER BY \"id\"", "LIMIT 2", "ORDER BY \"id\" LIMIT 2"] {
        let query = move |schema: &str, table: &str| {
            format!("SELECT \"id\" FROM {schema}.\"{table}\" {suffix}")
        };

        for statement in generated_statements(&query(VIRTUAL_SCHEMA, ITEMS)) {
            let upper = statement.to_uppercase();
            assert!(
                !upper.contains("ORDER BY"),
                "Exasol applies ORDER BY itself; the generated statement for `{suffix}` must not \
                 also sort: {statement}"
            );
            assert!(
                !upper.contains("LIMIT"),
                "a delegated LIMIT would return an arbitrary N rows for Exasol to sort, which is \
                 unsound once ORDER BY is withheld; the generated statement for `{suffix}`: \
                 {statement}"
            );
            assert!(
                !upper.contains("OFFSET"),
                "OFFSET is withheld for the same reason LIMIT is; the generated statement for \
                 `{suffix}`: {statement}"
            );
        }
    }

    let limited = |schema: &str, table: &str| {
        format!("SELECT \"id\" FROM {schema}.\"{table}\" ORDER BY \"id\" LIMIT 2")
    };
    let rows = assert_matches_mirror(ITEMS, limited);
    assert_eq!(
        column_values(&rows, "id"),
        vec!["1".to_string(), "2".to_string()],
        "withholding ORDER BY and LIMIT costs no answer: Exasol applies both itself over the \
         whole scan. The row comparison is made over the ordered form alone, because an \
         unordered LIMIT selects an arbitrary two rows on each side and would compare two \
         arbitrary choices: {rows:?}"
    );
}

// --- two drivers, one BucketFS, one Exasol ----------------------------------

/// The test that would fail if package resolution or metadata discovery
/// defaulted to one driver: each technology's library lands under its own
/// BucketFS path, and a query against each virtual schema returns the row its
/// own source holds rather than the other source's. Task 8.5 owns the
/// capability half of the two-driver claim and states its own mechanism, so
/// this test does not restate it.
#[cfg(not(feature = "exasol-personal"))]
#[test]
fn two_drivers_install_distinct_libraries_and_each_query_reaches_its_own_source() {
    let stack = stack::shared();

    let postgres_library = installed_driver(stack, Driver::Postgres).unwrap_or_else(|| {
        panic!(
            "BucketFS lists {:?}, which includes no PostgreSQL driver",
            stack.exasol.bucket_entries()
        )
    });
    let mysql_library = installed_driver(stack, Driver::MySql).unwrap_or_else(|| {
        panic!(
            "BucketFS lists {:?}, which includes no MySQL driver",
            stack.exasol.bucket_entries()
        )
    });

    assert_ne!(
        postgres_library, mysql_library,
        "each driver must land at its own path, not share one with the other"
    );
    assert!(
        postgres_library.starts_with("adbc-drivers/postgresql/")
            && postgres_library.ends_with("libadbc_driver_postgresql.so"),
        "the PostgreSQL library must sit under its own technology's prefix: {postgres_library}"
    );
    assert!(
        mysql_library.starts_with("adbc-drivers/mysql/")
            && mysql_library.ends_with("libadbc_driver_mysql.so"),
        "the MySQL library must sit under its own technology's prefix: {mysql_library}"
    );

    let postgres_rows = stack.exasol.query_rows(&format!(
        "SELECT \"id\", \"category\" FROM {VIRTUAL_SCHEMA}.\"items\" WHERE \"id\" = 1"
    ));
    assert_eq!(
        cell(&postgres_rows[0], "category"),
        Some("widget".to_string()),
        "the PostgreSQL schema must return the row PostgreSQL holds, not MySQL's: \
         {postgres_rows:?}"
    );

    let mysql_rows = stack.exasol.query_rows(&format!(
        "SELECT \"id\", \"name\" FROM {}.\"orders\" WHERE \"id\" = 1",
        stack::MYSQL_VIRTUAL_SCHEMA
    ));
    assert_eq!(
        cell(&mysql_rows[0], "name"),
        Some("Widget".to_string()),
        "the MySQL schema must return the row MySQL holds, not PostgreSQL's: {mysql_rows:?}"
    );
}

/// Enumerated from [`Driver::ALL`] rather than hand-listed, so a driver this
/// stack actually provisions inherits the assertion the moment it joins the
/// identity. Replaces no earlier scenario: this file never carried a
/// PostgreSQL-only "driver value is accepted" test.
///
/// Trino is skipped rather than matched: this binary's stack provisions no
/// Trino connection, and `e2e_trino_test.rs` carries its own version of this
/// assertion, scoped to the two drivers its own stack provisions (Trino and
/// MySQL), the same way this one is scoped to PostgreSQL and MySQL.
#[test]
fn every_driver_this_stack_provisions_creates_a_virtual_schema() {
    let stack = stack::shared();

    for driver in Driver::ALL {
        let connection = match driver {
            Driver::Postgres => stack::POSTGRES_CONNECTION,
            Driver::MySql => stack::MYSQL_CONNECTION,
            Driver::Trino => continue,
        };
        let schema_name = format!("EVERYDRIVER_{}", driver.as_str().to_ascii_uppercase());

        schema::create_virtual_schema(&stack.exasol, *driver, &schema_name, connection);

        assert!(
            !table_names(&schema_name).is_empty(),
            "{schema_name} must expose at least one table: a schema that created \
             successfully but discovered nothing would still pass a DDL-only assertion"
        );
        assert_eq!(
            adapter_notes(&schema_name)["driver"],
            serde_json::Value::String(driver.as_str().to_string()),
            "{schema_name}'s adapterNotes must record the driver it was created with"
        );

        schema::drop_virtual_schema(&stack.exasol, &schema_name);
    }
}

/// `driver-provisioning/driver-path-override`'s success half, which only a
/// live stack can show: a virtual schema handed a library an operator placed
/// loads it, discovers through it, and provisions nothing.
///
/// It is the release's answer to a cluster that lost its outbound egress, and
/// until now only its refusal path reached a running adapter — a `DRIVER_PATH`
/// that named nothing. What the two notes fields say is the observable
/// difference between the routes: the path is recorded exactly as given, and
/// no `driverVersion` is recorded at all, because nothing resolved one.
///
/// The path comes off the shared schema's own notes rather than being composed
/// here: it carries the version resolution chose at run time, which no
/// constant in this repository can state.
#[test]
fn driver_path_is_accepted_and_bypasses_provisioning() {
    let stack = stack::shared();
    const SUPPLIED_SCHEMA: &str = "PGVS_SUPPLIED";

    let library = adapter_notes(VIRTUAL_SCHEMA)["driverPath"]
        .as_str()
        .expect("the provisioned schema records the path it loaded its driver from")
        .to_string();

    schema::create_virtual_schema_through(
        &stack.exasol,
        schema::VirtualSchemaSpec {
            schema: SUPPLIED_SCHEMA,
            driver: Driver::Postgres,
            connection: stack::POSTGRES_CONNECTION,
            bucketfs_connection: None,
            driver_path: Some(&library),
        },
    );

    assert_eq!(
        table_names(SUPPLIED_SCHEMA),
        table_names(VIRTUAL_SCHEMA),
        "a schema that loaded a supplied library must discover exactly what a provisioned one \
         discovers; a different table set would mean it reached a different source"
    );

    let notes = adapter_notes(SUPPLIED_SCHEMA);
    assert_eq!(
        notes["driverPath"], library,
        "the supplied path is recorded as given, neither re-rooted under the mount nor extended \
         with the driver and version segments self-install computes: {notes}"
    );
    assert!(
        notes.get("driverVersion").is_none(),
        "nothing resolved a version for a library the operator supplied, and recording a \
         compiled-in one would report a version this schema is not running: {notes}"
    );

    schema::drop_virtual_schema(&stack.exasol, SUPPLIED_SCHEMA);
}

// --- omission recording and credential redaction ----------------------------

/// `items` mixes mapped and unmapped columns on purpose (`docker/postgres/
/// init.sql`), so every one of its unmapped columns must be left out of the
/// virtual table and recorded under its own PostgreSQL type and reason, while
/// the wholly unmapped `all_unmapped` table must be absent from
/// `schemaMetadata` entirely rather than exposed thin — and the shared
/// `PGVS` schema already having been created over both is this test's proof
/// that creation still succeeds despite the omissions.
#[test]
fn unmapped_items_columns_are_recorded_and_all_unmapped_table_is_absent() {
    let columns = column_names(VIRTUAL_SCHEMA, "items");
    let notes = adapter_notes(VIRTUAL_SCHEMA);
    let omitted = notes["omitted"]
        .as_array()
        .expect("adapter notes carry an omitted array");

    for (column, source_type) in [
        ("code", "character"),
        ("description", "text"),
        ("details", "jsonb"),
        ("photo", "bytea"),
        ("uid", "uuid"),
        ("seen_at", "timestamp with time zone"),
    ] {
        assert!(
            !columns.contains(&column.to_string()),
            "unmapped column {column} must not appear in the virtual table: {columns:?}"
        );
        let entry = omitted
            .iter()
            .find(|entry| entry["table"] == "items" && entry["column"] == column)
            .unwrap_or_else(|| panic!("the omitted {column} column must be recorded: {omitted:?}"));
        assert_eq!(
            entry["sourceType"], source_type,
            "the recorded sourceType must be PostgreSQL's own name for the column's type: \
             {entry}"
        );
        assert!(
            entry["reason"]
                .as_str()
                .is_some_and(|reason| !reason.is_empty()),
            "the omission must carry a non-empty reason: {entry}"
        );
    }

    let tables = table_names(VIRTUAL_SCHEMA);
    assert!(
        !tables.contains(&"all_unmapped".to_string()),
        "a table with no mappable column must not be exposed: {tables:?}"
    );
    assert!(
        omitted
            .iter()
            .any(|entry| entry["table"] == "all_unmapped" && entry["column"].is_null()),
        "the wholly omitted table must be recorded: {omitted:?}"
    );
}

// --- identifier case, view exposure, and refresh ----------------------------

/// `"MixedCase"` (`docker/postgres/init.sql`) is the one fixture object whose
/// table name and column names are all mixed case, which is what separates
/// case *preservation* from case folding: an unquoted identifier folds to
/// lower case under PostgreSQL's own rules, so a lower-case-only fixture could
/// not tell the two apart.
#[test]
fn identifiers_keep_their_case_through_discovery_and_rendering() {
    let tables = table_names(VIRTUAL_SCHEMA);
    assert!(
        tables.contains(&"items".to_string())
            && tables.contains(&"orders".to_string())
            && tables.contains(&"MixedCase".to_string()),
        "discovery must keep every source identifier's original case: {tables:?}"
    );
    assert!(
        !tables.contains(&"ITEMS".to_string()) && !tables.contains(&"MIXEDCASE".to_string()),
        "no identifier may be folded to upper case: {tables:?}"
    );

    assert_eq!(
        column_names(VIRTUAL_SCHEMA, "MixedCase"),
        vec!["ItemId".to_string(), "Label".to_string()],
        "the mixed-case table's own columns must keep their source case and ordinal order"
    );

    let query = format!(
        "SELECT \"ItemId\", \"Label\" FROM {VIRTUAL_SCHEMA}.\"MixedCase\" ORDER BY \"ItemId\""
    );
    let statement = generated_statement(&query);
    assert!(
        statement.contains("\"public\".\"MixedCase\"") && statement.contains("\"ItemId\""),
        "the generated statement must carry the source's own case for the schema-qualified \
         table and the ItemId column, which is what separates a correctly rendered identifier \
         from one the source would have rejected: {statement}"
    );

    let rows = stack::shared().exasol.query_rows(&query);
    assert_eq!(
        rows.len(),
        2,
        "docker/postgres/init.sql seeds two rows into \"MixedCase\": {rows:?}"
    );
    assert_eq!(cell(&rows[0], "ItemId"), Some("1".to_string()));
    assert_eq!(cell(&rows[0], "Label"), Some("First".to_string()));
    assert_eq!(cell(&rows[1], "ItemId"), Some("2".to_string()));
    assert_eq!(cell(&rows[1], "Label"), Some("Second".to_string()));
}

/// `order_names` (`docker/postgres/init.sql`) is a view over `orders`,
/// projecting only mapped columns. `generated_statement` is what tells a view
/// queried through the pushdown path from one Exasol resolved on its own: a
/// name the adapter never discovered would leave `EXPLAIN VIRTUAL` reporting
/// no statement for it at all.
#[test]
fn views_are_exposed_alongside_base_tables() {
    let tables = table_names(VIRTUAL_SCHEMA);
    assert!(
        tables.contains(&"order_names".to_string()) && tables.contains(&"orders".to_string()),
        "the exposed metadata must list both the base table and the view, not the view alone: \
         {tables:?}"
    );
    assert_eq!(
        column_names(VIRTUAL_SCHEMA, "order_names"),
        vec!["id".to_string(), "name".to_string()],
        "the view projects exactly the two mapped columns of orders it selects in \
         docker/postgres/init.sql"
    );

    let query =
        format!("SELECT \"id\", \"name\" FROM {VIRTUAL_SCHEMA}.\"order_names\" ORDER BY \"id\"");
    let statement = generated_statement(&query);
    assert!(
        statement.contains("\"public\".\"order_names\""),
        "the generated statement must name the view as a table reached through the pushdown \
         path, not one Exasol resolved itself: {statement}"
    );

    let rows = stack::shared().exasol.query_rows(&query);
    assert_eq!(
        column_values(&rows, "id"),
        vec!["1".to_string(), "2".to_string(), "3".to_string()],
        "order_names projects every row docker/postgres/init.sql seeds into orders: {rows:?}"
    );
    assert_eq!(
        column_values(&rows, "name"),
        vec![
            "Widget".to_string(),
            "Gadget".to_string(),
            "Gizmo".to_string()
        ],
        "the view's second column must carry the names docker/postgres/init.sql seeds into \
         orders, in id order: {rows:?}"
    );
}

/// `refresh_target` (`docker/postgres/init.sql`) is dedicated to this test's
/// `ALTER TABLE`, so no other test's fixture is disturbed by it. Columns are
/// read before the `ADD COLUMN` and asserted only after the `DROP COLUMN` has
/// restored the source, so a panic between the two leaves no other test
/// reading a virtual schema that still declares a column the source no
/// longer has.
#[test]
fn refresh_picks_up_a_new_postgresql_column() {
    let stack = stack::shared();
    let before = declared_columns(VIRTUAL_SCHEMA, "refresh_target");

    stack
        .postgres
        .execute("ALTER TABLE refresh_target ADD COLUMN extra NUMERIC(9,3)");
    stack
        .exasol
        .execute(&format!("ALTER VIRTUAL SCHEMA {VIRTUAL_SCHEMA} REFRESH"));
    let after_add = declared_columns(VIRTUAL_SCHEMA, "refresh_target");

    stack
        .postgres
        .execute("ALTER TABLE refresh_target DROP COLUMN extra");
    stack
        .exasol
        .execute(&format!("ALTER VIRTUAL SCHEMA {VIRTUAL_SCHEMA} REFRESH"));
    let after_drop = declared_columns(VIRTUAL_SCHEMA, "refresh_target");

    assert!(
        !before.iter().any(|(name, _)| name == "extra"),
        "the fixture must not already carry the column the refresh is to discover: {before:?}"
    );
    assert!(
        after_add.contains(&("extra".to_string(), "DECIMAL(9,3)".to_string())),
        "a refresh must re-run discovery and expose the new source column under PostgreSQL's \
         own numeric mapping, which is what proves discovery ran rather than returning a cached \
         shape: {after_add:?}"
    );
    assert_eq!(
        after_drop, before,
        "the second refresh must leave the shared virtual schema declaring exactly the columns \
         the source still has"
    );
}

/// Every error `createVirtualSchema`, `refresh`, and `setProperties` surface
/// for one broken `CONNECTION`, over the three entry points that all funnel
/// through `create_schema::open`. `scenario` names the two schemas and
/// connections this run creates so a second call cannot collide with the
/// first's objects, `address` is the connection's address, and `password` is
/// the credential the run authenticates with.
///
/// The three failures are produced in the order `createVirtualSchema`,
/// `setProperties`, `refresh`: the second reuses a working schema/connection
/// pair created for it, and the third rotates that same connection's
/// credentials to the broken ones only after `setProperties` has already
/// been exercised over it, so neither call disturbs the other's fixture.
fn credential_failures(scenario: &str, address: &str, password: &str) -> [String; 3] {
    let stack = stack::shared();

    let bad_connection = format!("ADBC_VS_PG_{scenario}_BAD");
    stack.exasol.execute(&format!(
        "CREATE OR REPLACE CONNECTION {bad_connection} TO {} USER {} IDENTIFIED BY {}",
        schema::quote_literal(address),
        schema::quote_literal(postgres::USER),
        schema::quote_literal(password)
    ));

    let create_schema_name = format!("PGFAIL_{scenario}_CREATE");
    let create_error = schema::create_virtual_schema_expecting_failure(
        &stack.exasol,
        Driver::Postgres,
        &create_schema_name,
        &bad_connection,
    );

    let working_connection = format!("ADBC_VS_PG_{scenario}_WORK");
    schema::create_postgres_connection(&stack.exasol, &working_connection, postgres::DATABASE);
    let working_schema = format!("PGFAIL_{scenario}_WORK");
    schema::create_virtual_schema(
        &stack.exasol,
        Driver::Postgres,
        &working_schema,
        &working_connection,
    );

    let set_properties_error = stack.exasol.execute_expecting_failure(&format!(
        "ALTER VIRTUAL SCHEMA {working_schema} SET CONNECTION_NAME = {}",
        schema::quote_literal(&bad_connection)
    ));

    stack.exasol.execute(&format!(
        "CREATE OR REPLACE CONNECTION {working_connection} TO {} USER {} IDENTIFIED BY {}",
        schema::quote_literal(address),
        schema::quote_literal(postgres::USER),
        schema::quote_literal(password)
    ));
    let refresh_error = stack
        .exasol
        .execute_expecting_failure(&format!("ALTER VIRTUAL SCHEMA {working_schema} REFRESH"));

    schema::drop_virtual_schema(&stack.exasol, &working_schema);
    schema::drop_connection(&stack.exasol, &working_connection);
    schema::drop_connection(&stack.exasol, &bad_connection);

    [create_error, set_properties_error, refresh_error]
}

#[test]
fn wrong_password_never_leaks_across_all_three_entry_points() {
    let bad_password = "not-the-real-password-please-do-not-leak";
    let errors = credential_failures(
        "WRONGPASS",
        &postgres::driver_uri(postgres::DATABASE),
        bad_password,
    );

    for (entry_point, error) in ["createVirtualSchema", "setProperties", "refresh"]
        .iter()
        .zip(errors.iter())
    {
        assert!(
            !error.contains(bad_password) && !error.contains(postgres::PASSWORD),
            "{entry_point} must not surface a password, wrong or real: {error}"
        );
        assert!(
            error.contains(postgres::ADDRESS),
            "{entry_point} must still name the host it could not reach: {error}"
        );
        assert!(
            error.contains(postgres::DATABASE),
            "{entry_point} must still name the database: {error}"
        );
    }
}

#[test]
fn unreachable_host_never_leaks_the_password_across_all_three_entry_points() {
    let errors = credential_failures(
        "UNREACHABLE",
        &postgres::unreachable_uri(),
        postgres::PASSWORD,
    );

    for (entry_point, error) in ["createVirtualSchema", "setProperties", "refresh"]
        .iter()
        .zip(errors.iter())
    {
        assert!(
            !error.contains(postgres::PASSWORD),
            "{entry_point} must not surface the password: {error}"
        );
        assert!(
            error.contains(postgres::ADDRESS),
            "{entry_point} must still name the host it could not reach: {error}"
        );
        assert!(
            error.contains(postgres::DATABASE),
            "{entry_point} must still name the database: {error}"
        );
    }
}

// --- per-schema capability sets ---------------------------------------------

/// The standing regression test behind the plan's keystone precondition: a
/// `FULL OUTER JOIN` over the PostgreSQL schema's own joinable pair must
/// delegate whole, because `postgresql.toml` advertises `JOIN_TYPE_FULL_OUTER`,
/// while the same shape over the MySQL schema must be withdrawn into scans
/// carrying no join at all, because `mysql.toml` does not. Both schemas are
/// created through the ordinary `CREATE VIRTUAL SCHEMA` path against the
/// shipped `ADBC_ADAPTER`, so the only thing that can explain a difference is
/// `getCapabilities` resolving its dialect from `adapterNotes.driver`, live.
///
/// Every reported statement is read, not just the first: a withdrawn join
/// leaves one pushed-down statement per scanned table rather than one joined
/// statement, so reading only the first would silently pass over a withdrawal
/// it should be asserting.
#[test]
fn two_virtual_schemas_advertise_their_own_capability_sets() {
    let query = |schema: &str| {
        format!(
            "SELECT i.\"id\" AS ID, n.\"note\" AS NOTE FROM {schema}.\"items\" i FULL OUTER \
             JOIN {schema}.\"item_notes\" n ON i.\"id\" = n.\"item_id\""
        )
    };

    let postgres_statements = generated_statements(&query(VIRTUAL_SCHEMA));
    assert!(
        postgres_statements
            .iter()
            .any(|statement| statement.contains("FULL OUTER JOIN")),
        "the PostgreSQL schema advertises JOIN_TYPE_FULL_OUTER, so one generated statement must \
         carry the delegated join: {postgres_statements:?}"
    );

    let mysql_statements = generated_statements(&query(stack::MYSQL_VIRTUAL_SCHEMA));
    assert!(
        mysql_statements
            .iter()
            .all(|statement| !statement.to_uppercase().contains("JOIN")),
        "the MySQL schema advertises no JOIN_TYPE_FULL_OUTER, so Exasol must withdraw the join \
         and perform it itself, leaving no generated statement carrying one: {mysql_statements:?}"
    );
}

// --- helpers ----------------------------------------------------------------

/// The bucket-relative path of one technology's installed driver library,
/// whatever version resolution selected for it, or nothing when no version
/// is installed for that driver.
///
/// The listing is read rather than a path composed, because the version
/// segment is the registry's answer at run time and no constant in this
/// repository can state it.
///
/// Excludes any entry whose version segment does not parse through
/// [`common::driver::parse_driver_semver`] — in particular a `supplied`
/// object the MySQL E2E binary places under the same driver prefix to prove
/// self-install does not short-circuit off a pre-placed library. The two
/// binaries share one BucketFS, so a `supplied` object a MySQL-binary failure
/// left behind must not be returned here as the installed driver.
#[cfg(not(feature = "exasol-personal"))]
fn installed_driver(stack: &stack::Stack, driver: Driver) -> Option<String> {
    let prefix = format!("adbc-drivers/{}/", driver.as_str());
    let library = driver.package().library;
    stack
        .exasol
        .bucket_entries()
        .into_iter()
        .filter(|entry| entry.starts_with(&prefix) && entry.ends_with(library))
        .find(|entry| common::driver::installed_driver_version(entry, &prefix).is_some())
}

/// The `adapterNotes` Exasol persisted for `schema`, parsed.
fn adapter_notes(schema: &str) -> serde_json::Value {
    let notes = stack::shared()
        .exasol
        .query_scalar(&format!(
            "SELECT ADAPTER_NOTES FROM SYS.EXA_ALL_VIRTUAL_SCHEMAS \
             WHERE SCHEMA_NAME = '{schema}'"
        ))
        .unwrap_or_else(|| panic!("virtual schema {schema} records adapter notes"));
    serde_json::from_str(&notes).expect("adapter notes are a JSON object")
}

/// Every table name `schema` exposes, in name order.
fn table_names(schema: &str) -> Vec<String> {
    stack::shared()
        .exasol
        .query_rows(&format!(
            "SELECT TABLE_NAME FROM SYS.EXA_ALL_TABLES WHERE TABLE_SCHEMA = '{schema}' \
             ORDER BY TABLE_NAME"
        ))
        .iter()
        .filter_map(|row| cell(row, "TABLE_NAME"))
        .collect()
}

/// The column names `schema.table` declares, in ordinal-position order.
fn column_names(schema: &str, table: &str) -> Vec<String> {
    declared_columns(schema, table)
        .into_iter()
        .map(|(name, _)| name)
        .collect()
}

/// Runs `query(schema, table)` against `virtual_table` of the virtual schema
/// and against the mirror's `items`, and asserts the two answers are identical.
///
/// The two sides are independent computations — PostgreSQL evaluates one,
/// Exasol the other — so their agreement is the differential proof this tier
/// relies on: a semantic gap between the two engines shows up as a difference
/// rather than having to be predicted case by case. The query closure receives
/// the table name beside the schema, so the virtual side can read a table the
/// mirror has no counterpart of its own for — the mirror holds `items`, and
/// `items_c` is the same rows under a different source collation.
fn assert_matches_mirror(
    virtual_table: &str,
    query: impl Fn(&str, &str) -> String,
) -> Vec<serde_json::Map<String, serde_json::Value>> {
    let stack = stack::shared();
    let virtual_rows = stack
        .exasol
        .query_rows(&query(VIRTUAL_SCHEMA, virtual_table));
    let mirror_rows = stack
        .exasol
        .query_rows(&query(postgres_mirror::SCHEMA, ITEMS));
    assert_eq!(
        virtual_rows, mirror_rows,
        "the delegated query must return exactly the rows the local Exasol mirror returns: \
         virtual {virtual_rows:?}, mirror {mirror_rows:?}"
    );
    virtual_rows
}

/// The one statement the adapter generated for `query`, failing when it
/// generated any other number.
///
/// A withdrawn join leaves one statement per scanned table, so a test reading
/// "the" statement has to say that it expected exactly one — otherwise a
/// silently withdrawn fragment would be asserted against the first scan of two
/// and pass.
fn generated_statement(query: &str) -> String {
    let statements = generated_statements(query);
    assert_eq!(
        statements.len(),
        1,
        "this query must be delegated as one statement, and EXPLAIN VIRTUAL reports \
         {statements:?}"
    );
    statements
        .into_iter()
        .next()
        .expect("exactly one statement")
}

/// Every statement the adapter generated for `query`, read out of the scan
/// specification each pushdown response embedded.
///
/// The specification is parsed with the product's own parser rather than the
/// response text being matched directly: the response carries the statement as
/// an Exasol string literal inside JSON, so every quote in it is doubled and
/// every identifier escaped, and a substring assertion against that text would
/// be asserting the escaping rather than the statement.
fn generated_statements(query: &str) -> Vec<String> {
    let statements: Vec<String> = stack::shared()
        .exasol
        .query_rows(&format!(
            "SELECT PUSHDOWN_SQL FROM (EXPLAIN VIRTUAL {query})"
        ))
        .iter()
        .filter_map(|row| cell(row, "PUSHDOWN_SQL"))
        .map(|call| source_statement(&call))
        .collect();

    assert!(
        !statements.is_empty(),
        "every query here reads a virtual table, so at least one statement must be generated \
         for it; an empty report would make each assertion that a fragment is *absent* from \
         every statement pass over nothing: {query}"
    );
    statements
}

/// The source statement embedded in one `pushdown` response.
///
/// The response is `SELECT <scan script>('<spec>') EMITS (...) FROM DUAL`, with
/// the specification's own single quotes doubled by Exasol literal quoting. The
/// closing delimiter is found from the right, because the specification itself
/// may end in a doubled quote.
fn source_statement(call: &str) -> String {
    let prefix = format!("{SCAN_SCRIPT_NAME}('");
    let suffix = "') EMITS (";
    let start = call
        .find(&prefix)
        .unwrap_or_else(|| panic!("a pushdown response calls the scan script: {call}"))
        + prefix.len();
    let end = call
        .rfind(suffix)
        .unwrap_or_else(|| panic!("a pushdown response declares an EMITS clause: {call}"));
    assert!(
        start <= end,
        "the scan specification lies between the script call and the EMITS clause: {call}"
    );

    let spec_json = call[start..end].replace("''", "'");
    scan_spec::parse(&spec_json)
        .unwrap_or_else(|error| panic!("parsing the embedded scan specification: {error}\n{call}"))
        .sql
}

/// The values of `column` across `rows`, in the order the query returned them.
fn column_values(rows: &[serde_json::Map<String, serde_json::Value>], column: &str) -> Vec<String> {
    rows.iter().filter_map(|row| cell(row, column)).collect()
}

/// The name and declared Exasol type of every column of `schema.table`, in
/// ordinal-position order — the form that compares a mirror table against the
/// virtual table it mirrors.
fn declared_columns(schema: &str, table: &str) -> Vec<(String, String)> {
    stack::shared()
        .exasol
        .query_rows(&format!(
            "SELECT COLUMN_NAME, COLUMN_TYPE FROM SYS.EXA_ALL_COLUMNS \
             WHERE COLUMN_SCHEMA = '{schema}' AND COLUMN_TABLE = '{table}' \
             ORDER BY COLUMN_ORDINAL_POSITION"
        ))
        .iter()
        .filter_map(|row| Some((cell(row, "COLUMN_NAME")?, cell(row, "COLUMN_TYPE")?)))
        .collect()
}
