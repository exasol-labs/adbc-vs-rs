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
    AggregateCall, Aggregation, Arithmetic, BetweenPredicate, ColumnRef, Comparison, DatePart,
    Direction, Expression, FromClause, InConstListPredicate, LikePredicate, Literal, Operand,
    OperandConversion, Operator, Predicate, Query, ScalarCall, SelectItem, SourceTable, ValueClass,
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
    const TARGET: &'static str; // NOSONAR: an associated const cannot elide its lifetime
    /// How the "valid ..." clause of an empty select list error names the
    /// target's SQL.
    const SQL_NAME: &'static str; // NOSONAR: an associated const cannot elide its lifetime
    /// The character that quotes an identifier, doubled inside it.
    const QUOTE: char = '"';
    /// The keyword each `join_type` the dialect renders spells, which is also
    /// the closed set of values it renders at all.
    const JOIN_KEYWORDS: &'static [(&'static str, &'static str)]; // NOSONAR: an associated const cannot elide its lifetime
    /// What a `join_type` error says between the list of join types and the
    /// reason substitution is wrong, for a target missing a join type.
    const JOIN_NOTE: &'static str = ""; // NOSONAR: an associated const cannot elide its lifetime
    /// The target's double-precision type, which an `AVG` quotient casts to.
    const DOUBLE_TYPE: &'static str; // NOSONAR: an associated const cannot elide its lifetime
    /// The target's character type, which a number or a date casts to as text.
    const TEXT_TYPE: &'static str; // NOSONAR: an associated const cannot elide its lifetime
    /// The function that counts a string's characters rather than its bytes.
    const LENGTH_FUNCTION: &'static str; // NOSONAR: an associated const cannot elide its lifetime
    /// The timestamp type a date or a date's text casts to.
    const TIMESTAMP_TYPE: &'static str; // NOSONAR: an associated const cannot elide its lifetime
    /// The integer type a day count and a `SUBSTR` position cast to.
    const INTEGER_TYPE: &'static str; // NOSONAR: an associated const cannot elide its lifetime
    /// The function that removes a set of characters from both ends.
    const TRIM_SET_FUNCTION: &'static str = "TRIM"; // NOSONAR: an associated const cannot elide its lifetime

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

    /// A rendered scalar expression in the position this statement renders,
    /// which a dialect that must mark a grouped expression wraps.
    fn grouped(&self, rendered: String) -> String {
        rendered
    }

    /// `YEAR`, `MONTH`, or `DAY` of a date or timestamp.
    fn date_part(&self, part: DatePart, operand: &str) -> String {
        format!("{}({operand})", part.name())
    }

    /// Text in Exasol's default timestamp format read as a timestamp.
    fn text_to_timestamp(&self, operand: &str) -> String {
        format!("CAST({operand} AS {})", Self::TIMESTAMP_TYPE)
    }

    /// An exact number with `scale` fractional digits as Exasol's text, which
    /// drops trailing fractional zeros.
    fn number_text(&self, operand: &str, scale: u32) -> String;

    /// A date as `YYYY-MM-DD`, or a timestamp as `YYYY-MM-DD HH24:MI:SS.FF6`,
    /// Exasol's default formats.
    fn temporal_text(&self, operand: &str, class: ValueClass) -> String;

    /// `TRIM` over text, removing blanks or every character of `characters`.
    fn render_trim(&self, operand: &str, characters: Option<&str>) -> Result<String, RenderError> {
        Ok(match characters {
            Some(characters) => format!("{}({operand}, {characters})", Self::TRIM_SET_FUNCTION),
            None => format!("TRIM({operand})"),
        })
    }

    /// An exact operand of `operator`, widened so the operation cannot
    /// overflow before Exasol's would. `class` carries the precision and
    /// scale Exasol types the operand with.
    fn exact(&self, operand: &str, class: ValueClass, operator: Operator) -> String;

    /// A whole number as the integer type a target's day and position
    /// arguments take.
    fn integer(&self, operand: &str) -> String {
        format!("CAST({operand} AS {})", Self::INTEGER_TYPE)
    }

    /// A date or a timestamp, a date reading as midnight.
    fn timestamp(&self, operand: &str, class: ValueClass) -> String {
        match class {
            ValueClass::Date => self.text_to_timestamp(operand),
            _ => operand.to_string(),
        }
    }

    /// A date or timestamp shifted by `days`, an integer.
    fn shift_days(
        &self,
        temporal: &str,
        class: ValueClass,
        days: &str,
        direction: Direction,
    ) -> String;

    /// The whole days from `right` to `left`, two dates.
    fn days_between(&self, left: &str, right: &str) -> String;

    /// The interval from `right` to `left`, two timestamps, truncated to the
    /// milliseconds an Exasol interval holds.
    fn timestamp_difference(&self, left: &str, right: &str) -> String {
        format!("({left} - {right})")
    }

    /// A date or timestamp shifted by an interval; a date stays a date.
    fn shift_interval(
        &self,
        temporal: &str,
        class: ValueClass,
        interval: &str,
        direction: Direction,
    ) -> String {
        let sign = direction.sign();
        match class {
            ValueClass::Date => format!(
                "CAST({} {sign} {interval} AS DATE)",
                self.timestamp(temporal, class)
            ),
            _ => format!("({temporal} {sign} {interval})"),
        }
    }

    /// An interval times a double, truncated to milliseconds.
    fn interval_scale(&self, interval: &str, factor: &str) -> String {
        format!("({interval} * {factor})")
    }

    /// An interval as its count of seconds.
    fn interval_seconds(&self, interval: &str) -> String;

    /// Renders a scalar function over its converted operands.
    ///
    /// A character result that is empty renders as null, because Exasol reads
    /// an empty string as null and a source keeps them apart.
    fn render_scalar(&self, call: &ScalarCall) -> Result<String, RenderError> {
        Ok(match call {
            ScalarCall::DatePart { part, operand } => {
                let rendered = self.render_expression(&operand.expression)?;
                let temporal = match operand.class {
                    ValueClass::Date | ValueClass::Timestamp => rendered,
                    ValueClass::Character => self.text_to_timestamp(&rendered),
                    other => return Err(unconvertible("a date part", other)),
                };
                self.date_part(*part, &temporal)
            }
            ScalarCall::Upper(operand) => empty_as_null(&format!("UPPER({})", self.text(operand)?)),
            ScalarCall::Lower(operand) => empty_as_null(&format!("LOWER({})", self.text(operand)?)),
            ScalarCall::Trim {
                operand,
                characters,
            } => {
                let characters = characters
                    .as_ref()
                    .map(|characters| self.text(characters))
                    .transpose()?;
                empty_as_null(&self.render_trim(&self.text(operand)?, characters.as_deref())?)
            }
            ScalarCall::Length(operand) => format!(
                "{}({})",
                Self::LENGTH_FUNCTION,
                empty_as_null(&self.text(operand)?)
            ),
            ScalarCall::Substr {
                operand,
                start,
                length,
            } => self.render_substr(operand, start, length.as_ref())?,
            ScalarCall::Arithmetic(arithmetic) => self.render_arithmetic(arithmetic)?,
        })
    }

    /// Renders `SUBSTR` on Exasol's position rule: a position of 0 reads as 1,
    /// a negative one counts from the end, and a start left of the first
    /// character or a length below 1 yields null. A positive literal position
    /// or length needs no rule, which keeps the common statement short.
    fn render_substr(
        &self,
        operand: &Operand,
        start: &Operand,
        length: Option<&Operand>,
    ) -> Result<String, RenderError> {
        let text = self.text(operand)?;
        let mut null_when = Vec::new();

        let position = self.whole(start)?;
        let start = if is_positive_integer_literal(start) {
            position
        } else {
            let start = format!(
                "CASE WHEN {position} = 0 THEN 1 WHEN {position} < 0 THEN {}({text}) + {position} \
                 + 1 ELSE {position} END",
                Self::LENGTH_FUNCTION
            );
            null_when.push(format!("{start} < 1"));
            start
        };
        let mut arguments = vec![text, self.integer(&start)];
        if let Some(length) = length {
            let count = self.whole(length)?;
            if !is_positive_integer_literal(length) {
                null_when.push(format!("{count} < 1"));
            }
            arguments.push(self.integer(&count));
        }

        let substring = format!("SUBSTR({})", arguments.join(", "));
        Ok(empty_as_null(&if null_when.is_empty() {
            substring
        } else {
            format!(
                "CASE WHEN {} THEN NULL ELSE {substring} END",
                null_when.join(" OR ")
            )
        }))
    }

    /// Renders `ADD`, `SUB`, or `MULT` as the operation its operands resolved
    /// to.
    fn render_arithmetic(&self, arithmetic: &Arithmetic) -> Result<String, RenderError> {
        Ok(match arithmetic {
            Arithmetic::Exact {
                operator,
                left,
                right,
            } => format!(
                "({} {} {})",
                self.exact(&self.number(left)?, exact_class(left.class), *operator),
                operator.symbol(),
                self.exact(&self.number(right)?, exact_class(right.class), *operator)
            ),
            Arithmetic::Double {
                operator,
                left,
                right,
            } => format!(
                "({} {} {})",
                self.double(left)?,
                operator.symbol(),
                self.double(right)?
            ),
            Arithmetic::ShiftDays {
                temporal,
                days,
                direction,
            } => self.shift_days(
                &self.render_expression(&temporal.expression)?,
                temporal.class,
                &self.integer(&self.whole(days)?),
                *direction,
            ),
            Arithmetic::DaysBetween { left, right } => self.days_between(
                &self.render_expression(&left.expression)?,
                &self.render_expression(&right.expression)?,
            ),
            Arithmetic::TimestampDifference { left, right } => self.timestamp_difference(
                &self.timestamp(&self.render_expression(&left.expression)?, left.class),
                &self.timestamp(&self.render_expression(&right.expression)?, right.class),
            ),
            Arithmetic::ShiftInterval {
                temporal,
                interval,
                direction,
            } => self.shift_interval(
                &self.render_expression(&temporal.expression)?,
                temporal.class,
                &self.render_expression(&interval.expression)?,
                *direction,
            ),
            Arithmetic::IntervalSum {
                left,
                right,
                direction,
            } => format!(
                "({} {} {})",
                self.render_expression(&left.expression)?,
                direction.sign(),
                self.render_expression(&right.expression)?
            ),
            Arithmetic::IntervalScale { interval, factor } => self.interval_scale(
                &self.render_expression(&interval.expression)?,
                &self.double(factor)?,
            ),
        })
    }

    /// An operand as Exasol reads it in exact arithmetic: a boolean as 1 or
    /// 0, every other number as itself.
    fn number(&self, operand: &Operand) -> Result<String, RenderError> {
        let rendered = self.render_expression(&operand.expression)?;
        Ok(match operand.class {
            ValueClass::Boolean => boolean_number(&rendered),
            _ => rendered,
        })
    }

    /// An operand as a double, text read as a number with an empty value as
    /// null, as the `SUM` operand conversion reads it.
    fn double(&self, operand: &Operand) -> Result<String, RenderError> {
        let rendered = self.render_expression(&operand.expression)?;
        Ok(match operand.class {
            ValueClass::Character => self.text_double(&rendered),
            ValueClass::Boolean => format!(
                "CAST({} AS {})",
                boolean_number(&rendered),
                Self::DOUBLE_TYPE
            ),
            _ => format!("CAST({rendered} AS {})", Self::DOUBLE_TYPE),
        })
    }

    fn text_double(&self, rendered: &str) -> String {
        format!(
            "CAST({} AS {})",
            counted(Some(OperandConversion::Character), rendered),
            Self::DOUBLE_TYPE
        )
    }

    /// An operand rounded to a whole number half away from zero, as Exasol
    /// rounds a day count or a position.
    fn whole(&self, operand: &Operand) -> Result<String, RenderError> {
        Ok(match operand.class {
            ValueClass::Exact { scale: 0, .. } => self.render_expression(&operand.expression)?,
            ValueClass::Boolean => boolean_number(&self.render_expression(&operand.expression)?),
            ValueClass::Exact { .. } => {
                format!("ROUND({})", self.render_expression(&operand.expression)?)
            }
            _ => {
                let double = self.double(operand)?;
                format!("(SIGN({double}) * FLOOR(ABS({double}) + 0.5))")
            }
        })
    }

    /// An operand as the text Exasol converts it to under its default formats.
    fn text(&self, operand: &Operand) -> Result<String, RenderError> {
        let rendered = self.render_expression(&operand.expression)?;
        Ok(match operand.class {
            ValueClass::Character => rendered,
            ValueClass::Exact { scale: 0, .. } | ValueClass::Double => {
                format!("CAST({rendered} AS {})", Self::TEXT_TYPE)
            }
            ValueClass::Exact { scale, .. } => self.number_text(&rendered, scale),
            ValueClass::Date | ValueClass::Timestamp => {
                self.temporal_text(&rendered, operand.class)
            }
            ValueClass::Boolean => {
                format!("CASE WHEN {rendered} THEN 'TRUE' WHEN NOT {rendered} THEN 'FALSE' END")
            }
            ValueClass::Interval => return Err(unconvertible("a string function", operand.class)),
        })
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
    fn render_group_by_keys(&self, keys: &[Expression]) -> Result<Vec<String>, RenderError> {
        if keys.is_empty() {
            return Err(RenderError::new(format!(
                "a `group_by` aggregation names no grouping key, and {} writes no empty \
                 `GROUP BY` clause to render in its place; a `single_group` aggregation is the \
                 one that groups by nothing",
                Self::TARGET
            )));
        }

        keys.iter().map(|key| self.render_expression(key)).collect()
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
            Expression::Scalar(call) => Ok(self.grouped(self.render_scalar(call)?)),
            Expression::IntervalSeconds { interval, .. } => {
                Ok(self.interval_seconds(&self.render_expression(interval)?))
            }
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

/// Renders a whole query for a dialect whose `GROUP BY` keys are the plain
/// rendering of the grouping keys.
pub fn render_select<S: StatementRenderer>(
    statement: &S,
    query: &Query,
) -> Result<String, RenderError> {
    let projection = statement.render_projection(&query.select_list)?;
    let from = statement.render_from()?;
    let filter = optional_clause("WHERE", query.filter.as_ref(), |predicate| {
        statement.render_predicate(predicate)
    })?;
    let grouping = match &query.aggregation {
        Some(Aggregation::GroupBy(keys)) => {
            format!(
                " GROUP BY {}",
                statement.render_group_by_keys(keys)?.join(", ")
            )
        }
        _ => String::new(),
    };
    let having = optional_clause("HAVING", query.having.as_ref(), |predicate| {
        statement.render_predicate(predicate)
    })?;

    Ok(format!(
        "SELECT {projection} FROM {from}{filter}{grouping}{having}"
    ))
}

/// Renders a literal for a dialect that spells a `DOUBLE` like an exact
/// numeric, with the dialect's own numeric and string spellings.
pub fn render_literal_with(
    target: &str,
    literal: &Literal,
    numeric: fn(&str) -> Result<String, RenderError>,
    string: fn(&str) -> String,
) -> Result<String, RenderError> {
    match literal {
        Literal::Null => Ok("NULL".to_string()),
        Literal::ExactNumeric(value) | Literal::Double(value) => numeric(value),
        Literal::String(value) => Ok(string(value)),
        Literal::Date(value) => render_temporal_literal(target, "DATE", value),
        Literal::Timestamp(value) => render_temporal_literal(target, "TIMESTAMP", value),
    }
}

/// A character result read as null when empty, as Exasol reads it.
fn empty_as_null(rendered: &str) -> String {
    format!("NULLIF({rendered}, '')")
}

/// A boolean as the 1 or 0 Exasol reads it as in arithmetic.
fn boolean_number(rendered: &str) -> String {
    format!("CASE WHEN {rendered} THEN 1 WHEN NOT {rendered} THEN 0 END")
}

/// The exact class of an arithmetic operand, a boolean reading as one digit.
fn exact_class(class: ValueClass) -> ValueClass {
    match class {
        ValueClass::Exact { .. } => class,
        _ => ValueClass::integer(1),
    }
}

fn is_positive_integer_literal(operand: &Operand) -> bool {
    matches!(
        &operand.expression,
        Expression::Literal(Literal::ExactNumeric(value))
            if value.parse::<u64>().is_ok_and(|number| number > 0)
    )
}

fn unconvertible(position: &str, class: ValueClass) -> RenderError {
    RenderError::new(format!(
        "{position} over a {class:?} operand has no rendering path in this release"
    ))
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

/// Declares a dialect whose only per-statement decision is which table each
/// column belongs to: its `SqlDialect` impl, its `Statement`, and the
/// `StatementRenderer` constants and accessors every such dialect shares. The
/// braces hold the renderer's hooks. It reads the `JOIN_KEYWORDS` constant and
/// `render_literal` function of the invoking module.
macro_rules! plain_dialect {
    ($dialect:ident, $driver:ident, $target:literal, $sql_name:literal, $double:literal, { $($hooks:tt)* }) => {
        impl $crate::sql_dialect::SqlDialect for $dialect {
            fn capabilities(&self) -> &'static [&'static str] {
                adbc_vs_scan::driver::Driver::$driver.capabilities()
            }

            fn render(
                &self,
                query: &$crate::pushdown_query::Query,
            ) -> Result<String, $crate::sql_dialect::RenderError> {
                render_select(&Statement::of(query), query)
            }
        }

        struct Statement<'a> {
            tables: Tables<'a>,
        }

        impl<'a> Statement<'a> {
            fn of(query: &'a $crate::pushdown_query::Query) -> Self {
                Self {
                    tables: Tables::of(&query.from),
                }
            }
        }

        impl StatementRenderer for Statement<'_> {
            const TARGET: &'static str = $target;
            const SQL_NAME: &'static str = $sql_name;
            const JOIN_KEYWORDS: &'static [(&'static str, &'static str)] = JOIN_KEYWORDS;
            const DOUBLE_TYPE: &'static str = $double;

            fn tables(&self) -> &Tables<'_> {
                &self.tables
            }

            fn render_literal(
                &self,
                literal: &$crate::pushdown_query::Literal,
            ) -> Result<String, $crate::sql_dialect::RenderError> {
                render_literal(literal)
            }

            $($hooks)*
        }
    };
}
pub(crate) use plain_dialect;
