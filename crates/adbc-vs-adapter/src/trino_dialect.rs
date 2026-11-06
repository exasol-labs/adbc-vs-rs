//! Renders the adapter's query model as Trino SQL, and advertises exactly
//! the query fragments that rendering covers.
//!
//! Trino identifiers are quoted with double quotes, doubling any double quote
//! the identifier itself contains — the same rule `postgres_dialect` applies,
//! for the same reason: an unquoted identifier folds case, and quoting is
//! what keeps `vs-adapter/create-virtual-schema`'s rule that a source
//! identifier reaches the source character for character.
//!
//! A table is qualified by every segment the query model's namespace carries,
//! each quoted and joined by a dot — `"catalog"."schema"."table"`. A Trino
//! table sits under two names, its catalog and its schema, and `drivers/
//! trino.toml`'s `qualifier.segment_count = 2` is what the address check and
//! the metadata source agree on for how many. Qualifying fully rather than
//! relying on the connection's own selected catalog matters here specifically
//! because a generated statement carries no notion of "the current catalog":
//! nothing in the request or the rendered SQL depends on connection state
//! `EXPLAIN VIRTUAL` does not show and this adapter does not control.
//!
//! This dialect renders no collation suffix in any position, as every dialect
//! here renders none, and Trino differs from the others in one further way:
//! it has no suffix position to render into at all. `COLLATE` was measured
//! not to be a Trino keyword in any syntactic position at all — every one of
//! the four suffix positions a collation could occupy (`ORDER BY`, a
//! comparison operand, a `GROUP BY` key, and `COUNT(DISTINCT ...)`) rejects
//! it as a parse error, and the parser accepted it as a plain column alias in
//! the `GROUP BY` case before choking on what followed, which is direct
//! evidence `COLLATE` is an ordinary unreserved identifier rather than a
//! suffix grammar Trino has and this dialect declines to use. The two engines
//! were also measured agreeing without any suffix, live against Trino 483 and
//! Exasol 2025.2.1: identical `ORDER BY` order over `a, B, Z, z, A, ä, Ä, é,
//! e`, identical `MIN`/`MAX`, identical row sets for `<` and `<=`, identical
//! `GROUP BY` grouping, and identical `COUNT(DISTINCT)` over the same mixed-
//! case and accented fixture PostgreSQL's own gate used. The reason the two
//! agree is that Trino's `varchar` comparison is Unicode code-point order,
//! and Exasol's is its own byte-order comparison, which for UTF-8 is the same
//! order — confirmed by running both engines side by side rather than assumed
//! from either engine's documentation.
//!
//! A string literal renders as a plain quoted constant, `'<body>'`, with only
//! the single quote doubled and nothing else escaped. Trino defines no
//! backslash escape inside a string constant, so doubling a backslash the
//! way `postgres_dialect`'s `E'<body>'` form must would corrupt a value that
//! already contains one.
//!
//! An exact-numeric literal renders bare, through the same numeric-constant
//! grammar `postgres_dialect` uses: an optional sign, digits carrying at most
//! one decimal point, and an optional `e` exponent. A floating-point literal
//! renders as the typed form `DOUBLE '<value>'` instead. The plan's original
//! reason for that — that a bare decimal constant would hand Trino an
//! exact-numeric operand where Exasol meant a floating-point one, implying a
//! wrong answer — is false: Trino was measured promoting a
//! decimal operand to `double` in a mixed comparison, so `WHERE price =
//! 19.99` and `WHERE price = DOUBLE '19.99'` return the same row. The typed
//! form is still mandatory, for two reasons that are about totality rather
//! than correctness: a literal spelled with an exponent is already typed
//! `double` on its own (`typeof(1e308)` is `double`, whatever form the
//! literal that produced it takes), and a bare decimal constant above 38
//! significant digits is rejected outright rather than widened. The typed
//! form covers both where a bare constant does not.
//!
//! It does not cover every value Exasol can send. `render_double_literal`
//! guards its value with `is_numeric_constant`, which admits only an optional
//! sign, digits, and an optional exponent, so `Infinity` and `NaN` are
//! refused with a named error although the measurement recorded for
//! `vs-adapter/trino-pushdown` found that Trino executes `DOUBLE 'Infinity'`
//! and `DOUBLE 'NaN'`. The guard is narrower than the form it guards on
//! purpose: `vs-adapter/trino-pushdown` requires that a literal value not
//! spelled as a bare numeric constant fail with a named error, because the
//! value sits inside quotes and anything else in it could end the literal
//! early.
//!
//! `LIKE` renders as a plain `LIKE <pattern>` with no `ESCAPE` clause, and a
//! request that carries an `escapeChar` fails instead of rendering. Trino
//! defines no default escape character and rejects an escape character
//! followed by anything other than `%`, `_`, or itself, so neither Exasol's
//! default backslash rule nor a request's own escape character has a Trino
//! form that accepts every pattern Exasol accepts. `FN_PRED_LIKE_ESCAPE` is
//! therefore not advertised: a query that writes an explicit `ESCAPE` clause is
//! evaluated by Exasol, and a backslash in a delegated pattern is an ordinary
//! character at Trino.
//!
//! A join adds the one decision no column reference can answer alone: which
//! table it belongs to. Every table of a join renders under an alias derived
//! from its position in the `from` tree, and every reference renders
//! qualified by the alias of the table it resolves to — the same scheme
//! `postgres_dialect` uses, stated independently here because which alias a
//! table renders under is a rendering decision `vs-adapter/pushdown-join`
//! places in the dialect, not a fact one dialect can hand another.
//! `FULL OUTER JOIN` is unrestricted on Trino: all four join keywords were
//! measured executing over both equi and non-equi conditions, null-
//! extending both sides with no condition restriction PostgreSQL's merge-
//! joinable/hash-joinable rule imposes — not that it matters here, since only
//! `JOIN_CONDITION_EQUI` is advertised.

use adbc_vs_scan::driver::Driver;

use crate::dialect_common::{
    counted, is_numeric_constant, render_select, render_temporal_literal, StatementRenderer, Tables,
};
use crate::exa_type::ExasolType;
use crate::pushdown_query::{
    ColumnRef, Expression, LikePredicate, Literal, OperandConversion, Query,
};
use crate::sql_dialect::{RenderError, SqlDialect};

use crate::trino_type_mapping;

/// The widest precision Trino's `decimal` holds, which a `SUM` over an
/// exact-numeric operand widens to so the sum cannot overflow its input type.
const MAX_DECIMAL_PRECISION: u32 = 38;

/// The Trino keyword each `join_type` this dialect renders spells, and the
/// closed set of values it renders at all. Every one of these was measured
/// executing over an equi condition with no restriction, so the set is safe
/// to advertise in full.
const JOIN_KEYWORDS: &[(&str, &str)] = &[
    ("inner", "INNER JOIN"),
    ("left_outer", "LEFT OUTER JOIN"),
    ("right_outer", "RIGHT OUTER JOIN"),
    ("full_outer", "FULL OUTER JOIN"),
];

/// The Trino dialect.
pub struct TrinoDialect;

impl SqlDialect for TrinoDialect {
    /// The set `drivers/trino.toml` declares, which is this dialect's claim
    /// about what the renderer below covers rather than a fact about Trino:
    /// the values are a data edit, but advertising one this module cannot
    /// render still returns wrong rows, so the two stay one decision that
    /// `capabilities_tests` renders entry by entry.
    fn capabilities(&self) -> &'static [&'static str] {
        Driver::Trino.capabilities()
    }

    fn render(&self, query: &Query) -> Result<String, RenderError> {
        render_select(
            &Statement {
                tables: Tables::of(&query.from),
            },
            query,
        )
    }
}

/// The one decision a Trino statement settles before any expression in it
/// renders: which table each column reference belongs to, and therefore how
/// a column is named.
///
/// This is the only decision a statement settles here: nothing in this dialect
/// collates, so no decision about which operand carries a suffix exists to
/// thread through rendering.
struct Statement<'a> {
    tables: Tables<'a>,
}

impl StatementRenderer for Statement<'_> {
    const TARGET: &'static str = "Trino";
    const SQL_NAME: &'static str = "Trino SQL";
    const QUOTE: char = '"';
    const JOIN_KEYWORDS: &'static [(&'static str, &'static str)] = JOIN_KEYWORDS;
    const DOUBLE_TYPE: &'static str = "DOUBLE";

    fn tables(&self) -> &Tables<'_> {
        &self.tables
    }

    fn render_literal(&self, literal: &Literal) -> Result<String, RenderError> {
        render_literal(literal)
    }

    /// Adds the operand as Exasol converts it. Trino defines no `sum` over a
    /// boolean or a character type, so a boolean, which for Trino is a
    /// predicate, sums as an `INTEGER` and a character operand as a `DOUBLE`
    /// with an empty value read as null. An exact-numeric column is widened to
    /// `DECIMAL(38, s)`, because Trino's `sum` returns its input's type and a
    /// sum over `bigint` would otherwise fail past the `bigint` range.
    fn summed(&self, argument: &Expression, operand: &str) -> String {
        let conversion = argument.aggregate_operand_conversion();
        match conversion {
            Some(OperandConversion::Boolean) => format!("CAST({operand} AS INTEGER)"),
            Some(OperandConversion::Character) => {
                format!("CAST({} AS DOUBLE)", counted(conversion, operand))
            }
            None => match decimal_scale(argument) {
                Some(scale) => {
                    format!("CAST({operand} AS DECIMAL({MAX_DECIMAL_PRECISION},{scale}))")
                }
                None => operand.to_string(),
            },
        }
    }

    /// Renders a `LIKE` with no `ESCAPE` clause. A request that carries an
    /// `escapeChar` fails, because rendering it without the escape character
    /// would change which rows match, and the capability that admits it is not
    /// advertised.
    fn render_like(&self, like: &LikePredicate) -> Result<String, RenderError> {
        if let Some(escape_char) = &like.escape_char {
            return Err(RenderError::new(format!(
                "a `predicate_like` node carries the `escapeChar` {}, and this dialect does not \
                 advertise `FN_PRED_LIKE_ESCAPE`: Trino defines no default escape character and \
                 rejects an escape character followed by anything other than `%`, `_`, or \
                 itself, so Exasol evaluates a `LIKE` with an `ESCAPE` clause itself",
                self.render_expression(escape_char)?
            )));
        }

        Ok(format!(
            "({} LIKE {})",
            self.render_expression(&like.expression)?,
            self.render_expression(&like.pattern)?
        ))
    }

    /// Renders one column reference through the read expression Trino's type
    /// mapping names for its recorded source type, so every position acts on
    /// the value the scan emits for it.
    fn render_column(&self, column: &ColumnRef) -> Result<String, RenderError> {
        let named = self.name_column(column)?;

        Ok(trino_type_mapping::column_read_sql(
            column.source_type.as_deref(),
            column.declared_type,
            &named,
        ))
    }
}

fn decimal_scale(operand: &Expression) -> Option<u32> {
    match operand {
        Expression::Column(column) => match column.declared_type {
            ExasolType::Decimal { scale, .. } => Some(scale),
            _ => None,
        },
        _ => None,
    }
}

fn render_literal(literal: &Literal) -> Result<String, RenderError> {
    match literal {
        Literal::Null => Ok("NULL".to_string()),
        Literal::ExactNumeric(value) => render_numeric_literal(value),
        Literal::Double(value) => render_double_literal(value),
        Literal::String(value) => Ok(render_string_literal(value)),
        Literal::Date(value) => render_temporal_literal("Trino", "DATE", value),
        Literal::Timestamp(value) => render_temporal_literal("Trino", "TIMESTAMP", value),
    }
}

/// Renders a string literal as a plain quoted constant, doubling only the
/// single quote. Trino defines no backslash escape inside a string constant,
/// so, unlike `postgres_dialect`'s `E'<body>'` form, a backslash the value
/// carries needs no doubling and passes through unchanged.
fn render_string_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn render_numeric_literal(value: &str) -> Result<String, RenderError> {
    if !is_numeric_constant(value) {
        return Err(RenderError::new(format!(
            "the numeric literal value `{value}` is not spelled as a bare numeric constant — an \
             optional sign, digits carrying at most one decimal point, and an optional `e` \
             exponent; Trino accepts a number only unquoted, so any other character in it could \
             change the statement's shape"
        )));
    }

    Ok(value.to_string())
}

/// Renders a floating-point literal as the typed form `DOUBLE '<value>'`,
/// this module's own doc comment states why it is mandatory. The value is
/// checked against the same grammar [`render_numeric_literal`] uses before
/// it is placed inside quotes, so a value that could end the literal early
/// is rejected rather than rendered.
fn render_double_literal(value: &str) -> Result<String, RenderError> {
    if !is_numeric_constant(value) {
        return Err(RenderError::new(format!(
            "the floating-point literal value `{value}` is not spelled as a bare numeric \
             constant — an optional sign, digits carrying at most one decimal point, and an \
             optional `e` exponent; Trino accepts a `DOUBLE` literal's body only inside quotes, \
             so any other character in it could end the literal early"
        )));
    }

    Ok(format!("DOUBLE '{value}'"))
}

/// Visible to the crate's own tests: `capabilities_tests` drives the
/// per-dialect capability invariants through each dialect's fixture table,
/// and this module owns Trino's.
#[cfg(test)]
#[path = "trino_dialect_tests.rs"]
pub(crate) mod tests;
