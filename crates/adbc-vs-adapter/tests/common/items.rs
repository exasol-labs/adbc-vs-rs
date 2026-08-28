//! The `items` fixture rows and their join partner's, owned here once so the
//! MySQL source and the local Exasol mirror are seeded from the same values.
//!
//! Both engines' `INSERT` statements render from the constants below rather
//! than being authored twice, and that is a correctness requirement rather
//! than tidiness. The differential end-to-end tests compare a delegated
//! query's rows against the mirror's, so a value that differs between the two
//! fixtures makes every one of those comparisons report a pushdown defect
//! that does not exist — and the two values most likely to drift are the two
//! a reader cannot see drifting:
//!
//! - `Gîzmo` carries a precomposed `î`. The same character has a second,
//!   byte-different Unicode spelling as `i` plus a combining circumflex, and
//!   decision-log `[19]` measured MySQL's `utf8mb4_0900_as_cs` reporting the
//!   two spellings equal. A decomposed mirror value would therefore group
//!   with the source's under the delegated collation and still return
//!   different bytes to compare.
//! - `'ab '` differs from `'ab'` by a trailing space, which decision-log
//!   `[6]` measured as the case where a `PAD SPACE` collation makes MySQL's
//!   `MIN` disagree with Exasol's.
//!
//! Neither is legible in a diff, so neither is left to two independent
//! authors.
//!
//! What this module owns is the values and their SQL literal form. Each
//! consumer names its own columns in its own quoting: the source's types,
//! `latin1` column, and primary keys belong to [`super::mysql`], and the
//! Exasol types the mirror declares belong to [`super::mirror`].

/// One `items` row, in the source table's column order.
pub struct Item {
    pub id: i64,
    pub category: &'static str,
    pub code: &'static str,
    pub batch: &'static str,
    pub quantity: Option<i64>,
    pub rank_key: i64,
    pub tie_break: i64,
    /// `latin1` at the source, and therefore absent from the mirror:
    /// metadata discovery unmaps a non-`utf8mb4` character column, so no
    /// delegated query can reach this one and no mirror column corresponds
    /// to it.
    pub region: &'static str,
}

/// One `item_notes` row. The same columns on both sides, so one rendering
/// serves both.
pub struct Note {
    pub item_id: i64,
    pub note: &'static str,
}

/// Every `items` row, seeded out of the order `ORDER BY rank_key, tie_break`
/// returns so an ordering test cannot pass on storage order alone.
pub const ROWS: [Item; 8] = [
    Item {
        id: 1,
        category: "Widget",
        code: "ITEM0001",
        batch: "ab ",
        quantity: Some(10),
        rank_key: 3,
        tie_break: 2,
        region: "North",
    },
    Item {
        id: 2,
        category: "Widget",
        code: "ITEM0002",
        batch: "ab",
        quantity: Some(2),
        rank_key: 3,
        tie_break: 1,
        region: "South",
    },
    Item {
        id: 3,
        category: "widget",
        code: "ITEM0003",
        batch: "ab!",
        quantity: None,
        rank_key: 1,
        tie_break: 2,
        region: "East",
    },
    Item {
        id: 4,
        category: "widget",
        code: "ITEM0004",
        batch: "ab ",
        quantity: Some(7),
        rank_key: 1,
        tie_break: 1,
        region: "West",
    },
    Item {
        id: 5,
        category: "Gizmo",
        code: "ITEM0005",
        batch: "ab",
        quantity: Some(1),
        rank_key: 4,
        tie_break: 2,
        region: "North",
    },
    Item {
        id: 6,
        category: "Gizmo",
        code: "ITEM0006",
        batch: "ab!",
        quantity: None,
        rank_key: 4,
        tie_break: 1,
        region: "South",
    },
    Item {
        id: 7,
        category: "Gîzmo",
        code: "ITEM0007",
        batch: "ab ",
        quantity: Some(9),
        rank_key: 2,
        tie_break: 2,
        region: "East",
    },
    Item {
        id: 8,
        category: "Gîzmo",
        code: "ITEM0008",
        batch: "ab",
        quantity: Some(3),
        rank_key: 2,
        tie_break: 1,
        region: "West",
    },
];

/// Every `item_notes` row. `items.id` 1 carries none, so it is unmatched on
/// the `items` side, and `item_id` 99 matches no `items` row, so it is
/// unmatched on the `item_notes` side.
pub const NOTES: [Note; 8] = [
    Note {
        item_id: 2,
        note: "restocked",
    },
    Note {
        item_id: 3,
        note: "discontinued",
    },
    Note {
        item_id: 4,
        note: "seasonal",
    },
    Note {
        item_id: 5,
        note: "restocked",
    },
    Note {
        item_id: 6,
        note: "discontinued",
    },
    Note {
        item_id: 7,
        note: "seasonal",
    },
    Note {
        item_id: 8,
        note: "restocked",
    },
    Note {
        item_id: 99,
        note: "orphan",
    },
];

/// The `VALUES` tuple list of every row in the source's column order:
/// `id, category, code, batch, quantity, rank_key, tie_break, region`.
pub fn source_values() -> String {
    tuple_list(ROWS.iter().map(|row| {
        format!(
            "{}, {}, {}, {}",
            shared_fields(row),
            row.rank_key,
            row.tie_break,
            text(row.region)
        )
    }))
}

/// The `VALUES` tuple list of every row in the mirror's column order, which
/// is the source's without `region`:
/// `id, category, code, batch, quantity, rank_key, tie_break`.
pub fn mirror_values() -> String {
    tuple_list(ROWS.iter().map(|row| {
        format!(
            "{}, {}, {}",
            shared_fields(row),
            row.rank_key,
            row.tie_break
        )
    }))
}

/// The `VALUES` tuple list of every join-partner row, in the order
/// `item_id, note` both sides declare.
pub fn note_values() -> String {
    tuple_list(
        NOTES
            .iter()
            .map(|row| format!("{}, {}", row.item_id, text(row.note))),
    )
}

/// The fields every consumer renders identically, up to and including
/// `quantity`. Keeping them in one place is what makes the mirror's tuple the
/// source's tuple with a column removed rather than a second authoring of the
/// same values.
fn shared_fields(row: &Item) -> String {
    format!(
        "{}, {}, {}, {}, {}",
        row.id,
        text(row.category),
        text(row.code),
        text(row.batch),
        number_or_null(row.quantity)
    )
}

fn tuple_list(tuples: impl Iterator<Item = String>) -> String {
    tuples
        .map(|fields| format!("({fields})"))
        .collect::<Vec<String>>()
        .join(", ")
}

/// One value's SQL literal form, identical in both engines.
///
/// A doubled quote is the one escape MySQL and Exasol read the same way. A
/// backslash is not: MySQL's default `sql_mode` reads it as an escape
/// introducer and Exasol reads it as an ordinary character, which decision-log
/// `[8]` measured, so a value carrying one would leave the two fixtures
/// holding different bytes. Rejecting it keeps that divergence from being
/// seeded silently.
fn text(value: &str) -> String {
    assert!(
        !value.contains('\\'),
        "fixture value {value:?} carries a backslash: MySQL's default sql_mode reads it as an \
         escape introducer and Exasol reads it as a character, so the source and the mirror \
         would hold different values"
    );
    format!("'{}'", value.replace('\'', "''"))
}

/// A nullable number's SQL literal form.
fn number_or_null(value: Option<i64>) -> String {
    match value {
        Some(number) => number.to_string(),
        None => "NULL".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_values_renders_every_row_with_its_region() {
        let values = source_values();

        assert_eq!(
            values.matches("ITEM").count(),
            ROWS.len(),
            "every seeded row must be rendered: {values}"
        );
        assert!(
            values.contains("'North'"),
            "the source carries the latin1 region column: {values}"
        );
        assert!(
            values.starts_with("(1, 'Widget', 'ITEM0001', 'ab ', 10, 3, 2, 'North')"),
            "the tuple must follow the source's own column order: {values}"
        );
    }

    #[test]
    fn mirror_values_renders_every_row_without_the_unmapped_region_column() {
        let values = mirror_values();

        assert_eq!(
            values.matches("ITEM").count(),
            ROWS.len(),
            "every seeded row must be rendered: {values}"
        );
        assert!(
            !values.contains("'North'") && !values.contains("'West'"),
            "the mirror has no counterpart for the latin1 region column, which metadata \
             discovery unmaps: {values}"
        );
        assert!(
            values.starts_with("(1, 'Widget', 'ITEM0001', 'ab ', 10, 3, 2)"),
            "the tuple must follow the mirror's own column order: {values}"
        );
    }

    #[test]
    fn a_trailing_space_survives_rendering() {
        assert!(
            source_values().contains("'ab '") && mirror_values().contains("'ab '"),
            "the value differing from another only by a trailing space is what decision [6] \
             measured MIN diverging over, so it must reach both fixtures intact"
        );
    }

    #[test]
    fn a_null_renders_as_null_rather_than_as_a_value() {
        assert_eq!(number_or_null(None), "NULL");
        assert_eq!(number_or_null(Some(7)), "7");
        assert_eq!(number_or_null(Some(0)), "0");
    }

    #[test]
    fn note_values_renders_the_unmatched_row_on_each_side() {
        let values = note_values();

        assert_eq!(
            values.matches('(').count(),
            NOTES.len(),
            "every join-partner row must be rendered: {values}"
        );
        assert!(
            values.contains("(99, 'orphan')"),
            "the row no items row matches must be rendered, so the join is unmatched on the \
             item_notes side: {values}"
        );
        assert!(
            !values.contains("(1, "),
            "no row may match items.id 1, so the join is unmatched on the items side: {values}"
        );
    }

    #[test]
    fn the_accented_category_is_precomposed_so_both_fixtures_hold_the_same_bytes() {
        let accented: Vec<&str> = ROWS
            .iter()
            .map(|row| row.category)
            .filter(|category| !category.is_ascii())
            .collect();

        assert!(
            !accented.is_empty(),
            "the fixture must carry an accented category value"
        );
        for category in accented {
            assert_eq!(
                category.chars().count(),
                "Gizmo".chars().count(),
                "{category:?} must spell its accent as one precomposed code point, not as a \
                 base character plus a combining mark: MySQL's utf8mb4_0900_as_cs reports the \
                 two spellings equal, so a decomposed value would group with the other \
                 fixture's and still return different bytes"
            );
        }
    }

    #[test]
    fn a_quote_in_a_value_cannot_end_the_literal() {
        assert_eq!(text("a'b"), "'a''b'");
    }

    #[test]
    #[should_panic(expected = "carries a backslash")]
    fn a_backslash_in_a_value_is_rejected_rather_than_rendered() {
        text("a\\b");
    }
}
