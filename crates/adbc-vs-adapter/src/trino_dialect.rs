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
//! This dialect renders no collation suffix in any position, and that is a
//! stronger claim than `postgres_dialect`'s: PostgreSQL still writes
//! `COLLATE "C"` for an operand whose order decides the answer, where this
//! module writes nothing there ever. Two measured facts license it, not one.
//! `COLLATE` was measured not to be a Trino keyword in any syntactic
//! position at all — every one of the four suffix positions a collation could
//! occupy (`ORDER BY`, a comparison operand, a `GROUP BY` key, and
//! `COUNT(DISTINCT ...)`) rejects it as a parse error, and the parser
//! accepted it as a plain column alias in the `GROUP BY` case before
//! choking on what followed, which is direct evidence `COLLATE` is an
//! ordinary unreserved identifier rather than a suffix grammar Trino has and
//! this dialect declines to use. There is therefore no suffix position to
//! render into even if a rule wanted one. The two engines were also measured
//! agreeing without any suffix, live against Trino 483 and Exasol 2025.2.1: identical `ORDER BY` order over `a, B, Z, z, A, ä, Ä, é, e`,
//! identical `MIN`/`MAX`, identical row sets for `<` and `<=`, identical
//! `GROUP BY` grouping, and identical `COUNT(DISTINCT)` over the same mixed-
//! case and accented fixture PostgreSQL's own gate used. The reason the two
//! agree is that Trino's `varchar` comparison is Unicode code-point order,
//! and Exasol's is its own byte-order comparison, which for UTF-8 is the same
//! order — confirmed by running both engines side by side rather than
//! assumed from either engine's documentation.
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
//! `LIKE` always renders an explicit `ESCAPE` clause, carrying the request's
//! own escape character when it named one and a backslash when it did not.
//! Trino was measured defining no default escape character at all, and — the
//! stronger fact — Exasol's own default escape character IS the backslash, so every pattern rendered with no `ESCAPE` clause would
//! already disagree between the two engines before either encounters a
//! pattern that needs escaping. Trino's escape rule is also strict where
//! Exasol's is permissive: the exact rejected set was measured as any
//! character other than `%`, `_`, or the escape character itself, raised at
//! execution rather than parse time as `Escape character must be followed by
//! '%', '_' or the escape character itself`. Rendering a pattern whose escape
//! character precedes such a character would send a statement the source
//! itself refuses, so this dialect fails to render it instead, naming the
//! offending sequence.
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

use crate::pushdown_query::{
    AggregateCall, Aggregation, BetweenPredicate, ColumnRef, Comparison, Expression, FromClause,
    InConstListPredicate, LikePredicate, Literal, Predicate, Query, SelectItem, SourceTable,
};
use crate::sql_dialect::{RenderError, SqlDialect};

/// The constant an empty select list projects, one digit wide because
/// `pushdown_query` declares that select-list item `DECIMAL(1,0)`.
///
/// Exasol sends an empty select list when only the row count matters, and a
/// statement projecting no column would both be invalid Trino SQL and lose
/// the count it was sent for.
const ROW_MARKER_CONSTANT: &str = "1";

/// The escape character a `LIKE` renders when the request named none.
///
/// Exasol's own default escape character was measured to be the
/// backslash, so rendering this one keeps a no-`escapeChar` request's meaning
/// rather than substituting Trino's absent default.
const DEFAULT_ESCAPE_CHARACTER: char = '\\';

/// The aggregate functions this dialect renders, spelled in Trino's canonical
/// upper case, which is also how the query model carries a function name: it
/// normalizes on parse, so a name that reaches here in any other case fails
/// this list rather than reaching the statement non-canonically. A function
/// outside the list has no rendering path, and rendering its argument in its
/// place would leave Exasol aggregating nothing and returning the wrong
/// value.
///
/// `SUM` and `AVG` are absent for a reason `vs-adapter/pushdown-aggregation`
/// states over every dialect rather than one about Trino: a capability is
/// advertised once per session and admits every numeric operand, including
/// `double`, whose sum is order-dependent under a distributed plan that
/// combines partial results.
const AGGREGATE_FUNCTIONS: &[&str] = &["COUNT", "MIN", "MAX"];

/// The one aggregate Trino writes over no argument, as `COUNT(*)`.
const COUNT_FUNCTION: &str = "COUNT";

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

/// The prefix a joined table's alias carries, completed by the table's
/// 1-based position in the `from` tree.
const TABLE_ALIAS_PREFIX: &str = "t";

/// The characters a `DATE` or `TIMESTAMP` typed literal's value may
/// carry. The value sits inside the quotes Trino requires for a typed
/// literal, so a quote reaching it could end the literal early, and this set
/// admits none.
const TEMPORAL_LITERAL_CHARACTERS: &str = "0123456789-:. ";

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

/// The `from` tree with the alias every table renders under already
/// assigned. Mirrors `postgres_dialect::AliasedFrom` exactly: an alias
/// derived from a table's position keeps the alias set unique by
/// construction, and assigning it into a tree of its own rather than looking
/// it up while rendering leaves no way for the two steps to disagree about
/// which table got which alias.
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

/// The one decision a Trino statement settles before any expression in it
/// renders: which table each column reference belongs to, and therefore how
/// a column is named. It cannot be answered from a column reference alone,
/// because it is a property of the `from` tree, and the same reference
/// reaches the renderer as a projected column, a predicate operand, an
/// aggregate argument, a join condition operand, and a grouping key alike.
///
/// Unlike `postgres_dialect::Statement`, this is the *only* decision a
/// statement settles here: nothing in this dialect collates, so no second
/// decision about which operand carries a suffix exists to thread through
/// rendering.
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

    /// Renders the `FROM` clause: one qualified table name, or the whole
    /// join tree with every table aliased.
    fn render_from(&self) -> Result<String, RenderError> {
        match self {
            Self::OneTable(table) => Ok(qualified_table(table)),
            Self::Joined(from) => self.render_join_tree(from),
        }
    }

    /// Renders one element of an aliased `from` tree. A join renders inside
    /// its own parentheses, so the request's own nesting reaches Trino
    /// without either side reasoning about associativity, and an outer join
    /// whose nesting shifted would return different rows. The condition
    /// renders through the same predicate renderer a `WHERE` clause uses,
    /// because a condition compared one way in `ON` and another way in
    /// `WHERE` would be two answers to one question.
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

    /// Renders the select list, one entry per emitted column, in the order
    /// the request gave them. Every entry renders through the one expression
    /// renderer every other position uses.
    fn render_projection(&self, select_list: &[SelectItem]) -> Result<String, RenderError> {
        if select_list.is_empty() {
            return Err(RenderError::new(
                "the query's select list is empty, and a statement projecting no column is not \
                 valid Trino SQL",
            ));
        }

        Ok(select_list
            .iter()
            .map(|item| self.render_expression(&item.expression))
            .collect::<Result<Vec<_>, _>>()?
            .join(", "))
    }

    /// Renders the `GROUP BY` clause an aggregation needs, which is nothing
    /// for a single-group one. Every grouping key renders through the same
    /// column renderer its select-list occurrence takes, so a request
    /// grouping by more than one column — `AGGREGATE_GROUP_BY_TUPLE` — needs
    /// nothing beyond joining every key by a comma: a live Trino was measured
    /// executing a two- and a three-column `GROUP BY`, returning one row per
    /// group.
    fn render_grouping(&self, aggregation: &Aggregation) -> Result<String, RenderError> {
        let Aggregation::GroupBy(columns) = aggregation else {
            return Ok(String::new());
        };
        if columns.is_empty() {
            return Err(RenderError::new(
                "a `group_by` aggregation names no grouping column, and Trino writes no empty \
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
                "a `predicate_{}` node combines no nested predicate, and Trino writes no empty \
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

    /// Renders a binary comparison. Every operand renders bare: Trino and
    /// Exasol were measured returning byte-identical answers for `<`, `<=`,
    /// and equality with no collation suffix, so no comparison here
    /// needs one — the single rule this module's own doc comment states.
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

    /// Renders a `LIKE` with an explicit `ESCAPE` clause for every pattern,
    /// this module's own doc comment states why. When the pattern and the
    /// escape character are both known literal text, the pattern is checked
    /// against the measured rejection set before rendering, so a
    /// pattern Trino would refuse at execution fails here instead, naming
    /// the offending sequence.
    fn render_like(&self, like: &LikePredicate) -> Result<String, RenderError> {
        let escape_expression = like
            .escape_char
            .clone()
            .unwrap_or_else(default_escape_expression);

        if let (Expression::Literal(Literal::String(pattern)), Some(escape_char)) =
            (&like.pattern, literal_escape_char(&escape_expression))
        {
            validate_like_pattern(pattern, escape_char)?;
        }

        Ok(format!(
            "({} LIKE {} ESCAPE {})",
            self.render_expression(&like.expression)?,
            self.render_expression(&like.pattern)?,
            self.render_expression(&escape_expression)?
        ))
    }

    /// Renders a `BETWEEN`. Every operand renders bare, the same rule
    /// [`Self::render_comparison`] applies.
    fn render_between(&self, between: &BetweenPredicate) -> Result<String, RenderError> {
        Ok(format!(
            "({} BETWEEN {} AND {})",
            self.render_expression(&between.expression)?,
            self.render_expression(&between.left)?,
            self.render_expression(&between.right)?
        ))
    }

    /// Renders an `IN` list. Every operand renders bare: `IN` asks equality,
    /// and Trino's and Exasol's equality were measured agreeing already with
    /// no suffix.
    fn render_in_const_list(
        &self,
        in_const_list: &InConstListPredicate,
    ) -> Result<String, RenderError> {
        if in_const_list.arguments.is_empty() {
            return Err(RenderError::new(
                "a `predicate_in_constlist` node carries no argument, and Trino writes no empty \
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

    /// Renders one aggregate over the current group. The function name is
    /// taken from [`AGGREGATE_FUNCTIONS`] rather than from the request, so
    /// the statement carries Trino's canonical spelling whatever case Exasol
    /// sent. The argument's absence, not a separate field, is what tells
    /// `COUNT(*)` apart from `COUNT(column)`. Every argument renders bare,
    /// `DISTINCT` included: this dialect collates nothing.
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
                 renders {} only; rendering a join type it does have in its place would return \
                 rows Exasol never asked for",
                quoted_names(JOIN_KEYWORDS.iter().map(|(spelling, _)| *spelling))
            ))
        })
}

/// What a column node names `table` by: the alias the request assigned it,
/// or the table's own name when the request assigned none.
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
/// qualification this dialect can render, and both fail naming the column
/// and the reference, because Trino would otherwise resolve the name
/// against whichever joined table declares it.
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

/// Renders the aggregate Trino writes over no argument.
///
/// `COUNT(*)` counts rows rather than values, so it is the only such form,
/// and `DISTINCT` has no values to reduce: Trino writes no
/// `COUNT(DISTINCT *)`. Both departures fail rather than render a near miss,
/// because dropping the `DISTINCT` would count every row where Exasol asked
/// for the distinct values, and supplying a `*` to another function would
/// aggregate the wrong thing.
fn render_star_aggregate(function: &str, call: &AggregateCall) -> Result<String, RenderError> {
    if function != COUNT_FUNCTION {
        return Err(RenderError::new(format!(
            "the `{function}` aggregate carries no argument, and Trino writes `{function}` only \
             over one; `{COUNT_FUNCTION}` is the only aggregate it writes over no argument"
        )));
    }
    if call.distinct {
        return Err(RenderError::new(format!(
            "the `{COUNT_FUNCTION}` aggregate carries `distinct` but no argument, and Trino \
             writes no `{COUNT_FUNCTION}(DISTINCT *)`"
        )));
    }

    Ok(format!("{COUNT_FUNCTION}(*)"))
}

/// The escape expression a `LIKE` carries when the request named no
/// `escapeChar`: [`DEFAULT_ESCAPE_CHARACTER`], rendered the way any other
/// string literal is.
fn default_escape_expression() -> Expression {
    Expression::Literal(Literal::String(DEFAULT_ESCAPE_CHARACTER.to_string()))
}

/// The single character `expression` names, when it is a one-character
/// string literal. `None` for anything else — a column reference or a
/// multi-character value — which is a shape the rejection set
/// cannot be checked against at render time, so [`Statement::render_like`]
/// renders it and lets Trino itself be the judge.
fn literal_escape_char(expression: &Expression) -> Option<char> {
    let Expression::Literal(Literal::String(value)) = expression else {
        return None;
    };
    let mut characters = value.chars();
    let first = characters.next()?;
    match characters.next() {
        None => Some(first),
        Some(_) => None,
    }
}

/// Checks `pattern` against `vs-adapter/trino-pushdown`'s measured rejection set: an
/// occurrence of `escape_char` MUST be followed by `%`, `_`, or
/// `escape_char` itself, or Trino refuses the statement at execution with
/// `Escape character must be followed by '%', '_' or the escape character
/// itself`.
fn validate_like_pattern(pattern: &str, escape_char: char) -> Result<(), RenderError> {
    let mut characters = pattern.chars();
    while let Some(character) = characters.next() {
        if character != escape_char {
            continue;
        }
        match characters.next() {
            Some(next) if next == '%' || next == '_' || next == escape_char => {}
            Some(other) => {
                return Err(RenderError::new(format!(
                    "the LIKE pattern `{pattern}` escapes `{other}` with `{escape_char}`, and \
                     Trino accepts an escaped `%`, `_`, or `{escape_char}` itself only — its own \
                     diagnosis is \"Escape character must be followed by '%', '_' or the escape \
                     character itself\""
                )));
            }
            None => {
                return Err(RenderError::new(format!(
                    "the LIKE pattern `{pattern}` ends with the escape character `{escape_char}` \
                     and names nothing for it to escape"
                )));
            }
        }
    }
    Ok(())
}

fn render_literal(literal: &Literal) -> Result<String, RenderError> {
    match literal {
        Literal::Null => Ok("NULL".to_string()),
        Literal::ExactNumeric(value) => render_numeric_literal(value),
        Literal::Double(value) => render_double_literal(value),
        Literal::String(value) => Ok(render_string_literal(value)),
        Literal::Date(value) => render_temporal_literal("DATE", value),
        Literal::Timestamp(value) => render_temporal_literal("TIMESTAMP", value),
    }
}

/// Renders a string literal as a plain quoted constant, doubling only the
/// single quote. Trino defines no backslash escape inside a string constant,
/// so — unlike `postgres_dialect`'s `E'<body>'` form — a backslash the value
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

/// Whether `value` is spelled as a bare numeric constant: an optional sign,
/// digits carrying at most one decimal point, and an optional `e` exponent.
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
             `{TEMPORAL_LITERAL_CHARACTERS}` alone; Trino accepts a typed temporal literal only \
             inside quotes, so any other character in it could end the literal early"
        )));
    }

    Ok(format!("{keyword} '{value}'"))
}

/// Qualifies a table by the ordered segments the query model carries for it,
/// each quoted and joined by a dot — `"catalog"."schema"."table"` for the two
/// segments `drivers/trino.toml` declares.
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
/// and this module owns Trino's.
#[cfg(test)]
#[path = "trino_dialect_tests.rs"]
pub(crate) mod tests;
