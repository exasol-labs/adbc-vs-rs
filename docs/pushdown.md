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
| `GROUP BY` several columns | `AGGREGATE_GROUP_BY_TUPLE` | ❌ | ❌ | ✅ |
| `COUNT`, `COUNT(*)`, `COUNT(DISTINCT)` | `FN_AGG_COUNT`, `FN_AGG_COUNT_STAR`, `FN_AGG_COUNT_DISTINCT` | ✅ | ✅ | ✅ |
| `MIN`, `MAX` | `FN_AGG_MIN`, `FN_AGG_MAX` | ✅ | ✅ | ✅ |
| `SUM`, `SUM(DISTINCT)` | `FN_AGG_SUM`, `FN_AGG_SUM_DISTINCT` | ✅ | ✅ | ✅ |
| `AVG`, `AVG(DISTINCT)` | `FN_AGG_AVG`, `FN_AGG_AVG_DISTINCT` | ✅ | ✅ | ✅ |
| `HAVING` | `AGGREGATE_HAVING` | ✅ | ✅ | ✅ |
| Inner, left outer, and right outer join on an equality | `JOIN`, `JOIN_TYPE_INNER`, `JOIN_TYPE_LEFT_OUTER`, `JOIN_TYPE_RIGHT_OUTER`, `JOIN_CONDITION_EQUI` | ✅ | ✅ | ✅ |
| Full outer join on an equality | `JOIN_TYPE_FULL_OUTER` | ❌ | ✅ | ✅ |

## What runs in Exasol

Exasol runs these parts for every database. A query with such a part still works, but Exasol fetches more rows.

| Part of a query | Example |
|-----------------|---------|
| `ORDER BY` | `ORDER BY "price"` |
| `LIMIT` and `OFFSET` | `LIMIT 10 OFFSET 5` |
| `GROUP BY` over an expression | `GROUP BY "price" + 0` |
| A join on a condition other than equality | `ON a."x" < b."y"` |
| Scalar functions and arithmetic | `"price" + 0` |

## Floating-point sums follow the source

An exact-numeric `SUM` or `AVG` that runs at the source equals the result that Exasol computes. Over a `DOUBLE` column, and over a character column, which the source and Exasol both convert to `DOUBLE`, the source adds the values in its own order. The result can then differ from the Exasol result in the low-order digits. It can differ further where values cancel, and it can differ between runs. A `NaN` value makes the result null.

To get the Exasol result, wrap the operand in an expression that `adbc-vs-rs` does not send, for example `SUM("price" + 0)`. Exasol then runs the sum. This follows the same rule as [String comparison follows the source](how-it-works.md#string-comparison-follows-the-source).

## Database differences

### MySQL

- MySQL has no `FULL OUTER JOIN`. Exasol runs a full outer join.
- A delegated `AVG`, and a delegated `SUM` over a character column, need MySQL 8.0.17 or later, because the query that `adbc-vs-rs` sends uses `CAST(... AS DOUBLE)`. On an older MySQL, the query fails. To run it there, wrap the operand in an expression that `adbc-vs-rs` does not send, for example `AVG("price" + 0)`, so that Exasol computes the aggregate.
- A `SUM` or `AVG` over a character column reads a value that is not a number as its numeric prefix, or as 0. Exasol fails the query for such a value.

### PostgreSQL

- A `SUM` over a `real` column adds in double precision, so it returns the sum that Exasol computes over the same values as `DOUBLE`. PostgreSQL alone would add a `real` column in single precision.

### Trino

- A `SUM` over a `bigint` or `decimal` column adds in a 38-digit decimal, so a sum beyond the `bigint` range returns its value. Trino alone would stop the query with an overflow error.
- A backslash in a `LIKE` pattern is an ordinary character, unless the query has an `ESCAPE` clause. Exasol then runs that predicate.
