# Feature: pushdown_filter

Delegates a query's `WHERE` clause to the source, rendering each predicate in the source's own SQL so the source selects the rows.

## Background

- This feature owns what a filter fragment is, for every dialect: predicate nesting and grouping, operand order, a `BOOLEAN` column as a predicate on its own, and a select-list predicate rendered by the `WHERE` renderer.
- Each dialect's literal forms and advertised filter capabilities live in its `<technology>-pushdown` feature, and each `<technology>-collation` feature states what its source's collation does to a delegated character comparison.
- A character operand is Exasol `VARCHAR` or Exasol `CHAR`, and Exasol keeps a `CHAR` value blank-padded.
- `vs-adapter/pushdown-dialect` owns how a column operand renders and how comparison operands are read by position.
- `vs-adapter/pushdown-filter-text` owns `LIKE`, `REGEXP_LIKE`, literal forms, and how a delegated character comparison follows the source's collation.

## Scenarios

### Scenario: Numeric comparison is rendered as a WHERE clause

* *GIVEN* a virtual table `orders` with a mapped `DECIMAL` column `id`
* *WHEN* Exasol pushes down a filter comparing `id` to the literal 2, for each dialect this release supports
* *THEN* the generated statement MUST carry a `WHERE` clause comparing the quoted column to the literal
* *AND* the operands MUST appear in the order the request gave them
* *AND* the numeric literal MUST be rendered without quotes and with no collation suffix, because a numeric comparison needs none
* *AND* a `predicate_notequal` MUST render as `<>` in every dialect, because every target accepts that standard form and a dialect gains nothing from spelling a shared form differently

### Scenario: Conjunction, disjunction, and negation nest without losing grouping

* *GIVEN* a filter combining two comparisons under `predicate_or` and negating the result
* *WHEN* the dialect renders that filter
* *THEN* the rendered MySQL clause MUST parenthesize each nested predicate
* *AND* the PostgreSQL and Trino dialects MUST parenthesize each nested predicate the same way, because every target accepts the same standard form
* *AND* the rows MySQL returns MUST equal the rows Exasol returns for the same filter over the same data, where every compared character column declares a code-point collation at the source, as `vs-adapter/mysql-collation` states

### Scenario: IS NULL and IS NOT NULL are rendered without a comparison

* *GIVEN* a nullable mapped column
* *WHEN* Exasol pushes down `predicate_is_null` and `predicate_is_not_null` over it
* *THEN* the rendered clauses MUST be `IS NULL` and `IS NOT NULL`
* *AND* neither MUST be rendered as a comparison against a null literal, because that comparison is never true

### Scenario: BETWEEN and IN keep their operand order

* *GIVEN* a `predicate_between` over a numeric column and a `predicate_in_constlist` over a character column
* *WHEN* each dialect this release supports renders each predicate
* *THEN* the `BETWEEN` bounds MUST appear in the order the request gave them
* *AND* the `IN` list entries MUST appear in the order the request gave them
* *AND* the character `IN` comparison MUST carry no collation suffix, on the compared operand or on the list

### Scenario: Predicate in select-list position renders as a projected boolean

* *GIVEN* a `pushdown` request carrying no `group_by` aggregation, whose select list carries a `predicate_less` over a mapped column and whose `selectListDataTypes` declares that entry `BOOLEAN`
* *WHEN* each dialect this release supports renders the statement
* *THEN* the generated statement MUST project the parenthesized comparison the same renderer produces for a `WHERE` clause, so one renderer serves both positions
* *AND* the `EMITS` clause MUST declare that column `BOOLEAN`
* *AND* the request MUST NOT fail, because the query Exasol delegated returns a boolean column today with Exasol computing it, and Exasol does not re-plan after an adapter error
* *AND* a predicate whose operand is null MUST project a null rather than a false, because Exasol and both targets evaluate a comparison against a null operand to null

### Scenario: BOOLEAN column is a predicate on its own

* *GIVEN* a `pushdown` request whose `filter` is a `column` entry whose declared type is `BOOLEAN`, and requests carrying that entry as an operand of `predicate_and`, `predicate_or`, and `predicate_not`
* *WHEN* each dialect parses and renders each request
* *THEN* the rendered `WHERE` clause MUST carry the column's read expression as the predicate, with no comparison added: the bare quoted column for a PostgreSQL `boolean` column, `(position(B'1' IN <column>) > 0)` for a PostgreSQL `bit` column, and `(<column> <> 0)` for a MySQL `BIT` column
* *AND* for each of those three columns, the rows the source returns MUST equal the rows Exasol returns for the same filter over the same data
* *AND* a `column` entry of any other declared type in predicate position MUST fail with an error naming the entry type and the declared type, rather than render a non-boolean operand as a predicate
* *AND* a comparison whose operand is itself a predicate, such as a `predicate_equal` between a `BOOLEAN` column and a `predicate_less`, MUST render that operand parenthesized through the same renderer rather than fail
