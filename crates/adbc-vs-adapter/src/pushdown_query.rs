//! The dialect-neutral query model a `pushdown` request parses into, and the
//! parser that produces it.
//!
//! The model speaks Exasol's protocol vocabulary — a select list, a `from`
//! tree — and carries no target-SQL syntax, so every
//! [`SqlDialect`](crate::sql_dialect::SqlDialect) implementation is a pure
//! consumer: a second driver renders the same model rather than reshaping it.
//!
//! Two protocol facts are absorbed here rather than left for each dialect to
//! rediscover. `selectList` is three-way — absent, an empty array, or
//! populated — and the parser resolves all three into one non-empty list of
//! positional items, so a renderer has a single case to handle. And the
//! declared Exasol type of each item travels with the item rather than in a
//! parallel array: Exasol compares those types against the virtual table's
//! stored metadata for exact equality, so an index drifting between two
//! collections would be a hard runtime failure. The type is an Exasol
//! question, never a dialect one, which is why it belongs to this model.
//!
//! A `from` element carrying two or more tables names each column it reaches
//! by a `tableName` and, when the operator's query aliased the table, a
//! `tableAlias` as well, because two joined tables may declare a column of
//! the same name. Both spellings travel with the [`ColumnRef`] verbatim
//! rather than being resolved against the `from` tree's own aliases here:
//! which alias each table in that tree renders under is a rendering decision
//! a dialect makes, described in `vs-adapter/pushdown-join`. Resolving a
//! column's declared type still happens here, against every table the
//! request names as involved, because Exasol's type-equality check needs it
//! whether or not the reference later resolves to a rendered alias.

use serde_json::Value;

use crate::emit_type;
use crate::exa_type::{self, ExasolType};

/// The query fragment Exasol delegated, in the adapter's own vocabulary.
#[derive(Debug, Clone, PartialEq)]
pub struct Query {
    /// One item per emitted column, in the order the request gave them.
    /// Never empty: an empty `selectList` parses into a single
    /// [`Expression::RowMarker`], because a statement projecting no column
    /// is not valid SQL and would lose the row count Exasol asked for.
    pub select_list: Vec<SelectItem>,
    /// The `WHERE` clause Exasol delegated, from `pushdownRequest.filter`.
    /// `None` when the request carries no `filter` element, which is not an
    /// error: a query with no `WHERE` clause is ordinary.
    pub filter: Option<Predicate>,
    /// The aggregation Exasol delegated, from `pushdownRequest.aggregationType`
    /// and `pushdownRequest.groupBy`. `None` when the request carries no
    /// `aggregationType` element, which is not an error: a query with no
    /// aggregation is ordinary.
    pub aggregation: Option<Aggregation>,
    pub from: FromClause,
}

/// The query's aggregation, from `pushdownRequest.aggregationType` and, for
/// `group_by`, `pushdownRequest.groupBy`.
///
/// This release groups by columns only: `AGGREGATE_GROUP_BY_EXPRESSION` and
/// `AGGREGATE_GROUP_BY_TUPLE` are not advertised, so Exasol never sends a
/// `groupBy` entry that is not a `column`. Withholding `AGGREGATE_GROUP_BY_TUPLE`
/// caps a delegated `GROUP BY` at exactly one column rather than only barring a
/// tuple-valued grouping expression, as its name suggests: measured on Exasol
/// 2025.2.1, `AGGREGATE_GROUP_BY_COLUMN` advertised alone withdraws a
/// two-column `GROUP BY` whole, degrading the select list to plain columns
/// with no aggregation at all, rather than delegating it with one column
/// missing. `GroupBy` therefore never carries more than one [`ColumnRef`] in
/// this release, though nothing in this type enforces that — the cap is a
/// capability boundary the dialect's advertised set holds, not a limit this
/// model's shape encodes.
#[derive(Debug, Clone, PartialEq)]
pub enum Aggregation {
    /// The query aggregates its whole result into one row; the generated
    /// statement carries no `GROUP BY` clause.
    SingleGroup,
    /// The query groups by the carried columns, in the order `groupBy`
    /// gave them.
    GroupBy(Vec<ColumnRef>),
}

/// One emitted column: the expression that produces it and the Exasol type
/// the `EMITS` clause must declare for it.
#[derive(Debug, Clone, PartialEq)]
pub struct SelectItem {
    pub expression: Expression,
    pub declared_type: ExasolType,
}

/// A value the generated statement projects, or a filter predicate reads as
/// an operand.
#[derive(Debug, Clone, PartialEq)]
pub enum Expression {
    Column(ColumnRef),
    /// The placeholder an empty `selectList` projects. Exasol sends an empty
    /// select list when only the row count matters, and the statement must
    /// still project one value per source row for that count to survive. A
    /// dialect MUST render it as a single-digit integer constant, because
    /// the item's declared type is `DECIMAL(1,0)`.
    RowMarker,
    Literal(Literal),
    /// A `function_aggregate` node: an aggregate applied over the current
    /// group. Reachable as a bare select-list entry and as the operand of a
    /// select-list [`Predicate`], the shape `COUNT(*) > 0` arrives in.
    Aggregate(AggregateCall),
    /// A `predicate_*` node in select-list position, as `SELECTLIST_EXPRESSIONS`
    /// lets a delegated predicate reach the select list rather than only
    /// `WHERE`. Declared `BOOLEAN`.
    Predicate(Box<Predicate>),
}

/// A reference to a column of a table in the `from` tree, carrying the
/// Exasol type that table declares for it.
///
/// The type travels with the reference because a dialect's rendering of a
/// column depends on it: Exasol compares character values case-sensitively,
/// accent-sensitively, and without padding, and reproducing that at the
/// source takes an explicit collation a numeric or temporal column must not
/// carry. Resolving the type again by name at render time would put the
/// involved table's column list on both sides of the dialect boundary, and a
/// renderer that guessed instead — reading a character comparison off the
/// presence of a string literal — would render `note = code` bare and return
/// rows Exasol never asked for.
///
/// How the reference names its table travels unresolved, in both spellings
/// the protocol uses. Measured on Exasol 2025.2.1, a join whose query aliased
/// no table sends `tableName` alone and a join whose query aliased one sends
/// both, so neither field alone identifies the table across the shapes
/// Exasol sends. Which of the two resolves against the `from` tree, and which
/// alias each table there renders under, are rendering decisions
/// `vs-adapter/pushdown-join`'s dialect makes: resolving them here would
/// settle the generated statement's alias namespace in the one module that
/// has no target SQL in it. Resolving a column's declared type still happens
/// here, against every table the request names as involved, because Exasol's
/// type-equality check needs it whether or not the reference later resolves
/// to a rendered alias.
#[derive(Debug, Clone, PartialEq)]
pub struct ColumnRef {
    pub name: String,
    pub declared_type: ExasolType,
    /// The column node's own `tableName`, when the request supplied one: the
    /// real name of the table the column belongs to, the same name
    /// `involvedTables[].name` carries.
    pub table_name: Option<String>,
    /// The column node's own `tableAlias`, when the request supplied one.
    pub table_alias: Option<String>,
}

/// A `function_aggregate` node: an aggregate function applied over the
/// current group, over at most one argument.
///
/// `function` is normalized to upper case at parse time. Measured on Exasol
/// 2025.2.1, one request carried `sum`, `min`, and `max` in lower case
/// beside `AVG` and `COUNT` in upper case, so two requests differing only in
/// case parse into the identical model rather than comparing unequal
/// downstream. Which function names a dialect can render is the dialect's
/// own question, not this model's: a name the dialect does not render fails
/// at render time, naming the function it received.
#[derive(Debug, Clone, PartialEq)]
pub struct AggregateCall {
    pub function: String,
    /// `None` for `COUNT(*)`, which arrives carrying no `arguments`; `Some`
    /// otherwise. This release's aggregate functions each take at most one
    /// argument.
    pub argument: Option<Box<Expression>>,
    /// `true` when the `function_aggregate` node carries `"distinct": true`;
    /// `false` when the field is absent.
    pub distinct: bool,
}

/// A constant value a filter predicate compares against, carried as the raw
/// string Exasol's `value` field supplies. Never parsed into a Rust number
/// or date: a dialect renders each kind through its own typed-literal or
/// hex-encoding rule, and a value this model does not need to interpret is
/// one it cannot get wrong by mis-parsing.
///
/// Exactly the six literal kinds `vs-adapter/pushdown-filter`'s advertised
/// `LITERAL_*` capabilities cover; `literal_bool` and `literal_interval` have
/// no variant because this release advertises neither capability.
#[derive(Debug, Clone, PartialEq)]
pub enum Literal {
    Null,
    ExactNumeric(String),
    Double(String),
    String(String),
    Date(String),
    Timestamp(String),
}

/// The `WHERE` clause Exasol delegates, in the adapter's own vocabulary.
/// Mirrors the protocol's `predicate_*` node kinds one-to-one so a dialect
/// renders the tree without rediscovering the JSON shape.
#[derive(Debug, Clone, PartialEq)]
pub enum Predicate {
    And(Vec<Predicate>),
    Or(Vec<Predicate>),
    Not(Box<Predicate>),
    Equal(Comparison),
    NotEqual(Comparison),
    Less(Comparison),
    LessEqual(Comparison),
    IsNull(Expression),
    IsNotNull(Expression),
    Like(LikePredicate),
    Between(BetweenPredicate),
    InConstList(InConstListPredicate),
}

/// The two operands of a binary comparison, read by position and never
/// reordered: Exasol normalizes `>` and `>=` into `predicate_less` and
/// `predicate_lessequal` with the operands swapped, so `left` is not
/// reliably the column.
#[derive(Debug, Clone, PartialEq)]
pub struct Comparison {
    pub left: Expression,
    pub right: Expression,
}

/// A `predicate_like`: `expression` matched against `pattern`, with an
/// optional `escapeChar` naming the pattern's escape character.
#[derive(Debug, Clone, PartialEq)]
pub struct LikePredicate {
    pub expression: Expression,
    pub pattern: Expression,
    pub escape_char: Option<Expression>,
}

/// A `predicate_between`: `expression` bounded by `left` and `right`, kept
/// in the request's own order.
#[derive(Debug, Clone, PartialEq)]
pub struct BetweenPredicate {
    pub expression: Expression,
    pub left: Expression,
    pub right: Expression,
}

/// A `predicate_in_constlist`: `expression` matched against `arguments`,
/// kept in the request's own order.
#[derive(Debug, Clone, PartialEq)]
pub struct InConstListPredicate {
    pub expression: Expression,
    pub arguments: Vec<Expression>,
}

/// The tables the query reads from: a single table, or a join of two further
/// `from`-tree elements.
///
/// [`FromClause::Join`] is boxed because a [`Join`] carries a whole condition
/// tree and two nested `from` elements, several times the size of a
/// [`SourceTable`], and every single-table query — the shape release 0.1
/// served alone — would otherwise pay for it.
#[derive(Debug, Clone, PartialEq)]
pub enum FromClause {
    Table(SourceTable),
    Join(Box<Join>),
}

/// A table at the source, qualified by the database `createVirtualSchema`
/// recorded in `adapterNotes`, and carrying the alias the request assigned
/// it, when it assigned one.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceTable {
    pub schema: String,
    pub name: String,
    pub alias: Option<String>,
}

/// A join of two `from`-tree elements, read from a `from` element whose
/// `type` is `join`.
///
/// `join_type` is kept as the protocol's own snake-case spelling — `inner`,
/// `left_outer`, `right_outer`, `full_outer`, or any value a future Exasol
/// release sends — rather than parsed into a closed set here. Which of these
/// a dialect renders is a capability the dialect advertises
/// (`JOIN_TYPE_INNER`, `JOIN_TYPE_LEFT_OUTER`, `JOIN_TYPE_RIGHT_OUTER`), the
/// same question [`AggregateCall::function`] answers at render time rather
/// than here, so a value this release does not render fails naming the
/// value it received rather than failing here for every release to come.
#[derive(Debug, Clone, PartialEq)]
pub struct Join {
    pub join_type: String,
    pub left: Box<FromClause>,
    pub right: Box<FromClause>,
    /// The join's `condition`, read through the same predicate parser a
    /// `filter` reads through. This release advertises `JOIN_CONDITION_EQUI`
    /// alone, so Exasol sends an equality here, but which predicate shapes a
    /// dialect accepts as a join condition is a rendering question, not a
    /// parsing one.
    pub condition: Predicate,
}

/// Parses a `pushdown` request into the query model.
///
/// `database` is the source database `createVirtualSchema` recorded in
/// `adapterNotes`; a pushdown request never carries it, and the model needs
/// it to name the source table in full.
pub fn parse(request: &Value, database: &str) -> Result<Query, String> {
    reject_undelegated_elements(request)?;
    let tables = read_involved_tables(request)?;
    Ok(Query {
        select_list: read_select_list(request, &tables)?,
        filter: read_filter(request, &tables)?,
        aggregation: read_aggregation(request, &tables)?,
        from: read_from(request, &tables, database)?,
    })
}

/// The `pushdownRequest` elements this release refuses, each named beside the
/// capabilities whose withholding is the reason it refuses them.
///
/// `orderBy` and `limit` stay in Exasol because `vs-adapter/result-ordering`
/// withholds their capabilities: a script-emitted derived table carries no
/// ordering guarantee once an operator sits above the scan. `having` stays for
/// the reason decision `[10]` withholds `AGGREGATE_HAVING`: Exasol applies the
/// clause itself at no correctness cost, and no renderer here writes it.
///
/// All three are the same case. Exasol sends an element only when the matching
/// capability was advertised, so an element arriving here means the advertised
/// set and this list have drifted apart, and reading past it would return a
/// statement Exasol believes carries the clause — the wrong-rows failure mode
/// this codebase never accepts. The error names the capabilities rather than
/// the clause alone, because that set is the one place a change would have to
/// be made for the element to be legal.
const UNDELEGATED_ELEMENTS: [(&str, &str); 3] = [
    ("orderBy", "`ORDER_BY_COLUMN` and `ORDER_BY_EXPRESSION`"),
    ("limit", "`LIMIT` and `LIMIT_WITH_OFFSET`"),
    ("having", "`AGGREGATE_HAVING`"),
];

fn reject_undelegated_elements(request: &Value) -> Result<(), String> {
    for (element, capabilities) in UNDELEGATED_ELEMENTS {
        if request
            .pointer(&format!("/pushdownRequest/{element}"))
            .is_some()
        {
            return Err(format!(
                "the pushdown request carries the `{element}` element, which this release does \
                 not delegate: {capabilities} are withheld, so Exasol applies that clause itself \
                 and a delegated element arriving here would mean dropping a fragment Exasol \
                 believes was applied"
            ));
        }
    }
    Ok(())
}

/// The declared type of the constant an empty `selectList` projects, and
/// therefore the width of the integer constant a dialect renders for
/// [`Expression::RowMarker`].
const ROW_MARKER_TYPE: ExasolType = ExasolType::Decimal {
    precision: 1,
    scale: 0,
};

struct InvolvedTable {
    name: String,
    columns: Vec<InvolvedColumn>,
}

struct InvolvedColumn {
    name: String,
    declared_type: ExasolType,
}

/// Reads every entry of `involvedTables`: one table for a single-table
/// request, two or more once a join's `from` element names more than one.
fn read_involved_tables(request: &Value) -> Result<Vec<InvolvedTable>, String> {
    let entries = request
        .pointer("/involvedTables")
        .and_then(Value::as_array)
        .filter(|entries| !entries.is_empty())
        .ok_or_else(|| "the pushdown request names no involved table".to_string())?;

    entries.iter().map(read_involved_table).collect()
}

fn read_involved_table(table: &Value) -> Result<InvolvedTable, String> {
    let name = table
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| "an involved table has no name".to_string())?;
    let columns = table
        .get("columns")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            format!("the pushdown request's involved table `{name}` names no columns")
        })?;
    if columns.is_empty() {
        return Err(format!(
            "the pushdown request's involved table `{name}` declares no column, so the \
             pushed-down statement would project nothing"
        ));
    }

    Ok(InvolvedTable {
        name: name.to_string(),
        columns: columns
            .iter()
            .map(read_involved_column)
            .collect::<Result<Vec<_>, _>>()?,
    })
}

fn read_involved_column(column: &Value) -> Result<InvolvedColumn, String> {
    let name = column
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| "an involved table column has no name".to_string())?;
    let data_type = column
        .get("dataType")
        .ok_or_else(|| format!("involved table column `{name}` has no dataType"))?;

    Ok(InvolvedColumn {
        name: name.to_string(),
        declared_type: exa_type::from_json(data_type)?,
    })
}

/// Resolves a `column` node's declared type against `tables`, using
/// `table_name` to pick the table when the request supplied one.
///
/// `table_name` is the column node's own `tableName` field, which names an
/// involved table by its real name — the same name `involvedTables[].name`
/// carries — unlike `tableAlias`, which names whatever alias the `from` tree
/// assigns and may differ from it once one is explicit. Resolving the
/// declared type is therefore keyed on `tableName`, the identifier this
/// parser can already match against `tables`; `tableAlias` travels with the
/// built [`ColumnRef`] unresolved, because matching it needs the alias each
/// table in the `from` tree renders under, a rendering decision
/// `vs-adapter/pushdown-join`'s dialect makes.
///
/// A `table_name` that names none of `tables` is not a parse failure: it may
/// still name a real table once matched against the `from` tree's own
/// structure, which only a dialect can do, so resolution here falls back to
/// searching every table by column name alone. That fallback is what a
/// single-table request always takes, since it names no join and carries no
/// `tableName` at all in this release's fixtures. A name two tables both
/// declare, with no usable qualifier resolving the tie, is genuinely
/// unresolvable here and fails naming the column; `vs-adapter/pushdown-join`
/// observes that Exasol always qualifies a join's column references, so
/// this path is a safety net rather than the common case.
fn find_involved_column<'a>(
    tables: &'a [InvolvedTable],
    table_name: Option<&str>,
    name: &str,
    context: &str,
) -> Result<&'a InvolvedColumn, String> {
    if let Some(table_name) = table_name {
        if let Some(table) = tables.iter().find(|table| table.name == table_name) {
            return table
                .columns
                .iter()
                .find(|column| column.name == name)
                .ok_or_else(|| {
                    format!(
                        "{context} names column `{name}` of table `{table_name}`, which that \
                     involved table does not declare"
                    )
                });
        }
    }

    let mut matches = tables
        .iter()
        .filter_map(|table| table.columns.iter().find(|column| column.name == name));
    let first = matches.next().ok_or_else(|| {
        format!("{context} names column `{name}`, which no involved table declares")
    })?;
    if matches.next().is_some() {
        return Err(format!(
            "{context} names column `{name}`, which more than one involved table declares; \
             qualify it with a `tableName`"
        ));
    }
    Ok(first)
}

/// Reads a `column` node's `name`, optional `tableName`, and optional
/// `tableAlias`, and resolves its declared type against `tables`. Shared by
/// every site a `column` node can appear — a select-list entry, a `groupBy`
/// entry, and a filter or join condition operand — so the node's shape is
/// read in one place rather than rediscovered at each site.
fn read_column_ref(
    node: &Value,
    tables: &[InvolvedTable],
    context: &str,
) -> Result<ColumnRef, String> {
    let name = node
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{context} {node} names no column"))?;
    let table_name = node.get("tableName").and_then(Value::as_str);
    let table_alias = node.get("tableAlias").and_then(Value::as_str);
    let column = find_involved_column(tables, table_name, name, context)?;

    Ok(ColumnRef {
        name: name.to_string(),
        declared_type: column.declared_type,
        table_name: table_name.map(str::to_string),
        table_alias: table_alias.map(str::to_string),
    })
}

fn read_select_list(request: &Value, tables: &[InvolvedTable]) -> Result<Vec<SelectItem>, String> {
    let Some(select_list) = request.pointer("/pushdownRequest/selectList") else {
        return Ok(tables
            .iter()
            .flat_map(|table| table.columns.iter().map(project_column))
            .collect());
    };
    let entries = select_list
        .as_array()
        .ok_or_else(|| "pushdownRequest.selectList is present but is not an array".to_string())?;
    if entries.is_empty() {
        return Ok(vec![SelectItem {
            expression: Expression::RowMarker,
            declared_type: ROW_MARKER_TYPE,
        }]);
    }

    let declared_types = emit_type::read_select_list_data_types(
        request.pointer("/pushdownRequest").unwrap_or(&Value::Null),
        entries.len(),
    )?;

    entries
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let declared_type = declared_types.as_ref().map(|types| types[index]);
            read_select_item(entry, tables, declared_type)
        })
        .collect()
}

/// Reads one `selectList` entry, keyed on its `type` field: once
/// `SELECTLIST_EXPRESSIONS` is advertised a select list is no longer
/// columns only, so an entry MUST be recognized by its declared kind rather
/// than by which other fields it happens to carry.
///
/// `declared_type` is the type `selectListDataTypes` supplied for this
/// entry's position, if the request carried the field at all; a `column`
/// entry falls back to the involved column's own type when it is absent. A
/// `function_aggregate` or `predicate_*` entry has no such fallback — its
/// declared type comes from `selectListDataTypes` alone, because it has no
/// source column to copy.
fn read_select_item(
    entry: &Value,
    tables: &[InvolvedTable],
    declared_type: Option<ExasolType>,
) -> Result<SelectItem, String> {
    let entry_type = entry
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("selectList entry {entry} carries no `type` field"))?;

    match entry_type {
        "column" => read_column_select_item(entry, tables, declared_type),
        "function_aggregate" => {
            let declared_type = require_declared_type(entry_type, declared_type)?;
            Ok(SelectItem {
                expression: Expression::Aggregate(read_aggregate_call(entry, tables)?),
                declared_type,
            })
        }
        _ if entry_type.starts_with("predicate_") => {
            let declared_type = require_declared_type(entry_type, declared_type)?;
            Ok(SelectItem {
                expression: Expression::Predicate(Box::new(read_predicate(entry, tables)?)),
                declared_type,
            })
        }
        other => Err(match declared_type {
            Some(_) => {
                format!("selectList entry of type `{other}` has no rendering path in this release")
            }
            None => emit_type::missing_declared_type(other),
        }),
    }
}

fn require_declared_type(
    entry_type: &str,
    declared_type: Option<ExasolType>,
) -> Result<ExasolType, String> {
    declared_type.ok_or_else(|| emit_type::missing_declared_type(entry_type))
}

fn read_column_select_item(
    entry: &Value,
    tables: &[InvolvedTable],
    declared_type: Option<ExasolType>,
) -> Result<SelectItem, String> {
    let column_ref = read_column_ref(entry, tables, "selectList entry")?;
    let fallback_type = column_ref.declared_type;

    Ok(SelectItem {
        expression: Expression::Column(column_ref),
        declared_type: declared_type.unwrap_or(fallback_type),
    })
}

/// Reads a `pushdownRequest.aggregationType` and, for `group_by`, its
/// `groupBy` columns. `None` when the request carries no `aggregationType`
/// element, which is not an error: a query with no aggregation is ordinary.
fn read_aggregation(
    request: &Value,
    tables: &[InvolvedTable],
) -> Result<Option<Aggregation>, String> {
    let Some(aggregation_type) = request
        .pointer("/pushdownRequest/aggregationType")
        .and_then(Value::as_str)
    else {
        return Ok(None);
    };

    match aggregation_type {
        "single_group" => Ok(Some(Aggregation::SingleGroup)),
        "group_by" => {
            let entries = request
                .pointer("/pushdownRequest/groupBy")
                .and_then(Value::as_array)
                .ok_or_else(|| {
                    "pushdownRequest.aggregationType is `group_by` but pushdownRequest.groupBy \
                     is absent or is not an array"
                        .to_string()
                })?;
            let columns = entries
                .iter()
                .map(|entry| read_group_by_column(entry, tables))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Some(Aggregation::GroupBy(columns)))
        }
        other => Err(format!(
            "pushdownRequest.aggregationType `{other}` is not recognized"
        )),
    }
}

/// Reads one `groupBy` entry. This release groups by columns only —
/// `AGGREGATE_GROUP_BY_EXPRESSION` and `AGGREGATE_GROUP_BY_TUPLE` are not
/// advertised — so an entry whose `type` is not `column` has no rendering
/// path.
fn read_group_by_column(entry: &Value, tables: &[InvolvedTable]) -> Result<ColumnRef, String> {
    let entry_type = entry
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("groupBy entry {entry} carries no `type` field"))?;
    if entry_type != "column" {
        return Err(format!(
            "groupBy entry of type `{entry_type}` has no rendering path in this release"
        ));
    }

    read_column_ref(entry, tables, "groupBy entry")
}

/// Reads a `function_aggregate` node: its function name, normalized to upper
/// case so two requests differing only in case parse into the identical
/// model, its optional single argument, and its optional `distinct` flag.
///
/// This release's aggregate functions each take at most one argument, so
/// more than one `arguments` entry has no rendering path; `COUNT(*)` is the
/// zero-argument case, told apart from `COUNT(column)` by the argument's
/// absence.
fn read_aggregate_call(entry: &Value, tables: &[InvolvedTable]) -> Result<AggregateCall, String> {
    let name = entry
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("function_aggregate entry {entry} names no function"))?;

    let arguments = entry
        .get("arguments")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let argument = match arguments {
        [] => None,
        [single] => Some(Box::new(read_expression(single, tables)?)),
        _ => {
            return Err(format!(
                "function_aggregate entry {entry} carries more than one argument, which this \
                 release does not render"
            ));
        }
    };

    let distinct = entry
        .get("distinct")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    Ok(AggregateCall {
        function: name.to_uppercase(),
        argument,
        distinct,
    })
}

/// Projects one involved column, for the request that carries no
/// `selectList` at all.
///
/// The reference names no table, because the request named none: this
/// fallback stands in for a `selectList` Exasol did not send, and inventing a
/// qualifier the request never supplied would be this parser deciding which
/// table a column belongs to. Measured on Exasol 2025.2.1, a join always
/// arrives carrying a `selectList` — `SELECT *` over one sends every column
/// qualified — so a join never reaches here, and a single-table statement
/// qualifies nothing.
fn project_column(column: &InvolvedColumn) -> SelectItem {
    SelectItem {
        expression: Expression::Column(ColumnRef {
            name: column.name.clone(),
            declared_type: column.declared_type,
            table_name: None,
            table_alias: None,
        }),
        declared_type: column.declared_type,
    }
}

fn read_filter(request: &Value, tables: &[InvolvedTable]) -> Result<Option<Predicate>, String> {
    match request.pointer("/pushdownRequest/filter") {
        Some(filter) => read_predicate(filter, tables).map(Some),
        None => Ok(None),
    }
}

/// Reads one `predicate_*` node, keyed on its `type` field like a
/// `selectList` entry: the field states the node's kind, and every other
/// field is read only once that kind is known.
fn read_predicate(node: &Value, tables: &[InvolvedTable]) -> Result<Predicate, String> {
    let predicate_type = node
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("filter predicate {node} carries no `type` field"))?;

    match predicate_type {
        "predicate_and" => Ok(Predicate::And(read_predicate_list(node, tables)?)),
        "predicate_or" => Ok(Predicate::Or(read_predicate_list(node, tables)?)),
        "predicate_not" => Ok(Predicate::Not(Box::new(read_nested_predicate(
            node,
            "expression",
            tables,
        )?))),
        "predicate_equal" => Ok(Predicate::Equal(read_comparison(node, tables)?)),
        "predicate_notequal" => Ok(Predicate::NotEqual(read_comparison(node, tables)?)),
        "predicate_less" => Ok(Predicate::Less(read_comparison(node, tables)?)),
        "predicate_lessequal" => Ok(Predicate::LessEqual(read_comparison(node, tables)?)),
        "predicate_is_null" => Ok(Predicate::IsNull(read_named_expression(
            node,
            "expression",
            tables,
        )?)),
        "predicate_is_not_null" => Ok(Predicate::IsNotNull(read_named_expression(
            node,
            "expression",
            tables,
        )?)),
        "predicate_like" => Ok(Predicate::Like(read_like(node, tables)?)),
        "predicate_between" => Ok(Predicate::Between(read_between(node, tables)?)),
        "predicate_in_constlist" => Ok(Predicate::InConstList(read_in_constlist(node, tables)?)),
        other => Err(format!(
            "filter predicate of type `{other}` has no rendering path in this release"
        )),
    }
}

fn read_predicate_list(node: &Value, tables: &[InvolvedTable]) -> Result<Vec<Predicate>, String> {
    let entries = node
        .get("expressions")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("filter predicate {node} carries no `expressions` array"))?;

    entries
        .iter()
        .map(|entry| read_predicate(entry, tables))
        .collect()
}

fn read_nested_predicate(
    node: &Value,
    field: &str,
    tables: &[InvolvedTable],
) -> Result<Predicate, String> {
    let inner = node
        .get(field)
        .ok_or_else(|| format!("filter predicate {node} carries no `{field}` field"))?;
    read_predicate(inner, tables)
}

fn read_comparison(node: &Value, tables: &[InvolvedTable]) -> Result<Comparison, String> {
    Ok(Comparison {
        left: read_named_expression(node, "left", tables)?,
        right: read_named_expression(node, "right", tables)?,
    })
}

fn read_like(node: &Value, tables: &[InvolvedTable]) -> Result<LikePredicate, String> {
    Ok(LikePredicate {
        expression: read_named_expression(node, "expression", tables)?,
        pattern: read_named_expression(node, "pattern", tables)?,
        escape_char: node
            .get("escapeChar")
            .map(|inner| read_expression(inner, tables))
            .transpose()?,
    })
}

fn read_between(node: &Value, tables: &[InvolvedTable]) -> Result<BetweenPredicate, String> {
    Ok(BetweenPredicate {
        expression: read_named_expression(node, "expression", tables)?,
        left: read_named_expression(node, "left", tables)?,
        right: read_named_expression(node, "right", tables)?,
    })
}

fn read_in_constlist(
    node: &Value,
    tables: &[InvolvedTable],
) -> Result<InConstListPredicate, String> {
    let expression = read_named_expression(node, "expression", tables)?;
    let arguments = node
        .get("arguments")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("filter predicate {node} carries no `arguments` array"))?
        .iter()
        .map(|entry| read_expression(entry, tables))
        .collect::<Result<Vec<_>, _>>()?;

    Ok(InConstListPredicate {
        expression,
        arguments,
    })
}

fn read_named_expression(
    node: &Value,
    field: &str,
    tables: &[InvolvedTable],
) -> Result<Expression, String> {
    let inner = node
        .get(field)
        .ok_or_else(|| format!("filter predicate {node} carries no `{field}` field"))?;
    read_expression(inner, tables)
}

/// Reads one value expression: a `column` reference resolved against the
/// involved tables, one of the six literal kinds this release delegates, or
/// a `function_aggregate`, the shape a select-list predicate's operand takes
/// for `COUNT(*) > 0`. Keyed on `type` for the same reason a `selectList`
/// entry is: an operand is not always a column once a predicate can also
/// compare a literal, or an aggregate, against another operand.
fn read_expression(node: &Value, tables: &[InvolvedTable]) -> Result<Expression, String> {
    let expression_type = node
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("filter expression {node} carries no `type` field"))?;

    match expression_type {
        "column" => read_column_ref(node, tables, "filter expression").map(Expression::Column),
        "literal_null" => Ok(Expression::Literal(Literal::Null)),
        "literal_exactnumeric" => Ok(Expression::Literal(Literal::ExactNumeric(
            read_literal_value(node)?,
        ))),
        "literal_double" => Ok(Expression::Literal(Literal::Double(read_literal_value(
            node,
        )?))),
        "literal_string" => Ok(Expression::Literal(Literal::String(read_literal_value(
            node,
        )?))),
        "literal_date" => Ok(Expression::Literal(Literal::Date(read_literal_value(
            node,
        )?))),
        "literal_timestamp" => Ok(Expression::Literal(Literal::Timestamp(read_literal_value(
            node,
        )?))),
        "function_aggregate" => Ok(Expression::Aggregate(read_aggregate_call(node, tables)?)),
        other => Err(format!(
            "filter expression of type `{other}` has no rendering path in this release"
        )),
    }
}

fn read_literal_value(node: &Value) -> Result<String, String> {
    node.get("value")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("literal {node} carries no `value` field"))
}

fn read_from(
    request: &Value,
    tables: &[InvolvedTable],
    database: &str,
) -> Result<FromClause, String> {
    let from = request
        .pointer("/pushdownRequest/from")
        .ok_or_else(|| "the pushdown request carries no `from` element".to_string())?;
    read_from_element(from, tables, database)
}

/// Reads one `from`-tree element, recursively: a `table` leaf, or a `join`
/// whose `left` and `right` are each themselves a `from`-tree element.
///
/// Keyed on `type` like every other protocol node this parser reads, and,
/// unlike `join_type`, validated eagerly: `table` and `join` are the only
/// two node shapes this parser can even walk, so an unrecognized one cannot
/// be deferred to a dialect the way an unrenderable `join_type` value is.
fn read_from_element(
    node: &Value,
    tables: &[InvolvedTable],
    database: &str,
) -> Result<FromClause, String> {
    let element_type = node.get("type").and_then(Value::as_str).ok_or_else(|| {
        "the pushdown request's `from` element carries no `type` field".to_string()
    })?;

    match element_type {
        "table" => {
            let name = node.get("name").and_then(Value::as_str).ok_or_else(|| {
                "the pushdown request's `from` element names no table".to_string()
            })?;
            let alias = node
                .get("alias")
                .and_then(Value::as_str)
                .map(str::to_string);

            Ok(FromClause::Table(SourceTable {
                schema: database.to_string(),
                name: name.to_string(),
                alias,
            }))
        }
        "join" => {
            let join_type = node
                .get("join_type")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    "the pushdown request's `join` element carries no `join_type` field".to_string()
                })?;
            let left = node.get("left").ok_or_else(|| {
                "the pushdown request's `join` element carries no `left` element".to_string()
            })?;
            let right = node.get("right").ok_or_else(|| {
                "the pushdown request's `join` element carries no `right` element".to_string()
            })?;
            let condition = node.get("condition").ok_or_else(|| {
                "the pushdown request's `join` element carries no `condition` element".to_string()
            })?;

            Ok(FromClause::Join(Box::new(Join {
                join_type: join_type.to_string(),
                left: Box::new(read_from_element(left, tables, database)?),
                right: Box::new(read_from_element(right, tables, database)?),
                condition: read_predicate(condition, tables)?,
            })))
        }
        other => Err(format!(
            "the pushdown request's `from` element of type `{other}` has no rendering path in \
             this release"
        )),
    }
}

#[cfg(test)]
#[path = "pushdown_query_tests.rs"]
mod tests;
