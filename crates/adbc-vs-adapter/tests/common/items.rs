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
//!   byte-different Unicode spelling as `i` plus a combining circumflex.
//!   Exasol and the `utf8mb4_0900_bin` columns of `items_bin` both compare
//!   by code point, so a mirror value spelled the other way would hold
//!   different code points than the source's and fail every differential
//!   comparison over it for a reason that is not a pushdown defect.
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
//!
//! A backslash is the one character the two engines' literals read
//! differently: MySQL's default `sql_mode` reads it as an escape introducer
//! and Exasol reads it as an ordinary character, which decision-log `[8]`
//! measured. The rows below therefore render once per engine, so a value
//! holding a backslash reaches each engine as the same characters.

/// One `items` row, in the source table's column order.
pub struct Item {
    pub id: i64,
    pub category: &'static str,
    pub code: &'static str,
    pub batch: &'static str,
    pub quantity: Option<i64>,
    pub rank_key: i64,
    pub tie_break: i64,
    /// `latin1` at the source and `UTF8` in the mirror, so it holds only
    /// characters `latin1` can hold.
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
///
/// `region` repeats `North` with a trailing space and `South` in lower case, so
/// a comparison of the `latin1` column under its own `PAD SPACE`, case-insensitive
/// collation differs from one under `utf8mb4_0900_bin`.
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
        region: "North ",
    },
    Item {
        id: 6,
        category: "Gizmo",
        code: "ITEM0006",
        batch: "ab!",
        quantity: None,
        rank_key: 4,
        tie_break: 1,
        region: "south",
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

/// The two rows only [`CODE_POINT_ITEMS`](super::mysql::CODE_POINT_ITEMS)
/// holds, whose `batch` is a `LIKE` pattern's `%` and its look-alike: `5%x` is
/// matched by the pattern `5\%x` when a backslash escapes the `%`, and `5\yx`
/// is matched when the backslash is an ordinary character. They carry their own
/// category, `Zeta`, which sorts between `Widget` and `widget` in code-point order,
/// so the `MIN` and `MAX` over `category` are the same with or without them.
pub const CODE_POINT_ONLY_ROWS: [Item; 2] = [
    Item {
        id: 9,
        category: "Zeta",
        code: "ITEM0009",
        batch: "5%x",
        quantity: Some(4),
        rank_key: 5,
        tie_break: 1,
        region: "North",
    },
    Item {
        id: 10,
        category: "Zeta",
        code: "ITEM0010",
        batch: "5\\yx",
        quantity: Some(5),
        rank_key: 5,
        tie_break: 2,
        region: "South",
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

/// The `VALUES` tuple list of every `items` row as MySQL reads it, in the
/// source's column order:
/// `id, category, code, batch, quantity, rank_key, tie_break, region`.
pub fn source_values() -> String {
    tuple_list(ROWS.iter().map(|row| row_fields(row, mysql_text)))
}

/// The `VALUES` tuple list of every `items` row as Exasol reads it, in the
/// order [`source_values`] renders them.
pub fn mirror_values() -> String {
    tuple_list(ROWS.iter().map(|row| row_fields(row, exasol_text)))
}

/// [`source_values`] followed by the rows only the code-point table holds.
pub fn code_point_source_values() -> String {
    tuple_list(
        ROWS.iter()
            .chain(CODE_POINT_ONLY_ROWS.iter())
            .map(|row| row_fields(row, mysql_text)),
    )
}

/// [`mirror_values`] followed by the rows only the code-point table holds.
pub fn code_point_mirror_values() -> String {
    tuple_list(
        ROWS.iter()
            .chain(CODE_POINT_ONLY_ROWS.iter())
            .map(|row| row_fields(row, exasol_text)),
    )
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

/// One row's fields, each text value rendered by `literal`.
fn row_fields(row: &Item, literal: fn(&str) -> String) -> String {
    format!(
        "{}, {}, {}, {}, {}, {}, {}, {}",
        row.id,
        literal(row.category),
        literal(row.code),
        literal(row.batch),
        number_or_null(row.quantity),
        row.rank_key,
        row.tie_break,
        literal(row.region)
    )
}

fn tuple_list(tuples: impl Iterator<Item = String>) -> String {
    tuples
        .map(|fields| format!("({fields})"))
        .collect::<Vec<String>>()
        .join(", ")
}

/// One value's SQL literal form where both engines read it alike.
///
/// A doubled quote is the one escape MySQL and Exasol read the same way, and a
/// backslash is read differently, so a value carrying one is rejected here and
/// rendered by [`mysql_text`] or [`exasol_text`] instead.
fn text(value: &str) -> String {
    assert!(
        !value.contains('\\'),
        "fixture value {value:?} carries a backslash: MySQL's default sql_mode reads it as an \
         escape introducer and Exasol reads it as a character, so render it per engine"
    );
    exasol_text(value)
}

/// One value's literal as Exasol reads it: only the quote is special.
fn exasol_text(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

/// One value's literal as MySQL reads it under its default `sql_mode`, where a
/// backslash introduces an escape.
fn mysql_text(value: &str) -> String {
    format!("'{}'", value.replace('\\', "\\\\").replace('\'', "''"))
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
    fn mirror_values_renders_every_row_with_its_region_in_the_sources_order() {
        let values = mirror_values();

        assert_eq!(
            values.matches("ITEM").count(),
            ROWS.len(),
            "every seeded row must be rendered: {values}"
        );
        assert!(
            values.starts_with("(1, 'Widget', 'ITEM0001', 'ab ', 10, 3, 2, 'North')"),
            "the mirror's tuple must follow the source's column order, region included, since \
             a MySQL latin1 column maps and so has a mirror column: {values}"
        );
    }

    #[test]
    fn the_code_point_table_holds_the_two_like_rows_the_plain_table_lacks() {
        for values in [code_point_source_values(), code_point_mirror_values()] {
            assert_eq!(
                values.matches("ITEM").count(),
                ROWS.len() + CODE_POINT_ONLY_ROWS.len(),
                "every row, including the two `LIKE` rows, must be rendered: {values}"
            );
            assert!(
                values.contains("'5%x'"),
                "the row a backslash-escaped `%` matches must be seeded: {values}"
            );
        }
        assert_eq!(
            source_values().matches("ITEM").count(),
            ROWS.len(),
            "the plain table must not gain the two `LIKE` rows: every other differential test \
             counts its eight rows"
        );
    }

    #[test]
    fn a_backslash_reaches_each_engine_as_one_character() {
        assert!(
            code_point_source_values().contains("'5\\\\yx'"),
            "MySQL reads a backslash as an escape introducer, so the literal doubles it: {}",
            code_point_source_values()
        );
        assert!(
            code_point_mirror_values().contains("'5\\yx'")
                && !code_point_mirror_values().contains("'5\\\\yx'"),
            "Exasol reads a backslash as a character, so the literal keeps one: {}",
            code_point_mirror_values()
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
                 base character plus a combining mark: the two spellings hold different code \
                 points, so a decomposed value in one fixture would differ from the other \
                 fixture's under every code-point comparison"
            );
        }
    }

    #[test]
    fn a_quote_in_a_value_cannot_end_the_literal() {
        assert_eq!(text("a'b"), "'a''b'");
    }

    #[test]
    #[should_panic(expected = "carries a backslash")]
    fn a_backslash_in_a_value_both_engines_must_read_alike_is_rejected() {
        text("a\\b");
    }
}
