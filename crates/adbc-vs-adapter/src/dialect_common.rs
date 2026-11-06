//! The rendering the MySQL, PostgreSQL, and Trino dialects share.
//!
//! The three dialects walk the same query model and differ in a handful of
//! decisions: how an identifier is quoted, which join keywords exist, how a
//! literal and a column read are spelled, and how `SUM` converts its operand.
//! Those decisions stay in each dialect as associated constants and hook
//! methods of [`StatementRenderer`]; everything else renders here once. A
//! dialect still owns its own statement: nothing here knows a technology by
//! name beyond the [`StatementRenderer::TARGET`] its error messages quote.

use crate::pushdown_query::{
    AggregateCall, BetweenPredicate, ColumnRef, Comparison, Expression, FromClause,
    InConstListPredicate, LikePredicate, Literal, OperandConversion, Predicate, SelectItem,
    SourceTable,
};
use crate::sql_dialect::RenderError;

/// The aggregate functions every dialect renders, spelled in canonical upper
/// case, which is also how the query model carries a function name: it
/// normalizes on parse, so a name that reaches here in any other case fails
/// this list rather than reaching the statement non-canonically. A function
/// outside the list has no rendering path, and rendering its argument in its
/// place would leave Exasol aggregating nothing and returning the wrong value.
const AGGREGATE_FUNCTIONS: &[&str] = &["COUNT", "MIN", "MAX", "SUM", "AVG"];

const SUM_FUNCTION: &str = "SUM";

/// Rendered as the quotient of its `SUM` and `COUNT`, never as the source's own
/// `AVG`, which returns an exact numeric over an exact-numeric operand where
/// Exasol declares a `DOUBLE`.
const AVG_FUNCTION: &str = "AVG";

/// The one aggregate written over no argument, as `COUNT(*)`.
const COUNT_FUNCTION: &str = "COUNT";

/// The prefix a joined table's alias carries, completed by the table's 1-based
/// position in the `from` tree.
const TABLE_ALIAS_PREFIX: &str = "t";

/// The constant an empty select list projects, one digit wide because
/// `pushdown_query` declares that select-list item `DECIMAL(1,0)`.
///
/// Exasol sends an empty select list when only the row count matters, and a
/// statement projecting no column would both be invalid SQL and lose the count
/// it was sent for.
const ROW_MARKER_CONSTANT: &str = "1";

/// The characters a `DATE` or `TIMESTAMP` literal's value may carry. The value
/// sits inside the quotes a typed temporal literal requires, so a quote or a
/// backslash reaching it could end the literal early.
const TEMPORAL_LITERAL_CHARACTERS: &str = "0123456789-:. ";

/// The `from` tree with the alias every table renders under already assigned.
///
/// An alias is derived from the table's position in the tree, so the alias set
/// is unique by construction and the generated statement depends on the
/// delegated query's shape alone. The reference a column node names a table
/// by, its `tableAlias` when the operator's query aliased the table and the
/// table's own name otherwise, stays a separate namespace that is only ever
/// compared, never rendered.
///
/// Assigning the aliases into a tree of its own, rather than looking one up
/// again while rendering, is what leaves no way for the two steps to disagree
/// about which table got which alias.
pub enum AliasedFrom<'a> {
    Table {
        table: &'a SourceTable,
        alias: String,
    },
    Join {
        join_type: &'a str,
        left: Box<AliasedFrom<'a>>,
        right: Box<AliasedFrom<'a>>,
        condition: &'a Predicate,
    },
}

impl<'a> AliasedFrom<'a> {
    /// Assigns an alias to every table of `from`, numbering them in the order
    /// the tree names them from left to right.
    fn of(from: &'a FromClause) -> Self {
        let mut assigned = 0;
        Self::assign(from, &mut assigned)
    }

    fn assign(from: &'a FromClause, assigned: &mut usize) -> Self {
        match from {
            FromClause::Table(table) => {
                *assigned += 1;
                Self::Table {
                    table,
                    alias: format!("{TABLE_ALIAS_PREFIX}{assigned}"),
                }
            }
            FromClause::Join(join) => Self::Join {
                join_type: &join.join_type,
                left: Box::new(Self::assign(&join.left, assigned)),
                right: Box::new(Self::assign(&join.right, assigned)),
                condition: &join.condition,
            },
        }
    }

    /// Every table of the tree, in the order it names them: what a column node
    /// names each one by, paired with the alias it renders under.
    ///
    /// One walk answers both questions asked of the tree: which alias a
    /// reference resolves to, and which references exist at all, which is what
    /// an unresolvable one is reported against.
    fn tables(&self) -> Vec<(&str, &str)> {
        match self {
            Self::Table { table, alias } => vec![(table_reference(table), alias.as_str())],
            Self::Join { left, right, .. } => {
                let mut tables = left.tables();
                tables.extend(right.tables());
                tables
            }
        }
    }
}

/// The decision one statement settles before any expression in it renders:
/// which table a column reference belongs to.
///
/// It cannot be answered from a column reference alone, because it is a
/// property of the `from` tree.
pub enum Tables<'a> {
    /// One table, named by nothing: a single table's column reference cannot
    /// be ambiguous, so the statement carries no alias and no qualifier even
    /// when the request assigned the table an alias of its own.
    OneTable(&'a SourceTable),
    /// A join, every table aliased and every column reference qualified.
    Joined(AliasedFrom<'a>),
}

impl<'a> Tables<'a> {
    pub fn of(from: &'a FromClause) -> Self {
        match from {
            FromClause::Table(table) => Self::OneTable(table),
            FromClause::Join(_) => Self::Joined(AliasedFrom::of(from)),
        }
    }
}

/// One dialect's statement renderer: the decisions that differ per target as
/// constants and hooks, and every rendering that does not as provided methods.
pub trait StatementRenderer {
    /// How an error message names the target, as in "PostgreSQL writes no empty
    /// `IN` list".
    const TARGET: &'static str;
    /// How the "valid ..." clause of an empty select list error names the
    /// target's SQL.
    const SQL_NAME: &'static str;
    /// The character that quotes an identifier, doubled inside it.
    const QUOTE: char;
    /// The keyword each `join_type` the dialect renders spells, which is also
    /// the closed set of values it renders at all.
    const JOIN_KEYWORDS: &'static [(&'static str, &'static str)];
    /// What a `join_type` error says between the list of join types and the
    /// reason substitution is wrong, for a target missing a join type.
    const JOIN_NOTE: &'static str = "";
    /// The target's double-precision type, which an `AVG` quotient casts to.
    const DOUBLE_TYPE: &'static str;

    fn tables(&self) -> &Tables<'_>;

    fn render_column(&self, column: &ColumnRef) -> Result<String, RenderError>;

    fn render_literal(&self, literal: &Literal) -> Result<String, RenderError>;

    /// The operand an aggregate's argument renders to, `SUM` and `AVG`
    /// included.
    fn render_aggregate_operand(&self, argument: &Expression) -> Result<String, RenderError> {
        self.render_expression(argument)
    }

    /// The operand a `SUM` adds, converted as Exasol converts it before it
    /// sums.
    fn summed(&self, argument: &Expression, operand: &str) -> String;

    /// An aggregate other than `SUM` and `AVG` over an argument.
    fn render_plain_aggregate(
        &self,
        function: &str,
        _argument: &Expression,
        distinct: &str,
        operand: &str,
    ) -> String {
        format!("{function}({distinct}{operand})")
    }

    /// Renders the `FROM` clause: one qualified table name, or the whole join
    /// tree with every table aliased.
    fn render_from(&self) -> Result<String, RenderError> {
        match self.tables() {
            Tables::OneTable(table) => Ok(qualified_table(table, Self::QUOTE)),
            Tables::Joined(from) => self.render_join_tree(from),
        }
    }

    /// Renders one element of an aliased `from` tree.
    ///
    /// A join renders inside its own parentheses, so the request's own nesting
    /// reaches the target without either side reasoning about associativity,
    /// and an outer join whose nesting shifted would return different rows. The
    /// condition renders through the same predicate renderer a `WHERE` clause
    /// uses, because a condition compared one way in `ON` and another way in
    /// `WHERE` would be two answers to one question.
    fn render_join_tree(&self, from: &AliasedFrom<'_>) -> Result<String, RenderError> {
        match from {
            AliasedFrom::Table { table, alias } => Ok(format!(
                "{} AS {}",
                qualified_table(table, Self::QUOTE),
                quote_identifier(alias, Self::QUOTE)
            )),
            AliasedFrom::Join {
                join_type,
                left,
                right,
                condition,
            } => Ok(format!(
                "({} {} {} ON {})",
                self.render_join_tree(left)?,
                join_keyword(Self::JOIN_KEYWORDS, Self::JOIN_NOTE, join_type)?,
                self.render_join_tree(right)?,
                self.render_predicate(condition)?
            )),
        }
    }

    /// Renders the select list, one entry per emitted column, in the order the
    /// request gave them.
    fn render_projection(&self, select_list: &[SelectItem]) -> Result<String, RenderError> {
        if select_list.is_empty() {
            return Err(RenderError::new(format!(
                "the query's select list is empty, and a statement projecting no column is not \
                 valid {}",
                Self::SQL_NAME
            )));
        }

        Ok(select_list
            .iter()
            .map(|item| self.render_expression(&item.expression))
            .collect::<Result<Vec<_>, _>>()?
            .join(", "))
    }

    /// Renders the `GROUP BY` keys a `group_by` aggregation names.
    fn render_group_by_keys(&self, columns: &[ColumnRef]) -> Result<Vec<String>, RenderError> {
        if columns.is_empty() {
            return Err(RenderError::new(format!(
                "a `group_by` aggregation names no grouping column, and {} writes no empty \
                 `GROUP BY` clause to render in its place; a `single_group` aggregation is the \
                 one that groups by nothing",
                Self::TARGET
            )));
        }

        columns
            .iter()
            .map(|column| self.render_column(column))
            .collect()
    }

    fn render_predicate(&self, predicate: &Predicate) -> Result<String, RenderError> {
        match predicate {
            Predicate::BooleanColumn(column) => self.render_column(column),
            Predicate::And(predicates) => self.render_junction(predicates, "AND"),
            Predicate::Or(predicates) => self.render_junction(predicates, "OR"),
            Predicate::Not(inner) => Ok(format!("(NOT {})", self.render_predicate(inner)?)),
            Predicate::Equal(comparison) => self.render_comparison(comparison, "="),
            Predicate::NotEqual(comparison) => self.render_comparison(comparison, "<>"),
            Predicate::Less(comparison) => self.render_comparison(comparison, "<"),
            Predicate::LessEqual(comparison) => self.render_comparison(comparison, "<="),
            Predicate::IsNull(expression) => self.render_null_test(expression, "IS NULL"),
            Predicate::IsNotNull(expression) => self.render_null_test(expression, "IS NOT NULL"),
            Predicate::Like(like) => self.render_like(like),
            Predicate::Between(between) => self.render_between(between),
            Predicate::InConstList(in_const_list) => self.render_in_const_list(in_const_list),
        }
    }

    fn render_junction(
        &self,
        predicates: &[Predicate],
        operator: &str,
    ) -> Result<String, RenderError> {
        if predicates.is_empty() {
            return Err(RenderError::new(format!(
                "a `predicate_{}` node combines no nested predicate, and {} writes no empty \
                 `{operator}` expression to render in its place",
                operator.to_lowercase(),
                Self::TARGET
            )));
        }

        let operands = predicates
            .iter()
            .map(|predicate| self.render_predicate(predicate))
            .collect::<Result<Vec<_>, _>>()?;

        Ok(format!("({})", operands.join(&format!(" {operator} "))))
    }

    /// Renders a comparison, equality and ordering alike. Exasol normalizes `>`
    /// and `>=` into `predicate_less` and `predicate_lessequal` with the
    /// operands swapped, so the operands render in the order the request gave
    /// them.
    fn render_comparison(
        &self,
        comparison: &Comparison,
        operator: &str,
    ) -> Result<String, RenderError> {
        Ok(format!(
            "({} {operator} {})",
            self.render_expression(&comparison.left)?,
            self.render_expression(&comparison.right)?
        ))
    }

    fn render_null_test(
        &self,
        expression: &Expression,
        operator: &str,
    ) -> Result<String, RenderError> {
        Ok(format!(
            "({} {operator})",
            self.render_expression(expression)?
        ))
    }

    /// Renders a `LIKE`, with an `ESCAPE` clause exactly when the request
    /// carries an `escapeChar`.
    fn render_like(&self, like: &LikePredicate) -> Result<String, RenderError> {
        let escape = match &like.escape_char {
            Some(escape_char) => format!(" ESCAPE {}", self.render_expression(escape_char)?),
            None => String::new(),
        };

        Ok(format!(
            "({} LIKE {}{escape})",
            self.render_expression(&like.expression)?,
            self.render_expression(&like.pattern)?
        ))
    }

    fn render_between(&self, between: &BetweenPredicate) -> Result<String, RenderError> {
        Ok(format!(
            "({} BETWEEN {} AND {})",
            self.render_expression(&between.expression)?,
            self.render_expression(&between.left)?,
            self.render_expression(&between.right)?
        ))
    }

    fn render_in_const_list(
        &self,
        in_const_list: &InConstListPredicate,
    ) -> Result<String, RenderError> {
        if in_const_list.arguments.is_empty() {
            return Err(RenderError::new(format!(
                "a `predicate_in_constlist` node carries no argument, and {} writes no empty \
                 `IN` list to render in its place",
                Self::TARGET
            )));
        }

        let arguments = in_const_list
            .arguments
            .iter()
            .map(|argument| self.render_expression(argument))
            .collect::<Result<Vec<_>, _>>()?;

        Ok(format!(
            "({} IN ({}))",
            self.render_expression(&in_const_list.expression)?,
            arguments.join(", ")
        ))
    }

    /// Renders one aggregate over the current group.
    ///
    /// The function name is taken from [`AGGREGATE_FUNCTIONS`] rather than from
    /// the request, so the statement carries the canonical spelling whatever
    /// case Exasol sent. The argument's absence, not a separate field, is what
    /// tells `COUNT(*)` apart from `COUNT(column)`. `AVG` is the quotient of
    /// the `SUM` and the operand's `COUNT`.
    fn render_aggregate(&self, call: &AggregateCall) -> Result<String, RenderError> {
        let function = renderable_function(&call.function)?;
        let Some(argument) = &call.argument else {
            return render_star_aggregate(Self::TARGET, function, call);
        };

        let distinct = if call.distinct { "DISTINCT " } else { "" };
        let operand = self.render_aggregate_operand(argument)?;

        Ok(match function {
            SUM_FUNCTION => format!("SUM({distinct}{})", self.summed(argument, &operand)),
            AVG_FUNCTION => format!(
                "(CAST(SUM({distinct}{}) AS {}) / COUNT({distinct}{}))",
                self.summed(argument, &operand),
                Self::DOUBLE_TYPE,
                counted(argument.aggregate_operand_conversion(), &operand)
            ),
            _ => self.render_plain_aggregate(function, argument, distinct, &operand),
        })
    }

    fn render_expression(&self, expression: &Expression) -> Result<String, RenderError> {
        match expression {
            Expression::Column(column) => self.render_column(column),
            Expression::RowMarker => Ok(ROW_MARKER_CONSTANT.to_string()),
            Expression::Literal(literal) => self.render_literal(literal),
            Expression::Aggregate(call) => self.render_aggregate(call),
            Expression::Predicate(predicate) => self.render_predicate(predicate),
        }
    }

    /// Names `column`: qualified by the alias of the table it resolves to in a
    /// join, alone in a single-table statement.
    fn name_column(&self, column: &ColumnRef) -> Result<String, RenderError> {
        let Tables::Joined(from) = self.tables() else {
            return Ok(quote_identifier(&column.name, Self::QUOTE));
        };
        let alias = resolve_alias(from, column)?;

        Ok(format!(
            "{}.{}",
            quote_identifier(alias, Self::QUOTE),
            quote_identifier(&column.name, Self::QUOTE)
        ))
    }
}

/// Renders an optional clause as its keyword and rendered body, or nothing.
pub fn optional_clause(
    keyword: &str,
    predicate: Option<&Predicate>,
    render: impl FnOnce(&Predicate) -> Result<String, RenderError>,
) -> Result<String, RenderError> {
    match predicate {
        Some(predicate) => Ok(format!(" {keyword} {}", render(predicate)?)),
        None => Ok(String::new()),
    }
}

/// Wraps `identifier` in `quote`, doubling any `quote` it contains, so no name
/// can change the shape of the generated statement.
pub fn quote_identifier(identifier: &str, quote: char) -> String {
    let doubled = format!("{quote}{quote}");
    format!(
        "{quote}{}{quote}",
        identifier.replace(quote, doubled.as_str())
    )
}

/// Qualifies a table by the ordered segments the query model carries for it,
/// each quoted and joined by a dot.
pub fn qualified_table(table: &SourceTable, quote: char) -> String {
    let mut qualified = String::new();
    for segment in table.namespace.segments() {
        qualified.push_str(&quote_identifier(segment, quote));
        qualified.push('.');
    }
    qualified.push_str(&quote_identifier(&table.name, quote));
    qualified
}

fn join_keyword(
    keywords: &'static [(&'static str, &'static str)],
    note: &str,
    join_type: &str,
) -> Result<&'static str, RenderError> {
    keywords
        .iter()
        .find(|(protocol_spelling, _)| *protocol_spelling == join_type)
        .map(|(_, keyword)| *keyword)
        .ok_or_else(|| {
            RenderError::new(format!(
                "the `join_type` `{join_type}` has no rendering path in this release, which \
                 renders {} only; {note}rendering a join type it does have in its place would \
                 return rows Exasol never asked for",
                quoted_names(keywords.iter().map(|(spelling, _)| *spelling))
            ))
        })
}

/// What a column node names `table` by: the alias the request assigned it, or
/// the table's own name when the request assigned none. Measured on Exasol
/// 2025.2.1, the two agree: a `from` element carrying `alias` sends every one
/// of that table's column nodes a matching `tableAlias`, and one carrying none
/// sends `tableName` alone.
fn table_reference(table: &SourceTable) -> &str {
    table.alias.as_deref().unwrap_or(&table.name)
}

/// How `column` names the table it belongs to, `None` for a reference that
/// names none. A `tableAlias` outranks a `tableName`, because a self-join
/// sends the same `tableName` for both copies and only the alias tells them
/// apart.
fn column_reference(column: &ColumnRef) -> Option<&str> {
    column
        .table_alias
        .as_deref()
        .or(column.table_name.as_deref())
}

/// The alias of the table `column` belongs to, within a join.
///
/// A reference that resolves to no table, or to more than one, has no
/// qualification a dialect can render, and both fail naming the column and the
/// reference. Rendering the column unqualified instead would leave the source
/// to resolve the name against whichever joined table declares it, which is
/// the wrong copy in a self-join, where the choice decides which rows come
/// back.
fn resolve_alias<'a>(
    from: &'a AliasedFrom<'_>,
    column: &ColumnRef,
) -> Result<&'a str, RenderError> {
    let Some(reference) = column_reference(column) else {
        return Err(RenderError::new(format!(
            "the query joins two or more tables and its column `{}` carries neither a \
             `tableName` nor a `tableAlias`, so nothing in the request names the table it \
             belongs to",
            column.name
        )));
    };

    let tables = from.tables();
    let mut matching = tables
        .iter()
        .copied()
        .filter(|(candidate, _)| *candidate == reference);
    let Some((_, alias)) = matching.next() else {
        return Err(RenderError::new(format!(
            "the query names column `{}` of `{reference}`, and no table in its `from` tree is \
             named `{reference}`; that tree names {}",
            column.name,
            quoted_names(tables.iter().map(|(candidate, _)| *candidate))
        )));
    };
    if matching.next().is_some() {
        return Err(RenderError::new(format!(
            "the query names column `{}` of `{reference}`, and more than one table in its \
             `from` tree is named `{reference}`, so which of them provides the column is \
             undecided; one table joined to itself is told apart only by the alias the request \
             assigns each copy",
            column.name
        )));
    }

    Ok(alias)
}

/// The names an error offers an operator as the set the failing one was not
/// in, each quoted so an empty or space-carrying name is still visible.
fn quoted_names<'a>(names: impl Iterator<Item = &'a str>) -> String {
    names
        .map(|name| format!("`{name}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn renderable_function(function: &str) -> Result<&'static str, RenderError> {
    AGGREGATE_FUNCTIONS
        .iter()
        .copied()
        .find(|candidate| *candidate == function)
        .ok_or_else(|| {
            RenderError::new(format!(
                "the aggregate function `{function}` has no rendering path in this release, \
                 which renders {} only",
                AGGREGATE_FUNCTIONS.join(", ")
            ))
        })
}

/// The operand an `AVG` counts: the one Exasol divides by, before any number
/// conversion, so a character operand differs from the summed one only in that
/// it is not yet a double.
pub fn counted(conversion: Option<OperandConversion>, operand: &str) -> String {
    match conversion {
        Some(OperandConversion::Character) => format!("NULLIF({operand}, '')"),
        Some(OperandConversion::Boolean) | None => operand.to_string(),
    }
}

/// Renders the aggregate written over no argument.
///
/// `COUNT(*)` counts rows rather than values, so it is the only such form, and
/// `DISTINCT` has no values to reduce: no target writes `COUNT(DISTINCT *)`.
/// Both departures fail rather than render a near miss, because dropping the
/// `DISTINCT` would count every row where Exasol asked for the distinct
/// values, and supplying a `*` to another function would aggregate the wrong
/// thing.
fn render_star_aggregate(
    target: &str,
    function: &str,
    call: &AggregateCall,
) -> Result<String, RenderError> {
    if function != COUNT_FUNCTION {
        return Err(RenderError::new(format!(
            "the `{function}` aggregate carries no argument, and {target} writes `{function}` \
             only over one; `{COUNT_FUNCTION}` is the only aggregate it writes over no argument"
        )));
    }
    if call.distinct {
        return Err(RenderError::new(format!(
            "the `{COUNT_FUNCTION}` aggregate carries `distinct` but no argument, and {target} \
             writes no `{COUNT_FUNCTION}(DISTINCT *)`"
        )));
    }

    Ok(format!("{COUNT_FUNCTION}(*)"))
}

pub fn render_temporal_literal(
    target: &str,
    keyword: &str,
    value: &str,
) -> Result<String, RenderError> {
    let is_bare_temporal = !value.is_empty()
        && value
            .chars()
            .all(|character| TEMPORAL_LITERAL_CHARACTERS.contains(character));
    if !is_bare_temporal {
        return Err(RenderError::new(format!(
            "the `{keyword}` literal value `{value}` is not spelled out of \
             `{TEMPORAL_LITERAL_CHARACTERS}` alone; {target} accepts a typed temporal literal \
             only inside quotes, so any other character in it could end the literal early"
        )));
    }

    Ok(format!("{keyword} '{value}'"))
}

/// Whether `value` is spelled as a bare numeric constant: an optional sign,
/// digits carrying at most one decimal point, and an optional `e` exponent.
///
/// Spelled out as a grammar rather than as the set of characters a number may
/// contain, because a target whose `--` comment needs no following space would
/// otherwise admit `1--2` and comment out the rest of the statement.
pub fn is_numeric_constant(value: &str) -> bool {
    let unsigned = value.strip_prefix(['+', '-']).unwrap_or(value);
    match unsigned.split_once(['e', 'E']) {
        Some((mantissa, exponent)) => is_decimal_mantissa(mantissa) && is_signed_digits(exponent),
        None => is_decimal_mantissa(unsigned),
    }
}

fn is_decimal_mantissa(mantissa: &str) -> bool {
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));

    !(whole.is_empty() && fraction.is_empty())
        && whole.chars().all(|digit| digit.is_ascii_digit())
        && fraction.chars().all(|digit| digit.is_ascii_digit())
}

fn is_signed_digits(exponent: &str) -> bool {
    let unsigned = exponent.strip_prefix(['+', '-']).unwrap_or(exponent);

    !unsigned.is_empty() && unsigned.chars().all(|digit| digit.is_ascii_digit())
}
