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
//! Grouping needs no coordination as a result. MySQL's `ONLY_FULL_GROUP_BY`
//! mode, which `mysql:8.4` sets by default, rejects a select list whose
//! occurrence of a grouped column differs from its `GROUP BY` key. Both
//! occurrences are the bare column, rendered by the one function every
//! column reference takes, so they cannot differ, and a select-list
//! predicate reading the key is an expression over that same column.
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

use crate::mysql_type_mapping::SUPPORTED_CHARACTER_SET;
use crate::pushdown_query::{
    AggregateCall, Aggregation, BetweenPredicate, ColumnRef, Comparison, Expression, FromClause,
    InConstListPredicate, LikePredicate, Literal, Predicate, Query, SelectItem, SourceTable,
};
use crate::sql_dialect::{RenderError, SqlDialect};

/// The aggregate functions this dialect renders, spelled in MySQL's
/// canonical upper case, which is also how the query model carries a
/// function name: it normalizes on parse, so a name that reaches here in any
/// other case fails this list rather than reaching the statement
/// non-canonically. A function outside the list has no rendering path, and
/// rendering its argument in its place would leave Exasol aggregating
/// nothing and returning the wrong value.
const AGGREGATE_FUNCTIONS: &[&str] = &["COUNT", "MIN", "MAX"];

/// The one aggregate MySQL writes over no argument, as `COUNT(*)`.
const COUNT_FUNCTION: &str = "COUNT";

/// The MySQL keyword each `join_type` this dialect renders spells, and the
/// closed set of values it renders at all.
///
/// `full_outer` has no entry. MySQL implements no `FULL OUTER JOIN`, and
/// emulating one as a union of two outer joins is the query rewriting
/// mission.md places out of scope, so `JOIN_TYPE_FULL_OUTER` is not
/// advertised and a request carrying the value fails naming it — measured on
/// Exasol 2025.2.1, withholding the capability is enough that Exasol
/// withdraws the join and sends a single table instead, so the failure path
/// is a backstop rather than the expected case. Which values this list holds
/// is the same question [`AGGREGATE_FUNCTIONS`] answers for a function name,
/// and it belongs to the dialect for the same reason: substituting a join
/// type the renderer does know would return rows Exasol never asked for.
const JOIN_KEYWORDS: &[(&str, &str)] = &[
    ("inner", "INNER JOIN"),
    ("left_outer", "LEFT OUTER JOIN"),
    ("right_outer", "RIGHT OUTER JOIN"),
];

/// The prefix a joined table's alias carries, completed by the table's
/// 1-based position in the `from` tree.
const TABLE_ALIAS_PREFIX: &str = "t";

/// The characters a numeric literal's value may carry. None of them can end
/// a statement fragment or begin a comment: MySQL's `--` comment needs a
/// following space, which is not in this set.
const NUMERIC_LITERAL_CHARACTERS: &str = "0123456789+-.eE";

/// The characters a `DATE` or `TIMESTAMP` literal's value may carry. The
/// value sits inside the quotes MySQL requires for a typed temporal literal,
/// so a quote or a backslash reaching it could end the literal early.
const TEMPORAL_LITERAL_CHARACTERS: &str = "0123456789-:. ";

/// The MySQL dialect: the only [`SqlDialect`] this release implements.
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
        let statement = Statement::of(&query.from);
        let projection = statement.render_projection(&query.select_list)?;
        let from = statement.render_from()?;
        let filter = match &query.filter {
            Some(predicate) => format!(" WHERE {}", statement.render_predicate(predicate)?),
            None => String::new(),
        };
        let grouping = match &query.aggregation {
            Some(aggregation) => statement.render_grouping(aggregation)?,
            None => String::new(),
        };

        Ok(format!("SELECT {projection} FROM {from}{filter}{grouping}"))
    }
}

/// The `from` tree with the alias every table renders under already assigned.
///
/// An alias is derived from the table's position in the tree, so the alias
/// set is unique by construction and the generated statement depends on the
/// delegated query's shape alone. The reference a column node names a table
/// by — its `tableAlias` when the operator's query aliased the table, the
/// table's own name otherwise — stays a separate namespace that is only ever
/// compared, never rendered. Rendering an operator's alias verbatim instead
/// would put a name this dialect does not control into a namespace it has to
/// keep unambiguous: `FROM "orders" "t2" JOIN "customers"` would then render
/// `t2` twice, which MySQL rejects with error 1066, and a server configured
/// with `lower_case_table_names=1` would collapse two aliases differing only
/// in case, which Exasol does send for a pair of quoted aliases.
///
/// Assigning the aliases into a tree of its own, rather than looking one up
/// again while rendering, is what leaves no way for the two steps to disagree
/// about which table got which alias.
enum AliasedFrom<'a> {
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

    /// Every table of the tree, in the order it names them: what a column
    /// node names each one by, paired with the alias it renders under.
    ///
    /// One walk answers both questions asked of the tree — which alias a
    /// reference resolves to, and which references exist at all, which is
    /// what an unresolvable one is reported against.
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

/// The decision one statement settles before any expression in it renders,
/// and that every expression then reads: which table a column reference
/// belongs to.
///
/// It cannot be answered from a column reference alone, because it is a
/// property of the `from` tree, and the same reference reaches the renderer
/// as a projected column, a predicate operand, an aggregate argument, a join
/// condition operand, and a grouping key alike. Settling it once, here, is
/// what leaves a grouping key and its select-list occurrence unable to
/// disagree about their qualifier, which MySQL would reject with error 1055.
enum Statement<'a> {
    /// One table, named by nothing: a single table's column reference cannot
    /// be ambiguous, so the statement carries no alias and no qualifier even
    /// when the request assigned the table an alias of its own.
    OneTable(&'a SourceTable),
    /// A join, every table aliased and every column reference qualified.
    Joined(AliasedFrom<'a>),
}

impl<'a> Statement<'a> {
    fn of(from: &'a FromClause) -> Self {
        match from {
            FromClause::Table(table) => Self::OneTable(table),
            FromClause::Join(_) => Self::Joined(AliasedFrom::of(from)),
        }
    }

    /// Renders the `FROM` clause: one qualified table name, or the whole join
    /// tree with every table aliased.
    fn render_from(&self) -> Result<String, RenderError> {
        match self {
            Self::OneTable(table) => Ok(qualified_table(table)),
            Self::Joined(from) => self.render_join_tree(from),
        }
    }

    /// Renders one element of an aliased `from` tree.
    ///
    /// A join renders inside its own parentheses, for the reason a predicate
    /// does: the request's own nesting reaches MySQL without either side
    /// reasoning about associativity, and an outer join whose nesting shifted
    /// would return different rows. The condition renders through the same
    /// predicate renderer a `WHERE` clause uses, because a condition compared
    /// one way in `ON` and another way in `WHERE` would be two answers to one
    /// question.
    fn render_join_tree(&self, from: &AliasedFrom<'_>) -> Result<String, RenderError> {
        match from {
            AliasedFrom::Table { table, alias } => Ok(format!(
                "{} AS {}",
                qualified_table(table),
                quote_identifier(alias)
            )),
            AliasedFrom::Join {
                join_type,
                left,
                right,
                condition,
            } => Ok(format!(
                "({} {} {} ON {})",
                self.render_join_tree(left)?,
                join_keyword(join_type)?,
                self.render_join_tree(right)?,
                self.render_predicate(condition)?
            )),
        }
    }

    /// Renders the select list. Every entry renders exactly as it does in
    /// any other position, including a grouped column and a predicate
    /// reading one.
    fn render_projection(&self, select_list: &[SelectItem]) -> Result<String, RenderError> {
        if select_list.is_empty() {
            return Err(RenderError::new(
                "the query's select list is empty, and a statement projecting no column is not \
                 valid MySQL",
            ));
        }

        Ok(select_list
            .iter()
            .map(|item| self.render_expression(&item.expression))
            .collect::<Result<Vec<_>, _>>()?
            .join(", "))
    }

    /// Renders the `GROUP BY` clause an aggregation needs, which is nothing
    /// for a single-group one.
    ///
    /// The two aggregation kinds differ only in this clause: MySQL aggregates
    /// a whole result into one row for a select list of aggregates carrying
    /// no `GROUP BY` at all, which is exactly what Exasol asks for with
    /// `single_group`.
    ///
    /// Each key renders through [`Statement::render_column`], the renderer
    /// its select-list occurrence takes too. Exasol qualifies the two
    /// occurrences independently, `tableName` on one and `tableAlias` on the
    /// other, but both resolve to the same table and so render identically,
    /// which MySQL's `ONLY_FULL_GROUP_BY` requires: it rejects the statement
    /// with error 1055 when they differ.
    fn render_grouping(&self, aggregation: &Aggregation) -> Result<String, RenderError> {
        let Aggregation::GroupBy(columns) = aggregation else {
            return Ok(String::new());
        };
        if columns.is_empty() {
            return Err(RenderError::new(
                "a `group_by` aggregation names no grouping column, and MySQL writes no empty \
                 `GROUP BY` clause to render in its place; a `single_group` aggregation is the \
                 one that groups by nothing",
            ));
        }

        let grouping = columns
            .iter()
            .map(|column| self.render_column(column))
            .collect::<Result<Vec<_>, _>>()?
            .join(", ");

        Ok(format!(" GROUP BY {grouping}"))
    }

    fn render_predicate(&self, predicate: &Predicate) -> Result<String, RenderError> {
        match predicate {
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
                "a `predicate_{}` node combines no nested predicate, and MySQL writes no empty \
                 `{operator}` expression to render in its place",
                operator.to_lowercase()
            )));
        }

        let operands = predicates
            .iter()
            .map(|predicate| self.render_predicate(predicate))
            .collect::<Result<Vec<_>, _>>()?;

        Ok(format!("({})", operands.join(&format!(" {operator} "))))
    }

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
            return Err(RenderError::new(
                "a `predicate_in_constlist` node carries no argument, and MySQL writes no empty \
                 `IN` list to render in its place",
            ));
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
    /// The function name is taken from [`AGGREGATE_FUNCTIONS`] rather than
    /// from the request, so the statement carries MySQL's canonical spelling
    /// whatever case Exasol sent — the model already normalized it, and one
    /// owner of that decision is enough. The argument's absence, not a
    /// separate field, is what tells `COUNT(*)` apart from `COUNT(column)`.
    fn render_aggregate(&self, call: &AggregateCall) -> Result<String, RenderError> {
        let function = renderable_function(&call.function)?;
        let Some(argument) = &call.argument else {
            return render_star_aggregate(function, call);
        };

        let distinct = if call.distinct { "DISTINCT " } else { "" };
        let operand = self.render_expression(argument)?;

        Ok(format!("{function}({distinct}{operand})"))
    }

    fn render_expression(&self, expression: &Expression) -> Result<String, RenderError> {
        match expression {
            Expression::Column(column) => self.render_column(column),
            Expression::RowMarker => Ok("1".to_string()),
            Expression::Literal(literal) => render_literal(literal),
            Expression::Aggregate(call) => self.render_aggregate(call),
            Expression::Predicate(predicate) => self.render_predicate(predicate),
        }
    }

    /// Renders one column reference. Every reference goes through here — a
    /// projected one, a predicate operand, an aggregate's argument, a join
    /// condition operand, and a grouping column — so how a column is named
    /// is one decision, made once.
    ///
    /// A join qualifies the reference by the alias of the table it resolves
    /// to, and a single-table statement names the column alone.
    fn render_column(&self, column: &ColumnRef) -> Result<String, RenderError> {
        let Self::Joined(from) = self else {
            return Ok(quote_identifier(&column.name));
        };
        let alias = resolve_alias(from, column)?;

        Ok(format!(
            "{}.{}",
            quote_identifier(alias),
            quote_identifier(&column.name)
        ))
    }
}

fn join_keyword(join_type: &str) -> Result<&'static str, RenderError> {
    JOIN_KEYWORDS
        .iter()
        .find(|(protocol_spelling, _)| *protocol_spelling == join_type)
        .map(|(_, keyword)| *keyword)
        .ok_or_else(|| {
            RenderError::new(format!(
                "the `join_type` `{join_type}` has no rendering path in this release, which \
                 renders {} only; MySQL implements no `FULL OUTER JOIN`, and rendering a join \
                 type it does have in its place would return rows Exasol never asked for",
                quoted_names(JOIN_KEYWORDS.iter().map(|(spelling, _)| *spelling))
            ))
        })
}

/// What a column node names `table` by: the alias the request assigned it,
/// or the table's own name when the request assigned none. Measured on
/// Exasol 2025.2.1, the two agree — a `from` element carrying `alias` sends
/// every one of that table's column nodes a matching `tableAlias`, and one
/// carrying none sends `tableName` alone.
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
/// qualification this dialect can render, and both fail naming the column and
/// the reference. Rendering the column unqualified instead would leave MySQL
/// to resolve the name against whichever joined table declares it — error
/// 1052 when two of them do, and silently the wrong copy in a self-join,
/// where the choice decides which rows come back.
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
                "the aggregate function `{function}` has no rendering path in this release, which \
                 renders {} only",
                AGGREGATE_FUNCTIONS.join(", ")
            ))
        })
}

/// Renders the aggregate MySQL writes over no argument.
///
/// `COUNT(*)` counts rows rather than values, so it is the only such form,
/// and `DISTINCT` has no values to reduce: MySQL writes no
/// `COUNT(DISTINCT *)`. Both departures fail rather than render a near miss,
/// because dropping the `DISTINCT` would count every row where Exasol asked
/// for the distinct values, and supplying a `*` to another function would
/// aggregate the wrong thing.
fn render_star_aggregate(function: &str, call: &AggregateCall) -> Result<String, RenderError> {
    if function != COUNT_FUNCTION {
        return Err(RenderError::new(format!(
            "the `{function}` aggregate carries no argument, and MySQL writes `{function}` only \
             over one; `{COUNT_FUNCTION}` is the only aggregate it writes over no argument"
        )));
    }
    if call.distinct {
        return Err(RenderError::new(format!(
            "the `{COUNT_FUNCTION}` aggregate carries `distinct` but no argument, and MySQL \
             writes no `{COUNT_FUNCTION}(DISTINCT *)`"
        )));
    }

    Ok(format!("{COUNT_FUNCTION}(*)"))
}

fn render_literal(literal: &Literal) -> Result<String, RenderError> {
    match literal {
        Literal::Null => Ok("NULL".to_string()),
        Literal::ExactNumeric(value) | Literal::Double(value) => render_numeric_literal(value),
        Literal::String(value) => Ok(render_string_literal(value)),
        Literal::Date(value) => render_temporal_literal("DATE", value),
        Literal::Timestamp(value) => render_temporal_literal("TIMESTAMP", value),
    }
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

    format!("_{SUPPORTED_CHARACTER_SET} X'{hex}'")
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

fn render_temporal_literal(keyword: &str, value: &str) -> Result<String, RenderError> {
    let is_bare_temporal = !value.is_empty()
        && value
            .chars()
            .all(|character| TEMPORAL_LITERAL_CHARACTERS.contains(character));
    if !is_bare_temporal {
        return Err(RenderError::new(format!(
            "the `{keyword}` literal value `{value}` is not spelled out of \
             `{TEMPORAL_LITERAL_CHARACTERS}` alone; MySQL accepts a typed temporal literal only \
             inside quotes, so any other character in it could end the literal early"
        )));
    }

    Ok(format!("{keyword} '{value}'"))
}

/// Qualifies a table by the ordered segments the query model carries for it,
/// each quoted and joined by a dot.
///
/// A qualifier of one segment renders exactly as it did before qualifiers
/// became lists, which is what leaves MySQL's generated statements
/// unchanged.
fn qualified_table(table: &SourceTable) -> String {
    let mut qualified = String::new();
    for segment in table.namespace.segments() {
        qualified.push_str(&quote_identifier(segment));
        qualified.push('.');
    }
    qualified.push_str(&quote_identifier(&table.name));
    qualified
}

fn quote_identifier(identifier: &str) -> String {
    format!("`{}`", identifier.replace('`', "``"))
}

/// Visible to the crate's own tests: `capabilities_tests` drives the
/// per-dialect capability invariants through each dialect's fixture table,
/// and this module owns MySQL's.
#[cfg(test)]
#[path = "mysql_dialect_tests.rs"]
pub(crate) mod tests;
