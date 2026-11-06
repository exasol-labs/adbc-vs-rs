//! Renders the adapter's query model as MySQL, and advertises exactly the
//! query fragments that rendering covers.
//!
//! MySQL identifiers are quoted with backticks, doubling any backtick the
//! identifier itself contains. No identifier is interpolated unquoted, so a
//! name carrying SQL-significant characters cannot change the shape of the
//! generated statement. The same holds for every literal: a string literal
//! renders as hex, and a numeric or temporal literal — which MySQL accepts
//! only spelled out — renders only after its value is verified to carry
//! nothing that could end the literal early.
//!
//! Every predicate renders as its own parenthesized group, so nesting needs
//! no precedence reasoning and the select-list position renders through the
//! same path as `WHERE`.
//!
//! No operand carries a collation, per `vs-adapter/mysql-collation`. MySQL
//! evaluates every delegated character comparison, `LIKE`, `IN`, grouping
//! key, `DISTINCT` operand, `MIN` or `MAX` operand, and join key under the
//! collation it applies to the column. Over MySQL's default
//! `utf8mb4_0900_ai_ci` that matches case and accent variants Exasol keeps
//! apart, and over a column declaring `utf8mb4_0900_bin` it returns Exasol's
//! code-point answers. Choosing the collation is the source's decision, not
//! this dialect's.
//!
//! Every column reference renders through the read expression MySQL's type
//! mapping names for its recorded source type, in every position, and
//! grouping coordinates one thing as a result. MySQL's `ONLY_FULL_GROUP_BY`
//! mode, which `mysql:8.4` sets by default, matches a select-list expression
//! against a `GROUP BY` expression only whole and rejects any other
//! expression over a grouped column with error 1055. A key that reads bare
//! needs nothing: its select-list occurrence is the same bare column, and a
//! select-list predicate reading it is an expression over that column, which
//! MySQL accepts. A key read through a conversion is grouped by its read
//! expression, and its occurrence outside an aggregate in the select list or
//! the `HAVING` clause renders as `ANY_VALUE(<read expression>)`, which has the
//! grouping value within its group and which MySQL exempts from the check. Grouping by the bare column
//! instead would group a read expression that does not preserve equality,
//! such as a multi-bit `BIT` read as `BOOLEAN`, apart from the key the scan
//! emits.
//!
//! Which aggregate functions this dialect renders is its own question,
//! decided here rather than in the model: an aggregate the renderer does not
//! know fails naming itself, since rendering its argument alone would leave
//! Exasol aggregating nothing.
//!
//! A join adds the decision no column reference can answer alone: which
//! table it belongs to. Every table of a join renders under an alias this
//! dialect derives from the table's position in the `from` tree, and every
//! reference renders qualified by the alias of the table it resolves to.
//! Measured on MySQL 8.4, the alternatives each fail rather than diverge
//! quietly: an unqualified reference two joined tables both declare is
//! rejected with error 1052, one table named twice with no alias to tell the
//! copies apart with error 1066, and a grouping key whose select-list
//! occurrence carries a different alias with error 1055. A reference this
//! dialect cannot resolve to exactly one table fails naming it, because
//! MySQL would otherwise resolve the name against whichever joined table
//! happens to declare it. A single-table query qualifies and aliases
//! nothing: one table's column cannot be ambiguous, and its statement stays
//! the one this dialect generated before it rendered joins at all.

use adbc_vs_scan::driver::Driver;

use crate::dialect_common::{
    counted, optional_clause, render_literal_with, StatementRenderer, Tables,
};
use crate::mysql_type_mapping;
use crate::pushdown_query::{
    Aggregation, ColumnRef, Expression, Literal, OperandConversion, Query,
};
use crate::sql_dialect::{RenderError, SqlDialect};

/// The MySQL keyword each `join_type` this dialect renders spells, and the
/// closed set of values it renders at all.
///
/// `full_outer` has no entry. MySQL implements no `FULL OUTER JOIN`, and
/// emulating one as a union of two outer joins is the query rewriting
/// mission.md places out of scope, so `JOIN_TYPE_FULL_OUTER` is not
/// advertised and a request carrying the value fails naming it. Measured on
/// Exasol 2025.2.1, withholding the capability is enough that Exasol
/// withdraws the join and sends a single table instead, so the failure path
/// is a backstop rather than the expected case.
const JOIN_KEYWORDS: &[(&str, &str)] = &[
    ("inner", "INNER JOIN"),
    ("left_outer", "LEFT OUTER JOIN"),
    ("right_outer", "RIGHT OUTER JOIN"),
];

/// The characters a numeric literal's value may carry. None of them can end
/// a statement fragment or begin a comment: MySQL's `--` comment needs a
/// following space, which is not in this set.
const NUMERIC_LITERAL_CHARACTERS: &str = "0123456789+-.eE";

/// The character set a string literal's hex introducer names: the literal
/// carries UTF-8 bytes whatever the column's character set.
const LITERAL_CHARACTER_SET: &str = "utf8mb4";

/// The MySQL dialect.
pub struct MySqlDialect;

impl SqlDialect for MySqlDialect {
    /// The set `drivers/mysql.toml` declares, which is this dialect's claim
    /// about what the renderer below covers rather than a fact about MySQL:
    /// the values are a data edit, but advertising one this module cannot
    /// render still returns wrong rows, so the two stay one decision that
    /// `capabilities_tests` renders entry by entry.
    fn capabilities(&self) -> &'static [&'static str] {
        Driver::MySql.capabilities()
    }

    fn render(&self, query: &Query) -> Result<String, RenderError> {
        let tables = Tables::of(&query.from);
        let per_row = Statement {
            tables: &tables,
            any_value_keys: &[],
        };
        let grouping_keys = per_row.render_grouping_keys(query.aggregation.as_ref())?;
        let select_list = Statement {
            any_value_keys: &grouping_keys,
            ..per_row
        };

        let projection = select_list.render_projection(&query.select_list)?;
        let from = per_row.render_from()?;
        let filter = optional_clause("WHERE", query.filter.as_ref(), |predicate| {
            per_row.render_predicate(predicate)
        })?;
        let grouping = if grouping_keys.is_empty() {
            String::new()
        } else {
            format!(" GROUP BY {}", grouping_keys.join(", "))
        };
        let having = optional_clause("HAVING", query.having.as_ref(), |predicate| {
            select_list.render_predicate(predicate)
        })?;

        Ok(format!(
            "SELECT {projection} FROM {from}{filter}{grouping}{having}"
        ))
    }
}

/// One position of the statement being rendered: how it names its tables, and
/// which grouping keys read through `ANY_VALUE` there.
///
/// The keys are the second decision a MySQL statement settles before its
/// select list renders. `ONLY_FULL_GROUP_BY` matches a select-list expression
/// against a `GROUP BY` expression only whole, so outside an aggregate in the
/// select list and in the `HAVING` clause a key grouped by a read expression
/// other than the bare column reads as `ANY_VALUE(<read expression>)`, which
/// has the grouping value within its group. Every other position carries no
/// key: a `WHERE` or `ON` operand and an aggregate's argument are read per row.
#[derive(Clone, Copy)]
struct Statement<'s> {
    tables: &'s Tables<'s>,
    any_value_keys: &'s [String],
}

impl Statement<'_> {
    /// This statement in a position read per row, where no key reads through
    /// `ANY_VALUE`.
    fn per_row(self) -> Self {
        Self {
            any_value_keys: &[],
            ..self
        }
    }

    /// Renders the `GROUP BY` keys an aggregation names, each as its read
    /// expression, and none for a single-group aggregation or none at all.
    ///
    /// Each key renders through the renderer its select-list occurrence takes
    /// too. Exasol qualifies the two occurrences independently, `tableName` on
    /// one and `tableAlias` on the other, but both resolve to the same table
    /// and so render identically, which MySQL's `ONLY_FULL_GROUP_BY` requires:
    /// it rejects the statement with error 1055 when they differ. That
    /// identity is also how the select list recognizes a key it reads through
    /// `ANY_VALUE`.
    fn render_grouping_keys(
        &self,
        aggregation: Option<&Aggregation>,
    ) -> Result<Vec<String>, RenderError> {
        let Some(Aggregation::GroupBy(columns)) = aggregation else {
            return Ok(Vec::new());
        };

        self.per_row().render_group_by_keys(columns)
    }
}

impl StatementRenderer for Statement<'_> {
    const TARGET: &'static str = "MySQL";
    const SQL_NAME: &'static str = "MySQL";
    const QUOTE: char = '`';
    const JOIN_KEYWORDS: &'static [(&'static str, &'static str)] = JOIN_KEYWORDS;
    const JOIN_NOTE: &'static str = "MySQL implements no `FULL OUTER JOIN`, and ";
    const DOUBLE_TYPE: &'static str = "DOUBLE";

    fn tables(&self) -> &Tables<'_> {
        self.tables
    }

    fn render_literal(&self, literal: &Literal) -> Result<String, RenderError> {
        render_literal(literal)
    }

    /// Reads an aggregate's argument per row, where no grouping key reads
    /// through `ANY_VALUE`.
    fn render_aggregate_operand(&self, argument: &Expression) -> Result<String, RenderError> {
        self.per_row().render_expression(argument)
    }

    /// MySQL evaluates a boolean as 1 or 0, so only a character operand
    /// changes.
    fn summed(&self, argument: &Expression, operand: &str) -> String {
        match argument.aggregate_operand_conversion() {
            Some(OperandConversion::Character) => format!(
                "CAST({} AS DOUBLE)",
                counted(Some(OperandConversion::Character), operand)
            ),
            Some(OperandConversion::Boolean) | None => operand.to_string(),
        }
    }

    /// Renders one column reference through the read expression MySQL's type
    /// mapping names for its recorded source type, so every position acts on
    /// the value the scan emits for it. A read expression other than the bare
    /// column that is one of this position's grouping keys reads through
    /// `ANY_VALUE`; one that is no key stays unwrapped, so MySQL refuses it
    /// rather than return an arbitrary value of the group.
    fn render_column(&self, column: &ColumnRef) -> Result<String, RenderError> {
        let named = self.name_column(column)?;
        let read = mysql_type_mapping::column_read_sql(
            column.source_type.as_deref(),
            column.declared_type,
            &named,
        );

        if read != named && self.any_value_keys.contains(&read) {
            return Ok(format!("ANY_VALUE({read})"));
        }
        Ok(read)
    }
}

fn render_literal(literal: &Literal) -> Result<String, RenderError> {
    render_literal_with(
        "MySQL",
        literal,
        render_numeric_literal,
        render_string_literal,
    )
}

/// Renders a string literal as the hex encoding of its UTF-8 bytes.
///
/// Quoting would need an escaping rule, and no escaping rule is independent
/// of the source's `sql_mode`: measured, `LENGTH('a\b')` is 3 by default and
/// 4 under `NO_BACKSLASH_ESCAPES`, while `LENGTH(_utf8mb4 X'615C62')` is 3
/// in both. Hex removes the decision rather than getting it right, so no
/// value a predicate carries can change the statement's shape.
fn render_string_literal(value: &str) -> String {
    let hex: String = value
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect();

    format!("_{LITERAL_CHARACTER_SET} X'{hex}'")
}

fn render_numeric_literal(value: &str) -> Result<String, RenderError> {
    let is_bare_number = !value.is_empty()
        && value
            .chars()
            .all(|character| NUMERIC_LITERAL_CHARACTERS.contains(character));
    if !is_bare_number {
        return Err(RenderError::new(format!(
            "the numeric literal value `{value}` is not spelled out of \
             `{NUMERIC_LITERAL_CHARACTERS}` alone; MySQL accepts a number only unquoted, so any \
             other character in it could change the statement's shape"
        )));
    }

    Ok(value.to_string())
}

/// Visible to the crate's own tests: `capabilities_tests` drives the
/// per-dialect capability invariants through each dialect's fixture table,
/// and this module owns MySQL's.
#[cfg(test)]
#[path = "mysql_dialect_tests.rs"]
pub(crate) mod tests;
