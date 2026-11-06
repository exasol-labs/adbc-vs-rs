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

use adbc_vs_adapter::exa_type::ExasolType;
use adbc_vs_adapter::pushdown::SCAN_SCRIPT_NAME;
use adbc_vs_adapter::scan_spec;
use adbc_vs_scan::driver::Driver;
use common::exasol::cell;
use common::postgres_mirror;
use common::postgres_stack::{self as stack, POSTGRES_VIRTUAL_SCHEMA as VIRTUAL_SCHEMA};
use common::{postgres, probe, schema};

/// How many rows `docker/postgres/init.sql` seeds into `items`, and into
/// `items_c`, which holds the same six rows and two more.
const ITEM_ROWS: usize = 6;
const ITEM_C_ROWS: usize = 8;

/// The table every type-conversion test reads, one column per mapped type.
const TYPE_COVERAGE: &str = "type_coverage";

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
    for table in ["items", "items_c", "item_notes", TYPE_COVERAGE] {
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
        strings(&["1", "4", "7", "8"]),
        "only the four 'B' rows precede 'C' in code-point order, which `items_c`'s own collation \
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

    let like_query = |schema: &str, table: &str| {
        format!(
            "SELECT \"id\" FROM {schema}.\"{table}\" WHERE \"tag\" LIKE '5\\%x' ORDER BY \"id\""
        )
    };

    let like_statement = generated_statement(&like_query(VIRTUAL_SCHEMA, ITEMS_C));
    assert!(
        like_statement.contains("LIKE") && !like_statement.contains("ESCAPE"),
        "a pattern the query gave no ESCAPE clause for must reach PostgreSQL with none, so \
         PostgreSQL's own backslash rule reads it: {like_statement}"
    );

    let like_rows = assert_matches_mirror(ITEMS_C, like_query);
    assert_eq!(
        column_values(&like_rows, "id"),
        strings(&["7"]),
        "the backslash escapes the `%`, so only the row holding `5%x` matches, and the row \
         holding `5\\yx` does not: {like_rows:?}"
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
    assert_eq!(cell(answers, "N"), Some(ITEM_C_ROWS.to_string()));
    assert_eq!(
        cell(answers, "MINKEY"),
        Some("B".to_string()),
        "the delegated MIN over a `COLLATE \"C\"` column must return Exasol's code-point \
         minimum: {answers:?}"
    );
    assert_eq!(cell(answers, "MAXKEY"), Some("a".to_string()));
    assert_eq!(
        cell(answers, "MINTAG"),
        Some("5%x".to_string()),
        "the delegated MIN must return the code-point minimum, which no padding of the \
         trailing-space value `ab ` changes: {answers:?}"
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

    let between = format!(
        "SELECT \"id\" FROM {VIRTUAL_SCHEMA}.\"{ITEMS}\" WHERE \"sort_key\" BETWEEN 'C' AND 'a' \
         ORDER BY \"id\""
    );
    let between_statement = generated_statement(&between);
    assert!(
        between_statement.contains("BETWEEN E'C' AND E'a'")
            && !between_statement.contains("COLLATE"),
        "the BETWEEN must reach PostgreSQL bare: {between_statement}"
    );
    let rows = stack::shared().exasol.query_rows(&between);
    assert!(
        rows.is_empty(),
        "the en_US.utf8 locale orders 'a' before 'C', so the range 'C' to 'a' is empty, where \
         Exasol's code-point order selects four rows: {rows:?}"
    );
}

/// The whole rule, read off the statements the deployed adapter actually
/// generated: no operand in any position carries a collation. Absence is
/// asserted as the absence of the word anywhere in the statement, not of one
/// spelling, over `items` and over `items_c` alike, since the declared
/// collation of a column must not change what the dialect renders.
#[test]
fn no_generated_statement_carries_a_collation_keyword() {
    for query in [
        format!("SELECT \"id\" FROM {VIRTUAL_SCHEMA}.\"{TYPE_COVERAGE}\" WHERE \"uid\" < 'b'"),
        format!("SELECT \"id\" FROM {VIRTUAL_SCHEMA}.\"{TYPE_COVERAGE}\" WHERE \"uid\" = 'a'"),
        format!(
            "SELECT \"id\" FROM {VIRTUAL_SCHEMA}.\"{TYPE_COVERAGE}\" WHERE \"doc_json\" LIKE 'x%'"
        ),
        format!(
            "SELECT \"id\" FROM {VIRTUAL_SCHEMA}.\"{TYPE_COVERAGE}\" \
             WHERE \"unbounded_amount\" IN ('1.0', 'NaN')"
        ),
        format!(
            "SELECT \"uid\", COUNT(*) FROM {VIRTUAL_SCHEMA}.\"{TYPE_COVERAGE}\" GROUP BY \"uid\""
        ),
        format!("SELECT COUNT(DISTINCT \"uid\") FROM {VIRTUAL_SCHEMA}.\"{TYPE_COVERAGE}\""),
        format!(
            "SELECT MIN(\"uid\"), MAX(\"unbounded_amount\") FROM \
             {VIRTUAL_SCHEMA}.\"{TYPE_COVERAGE}\""
        ),
        format!(
            "SELECT a.\"id\" FROM {VIRTUAL_SCHEMA}.\"{TYPE_COVERAGE}\" a INNER JOIN \
             {VIRTUAL_SCHEMA}.\"{TYPE_COVERAGE}\" b ON a.\"uid\" = b.\"uid\""
        ),
    ] {
        for statement in generated_statements(&query) {
            assert!(
                !statement.contains("COLLATE"),
                "a column read as text keeps the collation PostgreSQL applies to it: `{query}` \
                 generated: {statement}"
            );
        }
    }

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

    let maximum = |schema: &str, table: &str| {
        format!("SELECT MAX(\"sort_key\" || '') AS HI FROM {schema}.\"{table}\"")
    };
    let maximum_statement = generated_statement(&maximum(VIRTUAL_SCHEMA, ITEMS));
    assert!(
        !maximum_statement.contains("MAX("),
        "the MAX over the wrapped operand must not reach PostgreSQL: {maximum_statement}"
    );
    let rows = assert_matches_mirror(ITEMS, maximum);
    assert_eq!(cell(&rows[0], "HI"), Some("a".to_string()), "{rows:?}");
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

    for (predicate, expected_ids) in [
        ("\"category\" = 'Widget'", ["2", "4"].as_slice()),
        ("\"category\" LIKE 'Wid%'", ["2", "4"].as_slice()),
        (
            "\"category\" IN ('cafe', 'Widget')",
            ["2", "4", "5"].as_slice(),
        ),
    ] {
        let query = |schema: &str, table: &str| {
            format!("SELECT \"id\" FROM {schema}.\"{table}\" WHERE {predicate} ORDER BY \"id\"")
        };

        let statement = generated_statement(&query(VIRTUAL_SCHEMA, ITEMS));
        assert!(
            statement.contains("WHERE") && !statement.contains("COLLATE"),
            "`{predicate}` must reach PostgreSQL bare, or the rows below measure Exasol: \
             {statement}"
        );
        let rows = assert_matches_mirror(ITEMS, query);
        assert_eq!(
            column_values(&rows, "id"),
            strings(expected_ids),
            "`{predicate}` must keep case and accent variants apart, as Exasol does: {rows:?}"
        );
    }
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

/// The table `SUM`, `AVG`, and `HAVING` read, whose local mirror holds the same
/// rows, and the table holding a `NaN`, which the mirror does not copy.
const MEASURES: &str = "measures";
const NAN_MEASURES: &str = "nan_measures";

/// The four aggregates a numeric operand answers, each aliased so a result
/// row names them.
fn numeric_aggregates(column: &str) -> String {
    format!(
        "SUM(\"{column}\") AS S, SUM(DISTINCT \"{column}\") AS SD, AVG(\"{column}\") AS A, \
         AVG(DISTINCT \"{column}\") AS AD"
    )
}

/// `aggregates` over the whole of `table`, and per group of its `grp` column.
fn single_group_and_grouped(
    aggregates: String,
) -> (
    impl Fn(&str, &str) -> String + Clone,
    impl Fn(&str, &str) -> String + Clone,
) {
    let single_aggregates = aggregates.clone();
    (
        move |schema: &str, table: &str| {
            format!("SELECT {single_aggregates} FROM {schema}.\"{table}\"")
        },
        move |schema: &str, table: &str| {
            format!(
                "SELECT \"grp\", {aggregates} FROM {schema}.\"{table}\" GROUP BY \"grp\" \
                 ORDER BY \"grp\""
            )
        },
    )
}

/// Asserts the statement PostgreSQL received carries the `SUM` and the `SUM`
/// and `COUNT` quotient the aggregates of [`numeric_aggregates`] render as, and
/// no PostgreSQL `avg`.
fn assert_sum_and_avg_delegated(statement: &str) {
    for fragment in [
        "SUM(",
        "SUM(DISTINCT ",
        "CAST(SUM(",
        "AS DOUBLE PRECISION) / COUNT(",
    ] {
        assert!(
            statement.contains(fragment),
            "the generated statement must carry {fragment}: {statement}"
        );
    }
    assert!(
        !statement.to_uppercase().contains("AVG("),
        "`AVG` must render as the quotient of `SUM` and `COUNT`, never as PostgreSQL's own: \
         {statement}"
    );
}

/// `pushdown-aggregation-delegation`: `SUM` and `AVG`, each with and without
/// `DISTINCT`, over every numeric column, once without grouping and once
/// grouped, return the rows Exasol returns over the mirror. The `bigint` sum
/// passes the `bigint` range, and group `c`, which holds only null operands,
/// returns null for both functions.
#[test]
fn sum_and_avg_are_delegated_and_match_the_mirror() {
    for column in ["big_value", "decimal_value", "double_value", "float_value"] {
        let (single_group, grouped) = single_group_and_grouped(numeric_aggregates(column));

        let single_statement = generated_statement(&single_group(VIRTUAL_SCHEMA, MEASURES));
        assert_sum_and_avg_delegated(&single_statement);
        assert!(
            !single_statement.contains("GROUP BY"),
            "a single-group aggregation must render no GROUP BY clause: {single_statement}"
        );
        let single_rows = assert_matches_mirror(MEASURES, single_group);
        assert_eq!(single_rows.len(), 1, "{column}: {single_rows:?}");

        assert_sum_and_avg_delegated(&generated_statement(&grouped(VIRTUAL_SCHEMA, MEASURES)));
        let grouped_rows = assert_matches_mirror(MEASURES, grouped);
        assert_eq!(grouped_rows.len(), 3, "{column}: {grouped_rows:?}");
        let null_group = &grouped_rows[2];
        for aggregate in ["S", "SD", "A", "AD"] {
            assert_eq!(
                cell(null_group, aggregate),
                None,
                "{column}: the group holding only null operands must return null for \
                 {aggregate}: {null_group:?}"
            );
        }
    }
}

/// `pushdown-aggregation-delegation`: `SUM` and `AVG` over a `boolean` column
/// and over a `character varying` column holding numbers as text convert the
/// operand as Exasol does. `label` holds an empty string, which Exasol reads as
/// null, and `'5'` beside `'5.0'`, which `SUM(DISTINCT)` takes as one value and
/// `AVG(DISTINCT)` divides as two.
#[test]
fn sum_and_avg_over_boolean_and_character_columns_match_the_mirror() {
    for (column, converted_fragment) in [
        ("flag", "SUM(CAST(\"flag\" AS INTEGER))"),
        ("label", "CAST(NULLIF(\"label\", "),
    ] {
        let (single_group, grouped) = single_group_and_grouped(numeric_aggregates(column));

        let statement = generated_statement(&single_group(VIRTUAL_SCHEMA, MEASURES));
        assert_sum_and_avg_delegated(&statement);
        assert!(
            statement.contains(converted_fragment),
            "the operand must render as {converted_fragment}: {statement}"
        );
        let single_rows = assert_matches_mirror(MEASURES, single_group);
        assert_eq!(single_rows.len(), 1, "{column}: {single_rows:?}");

        let grouped_rows = assert_matches_mirror(MEASURES, grouped);
        assert_eq!(grouped_rows.len(), 3, "{column}: {grouped_rows:?}");
    }

    let predicate = |schema: &str, measures: &str| {
        format!(
            "SELECT SUM(\"big_value\" > 5) AS S, AVG(\"big_value\" > 5) AS A \
             FROM {schema}.\"{measures}\""
        )
    };
    let statement = generated_statement(&predicate(VIRTUAL_SCHEMA, MEASURES));
    assert!(
        statement.contains("CAST((5 < \"big_value\") AS INTEGER)"),
        "a predicate operand must render as an INTEGER: {statement}"
    );
    assert_matches_mirror(MEASURES, predicate);
}

/// `pushdown-aggregation-delegation`: a `HAVING` clause over `COUNT(*)`, over
/// `SUM`, and over `AVG` reaches PostgreSQL and keeps the groups the mirror
/// keeps, which are fewer than all three.
#[test]
fn having_is_delegated_and_matches_the_mirror() {
    for (having, kept_groups) in [
        ("COUNT(*) > 2", 2),
        ("SUM(\"decimal_value\") > 1", 1),
        ("AVG(\"double_value\") > 2", 1),
    ] {
        let query = |schema: &str, measures: &str| {
            format!(
                "SELECT \"grp\", COUNT(*) AS N, SUM(\"decimal_value\") AS S \
                 FROM {schema}.\"{measures}\" GROUP BY \"grp\" HAVING {having} ORDER BY \"grp\""
            )
        };

        let statement = generated_statement(&query(VIRTUAL_SCHEMA, MEASURES));
        assert!(
            statement.contains("GROUP BY \"grp\" HAVING"),
            "the generated statement must carry the HAVING clause after the grouping for \
             {having}: {statement}"
        );
        let rows = assert_matches_mirror(MEASURES, query);
        assert_eq!(rows.len(), kept_groups, "{having}: {rows:?}");
    }
}

/// `postgresql-pushdown`, scenario "SUM over a real column accumulates in
/// double precision": group `b` of `float_value`, a `real` column, holds
/// `16777216` and `1`, whose single-precision sum rounds back to `16777216`.
/// The delegated `SUM` casts the operand to `DOUBLE PRECISION` and returns
/// `16777217`, with the mean `8388608.5`, as the mirror does.
#[test]
fn sum_over_a_real_column_accumulates_in_double_precision() {
    let query = |schema: &str, measures: &str| {
        format!(
            "SELECT \"grp\", SUM(\"float_value\") AS S, AVG(\"float_value\") AS A \
             FROM {schema}.\"{measures}\" GROUP BY \"grp\" ORDER BY \"grp\""
        )
    };

    let statement = generated_statement(&query(VIRTUAL_SCHEMA, MEASURES));
    assert!(
        statement.contains("SUM(CAST(\"float_value\" AS DOUBLE PRECISION))"),
        "a `real` operand must be summed in double precision: {statement}"
    );
    let rows = assert_matches_mirror(MEASURES, query);
    let group_b = &rows[1];
    assert_eq!(
        cell(group_b, "S").and_then(|sum| sum.parse::<f64>().ok()),
        Some(16_777_217.0),
        "{group_b:?}"
    );
    assert_eq!(
        cell(group_b, "A").and_then(|mean| mean.parse::<f64>().ok()),
        Some(8_388_608.5),
        "{group_b:?}"
    );
}

/// `pushdown-aggregation-delegation`: a `NaN` in a `double precision` operand
/// makes the delegated `SUM` and `AVG` null, because PostgreSQL's sum is `NaN`
/// and Exasol reads a `NaN` as null. Group `a` of `nan_measures` holds `NaN`
/// beside a finite value, and group `b` holds finite values only. The mirror
/// cannot hold the row, so the answers are stated by hand.
#[test]
fn a_nan_operand_makes_the_delegated_sum_and_avg_null() {
    let aggregates = "SUM(\"value\") AS S, AVG(\"value\") AS A";
    let single = format!("SELECT {aggregates} FROM {VIRTUAL_SCHEMA}.\"{NAN_MEASURES}\"");
    let grouped = format!(
        "SELECT \"grp\", {aggregates} FROM {VIRTUAL_SCHEMA}.\"{NAN_MEASURES}\" \
         GROUP BY \"grp\" ORDER BY \"grp\""
    );

    assert!(generated_statement(&single).contains("SUM("));
    let exasol = &stack::shared().exasol;
    let single_rows = exasol.query_rows(&single);
    assert_eq!(cell(&single_rows[0], "S"), None, "{single_rows:?}");
    assert_eq!(cell(&single_rows[0], "A"), None, "{single_rows:?}");

    assert!(generated_statement(&grouped).contains("GROUP BY"));
    let rows = exasol.query_rows(&grouped);
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert_eq!(cell(&rows[0], "S"), None, "group a holds the NaN: {rows:?}");
    assert_eq!(cell(&rows[0], "A"), None, "group a holds the NaN: {rows:?}");
    let number = |row: &serde_json::Map<String, serde_json::Value>, name: &str| {
        cell(row, name).and_then(|value| value.parse::<f64>().ok())
    };
    assert_eq!(number(&rows[1], "S"), Some(7.0), "{rows:?}");
    assert_eq!(number(&rows[1], "A"), Some(3.5), "{rows:?}");
}

// --- what is deliberately not delegated -------------------------------------

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

/// `items` mixes mapped columns with one array column (`docker/postgres/
/// init.sql`), so the array must be left out of the virtual table and recorded
/// under its own PostgreSQL type and reason while every other column of the
/// table maps, and the wholly unmapped `all_unmapped` table must be absent from
/// `schemaMetadata` entirely rather than exposed thin, with each of its array
/// columns recorded. The shared `PGVS` schema already having been created over
/// both is this test's proof that creation still succeeds despite the
/// omissions.
#[test]
fn unmapped_items_columns_are_recorded_and_all_unmapped_table_is_absent() {
    let columns = column_names(VIRTUAL_SCHEMA, "items");
    let notes = adapter_notes(VIRTUAL_SCHEMA);
    let omitted = notes["omitted"]
        .as_array()
        .expect("adapter notes carry an omitted array");

    for column in ["code", "description", "details", "photo", "uid", "seen_at"] {
        assert!(
            columns.contains(&column.to_string()),
            "{column} is mapped and must appear in the virtual table: {columns:?}"
        );
    }
    assert!(
        !columns.contains(&"labels".to_string()),
        "the array column must not appear in the virtual table: {columns:?}"
    );

    for (table, column) in [
        ("items", "labels"),
        ("all_unmapped", "tags"),
        ("all_unmapped", "numbers"),
    ] {
        let entry = omitted
            .iter()
            .find(|entry| entry["table"] == table && entry["column"] == column)
            .unwrap_or_else(|| {
                panic!("the omitted {table}.{column} column must be recorded: {omitted:?}")
            });
        assert_eq!(
            entry["sourceType"], "ARRAY",
            "the recorded sourceType must be PostgreSQL's own name for the column's type: \
             {entry}"
        );
        assert!(
            entry["reason"]
                .as_str()
                .is_some_and(|reason| reason.contains("ARRAY")),
            "the omission must carry a reason naming ARRAY: {entry}"
        );
    }
    assert_eq!(
        omitted
            .iter()
            .filter(|entry| entry["column"].is_string())
            .count(),
        3,
        "no column but the three arrays is omitted: {omitted:?}"
    );

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
/// `ALTER TABLE`, so no other test's fixture is disturbed by it. The refresh
/// runs against a schema of its own that supplies the shared schema's driver
/// library, so it never reaches the live registry. Columns are asserted only
/// after the `DROP COLUMN` has restored the source.
#[test]
fn refresh_picks_up_a_new_postgresql_column() {
    const REFRESH_SCHEMA: &str = "PGREFRESHVS";
    let stack = stack::shared();
    let library = schema::recorded_driver_path(&stack.exasol, VIRTUAL_SCHEMA);
    schema::create_virtual_schema_through(
        &stack.exasol,
        schema::supplied_driver_spec(
            REFRESH_SCHEMA,
            Driver::Postgres,
            stack::POSTGRES_CONNECTION,
            &library,
        ),
    );
    let before = declared_columns(REFRESH_SCHEMA, "refresh_target");

    stack
        .postgres
        .execute("ALTER TABLE refresh_target ADD COLUMN extra NUMERIC(9,3)");
    stack
        .exasol
        .execute(&format!("ALTER VIRTUAL SCHEMA {REFRESH_SCHEMA} REFRESH"));
    let after_add = declared_columns(REFRESH_SCHEMA, "refresh_target");

    stack
        .postgres
        .execute("ALTER TABLE refresh_target DROP COLUMN extra");
    stack
        .exasol
        .execute(&format!("ALTER VIRTUAL SCHEMA {REFRESH_SCHEMA} REFRESH"));
    let after_drop = declared_columns(REFRESH_SCHEMA, "refresh_target");
    schema::drop_virtual_schema(&stack.exasol, REFRESH_SCHEMA);

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
        "the second refresh must leave the virtual schema declaring exactly the columns the \
         source still has"
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
    let library = schema::recorded_driver_path(&stack.exasol, VIRTUAL_SCHEMA);

    let bad_connection = format!("ADBC_VS_PG_{scenario}_BAD");
    stack.exasol.execute(&format!(
        "CREATE OR REPLACE CONNECTION {bad_connection} TO {} USER {} IDENTIFIED BY {}",
        schema::quote_literal(address),
        schema::quote_literal(postgres::USER),
        schema::quote_literal(password)
    ));

    let create_schema_name = format!("PGFAIL_{scenario}_CREATE");
    let create_error = schema::create_virtual_schema_through_expecting_failure(
        &stack.exasol,
        schema::supplied_driver_spec(
            &create_schema_name,
            Driver::Postgres,
            &bad_connection,
            &library,
        ),
    );

    let working_connection = format!("ADBC_VS_PG_{scenario}_WORK");
    schema::create_postgres_connection(&stack.exasol, &working_connection, postgres::DATABASE);
    let working_schema = format!("PGFAIL_{scenario}_WORK");
    schema::create_virtual_schema_through(
        &stack.exasol,
        schema::supplied_driver_spec(
            &working_schema,
            Driver::Postgres,
            &working_connection,
            &library,
        ),
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

/// `type_coverage` (`docker/postgres/init.sql`) holds one column of every
/// mapped type, and the mirror holds the values Exasol reads for each. A test
/// below compares a delegated query over it with the same query over the
/// mirror, and checks each column the declared Exasol type the mirror declares
/// for it, so a conversion that read the wrong value or declared the wrong type
/// shows as a difference.
fn assert_declares(table: &str, column: &str, exasol_type: &str) {
    let declared = declared_columns(VIRTUAL_SCHEMA, table);
    assert!(
        declared.contains(&(column.to_string(), exasol_type.to_string())),
        "{table}.{column} must be declared {exasol_type}: {declared:?}"
    );
}

/// Narrow integers map to `DECIMAL` of their own width's digits and read bare.
#[test]
fn smallint_column_matches_the_mirror() {
    assert_declares(TYPE_COVERAGE, "small_id", "DECIMAL(5,0)");

    let query = |schema: &str, table: &str| {
        format!(
            "SELECT \"id\", \"small_id\" FROM {schema}.\"{table}\" WHERE \"small_id\" >= 2 \
             ORDER BY \"id\""
        )
    };

    let statement = generated_statement(&query(VIRTUAL_SCHEMA, TYPE_COVERAGE));
    assert!(
        statement.contains("WHERE")
            && statement.contains("\"small_id\"")
            && !statement.contains("CAST"),
        "the filter must reach PostgreSQL over the bare column: {statement}"
    );

    let rows = assert_matches_mirror(TYPE_COVERAGE, query);
    assert_eq!(
        column_values(&rows, "small_id"),
        strings(&["2", "3", "4", "5"])
    );
}

/// A `numeric` beyond Exasol's 36 digits, and one with no declared precision,
/// map to `VARCHAR` read as PostgreSQL's text, which keeps every digit, the
/// trailing zeros, and the values no `DECIMAL` holds.
#[test]
fn wide_and_unconstrained_numeric_read_as_text_like_the_mirror() {
    assert_declares(TYPE_COVERAGE, "unbounded_amount", "VARCHAR(2000000) UTF8");
    assert_declares(TYPE_COVERAGE, "wide_amount", "VARCHAR(2000000) UTF8");

    let query = |schema: &str, table: &str| {
        format!(
            "SELECT \"id\", \"unbounded_amount\", \"wide_amount\" FROM {schema}.\"{table}\" \
             ORDER BY \"id\""
        )
    };

    let statement = generated_statement(&query(VIRTUAL_SCHEMA, TYPE_COVERAGE));
    assert!(
        statement.contains("CAST(\"unbounded_amount\" AS TEXT)")
            && statement.contains("CAST(\"wide_amount\" AS TEXT)"),
        "both columns must read as text in the select list: {statement}"
    );

    let rows = assert_matches_mirror(TYPE_COVERAGE, query);
    assert_eq!(
        column_values(&rows, "unbounded_amount")[..4],
        strings(&["1.0", "1.00", "NaN", "Infinity"]),
        "1.0 and 1.00 are two distinct values, and NaN and Infinity come back as text rather \
         than failing the query: {rows:?}"
    );
    assert_eq!(
        cell(&rows[4], "unbounded_amount").map(|text| text.len()),
        Some(401),
        "1e400 comes back as its 401 digits: {rows:?}"
    );
    assert_eq!(
        cell(&rows[0], "wide_amount"),
        Some("12345678901234567890123456789012345678.90".to_string())
    );

    let equality = |schema: &str, table: &str| {
        format!(
            "SELECT \"id\" FROM {schema}.\"{table}\" WHERE \"unbounded_amount\" = '1.0' \
             ORDER BY \"id\""
        )
    };
    let equality_statement = generated_statement(&equality(VIRTUAL_SCHEMA, TYPE_COVERAGE));
    assert!(
        equality_statement.contains("(CAST(\"unbounded_amount\" AS TEXT) = E'1.0')"),
        "the equality must reach PostgreSQL over the text: {equality_statement}"
    );
    let rows = assert_matches_mirror(TYPE_COVERAGE, equality);
    assert_eq!(
        column_values(&rows, "id"),
        strings(&["1"]),
        "an equality with '1.0' selects the row holding 1.0 and not the one holding 1.00: \
         {rows:?}"
    );

    let specials = stack::shared().exasol.query_rows(&format!(
        "SELECT \"unbounded_amount\" FROM {VIRTUAL_SCHEMA}.\"numeric_specials\" ORDER BY \"id\""
    ));
    assert_eq!(
        specials
            .iter()
            .map(|row| cell(row, "unbounded_amount"))
            .collect::<Vec<_>>(),
        vec![
            Some("100".to_string()),
            None,
            Some("Infinity".to_string()),
            Some("-Infinity".to_string()),
        ],
        "the infinite values of an unconstrained numeric come back as PostgreSQL's text"
    );
}

/// `SELECT *` reads every column through its own read expression, and the whole
/// row is what the mirror holds.
#[test]
fn select_star_over_type_coverage_matches_the_mirror() {
    let query =
        |schema: &str, table: &str| format!("SELECT * FROM {schema}.\"{table}\" ORDER BY \"id\"");

    let statement = generated_statement(&query(VIRTUAL_SCHEMA, TYPE_COVERAGE));
    for fragment in [
        "CAST(\"uid\" AS TEXT)",
        "(position(B'1' IN \"bits\") > 0)",
        "CAST(CAST(\"price\" AS NUMERIC) AS DOUBLE PRECISION)",
        "CAST(\"wide_code\" AS TEXT)",
    ] {
        assert!(
            statement.contains(fragment),
            "the select list must carry {fragment}: {statement}"
        );
    }

    let rows = assert_matches_mirror(TYPE_COVERAGE, query);
    assert_eq!(rows.len(), 5);
    assert_eq!(
        rows[0].len(),
        28,
        "every mapped column comes back: {:?}",
        rows[0]
    );
}

/// A value read as text compares as the text the scan emits, so a comparison
/// PostgreSQL's own type would answer differently follows the text instead.
#[test]
fn values_read_as_text_compare_as_the_emitted_text() {
    const JSON_TEXT: &str = r#"{"a":   1,  "b": [1,2]}"#;
    const JSON_NORMALIZED: &str = r#"{"a": 1, "b": [1,2]}"#;
    const JSONB_SORTED: &str = r#"{"a": 2, "b": 1}"#;
    const JSONB_UNSORTED: &str = r#"{"b": 1, "a": 2}"#;
    const UUID_UPPER: &str = "A0EEBC99-9C0B-4EF8-BB6D-6BB9BD380A11";
    const UUID_LOWER: &str = "a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11";

    for (column, literal, expected_ids) in [
        ("uid", UUID_UPPER, &[][..]),
        ("uid", UUID_LOWER, &["1"][..]),
        ("doc_json", JSON_TEXT, &["1"][..]),
        ("doc_json", JSON_NORMALIZED, &[][..]),
        ("doc_xml", "<a>1</a>", &["1"][..]),
        ("doc_jsonb", JSONB_SORTED, &["1", "2"][..]),
        ("doc_jsonb", JSONB_UNSORTED, &[][..]),
    ] {
        let query = |schema: &str, table: &str| {
            format!(
                "SELECT \"id\" FROM {schema}.\"{table}\" WHERE \"{column}\" = '{literal}' \
                 ORDER BY \"id\""
            )
        };

        let statement = generated_statement(&query(VIRTUAL_SCHEMA, TYPE_COVERAGE));
        assert!(
            statement.contains(&format!("(CAST(\"{column}\" AS TEXT) = E'")),
            "the equality over {column} must compare the text: {statement}"
        );
        let rows = assert_matches_mirror(TYPE_COVERAGE, query);
        assert_eq!(
            column_values(&rows, "id"),
            strings(expected_ids),
            "{column} = '{literal}' compares the emitted text: {rows:?}"
        );
    }
}

/// A `timestamp(6)` keeps all six fractional digits through the scan and
/// through a delegated comparison with a timestamp literal.
#[test]
fn microsecond_timestamp_keeps_six_digits_like_the_mirror() {
    assert_declares(TYPE_COVERAGE, "precise_at", "TIMESTAMP(6)");
    assert_declares(TYPE_COVERAGE, "whole_at", "TIMESTAMP(0)");

    let digits = |schema: &str, table: &str| {
        format!(
            "SELECT \"id\", TO_CHAR(\"precise_at\", 'YYYY-MM-DD HH24:MI:SS.FF6') AS TEXT_AT \
             FROM {schema}.\"{table}\" WHERE \"precise_at\" IS NOT NULL ORDER BY \"id\""
        )
    };
    let rows = assert_matches_mirror(TYPE_COVERAGE, digits);
    assert_eq!(
        column_values(&rows, "TEXT_AT"),
        strings(&[
            "2024-01-01 12:00:00.123456",
            "2024-01-01 12:00:00.123457",
            "2024-01-01 12:00:00.123456",
        ])
    );

    let equality = |schema: &str, table: &str| {
        format!(
            "SELECT \"id\" FROM {schema}.\"{table}\" \
             WHERE \"precise_at\" = TIMESTAMP '2024-01-01 12:00:00.123456' ORDER BY \"id\""
        )
    };
    let statement = generated_statement(&equality(VIRTUAL_SCHEMA, TYPE_COVERAGE));
    assert!(
        statement.contains(".123456"),
        "the literal must reach PostgreSQL with its six fractional digits: {statement}"
    );
    let rows = assert_matches_mirror(TYPE_COVERAGE, equality);
    assert_eq!(
        column_values(&rows, "id"),
        strings(&["1", "4"]),
        "the equality selects the two rows holding .123456 and not the one holding .123457: \
         {rows:?}"
    );
}

/// A `real` is emitted as its single-precision value widened to double, so a
/// stored `0.1` equals no decimal literal `0.1`, and `0.5`, which a
/// single-precision value holds exactly, still equals its literal.
#[test]
fn real_column_holds_the_widened_single_precision_value() {
    assert_declares(TYPE_COVERAGE, "approx", "DOUBLE");

    let projection = |schema: &str, table: &str| {
        format!(
            "SELECT \"id\", \"approx\" FROM {schema}.\"{table}\" WHERE \"approx\" IS NOT NULL \
             ORDER BY \"id\""
        )
    };
    let rows = assert_matches_mirror(TYPE_COVERAGE, projection);
    let widened: f64 = cell(&rows[0], "approx")
        .expect("row 1 holds a real")
        .parse()
        .expect("Exasol returns a DOUBLE as a number");
    assert_eq!(widened, f64::from(0.1_f32), "{rows:?}");

    for (literal, expected_ids) in [("0.1", &[][..]), ("0.5", &["2"][..])] {
        let equality = |schema: &str, table: &str| {
            format!(
                "SELECT \"id\" FROM {schema}.\"{table}\" WHERE \"approx\" = {literal} \
                 ORDER BY \"id\""
            )
        };
        let statement = generated_statement(&equality(VIRTUAL_SCHEMA, TYPE_COVERAGE));
        assert!(
            statement.contains(&format!("(\"approx\" = {literal})")),
            "the equality must reach PostgreSQL: {statement}"
        );
        let rows = assert_matches_mirror(TYPE_COVERAGE, equality);
        assert_eq!(
            column_values(&rows, "id"),
            strings(expected_ids),
            "{rows:?}"
        );
    }
}

/// `boolean` reads bare and `bit` of every width reads as whether any bit is
/// set: a delegated filter, grouping, `MIN`, `MAX`, and `COUNT(DISTINCT)` over
/// each column return what Exasol returns over the emitted `BOOLEAN` values.
#[test]
fn boolean_and_bit_columns_filter_group_and_aggregate_like_the_mirror() {
    for column in ["flag", "other_flag", "bit_flag", "bits"] {
        assert_declares(TYPE_COVERAGE, column, "BOOLEAN");
    }

    for (predicate, fragment, expected_ids) in [
        ("\"flag\"", "WHERE \"flag\"", &["1", "4"][..]),
        ("NOT \"flag\"", "(NOT \"flag\")", &["2", "5"][..]),
        (
            "\"bit_flag\"",
            "WHERE (position(B'1' IN \"bit_flag\") > 0)",
            &["1", "4"][..],
        ),
        (
            "\"bits\"",
            "WHERE (position(B'1' IN \"bits\") > 0)",
            &["1", "2", "4"][..],
        ),
        (
            "NOT \"bits\"",
            "(NOT (position(B'1' IN \"bits\") > 0))",
            &["3"][..],
        ),
        (
            "\"flag\" = \"other_flag\"",
            "(\"flag\" = \"other_flag\")",
            &["1", "5"][..],
        ),
        (
            "\"bits\" = \"flag\"",
            "((position(B'1' IN \"bits\") > 0) = \"flag\")",
            &["1", "4"][..],
        ),
    ] {
        let query = |schema: &str, table: &str| {
            format!("SELECT \"id\" FROM {schema}.\"{table}\" WHERE {predicate} ORDER BY \"id\"")
        };

        let statement = generated_statement(&query(VIRTUAL_SCHEMA, TYPE_COVERAGE));
        assert!(
            statement.contains(fragment),
            "`WHERE {predicate}` must reach PostgreSQL as {fragment}: {statement}"
        );
        let rows = assert_matches_mirror(TYPE_COVERAGE, query);
        assert_eq!(
            column_values(&rows, "id"),
            strings(expected_ids),
            "`WHERE {predicate}`: {rows:?}"
        );
    }

    let aggregates = |schema: &str, table: &str| {
        format!(
            "SELECT MIN(\"flag\") AS MIN_FLAG, MAX(\"flag\") AS MAX_FLAG, \
             MIN(\"bit_flag\") AS MIN_BIT, MAX(\"bit_flag\") AS MAX_BIT, \
             MIN(\"bits\") AS MIN_BITS, MAX(\"bits\") AS MAX_BITS, \
             COUNT(DISTINCT \"bits\") AS DISTINCT_BITS \
             FROM {schema}.\"{table}\""
        )
    };
    let statement = generated_statement(&aggregates(VIRTUAL_SCHEMA, TYPE_COVERAGE));
    for fragment in [
        "bool_and(\"flag\")",
        "bool_or(\"flag\")",
        "bool_and((position(B'1' IN \"bit_flag\") > 0))",
        "bool_or((position(B'1' IN \"bits\") > 0))",
        "COUNT(DISTINCT (position(B'1' IN \"bits\") > 0))",
    ] {
        assert!(
            statement.contains(fragment),
            "the aggregates must reach PostgreSQL as {fragment}: {statement}"
        );
    }
    for forbidden in ["MIN(", "MAX("] {
        assert!(
            !statement.contains(forbidden),
            "PostgreSQL defines no {forbidden}boolean): {statement}"
        );
    }
    let rows = assert_matches_mirror(TYPE_COVERAGE, aggregates);
    let answers = &rows[0];
    for (name, expected) in [
        ("MIN_FLAG", "false"),
        ("MAX_FLAG", "true"),
        ("MIN_BIT", "false"),
        ("MAX_BIT", "true"),
        ("MIN_BITS", "false"),
        ("MAX_BITS", "true"),
        ("DISTINCT_BITS", "2"),
    ] {
        assert_eq!(
            cell(answers, name),
            Some(expected.to_string()),
            "{name}: {answers:?}"
        );
    }

    for column in ["flag", "bits"] {
        let grouped = |schema: &str, table: &str| {
            format!(
                "SELECT \"{column}\" AS GROUP_KEY, COUNT(*) AS N FROM {schema}.\"{table}\" \
                 GROUP BY \"{column}\" ORDER BY \"{column}\""
            )
        };
        let statement = generated_statement(&grouped(VIRTUAL_SCHEMA, TYPE_COVERAGE));
        assert!(
            statement.contains("GROUP BY"),
            "the grouping over {column} must reach PostgreSQL: {statement}"
        );
        let rows = assert_matches_mirror(TYPE_COVERAGE, grouped);
        assert_eq!(
            rows.len(),
            3,
            "false, true, and null are three groups: {rows:?}"
        );
    }
    let bits_groups = stack::shared().exasol.query_rows(&format!(
        "SELECT \"bits\" AS GROUP_KEY, COUNT(*) AS N FROM {VIRTUAL_SCHEMA}.\"{TYPE_COVERAGE}\" \
         GROUP BY \"bits\""
    ));
    let true_group = bits_groups
        .iter()
        .find(|row| cell(row, "GROUP_KEY") == Some("true".to_string()))
        .expect("a group of the set-bit values");
    assert_eq!(
        cell(true_group, "N"),
        Some("3".to_string()),
        "00000101 and 00001001 are one group: {bits_groups:?}"
    );
}

/// Exasol delegates a comparison of a `BOOLEAN` column with a predicate, with
/// the predicate's operands swapped, and the dialect renders it parenthesized.
#[test]
fn boolean_column_compared_with_a_predicate_matches_the_mirror() {
    let query = |schema: &str, table: &str| {
        format!(
            "SELECT \"id\" FROM {schema}.\"{table}\" WHERE \"flag\" = (\"small_id\" > 2) \
             ORDER BY \"id\""
        )
    };

    let statement = generated_statement(&query(VIRTUAL_SCHEMA, TYPE_COVERAGE));
    assert!(
        statement.contains("(\"flag\" = (2 < \"small_id\"))"),
        "the comparison must reach PostgreSQL with its predicate operand parenthesized: \
         {statement}"
    );
    let rows = assert_matches_mirror(TYPE_COVERAGE, query);
    assert_eq!(column_values(&rows, "id"), strings(&["2", "4"]), "{rows:?}");
}

/// `money` is read through `numeric` as a `DOUBLE`, so a comparison with a
/// decimal literal selects the row Exasol selects.
#[test]
fn money_column_compares_as_double_like_the_mirror() {
    assert_declares(TYPE_COVERAGE, "price", "DOUBLE");

    let query = |schema: &str, table: &str| {
        format!(
            "SELECT \"id\", \"price\" FROM {schema}.\"{table}\" WHERE \"price\" = 12.34 \
             ORDER BY \"id\""
        )
    };

    let statement = generated_statement(&query(VIRTUAL_SCHEMA, TYPE_COVERAGE));
    assert!(
        statement.contains("(CAST(CAST(\"price\" AS NUMERIC) AS DOUBLE PRECISION) = 12.34)"),
        "the comparison must reach PostgreSQL over the read expression: {statement}"
    );
    let rows = assert_matches_mirror(TYPE_COVERAGE, query);
    assert_eq!(column_values(&rows, "id"), strings(&["1"]), "{rows:?}");
    assert_eq!(
        cell(&rows[0], "price"),
        Some("12.34".to_string()),
        "{rows:?}"
    );
}

/// Every type without an Exasol counterpart comes back as PostgreSQL's text for
/// the value, at the size its mapping declares.
#[test]
fn text_read_types_match_the_mirror() {
    for (column, exasol_type) in [
        ("uid", "VARCHAR(2000000) UTF8"),
        ("doc_json", "VARCHAR(2000000) UTF8"),
        ("doc_jsonb", "VARCHAR(2000000) UTF8"),
        ("doc_xml", "VARCHAR(2000000) UTF8"),
        ("raw", "VARCHAR(2000000) UTF8"),
        ("span", "VARCHAR(2000000) UTF8"),
        ("host", "VARCHAR(2000000) UTF8"),
        ("varbits", "VARCHAR(16) UTF8"),
        ("feeling", "VARCHAR(2000000) UTF8"),
        ("clock", "VARCHAR(100) UTF8"),
        ("clock_tz", "VARCHAR(100) UTF8"),
    ] {
        assert_declares(TYPE_COVERAGE, column, exasol_type);
    }

    let query = |schema: &str, table: &str| {
        format!(
            "SELECT \"id\", \"uid\", \"doc_json\", \"doc_jsonb\", \"doc_xml\", \"raw\", \"span\", \
             \"host\", \"varbits\", \"feeling\", \"clock\", \"clock_tz\" \
             FROM {schema}.\"{table}\" WHERE \"id\" <= 2 ORDER BY \"id\""
        )
    };

    let statement = generated_statement(&query(VIRTUAL_SCHEMA, TYPE_COVERAGE));
    for column in ["raw", "feeling", "span", "host", "varbits", "clock_tz"] {
        assert!(
            statement.contains(&format!("CAST(\"{column}\" AS TEXT)")),
            "{column} must read as text: {statement}"
        );
    }
    let rows = assert_matches_mirror(TYPE_COVERAGE, query);
    assert_eq!(
        cell(&rows[0], "raw"),
        Some("\\xdeadbeef".to_string()),
        "a bytea comes back in the hex form bytea_output selects by default: {rows:?}"
    );
    assert_eq!(
        cell(&rows[0], "feeling"),
        Some("ok".to_string()),
        "{rows:?}"
    );
}

/// The text a column of `character(3000)` reads as, `CAST("wide_code" AS
/// TEXT)`, carries no trailing blanks, so it compares with `'ab'` as Exasol's
/// `VARCHAR` does.
#[test]
fn character_column_emits_padded_and_compares_like_the_source() {
    assert_declares(TYPE_COVERAGE, "fixed_code", "CHAR(5) UTF8");
    assert_declares(TYPE_COVERAGE, "wide_code", "VARCHAR(3000) UTF8");

    let projection = |schema: &str, table: &str| {
        format!(
            "SELECT \"id\", \"fixed_code\", \"wide_code\" FROM {schema}.\"{table}\" \
             WHERE \"fixed_code\" IS NOT NULL ORDER BY \"id\""
        )
    };
    let rows = assert_matches_mirror(TYPE_COVERAGE, projection);
    assert_eq!(
        cell(&rows[0], "fixed_code"),
        Some("ab   ".to_string()),
        "a CHAR(5) value comes back padded to five characters: {rows:?}"
    );
    assert_eq!(
        cell(&rows[0], "wide_code"),
        Some("ab".to_string()),
        "a character wider than Exasol's CHAR comes back without its blanks: {rows:?}"
    );

    for (predicate, expected_ids) in [
        ("\"fixed_code\" = 'ab'", &["1", "4"][..]),
        ("\"wide_code\" = 'ab'", &["1", "4"][..]),
    ] {
        let query = |schema: &str, table: &str| {
            format!("SELECT \"id\" FROM {schema}.\"{table}\" WHERE {predicate} ORDER BY \"id\"")
        };
        let statement = generated_statement(&query(VIRTUAL_SCHEMA, TYPE_COVERAGE));
        assert!(
            statement.contains("= E'ab')"),
            "`{predicate}` must reach PostgreSQL: {statement}"
        );
        let rows = assert_matches_mirror(TYPE_COVERAGE, query);
        assert_eq!(
            column_values(&rows, "id"),
            strings(expected_ids),
            "{predicate}: {rows:?}"
        );
    }

    let with_varchar = |schema: &str, table: &str| {
        format!(
            "SELECT \"id\" FROM {schema}.\"{table}\" WHERE \"fixed_code\" = \"label\" \
             ORDER BY \"id\""
        )
    };
    let statement = generated_statement(&with_varchar(VIRTUAL_SCHEMA, TYPE_COVERAGE));
    assert!(
        statement.contains("(\"fixed_code\" = \"label\")"),
        "the comparison must reach PostgreSQL: {statement}"
    );
    let query_rows = |schema: &str| {
        stack::shared()
            .exasol
            .query_rows(&with_varchar(schema, TYPE_COVERAGE))
    };
    assert_eq!(
        column_values(&query_rows(VIRTUAL_SCHEMA), "id"),
        strings(&["1", "4"]),
        "PostgreSQL ignores the trailing blanks of a character value"
    );
    assert!(
        query_rows(postgres_mirror::SCHEMA).is_empty(),
        "Exasol's own comparison of CHAR(5) with VARCHAR finds them unequal"
    );

    let like = |schema: &str, table: &str| {
        format!(
            "SELECT \"id\" FROM {schema}.\"{table}\" WHERE \"fixed_code\" LIKE 'ab' \
             ORDER BY \"id\""
        )
    };
    let like_statement = generated_statement(&like(VIRTUAL_SCHEMA, TYPE_COVERAGE));
    assert!(
        like_statement.contains("LIKE E'ab'"),
        "the LIKE must reach PostgreSQL: {like_statement}"
    );
    let like_rows = assert_matches_mirror(TYPE_COVERAGE, like);
    assert!(
        like_rows.is_empty(),
        "measured on PostgreSQL 18.6, a `character(5)` value `ab` does not match `LIKE 'ab'`, \
         where it equals `'ab'`: the padding is significant to LIKE but not to equality: \
         {like_rows:?}"
    );
}

/// `MIN` and `MAX` over a `character(5)` are declared `CHAR(5)` and return the
/// padded values, and the grouping and the distinct count over it are
/// Exasol's.
#[test]
fn min_and_max_over_char_are_declared_char_like_the_mirror() {
    let aggregates = |schema: &str, table: &str| {
        format!(
            "SELECT MIN(\"fixed_code\") AS LO, MAX(\"fixed_code\") AS HI, \
             COUNT(DISTINCT \"fixed_code\") AS N FROM {schema}.\"{table}\""
        )
    };

    let call = explain_calls(&aggregates(VIRTUAL_SCHEMA, TYPE_COVERAGE)).remove(0);
    let specification = scan_specification(&call);
    assert_eq!(
        specification.columns[0].exasol_type,
        ExasolType::Char { size: 5 },
        "MIN over a CHAR(5) must be declared CHAR(5): {call}"
    );
    assert!(
        specification.sql.contains("MIN(\"fixed_code\")")
            && specification.sql.contains("MAX(\"fixed_code\")"),
        "both aggregates must reach PostgreSQL: {}",
        specification.sql
    );

    let rows = assert_matches_mirror(TYPE_COVERAGE, aggregates);
    assert_eq!(cell(&rows[0], "LO"), Some("ab   ".to_string()), "{rows:?}");
    assert_eq!(cell(&rows[0], "HI"), Some("abc  ".to_string()), "{rows:?}");
    assert_eq!(cell(&rows[0], "N"), Some("2".to_string()), "{rows:?}");

    let grouped = |schema: &str, table: &str| {
        format!(
            "SELECT \"fixed_code\" AS CODE, COUNT(*) AS N FROM {schema}.\"{table}\" \
             GROUP BY \"fixed_code\" ORDER BY \"fixed_code\""
        )
    };
    let statement = generated_statement(&grouped(VIRTUAL_SCHEMA, TYPE_COVERAGE));
    assert!(
        statement.contains("GROUP BY \"fixed_code\""),
        "the grouping must reach PostgreSQL: {statement}"
    );
    let rows = assert_matches_mirror(TYPE_COVERAGE, grouped);
    assert_eq!(
        rows.len(),
        3,
        "ab, abc, and null are three groups: {rows:?}"
    );
}

/// A join on a `character` key pairs rows as PostgreSQL pairs them: with a
/// `character varying` key it ignores the blanks, which Exasol's own join does
/// not, and with another `character` key it agrees with Exasol's.
#[test]
fn char_key_joins_pair_rows_as_postgresql_pairs_them() {
    let join = |left: &'static str, right: &'static str| {
        move |schema: &str, table: &str| {
            format!(
                "SELECT a.\"id\" AS A_ID, b.\"id\" AS B_ID FROM {schema}.\"{table}\" a \
                 INNER JOIN {schema}.\"{table}\" b ON a.\"{left}\" = b.\"{right}\" \
                 ORDER BY a.\"id\", b.\"id\""
            )
        }
    };

    let mixed = join("fixed_code", "label");
    let statement = generated_statement(&mixed(VIRTUAL_SCHEMA, TYPE_COVERAGE));
    assert!(
        statement.contains("INNER JOIN")
            && statement.contains("(\"t1\".\"fixed_code\" = \"t2\".\"label\")"),
        "the join must be delegated whole: {statement}"
    );
    let virtual_pairs = stack::shared()
        .exasol
        .query_rows(&mixed(VIRTUAL_SCHEMA, TYPE_COVERAGE));
    assert_eq!(
        virtual_pairs.len(),
        6,
        "rows 1 and 4 hold `ab` in both columns, and each pairs with rows 1, 2, and 4 of the \
         `character varying` column: {virtual_pairs:?}"
    );
    assert!(
        stack::shared()
            .exasol
            .query_rows(&mixed(postgres_mirror::SCHEMA, TYPE_COVERAGE))
            .is_empty(),
        "Exasol's own join of CHAR(5) with VARCHAR(5) pairs nothing"
    );

    let same = join("fixed_code", "fixed_code");
    let pairs = assert_matches_mirror(TYPE_COVERAGE, same);
    assert_eq!(
        pairs.len(),
        5,
        "rows 1 and 4 pair with each other and themselves, and row 2 with itself: {pairs:?}"
    );
}

/// A `timestamp with time zone` is a `TIMESTAMP` holding the UTC wall-clock
/// value, and under a session `TimeZone` of UTC every delegated comparison,
/// join, and aggregate over it answers as Exasol does over the emitted values.
#[test]
fn timestamptz_column_compares_groups_and_joins_like_the_mirror_under_a_utc_session() {
    let zone = stack::shared().postgres.query_scalar("SHOW TimeZone");
    assert!(
        ["UTC", "Etc/UTC"].contains(&zone.as_str()),
        "the PostgreSQL session must run in UTC for this test to compare with the mirror, but \
         `SHOW TimeZone` reports {zone}"
    );
    assert_declares(TYPE_COVERAGE, "happened_at", "TIMESTAMP(6)");

    let projection = |schema: &str, table: &str| {
        format!(
            "SELECT \"id\", TO_CHAR(\"happened_at\", 'YYYY-MM-DD HH24:MI:SS.FF6') AS TEXT_AT \
             FROM {schema}.\"{table}\" WHERE \"happened_at\" IS NOT NULL ORDER BY \"id\""
        )
    };
    let rows = assert_matches_mirror(TYPE_COVERAGE, projection);
    assert_eq!(
        cell(&rows[0], "TEXT_AT"),
        Some("2024-01-01 12:00:00.123456".to_string()),
        "each value is emitted as its UTC wall-clock time: {rows:?}"
    );

    for (predicate, expected_ids) in [
        (
            "\"happened_at\" = TIMESTAMP '2024-01-01 12:00:00.123456'",
            &["1"][..],
        ),
        ("\"happened_at\" < DATE '2024-01-03'", &["1", "2"][..]),
    ] {
        let query = |schema: &str, table: &str| {
            format!("SELECT \"id\" FROM {schema}.\"{table}\" WHERE {predicate} ORDER BY \"id\"")
        };
        let statement = generated_statement(&query(VIRTUAL_SCHEMA, TYPE_COVERAGE));
        assert!(
            statement.contains("WHERE (\"happened_at\""),
            "`{predicate}` must reach PostgreSQL: {statement}"
        );
        let rows = assert_matches_mirror(TYPE_COVERAGE, query);
        assert_eq!(
            column_values(&rows, "id"),
            strings(expected_ids),
            "{predicate}: {rows:?}"
        );
    }

    let join = |schema: &str, table: &str| {
        format!(
            "SELECT a.\"id\" AS A_ID, b.\"id\" AS B_ID FROM {schema}.\"{table}\" a \
             INNER JOIN {schema}.\"{table}\" b ON a.\"happened_at\" = b.\"local_at\" \
             ORDER BY a.\"id\", b.\"id\""
        )
    };
    let join_statement = generated_statement(&join(VIRTUAL_SCHEMA, TYPE_COVERAGE));
    assert!(
        join_statement.contains("INNER JOIN"),
        "the join with a timestamp without time zone column must be delegated: {join_statement}"
    );
    let pairs = assert_matches_mirror(TYPE_COVERAGE, join);
    assert_eq!(
        pairs
            .iter()
            .map(|row| (cell(row, "A_ID"), cell(row, "B_ID")))
            .collect::<Vec<_>>(),
        vec![
            (Some("1".to_string()), Some("1".to_string())),
            (Some("4".to_string()), Some("4".to_string())),
        ],
        "only the rows whose local_at holds happened_at's UTC wall-clock value pair"
    );

    let aggregates = |schema: &str, table: &str| {
        format!(
            "SELECT MIN(\"happened_at\") AS LO, MAX(\"happened_at\") AS HI, \
             COUNT(DISTINCT \"happened_at\") AS N FROM {schema}.\"{table}\""
        )
    };
    let statement = generated_statement(&aggregates(VIRTUAL_SCHEMA, TYPE_COVERAGE));
    assert!(
        statement.contains("MIN(\"happened_at\")") && statement.contains("COUNT(DISTINCT"),
        "the aggregates must reach PostgreSQL: {statement}"
    );
    assert_eq!(
        cell(&assert_matches_mirror(TYPE_COVERAGE, aggregates)[0], "N"),
        Some("4".to_string())
    );

    let grouped = |schema: &str, table: &str| {
        format!(
            "SELECT \"happened_at\" AS HAPPENED, COUNT(*) AS N FROM {schema}.\"{table}\" \
             GROUP BY \"happened_at\" ORDER BY \"happened_at\""
        )
    };
    assert_eq!(assert_matches_mirror(TYPE_COVERAGE, grouped).len(), 5);
}

/// Grouping, aggregating, and joining on a converted column act on the text the
/// scan emits, in every position, and a full outer join keeps both unmatched
/// sides.
#[test]
fn converted_columns_group_join_and_aggregate_like_the_mirror() {
    let grouped = |schema: &str, table: &str| {
        format!(
            "SELECT \"uid\" AS UID, COUNT(*) AS N FROM {schema}.\"{table}\" GROUP BY \"uid\" \
             ORDER BY \"uid\""
        )
    };
    let statement = generated_statement(&grouped(VIRTUAL_SCHEMA, TYPE_COVERAGE));
    assert!(
        statement.contains("GROUP BY CAST(\"uid\" AS TEXT)"),
        "the grouping key must be the read expression: {statement}"
    );
    assert_eq!(
        assert_matches_mirror(TYPE_COVERAGE, grouped).len(),
        4,
        "three uuids and the null group"
    );

    let aggregates = |schema: &str, table: &str| {
        format!(
            "SELECT MIN(\"uid\") AS LO, MAX(\"uid\") AS HI, COUNT(DISTINCT \"uid\") AS N, \
             COUNT(DISTINCT \"unbounded_amount\") AS AMOUNTS FROM {schema}.\"{table}\""
        )
    };
    let statement = generated_statement(&aggregates(VIRTUAL_SCHEMA, TYPE_COVERAGE));
    assert!(
        statement.contains("MIN(CAST(\"uid\" AS TEXT))")
            && statement.contains("COUNT(DISTINCT CAST(\"unbounded_amount\" AS TEXT))"),
        "the aggregate operands must be the read expressions: {statement}"
    );
    let rows = assert_matches_mirror(TYPE_COVERAGE, aggregates);
    assert_eq!(cell(&rows[0], "N"), Some("3".to_string()), "{rows:?}");
    assert_eq!(cell(&rows[0], "AMOUNTS"), Some("5".to_string()), "{rows:?}");

    for (join_type, expected_rows) in [("INNER JOIN", 3), ("FULL OUTER JOIN", 7)] {
        let join = |schema: &str, table: &str| {
            format!(
                "SELECT a.\"id\" AS A_ID, b.\"id\" AS B_ID FROM {schema}.\"{table}\" a \
                 {join_type} {schema}.\"{table}\" b ON a.\"uid\" = b.\"uid\" \
                 ORDER BY a.\"id\", b.\"id\""
            )
        };
        let statement = generated_statement(&join(VIRTUAL_SCHEMA, TYPE_COVERAGE));
        assert!(
            statement.contains(join_type)
                && statement
                    .contains("CAST(\"t1\".\"uid\" AS TEXT) = CAST(\"t2\".\"uid\" AS TEXT)"),
            "the {join_type} must be delegated over the read expressions: {statement}"
        );
        let rows = assert_matches_mirror(TYPE_COVERAGE, join);
        assert_eq!(rows.len(), expected_rows, "{join_type}: {rows:?}");
    }
}

/// A column under an operator-created nondeterministic collation maps like any
/// other, and a delegated equality returns both values the collation equates,
/// where Exasol compares them as two.
#[test]
fn nondeterministic_collation_column_maps_and_compares_as_postgresql_compares() {
    assert_declares("collated_names", "name", "VARCHAR(20) UTF8");

    let query = format!(
        "SELECT \"id\" FROM {VIRTUAL_SCHEMA}.\"collated_names\" WHERE \"name\" = 'Widget' \
         ORDER BY \"id\""
    );
    let statement = generated_statement(&query);
    assert!(
        statement.contains("(\"name\" = E'Widget')") && !statement.contains("COLLATE"),
        "the equality must reach PostgreSQL bare: {statement}"
    );
    let rows = stack::shared().exasol.query_rows(&query);
    assert_eq!(
        column_values(&rows, "id"),
        strings(&["1", "2"]),
        "the collation equates Widget and widget: {rows:?}"
    );

    let local = stack::shared().exasol.query_rows(
        "SELECT \"id\" FROM (SELECT 1 AS \"id\", 'Widget' AS \"name\" FROM DUAL UNION ALL \
         SELECT 2, 'widget' FROM DUAL) WHERE \"name\" = 'Widget' ORDER BY \"id\"",
    );
    assert_eq!(
        column_values(&local, "id"),
        strings(&["1"]),
        "Exasol compares the same two values as two: {local:?}"
    );
}

/// A text one character beyond Exasol's `VARCHAR` maximum fails the query
/// rather than coming back truncated.
#[test]
fn text_value_beyond_the_varchar_maximum_fails_the_query() {
    assert_declares("text_overflow", "body", "VARCHAR(2000000) UTF8");

    let error = stack::shared().exasol.execute_expecting_failure(&format!(
        "SELECT \"body\" FROM {VIRTUAL_SCHEMA}.\"text_overflow\""
    ));
    assert!(
        error.contains("string data, right truncation: max length: 2000000, emitted: 2000001"),
        "a value of 2000001 characters must fail the query naming the overflow: {error}"
    );
}

/// What Exasol's compiler declares for a select-list entry, measured through
/// the probe adapter that advertises the PostgreSQL dialect's predicates: a
/// projection-only request carries `selectListDataTypes`, and every
/// select-list predicate, whatever its form, is declared `BOOLEAN`.
#[test]
fn exasol_declares_the_measured_select_list_types() {
    let stack = stack::shared();
    probe::create(&stack.exasol);
    let declared = |select_list: &str, suffix: &str| {
        probe::measured_select_list_data_types(
            &stack.exasol,
            &format!(
                "SELECT {select_list} FROM {}.\"t\"{suffix}",
                probe::VIRTUAL_SCHEMA
            ),
        )
    };

    let projection = declared("\"d\"", "");
    let predicates = [
        ("\"d\" > 5", ""),
        ("\"d\" IS NULL", ""),
        ("\"d\" BETWEEN 1 AND 9", ""),
        ("\"g\" LIKE 'a%'", ""),
        ("\"d\" IN (1,2,3)", ""),
        ("\"d\" > 1 AND \"d\" < 9", ""),
        ("COUNT(*) > 0", " GROUP BY \"g\""),
    ]
    .map(|(predicate, suffix)| (predicate, declared(predicate, suffix)));
    probe::drop_virtual_schema(&stack.exasol);

    assert_eq!(
        projection,
        vec![serde_json::json!({"type": "DECIMAL", "precision": 12, "scale": 4})],
        "a projection-only request must carry selectListDataTypes"
    );
    for (predicate, types) in predicates {
        assert_eq!(
            types,
            vec![serde_json::json!({"type": "BOOLEAN"})],
            "`{predicate}` must arrive declared BOOLEAN"
        );
    }
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
/// and against the mirror's table of the same name, and asserts the two answers
/// are identical.
///
/// The two sides are independent computations — PostgreSQL evaluates one,
/// Exasol the other — so their agreement is the differential proof this tier
/// relies on: a semantic gap between the two engines shows up as a difference
/// rather than having to be predicted case by case. The mirror holds `items`,
/// `items_c`, `item_notes`, and `type_coverage`, each under the name of its
/// source table, so one query template serves both schemas.
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
        .query_rows(&query(postgres_mirror::SCHEMA, virtual_table));
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

/// Every `pushdown` response `EXPLAIN VIRTUAL` reports for `query`, as the
/// `SELECT <scan script>(...) EMITS (...) FROM DUAL` text Exasol runs.
fn explain_calls(query: &str) -> Vec<String> {
    stack::shared()
        .exasol
        .query_rows(&format!(
            "SELECT PUSHDOWN_SQL FROM (EXPLAIN VIRTUAL {query})"
        ))
        .iter()
        .filter_map(|row| cell(row, "PUSHDOWN_SQL"))
        .collect()
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
fn source_statement(call: &str) -> String {
    scan_specification(call).sql
}

/// The scan specification embedded in one `pushdown` response.
///
/// The response is `SELECT <scan script>('<spec>') EMITS (...) FROM DUAL`, with
/// the specification's own single quotes doubled by Exasol literal quoting. The
/// closing delimiter is found from the right, because the specification itself
/// may end in a doubled quote.
fn scan_specification(call: &str) -> scan_spec::ScanSpec {
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
}

/// `values` as owned strings, for comparing against [`column_values`].
fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_string()).collect()
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

/// Task 2.1 gate: the SDK carries an Arrow microsecond timestamp into an
/// `EMITS` column declared `TIMESTAMP(6)` without cutting it to milliseconds.
///
/// Calls the scan UDF directly with a specification whose statement and
/// columns this test replaced, so the measurement isolates the emit path from
/// every mapping the adapter does.
#[test]
fn scan_emits_every_fractional_digit_of_a_declared_timestamp() {
    let stack = stack::shared();
    let call = stack
        .exasol
        .query_rows(&format!(
            "SELECT PUSHDOWN_SQL FROM (EXPLAIN VIRTUAL SELECT \"id\" FROM {VIRTUAL_SCHEMA}.\"orders\")"
        ))
        .iter()
        .find_map(|row| cell(row, "PUSHDOWN_SQL"))
        .expect("EXPLAIN VIRTUAL reports one pushdown statement");
    let mut specification = scan_specification(&call);
    specification.sql = "SELECT CAST('2024-01-01 12:00:00.123456' AS timestamp(6)) AS ts6, \
                         CAST('2024-01-01 12:00:01' AS timestamp(0)) AS ts0"
        .to_owned();
    specification.columns = vec![
        scan_spec::ScanColumn {
            name: "ts6".to_owned(),
            exasol_type: ExasolType::Timestamp { precision: 6 },
        },
        scan_spec::ScanColumn {
            name: "ts0".to_owned(),
            exasol_type: ExasolType::Timestamp { precision: 0 },
        },
    ];
    let literal = specification.to_json_string().replace('\'', "''");

    let rows = stack.exasol.query_rows(&format!(
        "SELECT TO_CHAR(TS6, 'YYYY-MM-DD HH24:MI:SS.FF6') AS TS6_TEXT, \
         TO_CHAR(TS0, 'YYYY-MM-DD HH24:MI:SS') AS TS0_TEXT \
         FROM (SELECT {SCAN_SCRIPT_NAME}('{literal}') EMITS (TS6 TIMESTAMP(6), TS0 TIMESTAMP(0)) \
         FROM DUAL)"
    ));

    assert_eq!(rows.len(), 1, "one row was selected: {rows:?}");
    assert_eq!(
        cell(&rows[0], "TS6_TEXT"),
        Some("2024-01-01 12:00:00.123456".to_owned()),
        "a TIMESTAMP(6) emit must keep all six fractional digits: {rows:?}"
    );
    assert_eq!(
        cell(&rows[0], "TS0_TEXT"),
        Some("2024-01-01 12:00:01".to_owned()),
        "a TIMESTAMP(0) emit must keep the whole second: {rows:?}"
    );
}

/// Task 2.2 gate: the request shapes `vs-adapter/pushdown-filter` and
/// `vs-adapter/type-mapping` rely on, measured through the probe adapter that
/// advertises the PostgreSQL dialect's filter and literal capabilities.
///
/// A `BOOLEAN` column alone arrives as a bare `column` entry, a comparison of
/// a column with a predicate arrives delegated with the predicate as its right
/// operand, and a `TIMESTAMP(6)` literal keeps all six fractional digits.
#[test]
fn exasol_sends_the_measured_boolean_and_timestamp_filter_shapes() {
    let stack = stack::shared();
    probe::create(&stack.exasol);
    let request_of = |predicate: &str| {
        probe::measured_pushdown_request(
            &stack.exasol,
            &format!(
                "SELECT \"d\" FROM {}.\"t\" WHERE {predicate}",
                probe::VIRTUAL_SCHEMA
            ),
        )
    };
    let filter_of = |predicate: &str| {
        let request = request_of(predicate);
        request
            .pointer("/pushdownRequest/filter")
            .unwrap_or_else(|| panic!("`WHERE {predicate}` was not delegated: {request}"))
            .clone()
    };

    let bare_request = request_of("\"b\"");
    let bare = bare_request
        .pointer("/pushdownRequest/filter")
        .unwrap_or_else(|| panic!("`WHERE b` was not delegated: {bare_request}"))
        .clone();
    let negated = filter_of("NOT \"b\"");
    let compared = filter_of("\"b\" = (\"d\" > 5)");
    let timestamp = filter_of("\"p\" = TIMESTAMP '2024-01-01 12:00:00.123456'");
    probe::drop_virtual_schema(&stack.exasol);

    let flag = serde_json::json!({"columnNr": 5, "name": "b", "tableName": "t", "type": "column"});
    assert_eq!(bare, flag, "WHERE b must arrive as a bare column entry");
    let declared_flag = bare_request
        .pointer("/involvedTables/0/columns")
        .and_then(serde_json::Value::as_array)
        .and_then(|columns| columns.iter().find(|column| column["name"] == "b"))
        .unwrap_or_else(|| panic!("involvedTables must declare column b: {bare_request}"));
    assert!(
        declared_flag.get("adapterNotes").is_none(),
        "a column declared without notes must arrive with no adapterNotes member: {declared_flag}"
    );
    assert_eq!(
        negated,
        serde_json::json!({"expression": flag, "type": "predicate_not"}),
        "WHERE NOT b must arrive as predicate_not over the bare column entry"
    );
    assert_eq!(
        compared,
        serde_json::json!({
            "type": "predicate_equal",
            "left": flag,
            "right": {
                "type": "predicate_less",
                "left": {"type": "literal_exactnumeric", "value": "5"},
                "right": {"columnNr": 0, "name": "d", "tableName": "t", "type": "column"},
            },
        }),
        "WHERE b = (d > 5) must arrive delegated, with the predicate as the right operand and \
         its operands swapped"
    );
    assert_eq!(
        timestamp,
        serde_json::json!({
            "type": "predicate_equal",
            "left": {"columnNr": 6, "name": "p", "tableName": "t", "type": "column"},
            "right": {"type": "literal_timestamp", "value": "2024-01-01 12:00:00.123456"},
        }),
        "a TIMESTAMP literal must carry all six fractional digits"
    );
}
