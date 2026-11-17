# Tasks: add-pushdown-scalar-functions

- [x] 1. `exa_type`/`emit_type`: a `selectListDataTypes` entry `INTERVAL DAY TO SECOND` reads as a day-to-second interval; `INTERVAL YEAR TO MONTH` still fails (pushdown-declared-types)
- [x] 2. `pushdown_query`: parse `function_scalar` nodes into scalar calls with a derived value class, resolve `ADD`/`SUB`/`MULT` per operand class, read `groupBy` entries as expressions, fail an unknown function by name (pushdown-scalar-function, pushdown-aggregation)
- [x] 3. `dialect_common`: render every scalar call, grouping expressions, and the interval-seconds select-list entry through per-dialect hooks; empty string results as null; `SUBSTR` on Exasol's rule (pushdown-scalar-function)
- [x] 4. MySQL forms: `CHAR_LENGTH`, `DATE_ADD`/`DATEDIFF`/`TIMESTAMPDIFF(MICROSECOND)`, `DECIMAL(65,s)`, `TRIM` refused, `ANY_VALUE` over expressions containing a grouped expression key (mysql-pushdown, pushdown-aggregation-delegation)
- [x] 5. PostgreSQL forms: `EXTRACT`, `char_length`, `btrim`, `numeric`, day intervals, `EXTRACT(EPOCH ...)` (postgresql-pushdown)
- [x] 6. Trino forms: `year`/`month`/`day`, `length`, `trim`, `DECIMAL(38,s)`, `date_add`, `date_diff`, `to_milliseconds` (trino-pushdown)
- [x] 7. Capabilities: add the scalar-function entries and `AGGREGATE_GROUP_BY_EXPRESSION` to each driver TOML; exact-set tests and the render-every-capability invariant (pushdown-scalar-function, mysql/postgresql/trino-pushdown)
- [x] 8. `pushdown`: an `INTERVAL DAY TO SECOND` entry emits as `DECIMAL(p + 5 + f, f)` and converts in an outer `SELECT` through `NUMTODSINTERVAL` (pushdown-scalar-function, pushdown-projection)
- [x] 9. Dead code removal: `read_group_by_column`, `Aggregation::GroupBy(Vec<ColumnRef>)`, the withholding comments, the withheld-expression capability assertions
- [x] 10. E2E fixtures: a `scalars` table at MySQL, PostgreSQL, and Trino, and its Exasol mirror
- [x] 11. E2E tests per scenario for each source (date parts, arithmetic, intervals, string functions, `SUBSTR`, empty results, source-following results, implicit conversion, grouping by an expression, declared types); update the aggregation-withdrawal and wrapped-operand tests
- [x] 12. `docs/pushdown.md`: `CAST(... AS DOUBLE)` instead of `+ 0`, the delegated scalar functions, per-database differences, the unfollowed NLS formats
- [x] 13. Verification: build, test, integration, E2E, lint, format, manual checks, verification report
