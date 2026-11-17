[Home](index.md) · [Install & Set Up](install.md) · [Create a Virtual Schema](create-virtual-schema.md) · [How Does It Work](how-it-works.md) · [Pushdown](pushdown.md) · [Type Mapping](type-mapping.md)

---

# Pushdown

`adbc-vs-rs` sends the parts of a query that the source can run, and Exasol runs the rest on the returned rows.

## What runs at the source

| Part of a query | Capabilities | MySQL | PostgreSQL | Trino |
|-----------------|--------------|-------|------------|-------|
| Column selection | `SELECTLIST_PROJECTION`, `SELECTLIST_EXPRESSIONS` | ✅ | ✅ | ✅ |
| Filters with comparisons, `IS NULL`, `IS NOT NULL`, `BETWEEN`, `IN` with a list of constants, `LIKE`, `AND`, `OR`, and `NOT` | `FILTER_EXPRESSIONS`, `FN_PRED_EQUAL`, `FN_PRED_NOTEQUAL`, `FN_PRED_LESS`, `FN_PRED_LESSEQUAL`, `FN_PRED_IS_NULL`, `FN_PRED_IS_NOT_NULL`, `FN_PRED_BETWEEN`, `FN_PRED_IN_CONSTLIST`, `FN_PRED_LIKE`, `FN_PRED_AND`, `FN_PRED_OR`, `FN_PRED_NOT` | ✅ | ✅ | ✅ |
| `LIKE` with an `ESCAPE` clause | `FN_PRED_LIKE_ESCAPE` | ✅ | ✅ | ❌ |
| `NULL`, number, string, date, and timestamp literals | `LITERAL_NULL`, `LITERAL_EXACTNUMERIC`, `LITERAL_DOUBLE`, `LITERAL_STRING`, `LITERAL_DATE`, `LITERAL_TIMESTAMP` | ✅ | ✅ | ✅ |
| Aggregation without `GROUP BY` | `AGGREGATE_SINGLE_GROUP` | ✅ | ✅ | ✅ |
| `GROUP BY` one column | `AGGREGATE_GROUP_BY_COLUMN` | ✅ | ✅ | ✅ |
| `GROUP BY` one expression, such as `YEAR("d")` | `AGGREGATE_GROUP_BY_EXPRESSION` | ✅ | ✅ | ✅ |
| `GROUP BY` several columns or expressions | `AGGREGATE_GROUP_BY_TUPLE` | ❌ | ❌ | ✅ |
| `COUNT`, `COUNT(*)`, `COUNT(DISTINCT)` | `FN_AGG_COUNT`, `FN_AGG_COUNT_STAR`, `FN_AGG_COUNT_DISTINCT` | ✅ | ✅ | ✅ |
| `MIN`, `MAX` | `FN_AGG_MIN`, `FN_AGG_MAX` | ✅ | ✅ | ✅ |
| `SUM`, `SUM(DISTINCT)` | `FN_AGG_SUM`, `FN_AGG_SUM_DISTINCT` | ✅ | ✅ | ✅ |
| `AVG`, `AVG(DISTINCT)` | `FN_AGG_AVG`, `FN_AGG_AVG_DISTINCT` | ✅ | ✅ | ✅ |
| `HAVING` | `AGGREGATE_HAVING` | ✅ | ✅ | ✅ |
| Inner, left outer, and right outer join on an equality | `JOIN`, `JOIN_TYPE_INNER`, `JOIN_TYPE_LEFT_OUTER`, `JOIN_TYPE_RIGHT_OUTER`, `JOIN_CONDITION_EQUI` | ✅ | ✅ | ✅ |
| Full outer join on an equality | `JOIN_TYPE_FULL_OUTER` | ❌ | ✅ | ✅ |
| `YEAR`, `MONTH`, `DAY` | `FN_YEAR`, `FN_MONTH`, `FN_DAY` | ✅ | ✅ | ✅ |
| `UPPER`, `LOWER`, `LENGTH`, `SUBSTR` | `FN_UPPER`, `FN_LOWER`, `FN_LENGTH`, `FN_SUBSTR` | ✅ | ✅ | ✅ |
| `TRIM` | `FN_TRIM` | ❌ | ✅ | ✅ |
| `+`, `-`, `*` | `FN_ADD`, `FN_SUB`, `FN_MULT` | ✅ | ✅ | ✅ |

## What runs in Exasol

Exasol runs these parts for every database. A query with such a part still works, but Exasol fetches more rows.

| Part of a query | Example |
|-----------------|---------|
| `ORDER BY` | `ORDER BY "price"` |
| `LIMIT` and `OFFSET` | `LIMIT 10 OFFSET 5` |
| A join on a condition other than equality | `ON a."x" < b."y"` |
| `CAST`, `/`, and every scalar function not listed above | `CAST("price" AS DOUBLE)`, `ROUND("price")` |

## Floating-point sums follow the source

An exact-numeric `SUM` or `AVG` that runs at the source equals the result that Exasol computes. Over a `DOUBLE` column, and over a character column, which the source and Exasol both convert to `DOUBLE`, the source adds the values in its own order. The result can then differ from the Exasol result in the low-order digits. It can differ further where values cancel, and it can differ between runs. A `NaN` value makes the result null.

To get the Exasol result, wrap the operand in an expression that `adbc-vs-rs` does not send, for example `SUM(CAST("price" AS DOUBLE))`. Exasol then runs the sum. `SUM("price" / 1)` does not work, because Exasol removes the division and sends the sum. This follows the same rule as [String comparison follows the source](how-it-works.md#string-comparison-follows-the-source).

## Scalar functions follow Exasol's rules

Each delegated function returns the Exasol result for its operand types:

- A number added to or subtracted from a `DATE` or `TIMESTAMP` counts days. Exasol rounds a fractional number of days half away from zero, so `"d" + 1.5` adds 2 days. `DATE` minus `DATE` is the number of days between them.
- `TIMESTAMP` minus `TIMESTAMP` is an `INTERVAL DAY TO SECOND`. The source sends it as a number of seconds, and Exasol converts it back to an interval. An Exasol interval holds milliseconds, so the source truncates the difference to milliseconds.
- Exact numbers add, subtract, and multiply exactly. A result beyond the `BIGINT` range comes back as a value.
- A number or a date in `LENGTH`, `SUBSTR`, `UPPER`, `LOWER`, or `TRIM` converts to text in Exasol's default format, for example `1.5` for a `DECIMAL(18,2)` value of 1.50 and `2024-01-31` for a date. Text in `YEAR`, `MONTH`, or `DAY` converts as a timestamp in Exasol's default format. The source follows the default `NLS_DATE_FORMAT` and `NLS_TIMESTAMP_FORMAT`, not a format that a session changes.
- An empty result of `UPPER`, `LOWER`, `TRIM`, or `SUBSTR` is null, as in Exasol.
- `SUBSTR` follows Exasol's position rule: a position of 0 reads as 1, and a negative position counts from the end.

Some results follow the source instead:

- `LENGTH` over a `CHAR` value counts the characters the source keeps. MySQL and PostgreSQL drop the padding, so `LENGTH` over `'ab'` in a `CHAR(5)` column returns 2, where Exasol returns 5.
- `UPPER` and `LOWER` use the source's case mapping for non-ASCII letters.
- `LENGTH`, `SUBSTR`, `UPPER`, `LOWER`, and `TRIM` over a `DOUBLE` value use the source's text form of the number, which can differ from Exasol's, for example `1e20`.
- `LENGTH`, `SUBSTR`, `UPPER`, `LOWER`, and `TRIM` over an `INTERVAL` value stop the query with an error.

## Database differences

### MySQL

- MySQL has no `FULL OUTER JOIN`. Exasol runs a full outer join.
- A delegated `AVG`, a delegated `SUM` over a character column, and arithmetic over a `DOUBLE` or character operand need MySQL 8.0.17 or later, because the query that `adbc-vs-rs` sends uses `CAST(... AS DOUBLE)`. On an older MySQL, the query fails. To run it there, wrap the operand in an expression that `adbc-vs-rs` does not send, for example `AVG(CAST("price" AS DOUBLE))`, so that Exasol computes the aggregate.
- MySQL's `TRIM` removes a substring rather than a set of characters, so Exasol runs `TRIM`.
- A character operand of arithmetic reads a value that is not a number as its numeric prefix, or as 0. Exasol fails the query for such a value.
- `YEAR`, `MONTH`, and `DAY` over text that is not a timestamp return null. Exasol fails the query for such a value.
- An expression over a grouped expression, such as `YEAR("d") + 1` with `GROUP BY YEAR("d")`, reads the grouped value through `ANY_VALUE`, because MySQL's `ONLY_FULL_GROUP_BY` mode refuses the expression otherwise.
- A `SUM` or `AVG` over a character column reads a value that is not a number as its numeric prefix, or as 0. Exasol fails the query for such a value.

### PostgreSQL

- A `SUM` over a `real` column adds in double precision, so it returns the sum that Exasol computes over the same values as `DOUBLE`. PostgreSQL alone would add a `real` column in single precision.

### Trino

- A `SUM` over a `bigint` or `decimal` column adds in a 38-digit decimal, so a sum beyond the `bigint` range returns its value. Trino alone would stop the query with an overflow error.
- A backslash in a `LIKE` pattern is an ordinary character, unless the query has an `ESCAPE` clause. Exasol then runs that predicate.
- Trino writes every `DOUBLE` value as text in scientific notation, for example `1.5E0`, so `LENGTH`, `SUBSTR`, `UPPER`, `LOWER`, and `TRIM` over a `DOUBLE` value return a result that differs from Exasol's.
- A product of two exact numbers whose precisions add up to more than 38 digits keeps at least 6 fractional digits, because Trino reduces the scale of such a product. Exasol keeps every fractional digit up to its 36-digit limit.
