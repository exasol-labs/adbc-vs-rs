//! Renders the adapter's query model as PostgreSQL, and advertises exactly
//! the query fragments that rendering covers.
//!
//! PostgreSQL identifiers are quoted with double quotes, doubling any double
//! quote the identifier itself contains. Every identifier is quoted, including
//! an all-lower-case one: PostgreSQL folds an *unquoted* identifier to lower
//! case, so a source name that is not already lower case would resolve to an
//! object that does not exist, and quoting is what keeps
//! `vs-adapter/create-virtual-schema`'s rule that a source identifier reaches
//! the source character for character.
//!
//! A table is qualified by the namespace `adapterNotes` recorded, which for
//! this source is the `public` schema rather than the database the connection
//! address names — a PostgreSQL database is not a SQL qualifier, so the
//! qualifier and the connection target are two different facts and only the
//! first reaches a statement.
//!
//! A string literal renders as the escape string constant `E'<body>'`, with a
//! backslash doubled and a single quote doubled, and it renders that way for
//! every value rather than only for one that needs it. The `E` prefix fixes
//! the escape processing whatever `standard_conforming_strings` is set to, and
//! doubling the quote rather than backslash-escaping it keeps the literal
//! independent of `backslash_quote`; measured under all four combinations of
//! the two settings, the same rendering reproduces the stored bytes. MySQL's
//! hex route is rejected here for typing rather than verbosity: a decoded hex
//! literal is `text`, and PostgreSQL coerces no `text` to a numeric or
//! temporal column, where `E'<body>'` yields an untyped literal PostgreSQL
//! resolves from the other operand.
//!
//! No operand carries a collation suffix. A delegated character comparison,
//! `LIKE`, `IN`, grouping key, `DISTINCT` operand, `MIN` or `MAX` operand, and
//! join key is evaluated under the collation PostgreSQL applies to the column,
//! which is the database default unless the column declares its own, and this
//! dialect reads no collation to decide anything. Equality, `LIKE`, `IN`,
//! `GROUP BY`, and `DISTINCT` therefore return Exasol's answer over a
//! deterministic collation, and *order* follows the collation's sort order.
//! A user who needs Exasol's code-point order declares `COLLATE "C"` at the
//! source or wraps the operand in an expression no dialect advertises, as
//! `vs-adapter/postgresql-collation` states.
//!
//! Every column reference renders through the read expression PostgreSQL's
//! type mapping names for its recorded source type, in every position. A
//! grouping key and its select-list occurrence therefore render as the same
//! expression, and PostgreSQL accepts a select-list expression over a grouped
//! one, so its full-group-by rule is satisfied with no coordination between
//! the `GROUP BY` clause and the select list.
//!
//! A join adds the one decision no column reference can answer alone: which
//! table it belongs to. Every table of a join renders under an alias this
//! dialect derives from the table's position in the `from` tree, and every
//! reference renders qualified by the alias of the table it resolves to. A
//! reference this dialect cannot resolve to exactly one table fails naming it,
//! because PostgreSQL would otherwise resolve the name against whichever
//! joined table happens to declare it — ambiguous where two do, and silently
//! the wrong copy in a self-join. A single-table query qualifies and aliases
//! nothing: one table's column cannot be ambiguous.
//!
//! `FULL OUTER JOIN` is the one join type this dialect renders and the MySQL
//! dialect does not, and it is the only capability the two sets differ in.
//! PostgreSQL rejects a full join whose condition is neither merge-joinable
//! nor hash-joinable — measured, `ON "l"."k" < "r"."k"` is refused on 18.6 —
//! but only `JOIN_CONDITION_EQUI` is advertised, and the equality that
//! capability admits is both, so the restriction does not bind. Measured on
//! the same server, the full join over a `character varying` key with no
//! collation suffix returns the unmatched row of each side.
//!
//! Nothing in this module derives from `mysql_dialect`, and neither governs
//! the other: the two answer the same questions about different targets, and
//! `vs-adapter/postgresql-collation` states this rule set independently of
//! `vs-adapter/mysql-collation`. The alias namespace is a second such
//! question — the two dialects happen to answer it the same way, and each
//! answers it for itself, because which alias a table renders under is a
//! rendering decision `vs-adapter/pushdown-join` places in the dialect.

use crate::dialect_common::{
    counted, is_numeric_constant, plain_dialect, render_literal_with, render_select,
    StatementRenderer, Tables,
};
use crate::exa_type::ExasolType;
use crate::postgres_type_mapping;
use crate::pushdown_query::{ColumnRef, Expression, Literal, OperandConversion};
use crate::sql_dialect::RenderError;

/// The PostgreSQL keyword each `join_type` this dialect renders spells, and
/// the closed set of values it renders at all.
///
/// `full_outer` has an entry, which is the one place this list differs from
/// the MySQL dialect's: PostgreSQL implements the keyword directly, where
/// MySQL implements no `FULL OUTER JOIN` and emulating one needs the union
/// rewrite mission.md places out of scope.
const JOIN_KEYWORDS: &[(&str, &str)] = &[
    ("inner", "INNER JOIN"),
    ("left_outer", "LEFT OUTER JOIN"),
    ("right_outer", "RIGHT OUTER JOIN"),
    ("full_outer", "FULL OUTER JOIN"),
];

/// The PostgreSQL dialect.
pub struct PostgresDialect;

plain_dialect!(
    PostgresDialect,
    Postgres,
    "PostgreSQL",
    "PostgreSQL",
    "DOUBLE PRECISION",
    {
        /// Adds the operand as Exasol converts it. PostgreSQL defines no `sum`
        /// over a boolean or a character type, so a boolean sums as an `INTEGER`
        /// and a character operand as a `DOUBLE PRECISION` with an empty value read
        /// as null. A column declared `DOUBLE` is cast too, because `sum` over
        /// `real` returns `real` and adds in single precision, which would round a
        /// sum Exasol computes in double precision.
        fn summed(&self, argument: &Expression, operand: &str) -> String {
            let conversion = argument.aggregate_operand_conversion();
            match conversion {
                Some(OperandConversion::Boolean) => format!("CAST({operand} AS INTEGER)"),
                Some(OperandConversion::Character) => {
                    format!("CAST({} AS DOUBLE PRECISION)", counted(conversion, operand))
                }
                None if is_double_column(argument) => {
                    format!("CAST({operand} AS DOUBLE PRECISION)")
                }
                None => operand.to_string(),
            }
        }

        /// Over a `BOOLEAN` operand, which PostgreSQL defines neither `MIN` nor
        /// `MAX` for, `MIN` is `bool_and` and `MAX` is `bool_or`.
        fn render_plain_aggregate(
            &self,
            function: &str,
            argument: &Expression,
            distinct: &str,
            operand: &str,
        ) -> String {
            if argument.aggregate_operand_conversion() == Some(OperandConversion::Boolean) {
                return format!("{}({distinct}{operand})", boolean_aggregate(function));
            }
            format!("{function}({distinct}{operand})")
        }

        /// Renders one column reference through the read expression PostgreSQL's
        /// type mapping names for its recorded source type, so every position acts
        /// on the value the scan emits for it.
        fn render_column(&self, column: &ColumnRef) -> Result<String, RenderError> {
            let named = self.name_column(column)?;

            Ok(postgres_type_mapping::column_read_sql(
                column.source_type.as_deref(),
                column.declared_type,
                &named,
            ))
        }
    }
);

fn is_double_column(operand: &Expression) -> bool {
    matches!(
        operand,
        Expression::Column(column) if column.declared_type == ExasolType::Double
    )
}

/// The aggregate PostgreSQL spells for `function` over a `BOOLEAN` operand.
///
/// Exasol orders `false` before `true`, so its `MIN` is true only when every
/// value is, and its `MAX` when any is: `bool_and` and `bool_or`. `COUNT`
/// takes any operand and keeps its name.
fn boolean_aggregate(function: &str) -> &str {
    match function {
        "MIN" => "bool_and",
        "MAX" => "bool_or",
        other => other,
    }
}

fn render_literal(literal: &Literal) -> Result<String, RenderError> {
    render_literal_with(
        "PostgreSQL",
        literal,
        render_numeric_literal,
        render_string_literal,
    )
}

/// Renders a string literal as the escape string constant `E'<body>'`.
///
/// Both replacements are unconditional, so one form covers every value and no
/// literal can change the statement's shape. Doubling the quote rather than
/// writing `\'` is what keeps the literal readable under
/// `backslash_quote = off`, which rejects `\'` outright; doubling the
/// backslash is what the `E` prefix requires, and the prefix is what makes the
/// result independent of `standard_conforming_strings`.
fn render_string_literal(value: &str) -> String {
    let body = value.replace('\\', r"\\").replace('\'', "''");

    format!("E'{body}'")
}

fn render_numeric_literal(value: &str) -> Result<String, RenderError> {
    if !is_numeric_constant(value) {
        return Err(RenderError::new(format!(
            "the numeric literal value `{value}` is not spelled as a bare PostgreSQL numeric \
             constant — an optional sign, digits carrying at most one decimal point, and an \
             optional `e` exponent; PostgreSQL accepts a number only unquoted, and its `--` \
             comment needs no following space, so any other character in it could change the \
             statement's shape"
        )));
    }

    Ok(value.to_string())
}

/// Visible to the crate's own tests: `capabilities_tests` drives the
/// per-dialect capability invariants through each dialect's fixture table,
/// and this module owns PostgreSQL's.
#[cfg(test)]
#[path = "postgres_dialect_tests.rs"]
pub(crate) mod tests;
