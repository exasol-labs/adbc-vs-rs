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
//! A grouping key and its select-list occurrence render as the same bare
//! expression, so PostgreSQL's full-group-by rule is satisfied with no
//! coordination between the `GROUP BY` clause and the select list.
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

use adbc_vs_scan::driver::Driver;

use crate::pushdown_query::{
    AggregateCall, Aggregation, BetweenPredicate, ColumnRef, Comparison, Expression, FromClause,
    InConstListPredicate, LikePredicate, Literal, Predicate, Query, SelectItem, SourceTable,
};
use crate::sql_dialect::{RenderError, SqlDialect};

/// The constant an empty select list projects, one digit wide because
/// `pushdown_query` declares that select-list item `DECIMAL(1,0)`.
///
/// Exasol sends an empty select list when only the row count matters, and a
/// statement projecting no column would both be invalid PostgreSQL and lose
/// the count it was sent for.
const ROW_MARKER_CONSTANT: &str = "1";

/// The aggregate functions this dialect renders, spelled in PostgreSQL's
/// canonical upper case, which is also how the query model carries a function
/// name: it normalizes on parse, so a name that reaches here in any other case
/// fails this list rather than reaching the statement non-canonically. A
/// function outside the list has no rendering path, and rendering its argument
/// in its place would leave Exasol aggregating nothing and returning the wrong
/// value.
///
/// `SUM` and `AVG` are absent for a reason `vs-adapter/pushdown-aggregation`
/// states over every dialect rather than one about PostgreSQL: a capability is
/// advertised once per session and admits every numeric operand, including
/// `double precision`, whose sum is order-dependent under a parallel plan.
const AGGREGATE_FUNCTIONS: &[&str] = &["COUNT", "MIN", "MAX"];

/// The one aggregate PostgreSQL writes over no argument, as `COUNT(*)`.
const COUNT_FUNCTION: &str = "COUNT";

/// The PostgreSQL keyword each `join_type` this dialect renders spells, and
/// the closed set of values it renders at all.
///
/// `full_outer` has an entry, which is the one place this list differs from
/// the MySQL dialect's: PostgreSQL implements the keyword directly, where
/// MySQL implements no `FULL OUTER JOIN` and emulating one needs the union
/// rewrite mission.md places out of scope. Which values this list holds is the
/// same question [`AGGREGATE_FUNCTIONS`] answers for a function name, and it
/// belongs to the dialect for the same reason: substituting a join type the
/// renderer does know would return rows Exasol never asked for.
const JOIN_KEYWORDS: &[(&str, &str)] = &[
    ("inner", "INNER JOIN"),
    ("left_outer", "LEFT OUTER JOIN"),
    ("right_outer", "RIGHT OUTER JOIN"),
    ("full_outer", "FULL OUTER JOIN"),
];

/// The prefix a joined table's alias carries, completed by the table's 1-based
/// position in the `from` tree.
const TABLE_ALIAS_PREFIX: &str = "t";

/// The characters a `DATE` or `TIMESTAMP` literal's value may carry. The value
/// sits inside the quotes PostgreSQL requires for a typed temporal literal, so
/// a quote or a backslash reaching it could end the literal early — under
/// `standard_conforming_strings = off` a backslash is an escape there, and the
/// set admits neither character in either setting.
const TEMPORAL_LITERAL_CHARACTERS: &str = "0123456789-:. ";

/// The PostgreSQL dialect.
pub struct PostgresDialect;

impl SqlDialect for PostgresDialect {
    /// The set `drivers/postgresql.toml` declares, which is this dialect's
    /// claim about what the renderer below covers rather than a fact about
    /// PostgreSQL: the values are a data edit, but advertising one this module
    /// cannot render still returns wrong rows, so the two stay one decision
    /// that `capabilities_tests` renders entry by entry.
    fn capabilities(&self) -> &'static [&'static str] {
        Driver::Postgres.capabilities()
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
/// An alias is derived from the table's position in the tree, so the alias set
/// is unique by construction and the generated statement depends on the
/// delegated query's shape alone. The reference a column node names a table
/// by — its `tableAlias` when the operator's query aliased the table, the
/// table's own name otherwise — stays a separate namespace that is only ever
/// compared, never rendered: rendering an operator's alias verbatim would put
/// a name this dialect does not control into a namespace it has to keep
/// unambiguous, and `FROM "orders" "t2" JOIN "customers"` would then render
/// `t2` for two different tables.
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

    /// Every table of the tree, in the order it names them: what a column node
    /// names each one by, paired with the alias it renders under.
    ///
    /// One walk answers both questions asked of the tree — which alias a
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

/// The one decision a PostgreSQL statement settles before any expression in it
/// renders: which table each column reference belongs to, and therefore how a
/// column is named.
///
/// It cannot be answered from a column reference alone, because it is a
/// property of the `from` tree, and the same reference reaches the renderer as
/// a projected column, a predicate operand, an aggregate argument, a join
/// condition operand, and a grouping key alike.
///
/// It is also the *only* such decision here, where the MySQL dialect carries a
/// second: which columns the query groups by. That question decides nothing in
/// this dialect, because a grouping key and its select-list occurrence render
/// identically whatever the query groups by.
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
    /// does: the request's own nesting reaches PostgreSQL without either side
    /// reasoning about associativity, and an outer join whose nesting shifted
    /// would return different rows. The condition renders through the same
    /// predicate renderer a `WHERE` clause uses, because a condition compared
    /// one way in `ON` and another way in `WHERE` would be two answers to one
    /// question — and since a join condition is an equality, that renderer
    /// leaves a character join key bare and the source's index on it usable.
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

    /// Renders the select list, one entry per emitted column, in the order the
    /// request gave them.
    ///
    /// Every entry renders through the one expression renderer every other
    /// position uses. Unlike the MySQL dialect, no entry is special-cased on
    /// what the query groups by: a grouping key renders bare here, so a
    /// grouped column's select-list occurrence is the same expression as its
    /// `GROUP BY` key with nothing to coordinate.
    fn render_projection(&self, select_list: &[SelectItem]) -> Result<String, RenderError> {
        if select_list.is_empty() {
            return Err(RenderError::new(
                "the query's select list is empty, and a statement projecting no column is not \
                 valid PostgreSQL",
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
    /// The two aggregation kinds differ only in this clause: PostgreSQL
    /// aggregates a whole result into one row for a select list of aggregates
    /// carrying no `GROUP BY` at all, which is exactly what Exasol asks for
    /// with `single_group`.
    ///
    /// Each key renders through the same column renderer its select-list
    /// occurrence takes, so the two occurrences carry the same alias and are the
    /// same bare expression.
    fn render_grouping(&self, aggregation: &Aggregation) -> Result<String, RenderError> {
        let Aggregation::GroupBy(columns) = aggregation else {
            return Ok(String::new());
        };
        if columns.is_empty() {
            return Err(RenderError::new(
                "a `group_by` aggregation names no grouping column, and PostgreSQL writes no \
                 empty `GROUP BY` clause to render in its place; a `single_group` aggregation is \
                 the one that groups by nothing",
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
                "a `predicate_{}` node combines no nested predicate, and PostgreSQL writes no \
                 empty `{operator}` expression to render in its place",
                operator.to_lowercase()
            )));
        }

        let operands = predicates
            .iter()
            .map(|predicate| self.render_predicate(predicate))
            .collect::<Result<Vec<_>, _>>()?;

        Ok(format!("({})", operands.join(&format!(" {operator} "))))
    }

    /// Renders a comparison, equality and ordering alike, with both operands
    /// bare.
    ///
    /// The source compares character operands under the collation it applies
    /// to the column, so an ordering comparison follows that collation's sort
    /// order rather than Exasol's code-point order. Exasol normalizes `>` and
    /// `>=` into `predicate_less` and `predicate_lessequal` with the operands
    /// swapped, so the operands render in the order the request gave them.
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
    ///
    /// Every operand renders bare, so the match runs under the collation the
    /// source applies to the column.
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

    /// Renders a `BETWEEN` with its compared operand and both bounds bare, so
    /// the source's own collation decides the range.
    fn render_between(&self, between: &BetweenPredicate) -> Result<String, RenderError> {
        Ok(format!(
            "({} BETWEEN {} AND {})",
            self.render_expression(&between.expression)?,
            self.render_expression(&between.left)?,
            self.render_expression(&between.right)?
        ))
    }

    /// Renders an `IN` list with every operand bare.
    fn render_in_const_list(
        &self,
        in_const_list: &InConstListPredicate,
    ) -> Result<String, RenderError> {
        if in_const_list.arguments.is_empty() {
            return Err(RenderError::new(
                "a `predicate_in_constlist` node carries no argument, and PostgreSQL writes no \
                 empty `IN` list to render in its place",
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
    /// from the request, so the statement carries PostgreSQL's canonical
    /// spelling whatever case Exasol sent. The argument's absence, not a
    /// separate field, is what tells `COUNT(*)` apart from `COUNT(column)`.
    ///
    /// The argument renders bare whatever the function and whatever `DISTINCT`
    /// it carries, so `MIN` and `MAX` over a character column return the value
    /// the source's collation sorts first or last.
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
            Expression::RowMarker => Ok(ROW_MARKER_CONSTANT.to_string()),
            Expression::Literal(literal) => render_literal(literal),
            Expression::Aggregate(call) => self.render_aggregate(call),
            Expression::Predicate(predicate) => self.render_predicate(predicate),
        }
    }

    /// Renders one column reference. Every reference goes through here — a
    /// projected one, a predicate operand, an aggregate's argument, a join
    /// condition operand, and a grouping column — so how a column is named is
    /// one decision, made once.
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
                 renders {} only; rendering a join type it does have in its place would return \
                 rows Exasol never asked for",
                quoted_names(JOIN_KEYWORDS.iter().map(|(spelling, _)| *spelling))
            ))
        })
}

/// What a column node names `table` by: the alias the request assigned it, or
/// the table's own name when the request assigned none. Measured on Exasol
/// 2025.2.1, the two agree — a `from` element carrying `alias` sends every one
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
/// qualification this dialect can render, and both fail naming the column and
/// the reference. Rendering the column unqualified instead would leave
/// PostgreSQL to resolve the name against whichever joined table declares it —
/// an ambiguous-column error when two of them do, and silently the wrong copy
/// in a self-join, where the choice decides which rows come back.
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

/// Renders the aggregate PostgreSQL writes over no argument.
///
/// `COUNT(*)` counts rows rather than values, so it is the only such form, and
/// `DISTINCT` has no values to reduce: PostgreSQL writes no
/// `COUNT(DISTINCT *)`. Both departures fail rather than render a near miss,
/// because dropping the `DISTINCT` would count every row where Exasol asked
/// for the distinct values, and supplying a `*` to another function would
/// aggregate the wrong thing.
fn render_star_aggregate(function: &str, call: &AggregateCall) -> Result<String, RenderError> {
    if function != COUNT_FUNCTION {
        return Err(RenderError::new(format!(
            "the `{function}` aggregate carries no argument, and PostgreSQL writes `{function}` \
             only over one; `{COUNT_FUNCTION}` is the only aggregate it writes over no argument"
        )));
    }
    if call.distinct {
        return Err(RenderError::new(format!(
            "the `{COUNT_FUNCTION}` aggregate carries `distinct` but no argument, and PostgreSQL \
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

/// Whether `value` is spelled as a bare PostgreSQL numeric constant.
///
/// Spelled out as a grammar rather than as the set of characters a number may
/// contain, which is the rule the MySQL dialect uses. That rule does not carry
/// over: MySQL's `--` comment needs a following space, PostgreSQL's does not,
/// so a character-set rule admitting `-` would admit `1--2` and comment out
/// the rest of the statement.
fn is_numeric_constant(value: &str) -> bool {
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

fn render_temporal_literal(keyword: &str, value: &str) -> Result<String, RenderError> {
    let is_bare_temporal = !value.is_empty()
        && value
            .chars()
            .all(|character| TEMPORAL_LITERAL_CHARACTERS.contains(character));
    if !is_bare_temporal {
        return Err(RenderError::new(format!(
            "the `{keyword}` literal value `{value}` is not spelled out of \
             `{TEMPORAL_LITERAL_CHARACTERS}` alone; PostgreSQL accepts a typed temporal literal \
             only inside quotes, so any other character in it could end the literal early"
        )));
    }

    Ok(format!("{keyword} '{value}'"))
}

/// Qualifies a table by the ordered segments the query model carries for it,
/// each quoted and joined by a dot.
///
/// A qualifier of one segment renders exactly as it did before qualifiers
/// became lists, which is what leaves PostgreSQL's generated statements
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
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

/// Visible to the crate's own tests: `capabilities_tests` drives the
/// per-dialect capability invariants through each dialect's fixture table,
/// and this module owns PostgreSQL's.
#[cfg(test)]
#[path = "postgres_dialect_tests.rs"]
pub(crate) mod tests;
