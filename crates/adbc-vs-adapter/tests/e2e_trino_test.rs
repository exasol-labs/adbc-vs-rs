//! Drives adbc-vs end to end against the Docker Compose stack over Trino: an
//! Exasol virtual schema over the `trino` container's `memory` catalog, with
//! the ADBC driver fetched from the public registry into BucketFS on the
//! way, beside a second virtual schema over MySQL that the per-schema
//! capability difference `vs-adapter/trino-pushdown` states needs.
//!
//! There is no skip path. A stack that is down, a driver that will not
//! install, or a query that returns the wrong rows fails the run, because a
//! tier that quietly skips itself proves nothing.
//!
//! Every delegated fragment is verified twice, which is the pattern this
//! tier exists for. `EXPLAIN VIRTUAL` proves the fragment reached the
//! statement the adapter generated for Trino, and a row comparison against
//! [`common::trino_mirror`] — a local Exasol table holding byte-identical
//! values — proves it produced Exasol's own answer. Either alone is blind: a
//! silently dropped fragment still returns rows, and a right answer says
//! nothing about where it was computed.
//!
//! The collation half of this tier reads the opposite of PostgreSQL's and
//! MySQL's: `vs-adapter/trino-collation` states, and this tier measured,
//! that Trino's own default `varchar` comparison already agrees with
//! Exasol's — code-point order, exact-value equality — so the dialect
//! renders no collation suffix anywhere, ever. The differential comparisons
//! below exist to measure that agreement against a live Trino rather than
//! assume it, and the absence assertions exist because Trino has no
//! `COLLATE` keyword at all: a dialect that rendered one would make Trino
//! reject the statement rather than silently ignore it.
//!
//! Exasol string literals are `CHAR`, not `VARCHAR`, and `CHAR` comparison is
//! blank-padded: `'a' = 'a '` is true for a bare literal and false once both
//! sides are cast to `VARCHAR`. Every comparison this file runs directly
//! against Exasol (rather than through the pushdown path) casts explicitly,
//! because the mapped columns are `VARCHAR` and an uncast literal would
//! silently compare under the wrong rule.
//!
//! Every test shares one stack, two virtual schemas, and the drivers
//! installed into one BucketFS, which is why this binary runs with
//! `--test-threads=1`.
#![cfg(feature = "exasol-e2e")]

mod common;

use adbc_vs_adapter::pushdown::SCAN_SCRIPT_NAME;
use adbc_vs_adapter::scan_spec;
use adbc_vs_scan::driver::Driver;
use common::exasol::cell;
use common::trino_mirror;
use common::trino_stack::{self as stack, TRINO_VIRTUAL_SCHEMA as VIRTUAL_SCHEMA};
use common::{schema, trino};

/// How many rows `docker/trino/init.sql` seeds into `items`.
const ITEM_ROWS: usize = 6;

/// How many distinct `category` values those rows hold under Exasol's
/// comparison rules: `widget`, `Widget`, `cafe`, and `café`. Trino's own
/// comparison is deterministic and agrees, which is why the dialect renders
/// no collation on a grouping key.
const CATEGORY_GROUPS: usize = 4;

/// The `memory.adbc_vs`-qualified `items` table, as the dialect writes it.
/// Both the catalog and the schema are recorded segments, per
/// `drivers/trino.toml`'s `[qualifier] segment_count = 2`.
const QUALIFIED_ITEMS: &str = "\"memory\".\"adbc_vs\".\"items\"";

// --- the mirror is a valid oracle -------------------------------------------

/// Every differential test below rests on the mirror answering under
/// Exasol's own rules rather than Trino's, so this test establishes that
/// before any comparison relies on it. Without it, a delegated query and a
/// mirror that both ordered by some other rule would agree and prove
/// nothing.
#[test]
fn mirror_answers_under_exasols_own_comparison_and_ordering_rules() {
    let stack = stack::shared();

    let rows = stack.exasol.query_rows(&format!(
        "SELECT COUNT(*) AS N, COUNT(DISTINCT \"category\") AS CATEGORIES, \
         MIN(\"sort_key\") AS MINKEY, MAX(\"sort_key\") AS MAXKEY, \
         MIN(\"tag\") AS MINTAG, MAX(\"tag\") AS MAXTAG FROM {}.\"items\"",
        trino_mirror::SCHEMA
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
        "Exasol orders by code point: 'B' precedes 'C' precedes 'a'"
    );
    assert_eq!(cell(answers, "MAXKEY"), Some("a".to_string()));
    assert_eq!(
        cell(answers, "MINTAG"),
        Some("ab".to_string()),
        "Exasol pads nothing, so MIN over 'ab ', 'ab', 'ab!' is 'ab'"
    );
    assert_eq!(cell(answers, "MAXTAG"), Some("ab!".to_string()));
}

/// A differential comparison is only meaningful while both sides declare the
/// same columns under the same Exasol types: a mirror declaring a wider
/// `VARCHAR` would change an aggregate's declared type for a reason that is
/// not a pushdown defect.
#[test]
fn mirror_declares_the_same_columns_and_types_as_the_virtual_table() {
    const MIRRORED_COLUMNS: usize = 4;

    let mirror = declared_columns(trino_mirror::SCHEMA, "items");
    assert_eq!(
        mirror.len(),
        MIRRORED_COLUMNS,
        "{}.\"items\" must declare its {MIRRORED_COLUMNS} mirrored columns; an empty or short \
         list means the mirror schema was not created or not seeded, and every differential \
         assertion in this file rests on it",
        trino_mirror::SCHEMA
    );

    for (mirror_column, mirror_type) in mirror {
        let virtual_type = declared_columns(VIRTUAL_SCHEMA, "items")
            .into_iter()
            .find(|(name, _)| *name == mirror_column)
            .map(|(_, kind)| kind);
        assert_eq!(
            Some(mirror_type.clone()),
            virtual_type,
            "mirror column {mirror_column} declares {mirror_type}, which must match the virtual \
             table's own declaration for the same column"
        );
    }
}

// --- pushdown (`vs-adapter/trino-pushdown`) ---------------------------------

#[test]
fn projection_reaches_trino_and_matches_the_mirror() {
    let query = |schema: &str| {
        format!("SELECT \"id\", \"category\" FROM {schema}.\"items\" ORDER BY \"id\"")
    };

    assert_eq!(
        generated_statement(&query(VIRTUAL_SCHEMA)),
        format!("SELECT \"id\", \"category\" FROM {QUALIFIED_ITEMS}"),
        "the statement sent to Trino must project the two named columns and no other, from the \
         catalog- and schema-qualified table"
    );

    let rows = assert_matches_mirror(query);
    assert_eq!(rows.len(), ITEM_ROWS);
    assert_eq!(rows[0].len(), 2, "only the two named columns come back");
    assert_eq!(cell(&rows[0], "category"), Some("widget".to_string()));
}

/// The combined filter proves equality, `LIKE`, and `BETWEEN` reach Trino and
/// select the rows Exasol selects. The ordering filter proves the same for a
/// comparison whose answer depends on order — `sort_key < 'C'` — over a
/// statement carrying no collation suffix at all, which is what tells a
/// correctly delegated comparison from one Exasol computed itself.
#[test]
fn filter_reaches_trino_and_matches_the_mirror() {
    let query = |schema: &str| {
        format!(
            "SELECT \"id\" FROM {schema}.\"items\" WHERE (\"category\" = 'Widget' OR \
             \"tag\" LIKE 'ab!%') AND \"id\" BETWEEN 1 AND 4 ORDER BY \"id\""
        )
    };

    let statement = generated_statement(&query(VIRTUAL_SCHEMA));
    for fragment in ["WHERE", " OR ", " AND ", "LIKE", "BETWEEN"] {
        assert!(
            statement.contains(fragment),
            "the generated statement must carry {fragment}: {statement}"
        );
    }
    assert!(
        !statement.to_uppercase().contains("ESCAPE"),
        "Trino defines no default escape character, so a LIKE with no ESCAPE clause must reach \
         it as a plain LIKE: {statement}"
    );
    assert!(
        !statement.to_uppercase().contains("COLLATE"),
        "Trino has no COLLATE keyword and would reject the statement rather than ignore it: \
         {statement}"
    );

    let rows = assert_matches_mirror(query);
    assert_eq!(
        column_values(&rows, "id"),
        vec!["2".to_string(), "3".to_string(), "4".to_string()],
        "row 2 is a Widget, row 3 carries tag 'ab!', row 4 is both, and row 1's tag 'ab ' matches \
         neither branch: {rows:?}"
    );

    let ordering_query = |schema: &str| {
        format!("SELECT \"id\" FROM {schema}.\"items\" WHERE \"sort_key\" < 'C' ORDER BY \"id\"")
    };
    let ordering_statement = generated_statement(&ordering_query(VIRTUAL_SCHEMA));
    assert!(
        ordering_statement.contains("\"sort_key\"") && !ordering_statement.contains("COLLATE"),
        "the row comparison below only tells a correctly delegated comparison from one Exasol \
         computed itself while the predicate actually reached Trino, and it must carry no \
         collation suffix: {ordering_statement}"
    );

    let ordering_rows = assert_matches_mirror(ordering_query);
    assert_eq!(
        column_values(&ordering_rows, "id"),
        vec!["1".to_string(), "4".to_string()],
        "only the two 'B' rows precede 'C' in code-point order: {ordering_rows:?}"
    );
}

/// `FN_PRED_LIKE_ESCAPE` is withheld, so Exasol keeps a `LIKE` that writes an
/// explicit `ESCAPE` clause and evaluates it itself. The pattern `ab!!`
/// escapes the escape character, so it matches the literal tag `ab!` only;
/// delegated as a plain `LIKE`, Trino would read it as the literal `ab!!` and
/// return no row, which is what tells Exasol's evaluation from a delegation.
#[test]
fn like_with_an_explicit_escape_clause_is_evaluated_by_exasol() {
    let query = |schema: &str| {
        format!(
            "SELECT \"id\" FROM {schema}.\"items\" WHERE \"tag\" LIKE 'ab!!' ESCAPE '!' \
             ORDER BY \"id\""
        )
    };

    let statement = generated_statement(&query(VIRTUAL_SCHEMA));
    assert!(
        !statement.to_uppercase().contains("LIKE"),
        "the generated statement must carry no LIKE, because Exasol keeps a predicate whose \
         capability the dialect does not advertise: {statement}"
    );

    let rows = assert_matches_mirror(query);
    assert_eq!(
        column_values(&rows, "id"),
        vec!["3".to_string(), "6".to_string()],
        "only the two rows tagged 'ab!' match the escaped pattern: {rows:?}"
    );
}

#[test]
fn aggregation_reaches_trino_and_matches_the_mirror() {
    let single_group_query = |schema: &str| {
        format!(
            "SELECT COUNT(*) AS N, MIN(\"sort_key\") AS MINKEY, MAX(\"sort_key\") AS MAXKEY, \
             MIN(\"tag\") AS MINTAG, MAX(\"tag\") AS MAXTAG FROM {schema}.\"items\""
        )
    };

    let statement = generated_statement(&single_group_query(VIRTUAL_SCHEMA));
    for fragment in ["COUNT(*)", "MIN(", "MAX("] {
        assert!(
            statement.contains(fragment),
            "the generated statement must carry {fragment}: {statement}"
        );
    }
    assert!(
        !statement.contains("GROUP BY"),
        "a single-group aggregation must render no GROUP BY clause: {statement}"
    );

    let rows = assert_matches_mirror(single_group_query);
    let answers = &rows[0];
    assert_eq!(cell(answers, "N"), Some(ITEM_ROWS.to_string()));
    assert_eq!(cell(answers, "MINKEY"), Some("B".to_string()));
    assert_eq!(cell(answers, "MAXKEY"), Some("a".to_string()));
    assert_eq!(cell(answers, "MINTAG"), Some("ab".to_string()));
    assert_eq!(cell(answers, "MAXTAG"), Some("ab!".to_string()));

    let grouped_query = |schema: &str| {
        format!(
            "SELECT \"category\" AS CATEGORY, COUNT(*) AS N FROM {schema}.\"items\" \
             GROUP BY \"category\" ORDER BY \"category\""
        )
    };

    let grouped_statement = generated_statement(&grouped_query(VIRTUAL_SCHEMA));
    assert!(
        grouped_statement.contains("GROUP BY") && grouped_statement.contains("COUNT(*)"),
        "a grouped aggregation must render both COUNT and GROUP BY: {grouped_statement}"
    );

    let grouped_rows = assert_matches_mirror(grouped_query);
    assert_eq!(grouped_rows.len(), CATEGORY_GROUPS);
}

/// `drivers/trino.toml` advertises `AGGREGATE_GROUP_BY_TUPLE`, the one
/// capability this release advertises beyond PostgreSQL parity (decision
/// `[16]`), so Exasol delegates a multi-column `GROUP BY` here rather than
/// withdrawing it the way it does for PostgreSQL and MySQL.
#[test]
fn two_column_group_by_is_delegated_and_matches_the_mirror() {
    let query = |schema: &str| {
        format!(
            "SELECT \"category\" AS CATEGORY, \"sort_key\" AS SORTKEY, COUNT(*) AS N FROM \
             {schema}.\"items\" GROUP BY \"category\", \"sort_key\" \
             ORDER BY \"category\", \"sort_key\""
        )
    };

    let statement = generated_statement(&query(VIRTUAL_SCHEMA));
    assert!(
        statement.contains("GROUP BY \"category\", \"sort_key\""),
        "both grouping columns must render in one GROUP BY clause, in request order: {statement}"
    );

    let rows = assert_matches_mirror(query);
    assert_eq!(
        rows.len(),
        ITEM_ROWS,
        "every row carries its own category/sort_key pair, so grouping by both yields one group \
         per row: {rows:?}"
    );
}

#[test]
fn join_reaches_trino_and_matches_the_mirror() {
    let query = |schema: &str| {
        format!(
            "SELECT i.\"id\" AS ID, n.\"note\" AS NOTE FROM {schema}.\"items\" i INNER JOIN \
             {schema}.\"item_notes\" n ON i.\"id\" = n.\"item_id\" ORDER BY i.\"id\""
        )
    };

    let statement = generated_statement(&query(VIRTUAL_SCHEMA));
    assert!(
        statement.contains("INNER JOIN"),
        "the join must reach Trino as one statement rather than being withdrawn into two scans: \
         {statement}"
    );

    let rows = assert_matches_mirror(query);
    assert_eq!(
        rows.len(),
        5,
        "the one items row with no matching note and the one note matching no item are both \
         excluded: {rows:?}"
    );
}

/// `FULL OUTER JOIN` is one of the two capabilities the Trino dialect
/// advertises and the MySQL one does not (the capability-difference tests below assert the other side of
/// that difference). Both unmatched rows have to come back, because a join
/// silently rendered as an inner one returns the five matched rows and looks
/// correct until they are counted.
#[test]
fn full_outer_join_reaches_trino_and_returns_both_unmatched_rows() {
    let query = |schema: &str| {
        format!(
            "SELECT i.\"id\" AS ID, n.\"note\" AS NOTE FROM {schema}.\"items\" i FULL OUTER JOIN \
             {schema}.\"item_notes\" n ON i.\"id\" = n.\"item_id\" ORDER BY i.\"id\", n.\"note\""
        )
    };

    let statement = generated_statement(&query(VIRTUAL_SCHEMA));
    assert!(
        statement.contains("FULL OUTER JOIN"),
        "the full outer join must be delegated whole: {statement}"
    );
    assert!(
        !statement.contains("COLLATE"),
        "a join condition is an equality over an integer column and needs no collation: \
         {statement}"
    );

    let rows = assert_matches_mirror(query);
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
        "the unmatched items row must return a null note: {unmatched_item:?}"
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

#[test]
fn avg_and_sum_are_withdrawn_and_match_the_mirror() {
    for function in ["AVG", "SUM"] {
        let query =
            |schema: &str| format!("SELECT {function}(\"id\") AS VAL FROM {schema}.\"items\"");

        for statement in generated_statements(&query(VIRTUAL_SCHEMA)) {
            assert!(
                !statement.to_uppercase().contains(function),
                "no generated statement may carry a delegated {function}, since this release \
                 withholds it and Exasol computes it itself: {statement}"
            );
        }

        let rows = assert_matches_mirror(query);
        assert_eq!(
            rows.len(),
            1,
            "a single-group aggregation returns exactly one row: {rows:?}"
        );
    }
}

#[test]
fn no_generated_statement_carries_order_limit_or_offset() {
    for suffix in ["ORDER BY \"id\"", "LIMIT 2", "ORDER BY \"id\" LIMIT 2"] {
        let query = move |schema: &str| format!("SELECT \"id\" FROM {schema}.\"items\" {suffix}");

        for statement in generated_statements(&query(VIRTUAL_SCHEMA)) {
            let upper = statement.to_uppercase();
            assert!(!upper.contains("ORDER BY"), "{suffix}: {statement}");
            assert!(!upper.contains("LIMIT"), "{suffix}: {statement}");
            assert!(!upper.contains("OFFSET"), "{suffix}: {statement}");
        }
    }

    let limited =
        |schema: &str| format!("SELECT \"id\" FROM {schema}.\"items\" ORDER BY \"id\" LIMIT 2");
    let rows = assert_matches_mirror(limited);
    assert_eq!(
        column_values(&rows, "id"),
        vec!["1".to_string(), "2".to_string()],
        "withholding ORDER BY and LIMIT costs no answer: Exasol applies both itself: {rows:?}"
    );
}

// --- collation (`vs-adapter/trino-collation`) -------------------------------

/// The whole collation rule, read off the statements the deployed adapter
/// actually generated: no rendered operand carries a suffix in any position,
/// because Trino has no `COLLATE` keyword to render one through. Absence is
/// asserted as the absence of the word anywhere in the statement, not as the
/// absence of one spelling.
#[test]
fn no_generated_statement_carries_a_collation_keyword() {
    let ordering = generated_statement(&format!(
        "SELECT \"id\" FROM {VIRTUAL_SCHEMA}.\"items\" WHERE \"sort_key\" < 'C'"
    ));
    let min_max = generated_statement(&format!(
        "SELECT MIN(\"sort_key\"), MAX(\"tag\") FROM {VIRTUAL_SCHEMA}.\"items\""
    ));
    let equality = generated_statement(&format!(
        "SELECT \"id\" FROM {VIRTUAL_SCHEMA}.\"items\" WHERE \"category\" = 'Widget'"
    ));
    let grouped = generated_statement(&format!(
        "SELECT \"category\", COUNT(*) FROM {VIRTUAL_SCHEMA}.\"items\" GROUP BY \"category\""
    ));
    let distinct = generated_statement(&format!(
        "SELECT COUNT(DISTINCT \"category\") FROM {VIRTUAL_SCHEMA}.\"items\""
    ));

    for statement in [&ordering, &min_max, &equality, &grouped, &distinct] {
        assert!(
            !statement.to_uppercase().contains("COLLATE"),
            "Trino declares no COLLATE keyword in any syntactic position; a dialect that \
             rendered one would make Trino reject the statement rather than ignore it: \
             {statement}"
        );
    }
    assert_eq!(
        grouped,
        format!("SELECT \"category\", COUNT(*) FROM {QUALIFIED_ITEMS} GROUP BY \"category\""),
        "the grouping key and its select-list occurrence must both render bare and identically"
    );
    assert_eq!(
        distinct,
        format!("SELECT COUNT(DISTINCT \"category\") FROM {QUALIFIED_ITEMS}")
    );
}

/// `vs-adapter/trino-collation`'s measured claim: Trino's own `varchar` comparison already
/// answers ordering, equality, `GROUP BY`, and `COUNT(DISTINCT)` the way
/// Exasol does, over the case-only, accent-only, and code-point fixture, with
/// no suffix rendered anywhere.
#[test]
fn uncollated_grouping_and_distinct_return_the_mirrors_group_count() {
    let grouped_query = |schema: &str| {
        format!(
            "SELECT \"category\" AS CATEGORY, COUNT(*) AS N FROM {schema}.\"items\" \
             GROUP BY \"category\" ORDER BY \"category\""
        )
    };

    let rows = assert_matches_mirror(grouped_query);
    assert_eq!(
        rows.len(),
        CATEGORY_GROUPS,
        "widget, Widget, cafe, and café must stay four groups: {rows:?}"
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
        assert_eq!(count, Some(expected.to_string()), "{rows:?}");
    }

    let distinct_query =
        |schema: &str| format!("SELECT COUNT(DISTINCT \"category\") AS N FROM {schema}.\"items\"");
    let distinct_rows = assert_matches_mirror(distinct_query);
    assert_eq!(
        cell(&distinct_rows[0], "N"),
        Some(CATEGORY_GROUPS.to_string())
    );
}

/// A select-list predicate over the query's own grouping key, a shape no
/// dialect renders a collation suffix for, must still reach Trino and match
/// the mirror.
#[test]
fn select_list_predicate_over_the_grouped_character_column_delegates_and_matches_the_mirror() {
    let query = |schema: &str| {
        format!(
            "SELECT \"category\" AS CATEGORY, \"category\" < 'm' AS UNDER_M FROM \
             {schema}.\"items\" GROUP BY \"category\" ORDER BY \"category\""
        )
    };

    let statement = generated_statement(&query(VIRTUAL_SCHEMA));
    assert!(
        statement.contains("GROUP BY \"category\"") && !statement.contains("COLLATE"),
        "the grouping key must render bare, with no collation suffix anywhere: {statement}"
    );

    let rows = assert_matches_mirror(query);
    assert_eq!(rows.len(), CATEGORY_GROUPS);
    let under_m = |category: &str| {
        rows.iter()
            .find(|row| cell(row, "CATEGORY") == Some(category.to_string()))
            .and_then(|row| cell(row, "UNDER_M"))
    };
    assert_eq!(under_m("Widget"), Some("true".to_string()), "{rows:?}");
    assert_eq!(under_m("cafe"), Some("true".to_string()), "{rows:?}");
    assert_eq!(under_m("café"), Some("true".to_string()), "{rows:?}");
    assert_eq!(under_m("widget"), Some("false".to_string()), "{rows:?}");
}

// --- credential redaction (`scan-udf/trino-scan`) ---------------------------

/// Every error `createVirtualSchema`, `refresh`, and `setProperties` surface
/// for one broken `CONNECTION`, over the three entry points that all funnel
/// through `create_schema::open`. `scenario` names the two schemas and
/// connections this run creates so a second call cannot collide with the
/// first's objects, `address` is the connection's address, and `password` is
/// the credential the run authenticates with. Every connection this helper
/// creates authenticates as [`trino::HOSTILE_USER`], per `scan-udf/trino-scan`: the
/// end-to-end path exercises the same credential the integration tier does
/// rather than a benign one.
///
/// The three failures are produced in the order `createVirtualSchema`,
/// `setProperties`, `refresh`: the second reuses a working schema/connection
/// pair created for it, and the third rotates that same connection's
/// credentials to the broken ones only after `setProperties` has already
/// been exercised over it, so neither call disturbs the other's fixture.
fn credential_failures(scenario: &str, address: &str, password: &str) -> [String; 3] {
    let stack = stack::shared();
    let library = schema::recorded_driver_path(&stack.exasol, VIRTUAL_SCHEMA);

    let bad_connection = format!("ADBC_VS_TR_{scenario}_BAD");
    create_connection(
        &stack.exasol,
        ConnectionSpec {
            name: &bad_connection,
            address,
            password,
        },
    );

    let create_schema_name = format!("TRFAIL_{scenario}_CREATE");
    let create_error = schema::create_virtual_schema_through_expecting_failure(
        &stack.exasol,
        schema::supplied_driver_spec(
            &create_schema_name,
            Driver::Trino,
            &bad_connection,
            &library,
        ),
    );

    let working_connection = format!("ADBC_VS_TR_{scenario}_WORK");
    create_connection(
        &stack.exasol,
        ConnectionSpec {
            name: &working_connection,
            address: &trino::driver_uri(),
            password: trino::HOSTILE_PASSWORD,
        },
    );
    let working_schema = format!("TRFAIL_{scenario}_WORK");
    schema::create_virtual_schema_through(
        &stack.exasol,
        schema::supplied_driver_spec(
            &working_schema,
            Driver::Trino,
            &working_connection,
            &library,
        ),
    );

    let set_properties_error = stack.exasol.execute_expecting_failure(&format!(
        "ALTER VIRTUAL SCHEMA {working_schema} SET CONNECTION_NAME = {}",
        schema::quote_literal(&bad_connection)
    ));

    create_connection(
        &stack.exasol,
        ConnectionSpec {
            name: &working_connection,
            address,
            password,
        },
    );
    let refresh_error = stack
        .exasol
        .execute_expecting_failure(&format!("ALTER VIRTUAL SCHEMA {working_schema} REFRESH"));

    schema::drop_virtual_schema(&stack.exasol, &working_schema);
    schema::drop_connection(&stack.exasol, &working_connection);
    schema::drop_connection(&stack.exasol, &bad_connection);

    [create_error, set_properties_error, refresh_error]
}

/// The wrong-password case is the one failure this driver cannot name a host
/// for: the driver checks no credential while the connection opens, so the
/// refusal surfaces through the scan's execute-time diagnostic, which names
/// the statement rather than the address. The catalog and the schema, which
/// survive inside the statement's qualified table reference, are what keep
/// it diagnosable. This is a deliberate, spec-narrowed behavior — see
/// `scan-udf/trino-scan`'s Background and the `[human-decision]` entry in
/// decision-log.md — not a gap: the host's absence is asserted explicitly
/// rather than merely not checked for.
#[test]
fn wrong_password_names_no_host_but_still_names_the_catalog_and_schema() {
    let bad_password = "not-the-real-password-please-do-not-leak";
    let errors = credential_failures("WRONGPASS", &trino::driver_uri(), bad_password);

    for (entry_point, error) in ["createVirtualSchema", "setProperties", "refresh"]
        .iter()
        .zip(errors.iter())
    {
        assert!(
            !error.contains(bad_password) && !error.contains(trino::HOSTILE_PASSWORD),
            "{entry_point} must not surface a password, wrong or real: {error}"
        );
        assert!(
            !error.contains(trino::ADDRESS),
            "{entry_point} must name no host for a wrong password: the driver checks the \
             credential only once a statement executes, and that diagnostic names the \
             statement rather than the address: {error}"
        );
        assert!(
            error.contains(trino::CATALOG) && error.contains(trino::SCHEMA),
            "{entry_point} must still name the catalog and the schema it addressed: {error}"
        );
    }
}

/// Unlike a wrong password, an unreachable host fails at the transport
/// before any statement runs, and the driver's own message quotes the
/// address it dialled — so this failure names the host as well as the
/// catalog and the schema.
#[test]
fn unreachable_host_never_leaks_the_password_and_still_names_the_host() {
    let errors = credential_failures(
        "UNREACHABLE",
        &trino::unreachable_uri(),
        trino::HOSTILE_PASSWORD,
    );

    for (entry_point, error) in ["createVirtualSchema", "setProperties", "refresh"]
        .iter()
        .zip(errors.iter())
    {
        assert!(
            !error.contains(trino::HOSTILE_PASSWORD),
            "{entry_point} must not surface the password: {error}"
        );
        assert!(
            error.contains(trino::ADDRESS),
            "{entry_point} must still name the host it could not reach: {error}"
        );
        assert!(
            error.contains(trino::CATALOG) && error.contains(trino::SCHEMA),
            "{entry_point} must still name the catalog and the schema: {error}"
        );
    }
}

/// The refusal is the Trino *server*'s rather than the driver's, as measured
/// against this container: `403 Forbidden`, because
/// `allow-insecure-over-http` defaults to `false`. `SSL=false` is the parameter that makes the driver actually send
/// the request in the clear rather than attempting HTTPS.
#[test]
fn a_password_over_an_unencrypted_transport_is_refused_by_the_trino_server() {
    let stack = stack::shared();
    const CONNECTION: &str = "ADBC_VS_TR_SSLFALSE";
    const SCHEMA_NAME: &str = "TRFAIL_SSLFALSE";

    create_connection(
        &stack.exasol,
        ConnectionSpec {
            name: CONNECTION,
            address: &trino::ssl_false_uri(),
            password: trino::HOSTILE_PASSWORD,
        },
    );
    let library = schema::recorded_driver_path(&stack.exasol, VIRTUAL_SCHEMA);
    let error = schema::create_virtual_schema_through_expecting_failure(
        &stack.exasol,
        schema::supplied_driver_spec(SCHEMA_NAME, Driver::Trino, CONNECTION, &library),
    );

    assert!(
        error.contains("403") && error.to_lowercase().contains("forbidden"),
        "the failure must surface the Trino server's own refusal rather than a driver-side one: \
         {error}"
    );
    assert!(
        error.contains(trino::CATALOG) && error.contains(trino::SCHEMA),
        "with SSL=false the driver opens the plain-HTTP connection successfully, so the \
         transport refusal only surfaces once Exasol's metadata-discovery query actually \
         executes against Trino — the same lazy-execution behavior as a wrong password. The 403 \
         response names no host, but the query text it quotes still carries the catalog and, as \
         a WHERE-clause literal, the schema: {error}"
    );
    assert!(
        !error.contains(trino::HOSTILE_PASSWORD),
        "the adapter must not strip the password by sending it anyway and hoping, nor may it \
         leak it while reporting the refusal: {error}"
    );

    schema::drop_connection(&stack.exasol, CONNECTION);
}

/// An address naming the plain-HTTP port with no `SSL=false` parameter never
/// reaches the server at all: the driver attempts HTTPS by default on every
/// port, so the failure is the driver's own, against a server speaking plain
/// HTTP on that port.
#[test]
fn an_address_on_the_plain_http_port_without_ssl_false_fails_inside_the_driver() {
    let stack = stack::shared();
    const CONNECTION: &str = "ADBC_VS_TR_PLAINPORT";
    const SCHEMA_NAME: &str = "TRFAIL_PLAINPORT";

    create_connection(
        &stack.exasol,
        ConnectionSpec {
            name: CONNECTION,
            address: &trino::plain_http_port_uri(),
            password: trino::HOSTILE_PASSWORD,
        },
    );
    let library = schema::recorded_driver_path(&stack.exasol, VIRTUAL_SCHEMA);
    let error = schema::create_virtual_schema_through_expecting_failure(
        &stack.exasol,
        schema::supplied_driver_spec(SCHEMA_NAME, Driver::Trino, CONNECTION, &library),
    );

    assert!(
        error.contains(trino::ADDRESS) && error.contains(&trino::PORT_PLAIN_HTTP.to_string()),
        "the surfaced error must name the host and port the attempt addressed; the driver's own \
         text was: {error}"
    );
    assert!(
        !error.contains(trino::HOSTILE_PASSWORD),
        "the surfaced error must carry no part of the password; the driver's own text was: \
         {error}"
    );

    schema::drop_connection(&stack.exasol, CONNECTION);
}

/// A `CONNECTION` object addressing an arbitrary Trino URI. The three strings
/// are named rather than positional because they are mutually substitutable at
/// a call site: swapping the address and the password compiles, and the test
/// then passes for the wrong reason.
struct ConnectionSpec<'a> {
    name: &'a str,
    address: &'a str,
    password: &'a str,
}

/// Creates a `CONNECTION` object from `spec`, always authenticating as the
/// hostile principal.
///
/// Defined here rather than reused from `schema.rs`, which this group does
/// not touch: every other technology's connection helper already lives there,
/// but none lets the credential tests above vary the address and the password
/// against a fixed principal, which is the whole shape they exercise.
fn create_connection(exasol: &common::exasol::Exasol, spec: ConnectionSpec<'_>) {
    exasol.execute(&format!(
        "CREATE OR REPLACE CONNECTION {} TO {} USER {} IDENTIFIED BY {}",
        spec.name,
        schema::quote_literal(spec.address),
        schema::quote_literal(trino::HOSTILE_USER),
        schema::quote_literal(spec.password)
    ));
}

// --- discovery (`vs-adapter/trino-type-mapping`) ----------------------------

/// Trino's `memory` connector folds every identifier to lower case
/// regardless of source quoting (measured directly against this container;
/// see `common/trino.rs`'s own header), so there is no mixed-case source
/// identifier to preserve the way PostgreSQL's and MySQL's fixtures have.
/// What this asserts instead: discovery and rendering keep whatever case
/// Trino's own catalog reports, byte for byte, rather than the adapter
/// re-casing anything of its own accord.
#[test]
fn identifiers_keep_trinos_own_case_through_discovery_and_rendering() {
    let tables = table_names(VIRTUAL_SCHEMA);
    assert!(
        tables.contains(&"items".to_string()) && tables.contains(&"orders".to_string()),
        "discovery must expose every source table under the exact case Trino's own catalog \
         reports: {tables:?}"
    );
    assert!(
        !tables
            .iter()
            .any(|name| name.chars().any(char::is_uppercase)),
        "Trino's memory connector folds every identifier to lower case, so no discovered name \
         may carry an upper-case letter the source itself does not report: {tables:?}"
    );

    let columns = column_names(VIRTUAL_SCHEMA, "items");
    assert_eq!(
        columns,
        vec![
            "id".to_string(),
            "category".to_string(),
            "sort_key".to_string(),
            "tag".to_string(),
        ],
        "the mapped columns must keep their source case and ordinal order"
    );

    let statement = generated_statement(&format!(
        "SELECT \"id\", \"category\" FROM {VIRTUAL_SCHEMA}.\"items\" ORDER BY \"id\""
    ));
    assert!(
        statement.contains(QUALIFIED_ITEMS),
        "the generated statement must carry the source's own case for the catalog-, schema-, \
         and table-qualified name: {statement}"
    );
}

/// `orders_view` (`docker/trino/init.sql`) is a view over `orders`,
/// projecting only mapped columns. `generated_statement` is what tells a
/// view queried through the pushdown path from one Exasol resolved on its
/// own: a name the adapter never discovered would leave `EXPLAIN VIRTUAL`
/// reporting no statement for it at all.
#[test]
fn views_are_exposed_alongside_base_tables() {
    let tables = table_names(VIRTUAL_SCHEMA);
    assert!(
        tables.contains(&"orders_view".to_string()) && tables.contains(&"orders".to_string()),
        "the exposed metadata must list both the base table and the view: {tables:?}"
    );
    assert_eq!(
        column_names(VIRTUAL_SCHEMA, "orders_view"),
        vec!["id".to_string(), "name".to_string()],
        "the view projects exactly the two mapped columns of orders it selects in \
         docker/trino/init.sql"
    );

    let query =
        format!("SELECT \"id\", \"name\" FROM {VIRTUAL_SCHEMA}.\"orders_view\" ORDER BY \"id\"");
    let statement = generated_statement(&query);
    assert!(
        statement.contains("\"memory\".\"adbc_vs\".\"orders_view\""),
        "the generated statement must name the view as a table reached through the pushdown \
         path, not one Exasol resolved itself: {statement}"
    );

    let rows = stack::shared().exasol.query_rows(&query);
    assert_eq!(
        column_values(&rows, "id"),
        vec!["1".to_string(), "2".to_string(), "3".to_string()],
        "orders_view projects every row docker/trino/init.sql seeds into orders: {rows:?}"
    );
}

/// `refresh_target` (`docker/trino/init.sql`) is dedicated to this test's
/// `ALTER TABLE`, so no other test's fixture is disturbed by it. The refresh
/// runs against a schema of its own that supplies the shared schema's driver
/// library, so it never reaches the live registry. Only the gain-a-column direction is exercised here: Trino's `memory` connector has
/// no `DROP COLUMN` support at all, so a drop-then-refresh assertion can
/// never pass against this source. Do not re-add one without first checking
/// whether that connector limit still holds.
#[test]
fn refresh_picks_up_a_new_trino_column() {
    const REFRESH_SCHEMA: &str = "TRREFRESHVS";
    let stack = stack::shared();
    let library = schema::recorded_driver_path(&stack.exasol, VIRTUAL_SCHEMA);
    schema::create_virtual_schema_through(
        &stack.exasol,
        schema::supplied_driver_spec(
            REFRESH_SCHEMA,
            Driver::Trino,
            stack::TRINO_CONNECTION,
            &library,
        ),
    );
    let before = declared_columns(REFRESH_SCHEMA, "refresh_target");

    stack
        .trino
        .execute("ALTER TABLE memory.adbc_vs.refresh_target ADD COLUMN extra DECIMAL(9,3)");
    stack
        .exasol
        .execute(&format!("ALTER VIRTUAL SCHEMA {REFRESH_SCHEMA} REFRESH"));
    let after_add = declared_columns(REFRESH_SCHEMA, "refresh_target");
    schema::drop_virtual_schema(&stack.exasol, REFRESH_SCHEMA);

    assert!(
        !before.iter().any(|(name, _)| name == "extra"),
        "the fixture must not already carry the column the refresh is to discover: {before:?}"
    );
    assert!(
        after_add.contains(&("extra".to_string(), "DECIMAL(9,3)".to_string())),
        "a refresh must re-run discovery and expose the new source column under Trino's own \
         decimal mapping: {after_add:?}"
    );
}

/// `items` mixes mapped and unmapped columns on purpose
/// (`docker/trino/init.sql`), so every one of its unmapped columns must be
/// left out of the virtual table and recorded under its own Trino type and
/// reason, while the wholly unmapped `all_unmapped` table must be absent
/// from `schemaMetadata` entirely rather than exposed thin.
#[test]
fn unmapped_items_columns_are_recorded_and_all_unmapped_table_is_absent() {
    let columns = column_names(VIRTUAL_SCHEMA, "items");
    let notes = adapter_notes(VIRTUAL_SCHEMA);
    let omitted = notes["omitted"]
        .as_array()
        .expect("adapter notes carry an omitted array");

    for (column, source_type) in [
        ("code", "char(8)"),
        ("bio", "varchar"),
        ("active", "boolean"),
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
            "the recorded sourceType must be Trino's own data_type string: {entry}"
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

/// `vs-adapter/create-virtual-schema`'s `database` field records the
/// namespace as an ordered segment list once a source needs more than one —
/// a Trino table is named by its catalog and its schema, so this is the
/// multi-segment case `SourceQualifier`'s own array form exists for.
#[test]
fn adapter_notes_record_the_two_segment_namespace() {
    let notes = adapter_notes(VIRTUAL_SCHEMA);
    assert_eq!(
        notes["database"],
        serde_json::json!([trino::CATALOG, trino::SCHEMA]),
        "a Trino source's namespace is recorded as its two segments, catalog then schema, in an \
         array rather than a joined string: {notes}"
    );
}

/// `driver-provisioning/driver-path-override`'s success half for Trino: a
/// virtual schema handed a library an operator placed loads it, discovers
/// through it, and provisions nothing.
#[test]
fn driver_path_is_accepted_and_bypasses_provisioning() {
    let stack = stack::shared();
    const SUPPLIED_SCHEMA: &str = "TRVS_SUPPLIED";

    let library = adapter_notes(VIRTUAL_SCHEMA)["driverPath"]
        .as_str()
        .expect("the provisioned schema records the path it loaded its driver from")
        .to_string();

    schema::create_virtual_schema_through(
        &stack.exasol,
        schema::VirtualSchemaSpec {
            schema: SUPPLIED_SCHEMA,
            driver: Driver::Trino,
            connection: stack::TRINO_CONNECTION,
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
    assert_eq!(notes["driverPath"], library);
    assert!(
        notes.get("driverVersion").is_none(),
        "nothing resolved a version for a library the operator supplied: {notes}"
    );

    schema::drop_virtual_schema(&stack.exasol, SUPPLIED_SCHEMA);
}

/// Scoped to the two drivers this binary's own stack provisions, Trino and
/// MySQL, the same way `e2e_postgres_test.rs`'s own version of this
/// assertion is scoped to PostgreSQL and MySQL — no single stack provisions
/// every driver (see the `Correction, round 2` entry in decision-log.md), so
/// each end-to-end tier asserts this over its own two.
#[test]
fn every_driver_this_stack_provisions_creates_a_virtual_schema() {
    let stack = stack::shared();

    for (driver, connection) in [
        (Driver::Trino, stack::TRINO_CONNECTION),
        (Driver::MySql, stack::MYSQL_CONNECTION),
    ] {
        let schema_name = format!("EVERYDRIVER_{}", driver.as_str().to_ascii_uppercase());

        schema::create_virtual_schema(&stack.exasol, driver, &schema_name, connection);

        assert!(
            !table_names(&schema_name).is_empty(),
            "{schema_name} must expose at least one table: a schema that created successfully \
             but discovered nothing would still pass a DDL-only assertion"
        );
        assert_eq!(
            adapter_notes(&schema_name)["driver"],
            serde_json::Value::String(driver.as_str().to_string()),
            "{schema_name}'s adapterNotes must record the driver it was created with"
        );

        schema::drop_virtual_schema(&stack.exasol, &schema_name);
    }
}

// --- per-schema capability difference (`vs-adapter/trino-pushdown`) ---------

/// The standing regression test behind this plan's keystone precondition: a
/// `FULL OUTER JOIN` over the Trino schema's own joinable pair must delegate
/// whole, because `trino.toml` advertises `JOIN_TYPE_FULL_OUTER`, while the
/// same shape over the MySQL schema must be withdrawn into scans carrying no
/// join at all, because `mysql.toml` does not. Both schemas are created
/// through the ordinary `CREATE VIRTUAL SCHEMA` path against the shipped
/// `ADBC_ADAPTER`, so the only thing that can explain a difference is
/// `getCapabilities` resolving its dialect from `adapterNotes.driver`, live.
#[test]
fn two_virtual_schemas_advertise_their_own_capability_sets() {
    let query = |schema: &str| {
        format!(
            "SELECT i.\"id\" AS ID, n.\"note\" AS NOTE FROM {schema}.\"items\" i FULL OUTER \
             JOIN {schema}.\"item_notes\" n ON i.\"id\" = n.\"item_id\""
        )
    };

    let trino_statements = generated_statements(&query(VIRTUAL_SCHEMA));
    assert!(
        trino_statements
            .iter()
            .any(|statement| statement.contains("FULL OUTER JOIN")),
        "the Trino schema advertises JOIN_TYPE_FULL_OUTER, so one generated statement must \
         carry the delegated join: {trino_statements:?}"
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

/// The test that would fail if package resolution or metadata discovery
/// defaulted to one driver: each technology's library lands under its own
/// BucketFS path, and a query against each virtual schema returns the row
/// its own source holds rather than the other source's.
#[cfg(not(feature = "exasol-personal"))]
#[test]
fn two_drivers_install_distinct_libraries_and_each_query_reaches_its_own_source() {
    let stack = stack::shared();

    let trino_library = installed_driver(stack, Driver::Trino).unwrap_or_else(|| {
        panic!(
            "BucketFS lists {:?}, which includes no Trino driver",
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
        trino_library, mysql_library,
        "each driver must land at its own path, not share one with the other"
    );
    assert!(
        trino_library.starts_with("adbc-drivers/trino/")
            && trino_library.ends_with("libadbc_driver_trino.so"),
        "the Trino library must sit under its own technology's prefix: {trino_library}"
    );
    assert!(
        mysql_library.starts_with("adbc-drivers/mysql/")
            && mysql_library.ends_with("libadbc_driver_mysql.so"),
        "the MySQL library must sit under its own technology's prefix: {mysql_library}"
    );

    let trino_rows = stack.exasol.query_rows(&format!(
        "SELECT \"id\", \"category\" FROM {VIRTUAL_SCHEMA}.\"items\" WHERE \"id\" = 1"
    ));
    assert_eq!(
        cell(&trino_rows[0], "category"),
        Some("widget".to_string()),
        "the Trino schema must return the row Trino holds, not MySQL's: {trino_rows:?}"
    );

    let mysql_rows = stack.exasol.query_rows(&format!(
        "SELECT \"id\", \"name\" FROM {}.\"orders\" WHERE \"id\" = 1",
        stack::MYSQL_VIRTUAL_SCHEMA
    ));
    assert_eq!(
        cell(&mysql_rows[0], "name"),
        Some("Widget".to_string()),
        "the MySQL schema must return the row MySQL holds, not Trino's: {mysql_rows:?}"
    );
}

// --- helpers ----------------------------------------------------------------

/// The bucket-relative path of one technology's installed driver library,
/// whatever version resolution selected for it, or nothing when no version
/// is installed for that driver.
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

/// Runs `query(schema)` against the virtual schema and against its local
/// mirror and asserts the two answers are identical.
fn assert_matches_mirror(
    query: impl Fn(&str) -> String,
) -> Vec<serde_json::Map<String, serde_json::Value>> {
    let stack = stack::shared();
    let virtual_rows = stack.exasol.query_rows(&query(VIRTUAL_SCHEMA));
    let mirror_rows = stack.exasol.query_rows(&query(trino_mirror::SCHEMA));
    assert_eq!(
        virtual_rows, mirror_rows,
        "the delegated query must return exactly the rows the local Exasol mirror returns: \
         virtual {virtual_rows:?}, mirror {mirror_rows:?}"
    );
    virtual_rows
}

/// The one statement the adapter generated for `query`, failing when it
/// generated any other number.
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
         for it: {query}"
    );
    statements
}

/// The source statement embedded in one `pushdown` response.
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

/// The values of `column` across `rows`, in the order the query returned
/// them.
fn column_values(rows: &[serde_json::Map<String, serde_json::Value>], column: &str) -> Vec<String> {
    rows.iter().filter_map(|row| cell(row, column)).collect()
}

/// The name and declared Exasol type of every column of `schema.table`, in
/// ordinal-position order.
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
