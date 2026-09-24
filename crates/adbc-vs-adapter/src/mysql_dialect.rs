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
//! Two rules make a rendered predicate select the rows Exasol would have
//! selected itself, and both read off the operand's declared type rather
//! than off the shape of the request. A character-typed operand carries an
//! explicit `COLLATE utf8mb4_0900_as_cs`, because MySQL's default
//! `utf8mb4_0900_ai_ci` ignores case and accent where Exasol does not, and
//! Exasol never re-applies a delegated filter. Numeric and temporal operands
//! carry no collation. Every predicate renders as its own parenthesized
//! group, so nesting needs no precedence reasoning and the select-list
//! position renders through the same path as `WHERE`.
//!
//! A rendered aggregate reads the same declared type for a different
//! question. `MIN` and `MAX` return a value chosen by how their operand
//! orders, and no collation answers ordering and equality at once, so a
//! character operand of those two carries `COLLATE utf8mb4_0900_bin` where a
//! comparison operand carries `utf8mb4_0900_as_cs`. Which functions this
//! dialect renders is its own question, decided here rather than in the
//! model: an aggregate the renderer does not know fails naming itself, since
//! rendering its argument alone would leave Exasol aggregating nothing.
//!
//! Grouping and `DISTINCT` ask the equality question a comparison asks, and
//! take its collation, but grouping cannot be answered from one operand
//! alone. MySQL's `ONLY_FULL_GROUP_BY` mode, which `mysql:8.4` sets by
//! default, rejects a select list whose bare column occurrence does not
//! carry the collation its `GROUP BY` key carries, so a grouped character
//! column and its projection are one decision rather than two: which columns
//! the query groups by is settled once, from the whole query, and both
//! occurrences render through the same function, which is what makes them
//! unable to disagree. `MIN` and `MAX` keep the ordering collation in the
//! same statement whatever `DISTINCT` they carry, because an aggregate is
//! never subject to the mode and the value those two return is still chosen
//! by order.
//!
//! A join adds the second decision no column reference can answer alone:
//! which table it belongs to. Every table of a join renders under an alias
//! this dialect derives from the table's position in the `from` tree, and
//! every reference renders qualified by the alias of the table it resolves
//! to. Measured on MySQL 8.4, the alternatives each fail rather than diverge
//! quietly: an unqualified reference two joined tables both declare is
//! rejected with error 1052, one table named twice with no alias to tell the
//! copies apart with error 1066, and a grouping key whose select-list
//! occurrence carries a different alias or a different collation with error
//! 1055. A reference this dialect cannot resolve to exactly one table fails
//! naming it, because MySQL would otherwise resolve the name against
//! whichever joined table happens to declare it. A single-table query
//! qualifies and aliases nothing: one table's column cannot be ambiguous,
//! and its statement stays the one this dialect generated before it rendered
//! joins at all.

use adbc_vs_scan::driver::Driver;

use crate::exa_type::ExasolType;
use crate::mysql_type_mapping::SUPPORTED_CHARACTER_SET;
use crate::pushdown_query::{
    AggregateCall, Aggregation, BetweenPredicate, ColumnRef, Comparison, Expression, FromClause,
    InConstListPredicate, LikePredicate, Literal, Predicate, Query, SelectItem, SourceTable,
};
use crate::sql_dialect::{RenderError, SqlDialect};

/// The collation every character-typed comparison operand carries. Measured
/// on MySQL 8.4 and Exasol 2025.2.1: it reproduces Exasol's case
/// sensitivity, accent sensitivity, and `VARCHAR` padding, and it is the
/// only collation the comparison evidence covers. Ordering is a different
/// question with a different answer — see `vs-adapter/pushdown-aggregation`
/// for the collation `MIN` and `MAX` carry.
///
/// `create_virtual_schema` reads it to check the source server supplies it,
/// so the collations that check requires and the collations this dialect
/// emits are one list: a source validated against a name the renderer no
/// longer emits would fail every delegated character predicate with MySQL
/// error 1273 at query time, which is the outcome the creation-time check
/// exists to prevent.
pub(crate) const COMPARISON_COLLATION: &str = "utf8mb4_0900_as_cs";

/// The collation a character operand of [`ORDERING_FUNCTIONS`] carries.
/// Measured on MySQL 8.4 and Exasol 2025.2.1: its weight is the `utf8mb4`
/// encoding bytes, so it reproduces Exasol's code-point order, and
/// `information_schema.COLLATIONS` reports it `NO PAD`, so it reproduces
/// Exasol's unpadded `VARCHAR` comparison. Neither alternative holds on both
/// counts: [`COMPARISON_COLLATION`] orders by collation weight and returns
/// `a` for `MIN` over `B`, `a`, `C` where Exasol returns `B`, and
/// `utf8mb4_bin` orders by code point but is `PAD SPACE` and returns `'ab '`
/// for `MIN` over `'ab '`, `'ab'`, `'ab!'` where Exasol returns `'ab'`. It
/// arrived in MySQL 8.0.17, the source-server floor
/// `vs-adapter/create-virtual-schema` enforces, and that check reads this
/// constant for the reason [`COMPARISON_COLLATION`] states.
pub(crate) const ORDERING_COLLATION: &str = "utf8mb4_0900_bin";

/// The aggregate functions this dialect renders, spelled in MySQL's
/// canonical upper case, which is also how the query model carries a
/// function name: it normalizes on parse, so a name that reaches here in any
/// other case fails this list rather than reaching the statement
/// non-canonically. A function outside the list has no rendering path, and
/// rendering its argument in its place would leave Exasol aggregating
/// nothing and returning the wrong value.
const AGGREGATE_FUNCTIONS: &[&str] = &["COUNT", "MIN", "MAX"];

/// The aggregate functions that return one of their operand's own values
/// rather than a number computed from them. Two answers follow from that one
/// fact: their result is chosen by how the operand orders, so a character
/// operand carries [`ORDERING_COLLATION`], and their result carries the
/// operand's own type, which is what [`is_character_typed`] reads for an
/// aggregate. `COUNT` reads no order and returns a number whatever it is
/// given.
const ORDERING_FUNCTIONS: &[&str] = &["MIN", "MAX"];

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
        let statement = Statement::of(query);
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

/// The tables one statement reads from, and how a column reference in it
/// names the table it belongs to.
enum TableScope<'a> {
    /// One table, named by nothing: a single table's column reference cannot
    /// be ambiguous, so the statement carries no alias and no qualifier even
    /// when the request assigned the table an alias of its own.
    OneTable(&'a SourceTable),
    /// A join, every table aliased and every column reference qualified.
    Joined(AliasedFrom<'a>),
}

impl<'a> TableScope<'a> {
    fn of(from: &'a FromClause) -> Self {
        match from {
            FromClause::Table(table) => Self::OneTable(table),
            FromClause::Join(_) => Self::Joined(AliasedFrom::of(from)),
        }
    }
}

/// The two decisions one statement settles before any expression in it
/// renders, and that every expression then reads.
///
/// Neither can be answered from a column reference alone. Which table a
/// reference belongs to is a property of the `from` tree, and which columns
/// the query groups by is a property of the whole query — the same reference
/// reaches the renderer as a projected column, a predicate operand, an
/// aggregate argument, a join condition operand, and a grouping key alike,
/// and both answers bear on what it renders as. Settling them once, here, is
/// what leaves a grouping key and its select-list occurrence unable to
/// disagree about either: MySQL rejects a mismatched collation and a
/// mismatched qualifier alike, with error 1055 and error 1052.
struct Statement<'a> {
    tables: TableScope<'a>,
    grouped: &'a [ColumnRef],
}

impl<'a> Statement<'a> {
    fn of(query: &'a Query) -> Self {
        Self {
            tables: TableScope::of(&query.from),
            grouped: grouped_columns(query),
        }
    }

    /// Renders the `FROM` clause: one qualified table name, or the whole join
    /// tree with every table aliased.
    fn render_from(&self) -> Result<String, RenderError> {
        match &self.tables {
            TableScope::OneTable(table) => Ok(qualified_table(table)),
            TableScope::Joined(from) => self.render_join_tree(from),
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

    /// Renders the select list: the one position where what the query groups
    /// by changes how an expression renders, or whether it renders at all.
    fn render_projection(&self, select_list: &[SelectItem]) -> Result<String, RenderError> {
        if select_list.is_empty() {
            return Err(RenderError::new(
                "the query's select list is empty, and a statement projecting no column is not \
                 valid MySQL",
            ));
        }

        Ok(select_list
            .iter()
            .map(|item| self.render_select_item(&item.expression))
            .collect::<Result<Vec<_>, _>>()?
            .join(", "))
    }

    /// Renders one select-list entry, the two grouping-sensitive cases first.
    ///
    /// A bare occurrence of a grouped column takes the `GROUP BY` clause's
    /// own renderer, which is what makes the two occurrences unable to carry
    /// different collations or different alias qualifiers. A predicate
    /// reading such a column is the mirror case, and the one entry with no
    /// rendering at all: it is rejected rather than rendered, because MySQL
    /// rejects every statement that expresses it. Every other expression
    /// renders exactly as it does in any other position.
    fn render_select_item(&self, expression: &Expression) -> Result<String, RenderError> {
        match expression {
            Expression::Column(column) if self.groups(column) => self.render_grouped_column(column),
            Expression::Predicate(predicate) => {
                self.reject_collated_grouping_key(predicate)?;
                self.render_expression(expression)
            }
            expression => self.render_expression(expression),
        }
    }

    /// Whether the query groups by the column `column` names, and so whether
    /// that occurrence renders through [`Statement::render_grouped_column`].
    ///
    /// The `GROUP BY` key and the select-list occurrence of one grouped
    /// column arrive as two separate references that Exasol qualifies
    /// independently — `tableName` on one and `tableAlias` on the other names
    /// one column — and MySQL rejects the statement with error 1055 under
    /// `ONLY_FULL_GROUP_BY` unless both occurrences render identically. So
    /// this question is asked of the column's name and the table it resolves
    /// to, not of the whole reference: answering `false` for a differently
    /// qualified occurrence of the grouping key renders the projection bare
    /// beside a collated key, which is the one statement the coordinated rule
    /// exists to prevent.
    fn groups(&self, column: &ColumnRef) -> bool {
        self.grouped
            .iter()
            .any(|key| self.names_the_same_column(key, column))
    }

    /// Whether two column references name one column of one table.
    ///
    /// Inside a join the table is the alias each reference resolves to, so
    /// two spellings of one table agree while two joined tables that both
    /// declare the name do not. Outside a join every reference resolves to
    /// the statement's only table, which is also why a single-table statement
    /// renders no qualifier at all. A reference this dialect cannot resolve
    /// names no table to compare; rendering it fails naming it, in
    /// [`Statement::render_column`].
    fn names_the_same_column(&self, one: &ColumnRef, other: &ColumnRef) -> bool {
        if one.name != other.name {
            return false;
        }
        let TableScope::Joined(from) = &self.tables else {
            return true;
        };

        match (resolve_alias(from, one), resolve_alias(from, other)) {
            (Ok(one), Ok(other)) => one == other,
            _ => false,
        }
    }

    fn reject_collated_grouping_key(&self, predicate: &Predicate) -> Result<(), RenderError> {
        let Some(column) = self.collated_grouping_key_read_by(predicate) else {
            return Ok(());
        };

        Err(RenderError::new(format!(
            "the select list carries a predicate reading `{}`, the character column this query \
             groups by; that column's `GROUP BY` key carries `COLLATE {COMPARISON_COLLATION}`, \
             and MySQL credits no sub-expression of a `GROUP BY` expression, so it rejects every \
             rendering of this shape with error 1055 under `ONLY_FULL_GROUP_BY`",
            column.name
        )))
    }

    /// The first collated grouping key a select-list predicate reads outside
    /// an aggregate, if it reads one at all — the shape no rendering
    /// satisfies.
    ///
    /// Which grouping keys render collated is
    /// [`Statement::render_grouped_column`]'s rule, read here rather than
    /// restated: the character-typed ones. A numeric grouping key renders as
    /// the same bare column in both clauses, so MySQL matches the predicate's
    /// operand against the whole `GROUP BY` expression and credits it —
    /// measured, four rows rather than error 1055.
    ///
    /// The walk descends every operand of every nested predicate but stops at
    /// an [`Expression::Aggregate`], because an aggregate reduces its group to
    /// one value that MySQL matches whole rather than reading the column
    /// inside it as a grouping key. Measured on MySQL 8.4 beside a collated
    /// grouping key on the same column, both `COUNT(*) > 0` and
    /// `MIN("g") > 'x'` run.
    fn collated_grouping_key_read_by<'p>(&self, predicate: &'p Predicate) -> Option<&'p ColumnRef> {
        match predicate {
            Predicate::And(predicates) | Predicate::Or(predicates) => predicates
                .iter()
                .find_map(|nested| self.collated_grouping_key_read_by(nested)),
            Predicate::Not(inner) => self.collated_grouping_key_read_by(inner),
            Predicate::Equal(comparison)
            | Predicate::NotEqual(comparison)
            | Predicate::Less(comparison)
            | Predicate::LessEqual(comparison) => self
                .collated_grouping_key_in(&comparison.left)
                .or_else(|| self.collated_grouping_key_in(&comparison.right)),
            Predicate::IsNull(expression) | Predicate::IsNotNull(expression) => {
                self.collated_grouping_key_in(expression)
            }
            Predicate::Like(like) => self
                .collated_grouping_key_in(&like.expression)
                .or_else(|| self.collated_grouping_key_in(&like.pattern))
                .or_else(|| {
                    like.escape_char
                        .as_ref()
                        .and_then(|escape_char| self.collated_grouping_key_in(escape_char))
                }),
            Predicate::Between(between) => self
                .collated_grouping_key_in(&between.expression)
                .or_else(|| self.collated_grouping_key_in(&between.left))
                .or_else(|| self.collated_grouping_key_in(&between.right)),
            Predicate::InConstList(in_const_list) => self
                .collated_grouping_key_in(&in_const_list.expression)
                .or_else(|| {
                    in_const_list
                        .arguments
                        .iter()
                        .find_map(|argument| self.collated_grouping_key_in(argument))
                }),
        }
    }

    fn collated_grouping_key_in<'p>(&self, expression: &'p Expression) -> Option<&'p ColumnRef> {
        match expression {
            Expression::Column(column) => {
                (self.groups(column) && is_character_column(column)).then_some(column)
            }
            Expression::Predicate(predicate) => self.collated_grouping_key_read_by(predicate),
            Expression::Aggregate(_) | Expression::Literal(_) | Expression::RowMarker => None,
        }
    }

    /// Renders the `GROUP BY` clause an aggregation needs, which is nothing
    /// for a single-group one.
    ///
    /// The two aggregation kinds differ only in this clause: MySQL aggregates
    /// a whole result into one row for a select list of aggregates carrying
    /// no `GROUP BY` at all, which is exactly what Exasol asks for with
    /// `single_group`.
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
            .map(|column| self.render_grouped_column(column))
            .collect::<Result<Vec<_>, _>>()?
            .join(", ");

        Ok(format!(" GROUP BY {grouping}"))
    }

    /// Renders a column the query groups by — the one operand whose rendering
    /// two clauses share.
    ///
    /// Grouping reduces rows by equality, the question
    /// [`COMPARISON_COLLATION`] answers, so a character-typed grouping column
    /// carries it and a numeric or temporal one carries nothing. Both the
    /// `GROUP BY` key and that column's bare select-list occurrence come
    /// through here rather than each deciding for itself: MySQL rejects the
    /// statement with error 1055 under `ONLY_FULL_GROUP_BY` when the two
    /// occurrences differ in any way — a different collation or, once a join
    /// qualifies them, a different alias — and one renderer for both is what
    /// makes them unable to differ.
    fn render_grouped_column(&self, column: &ColumnRef) -> Result<String, RenderError> {
        let operand = self.render_column(column)?;
        if !is_character_column(column) {
            return Ok(operand);
        }

        Ok(format!("{operand} COLLATE {COMPARISON_COLLATION}"))
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
            self.render_compared_operand(&comparison.left)?,
            self.render_compared_operand(&comparison.right)?
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
            self.render_compared_operand(&like.expression)?,
            self.render_expression(&like.pattern)?
        ))
    }

    fn render_between(&self, between: &BetweenPredicate) -> Result<String, RenderError> {
        Ok(format!(
            "({} BETWEEN {} AND {})",
            self.render_compared_operand(&between.expression)?,
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
            self.render_compared_operand(&in_const_list.expression)?,
            arguments.join(", ")
        ))
    }

    /// Renders the operand whose collation governs a predicate's comparison.
    ///
    /// An explicit `COLLATE` outranks the collation of every other operand,
    /// so one suffix settles the whole predicate: the pattern of a `LIKE`,
    /// the bounds of a `BETWEEN`, and the entries of an `IN` list are
    /// rendered plainly and compared under it. A binary comparison has no
    /// single such operand — Exasol swaps the operands of `>` and `>=`, so
    /// neither position reliably holds the column — so both of its operands
    /// come through here and each character-typed one carries the suffix.
    /// A join condition takes the same path, so an equi-join over two
    /// character keys matches the rows Exasol would match: measured on
    /// MySQL 8.4, the same join returns 8 rows uncollated where the collated
    /// form returns Exasol's 4.
    ///
    /// A `DISTINCT` aggregate operand comes through here too: `DISTINCT`
    /// reduces values by equality, which is this collation's own question, so
    /// it takes the same answer rather than a second one of its own.
    fn render_compared_operand(&self, expression: &Expression) -> Result<String, RenderError> {
        let operand = self.render_expression(expression)?;
        if !is_character_typed(expression) {
            return Ok(operand);
        }

        Ok(format!("{operand} COLLATE {COMPARISON_COLLATION}"))
    }

    /// Renders one aggregate over the current group.
    ///
    /// The function name is taken from [`AGGREGATE_FUNCTIONS`] rather than
    /// from the request, so the statement carries MySQL's canonical spelling
    /// whatever case Exasol sent — the model already normalized it, and one
    /// owner of that decision is enough. The argument's absence, not a
    /// separate field, is what tells `COUNT(*)` apart from `COUNT(column)`.
    ///
    /// Which collation the argument carries is the argument's own question,
    /// asked three ways. [`ORDERING_FUNCTIONS`] return the value their
    /// operand's collation sorts first or last, so a character argument of
    /// one carries [`ORDERING_COLLATION`] whatever `DISTINCT` it also
    /// carries — the value is still chosen by order, and reducing the values
    /// first cannot change it. A `DISTINCT` argument of any other function is
    /// reduced by equality and carries [`COMPARISON_COLLATION`], because
    /// MySQL's default collation equates values Exasol keeps apart and a
    /// delegated `COUNT(DISTINCT)` would otherwise return a smaller count
    /// than Exasol asked for. An argument that is neither ordered nor reduced
    /// carries no collation at all.
    fn render_aggregate(&self, call: &AggregateCall) -> Result<String, RenderError> {
        let function = renderable_function(&call.function)?;
        let Some(argument) = &call.argument else {
            return render_star_aggregate(function, call);
        };

        let distinct = if call.distinct { "DISTINCT " } else { "" };
        let operand = if ORDERING_FUNCTIONS.contains(&function) {
            self.render_ordering_operand(argument)?
        } else if call.distinct {
            self.render_compared_operand(argument)?
        } else {
            self.render_expression(argument)?
        };

        Ok(format!("{function}({distinct}{operand})"))
    }

    /// Renders the operand whose ordering decides an aggregate's result, the
    /// mirror of [`Statement::render_compared_operand`] for the other
    /// collation.
    ///
    /// `MIN` and `MAX` return the value their operand's collation sorts first
    /// or last, where a comparison asks only whether two values are equal,
    /// and no collation answers both questions the way Exasol does. The
    /// suffix reads off the operand's declared type, as every collation
    /// decision here does, so a numeric or temporal operand carries none.
    fn render_ordering_operand(&self, expression: &Expression) -> Result<String, RenderError> {
        let operand = self.render_expression(expression)?;
        if !is_character_typed(expression) {
            return Ok(operand);
        }

        Ok(format!("{operand} COLLATE {ORDERING_COLLATION}"))
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
        let TableScope::Joined(from) = &self.tables else {
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

/// The columns the query's `GROUP BY` clause names, empty for a query that
/// groups by nothing.
///
/// Which columns a query groups by is settled here, once, from the whole
/// query, because MySQL's `ONLY_FULL_GROUP_BY` mode makes the grouping list
/// and the select list one question and no single column reference can answer
/// it: the same reference reaches the renderer as a projected column, a
/// predicate operand, an aggregate argument, and a grouping key alike, and
/// which of those it is decides which collation — if any — it carries. A
/// projected column is the one that cannot tell from itself.
fn grouped_columns(query: &Query) -> &[ColumnRef] {
    match &query.aggregation {
        Some(Aggregation::GroupBy(columns)) => columns,
        Some(Aggregation::SingleGroup) | None => &[],
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

fn is_character_column(column: &ColumnRef) -> bool {
    matches!(
        column.declared_type,
        ExasolType::Varchar { .. } | ExasolType::Char { .. }
    )
}

/// Whether an expression's value is character-typed, and so whether a
/// collation decides how it compares.
///
/// Every answer reads a declared type rather than the shape of the request.
/// An aggregate is the one expression with no declared type of its own to
/// read — a select-list predicate over it is declared `BOOLEAN`, which says
/// nothing about the operand — so the answer comes from what the function
/// returns: [`ORDERING_FUNCTIONS`] return one of their argument's own values
/// and carry its type, and every other function this dialect renders returns
/// a number. Answering `false` for a character-typed aggregate would compare
/// `MIN("note")` under MySQL's default collation and match rows Exasol keeps
/// apart. A predicate's own value is `BOOLEAN`, which no collation compares.
fn is_character_typed(expression: &Expression) -> bool {
    match expression {
        Expression::Column(column) => is_character_column(column),
        Expression::Literal(Literal::String(_)) => true,
        Expression::Literal(_) | Expression::RowMarker => false,
        Expression::Aggregate(call) => is_character_aggregate(call),
        Expression::Predicate(_) => false,
    }
}

fn is_character_aggregate(call: &AggregateCall) -> bool {
    ORDERING_FUNCTIONS.contains(&call.function.as_str())
        && call
            .argument
            .as_ref()
            .is_some_and(|argument| is_character_typed(argument))
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
