# Plan: change-type-mapping-coverage

## Summary

PostgreSQL and MySQL map the common column types whose delegated comparisons agree with Exasol's: PostgreSQL maps 15 `data_type` spellings instead of 7, and MySQL maps 17 types instead of 8. PostgreSQL `timestamp with time zone` and MySQL `TIMESTAMP` map to `TIMESTAMP` as their UTC wall-clock value, because every PostgreSQL and MySQL connection pins its session time zone to UTC. `docs/` gains a type-mapping table per dialect. A type whose comparison at the source differs from Exasol's stays omitted with a reason that names the difference. The interview requested MySQL `JSON`, `TIME`, and `YEAR` and the Exasol type `TIMESTAMP WITH LOCAL TIME ZONE`, and this plan delivers none of them, which departs from the interview answer for the reasons decision-log [4] states.

## Context

- Issue exasol-labs/adbc-vs-rs#17: PostgreSQL maps seven types and omits every other column, so a 19-column ticket table kept two columns.
- Exasol never re-applies a delegated fragment and negotiates per capability, so a mapped type must compare at the source as Exasol compares the declared type.
- The delegated operations are comparisons, `LIKE`, `BETWEEN`, `IN`, `IS NULL`, `AND`, `OR`, `NOT`, `COUNT`, `COUNT(DISTINCT)`, `MIN`, `MAX`, `GROUP BY` on a column, and equi-joins. No dialect advertises `SUM`, `AVG`, or a scalar function (`crates/adbc-vs-scan/drivers/postgresql.toml`, `mysql.toml`).
- The per-type decisions rest on the PostgreSQL 18, MySQL 8.4, and Exasol documentation and on SELECT-only literal queries against an Exasol 2025.1 database: Exasol orders `FALSE` before `TRUE` and computes `MIN` and `MAX` over `BOOLEAN`, compares `TIMESTAMP WITH LOCAL TIME ZONE` in the session time zone, and refuses `NaN` and infinity in `DOUBLE`.
- The evidence for omitting MySQL `JSON`, `TIME`, and `YEAR` is the MySQL 8.4 reference, not a measurement.
- Planning runs no containers, so the differential E2E test of each mapped type is the measurement against the test sources. For the two UTC-mapped timestamp types, a measured divergence from Exasol returns the type to omission (tasks 2.9 and 3.5).
- Whether the ADBC MySQL driver keeps a session setting made by a statement for the later statements of the same ADBC connection is not documented. The integration test of task 4.5 measures it.
- The query model already resolves each column reference's declared Exasol type from `involvedTables` (`pushdown_query.rs:135-137`), and no dialect reads it yet.
- A `column` entry in predicate position fails today (`pushdown_query.rs:760-762`). PostgreSQL defines no `min` or `max` over `boolean`.
- `exa_type::from_json` reads only the `type` field, so a `TIMESTAMP WITH LOCAL TIME ZONE` entry is read as `TIMESTAMP` (`exa_type.rs:95-119`).
- The fixture tables `items`, `all_unmapped`, and `numeric_specials` (`docker/postgres/init.sql`) and `unmapped_only` (`tests/common/mysql.rs:105-106`) hold columns this plan maps.
- The MySQL driver's Arrow types for `FLOAT`, `BIGINT UNSIGNED`, `TINYINT(1)`, and `TIMESTAMP` are not documented. Task 3.4 states the response when an E2E test shows a returned value that differs from the value MySQL compares.
- The ADBC PostgreSQL driver reads results through binary `COPY`, in which a `timestamptz` is a UTC instant. Its Arrow type is not measured, so task 4.3 covers a UTC-zoned Arrow timestamp in the coercion.
- The architecture delta (`architecture.md` in this directory) adds the mapping-safety rule with its two exceptions, the PostgreSQL column-reference rule, and the UTC session pin to § Constraints.

## Features

| Feature | Status | Spec |
|---------|--------|------|
| type-mapping | CHANGED | `vs-adapter/type-mapping/spec.md` |
| postgresql-type-mapping | CHANGED | `vs-adapter/postgresql-type-mapping/spec.md` |
| mysql-type-mapping | CHANGED | `vs-adapter/mysql-type-mapping/spec.md` |
| trino-type-mapping | CHANGED | `vs-adapter/trino-type-mapping/spec.md` |
| pushdown-dialect | CHANGED | `vs-adapter/pushdown-dialect/spec.md` |
| pushdown-filter | CHANGED | `vs-adapter/pushdown-filter/spec.md` |
| postgresql-pushdown | CHANGED | `vs-adapter/postgresql-pushdown/spec.md` |
| postgresql-collation | CHANGED | `vs-adapter/postgresql-collation/spec.md` |
| create-virtual-schema | CHANGED | `vs-adapter/create-virtual-schema/spec.md` |
| arrow-scan | CHANGED | `scan-udf/arrow-scan/spec.md` |
| postgresql-scan | CHANGED | `scan-udf/postgresql-scan/spec.md` |
| mysql-scan | CHANGED | `scan-udf/mysql-scan/spec.md` |

## Impact

- After `ALTER VIRTUAL SCHEMA ... REFRESH`, a PostgreSQL virtual table exposes `smallint`, `real`, `boolean`, `text`, unbounded `character varying`, `uuid`, `json`, `jsonb`, unconstrained `numeric`, and `timestamp with time zone` columns, and a MySQL virtual table exposes `TINYINT`, `SMALLINT`, `MEDIUMINT`, unsigned integer, `FLOAT`, text-type, and `TIMESTAMP` columns. A `SELECT *` then returns more columns, and a table whose columns were all unmapped can appear.
- An existing virtual schema keeps its current tables and columns until it is refreshed. The `adapterNotes` format does not change.
- PostgreSQL generated statements change: a `VARCHAR` or `DOUBLE` column reference is wrapped in a `CAST`. For `text`, `character varying`, and `double precision` columns the source plan does not change. A delegated filter, join key, grouping key, or `MIN` or `MAX` operand on a `real`, unconstrained `numeric`, `uuid`, `json`, or `jsonb` column cannot use a source index on that column. The main cost case is a `uuid` primary or foreign key: a point lookup becomes a sequential scan, and a delegated join on `uuid` keys cannot use an index nested-loop join. An operator restores index use with a PostgreSQL expression index on the rendered form, which `docs/type-mapping.md` names.
- Breaking for data with non-finite values: a query that reads a `NaN` or infinite value from a `DOUBLE` column now fails with a named error, where before the value reached Exasol, which reads `NaN` as a null. A delegated filter or `COUNT` over such rows, in a query that does not project the column, succeeds and follows PostgreSQL's ordering, which places `NaN` above every number. An unconstrained `numeric` value outside the double-precision range fails the query with PostgreSQL's out-of-range error.
- A query that reads a value longer than 2000000 characters from a newly mapped text column fails rather than returning a truncated value.
- Every PostgreSQL and MySQL connection runs with its session time zone set to UTC, for metadata reads and scans alike. A `timestamp with time zone` or MySQL `TIMESTAMP` column reads as its UTC wall-clock time, not the source server's local time. Breaking for source views that depend on the session time zone: a view using PostgreSQL `current_date`, or converting a `timestamptz` to `date`, evaluates in UTC when read through adbc-vs.
- PostgreSQL `character(n)` and MySQL `JSON`, `TIME`, and `YEAR` stay omitted. After a refresh, their recorded omission reasons name the comparison difference.

## Implementation Tasks

### 1. Type model and predicates

- [ ] 1.1 Add `MappedType::Boolean` and its `From<MappedType> for ExasolType` arm (`type_mapping.rs:35-60`), rewrite the `MappedType` doc comment for six types, and extend the exhaustive match in `metadata_source_tests.rs:214-247`.
- [ ] 1.2 Make `exa_type::from_json` (`exa_type.rs:95-119`) fail with an error naming the type for a `TIMESTAMP` entry carrying `withLocalTimeZone: true`, with a test in `exa_type_tests.rs`.
- [ ] 1.3 Accept a `column` entry whose declared type is `BOOLEAN` as a predicate in `read_predicate` (`pushdown_query.rs:721-786`): as the whole filter and as an operand of `predicate_and`, `predicate_or`, and `predicate_not`. Keep the error for any other declared type and name that type in it. Update the `ColumnRef.declared_type` doc comment (`pushdown_query.rs:116-118`), which says the type never decides rendering. Tests go in `pushdown_query_tests.rs`. Whether Exasol sends a predicate as a comparison operand is measured in task 2.8, because the measurement needs a mapped `boolean` column.
- [ ] 1.4 Render the `BOOLEAN`-column predicate as the bare quoted column in `postgres_dialect.rs`, `mysql_dialect.rs`, and `trino_dialect.rs`, with a test in each `*_dialect_tests.rs`.
- [ ] 1.5 Change Trino's `boolean` omission reason in `trino_type_mapping.rs` to name this release's Trino scope, and update `trino_type_mapping_tests.rs:178`.

### 2. PostgreSQL mapping, dialect, and E2E

- [ ] 2.1 Extend `postgres_type_mapping.rs::map_data_type` (`:82-106`) per the `postgresql-type-mapping` delta: `smallint`, `real`, unconstrained `numeric`, unbounded `character varying`, `text`, `json`, `jsonb`, `uuid`, `boolean`, and `timestamp with time zone` to `TIMESTAMP`. Update the module doc comment. Replace the tests at `postgres_type_mapping_tests.rs:143`, `:173`, `:272`, and `:286`, and remove `timestamp with time zone` from the unmapped list at `:221-229`.
- [ ] 2.2 Render each column reference in `postgres_dialect.rs` (`:523-533`) from `ColumnRef.declared_type`: `VARCHAR` as `CAST(<ref> AS TEXT)`, `DOUBLE` as `CAST(<ref> AS DOUBLE PRECISION)`, and every other type bare. Apply it in the select list, the filter, `GROUP BY`, aggregate operands, and join conditions, so a grouped key renders identically in `GROUP BY` and the select list. Unit tests cover each position and a full outer join on two converted keys. Existing `postgres_dialect_tests.rs` and `e2e_postgres_test.rs` assertions that expect a bare character or `DOUBLE` reference change to the converted form. [expert]
- [ ] 2.3 Render `MIN` and `MAX` over a `BOOLEAN` operand as `bool_and` and `bool_or` in `postgres_dialect.rs` (`:494-503`, `:650-664`), keeping `DISTINCT`. Add the `BOOLEAN` arm to `every_advertised_capability_renders_per_dialect` (`capabilities_tests.rs:51`).
- [ ] 2.4 In `docker/postgres/init.sql`, add a `type_coverage` table. Its rows separate the semantics: a lower-case `uuid` probed with an upper-case literal, `json` with irregular whitespace, `jsonb` with reordered keys, a `real` holding `0.1`, two unconstrained `numeric` values that round to one `DOUBLE`, `boolean` columns `flag` and `other_flag` holding `true`, `false`, and null, with `other_flag` differing from `flag` in at least one row, and a `timestamp` column `precise_at` holding `2024-01-01 12:00:00.123456`. Its `timestamptz` column `happened_at` holds instants on both sides of the Europe/Berlin transitions, written with explicit `+00` offsets: `2024-10-27 00:30:00+00` and `2024-10-27 01:30:00+00` (both `02:30` local time), and `2024-03-31 00:30:00+00` and `2024-03-31 01:30:00+00`. A `timestamp` column `local_at` holds the UTC wall-clock value of `happened_at` in some rows and a different value in others, as the join partner. Add `ALTER DATABASE adbc_vs SET TimeZone TO 'Europe/Berlin'`, so a connection without the pin of task 4.4 reads literals in a daylight-saving zone. Add one `text` value of 2000001 characters in a separate table, and an unconstrained `numeric` value `1e400` in a separate table `numeric_out_of_range`. Add every new table name to `SEEDED_OBJECTS` (`tests/common/postgres.rs:48-58`). Reduce `all_unmapped` to still-unmapped types, without `timestamptz`, with a still-unmapped key such as `CHARACTER(36)`, and update the comment at `init.sql:44-46` that lists `items`'s unmapped columns. After editing, recreate the volume with `docker compose rm -sfv postgres && make stack-up-postgres`, per `AGENTS.md`.
- [ ] 2.5 In `tests/common/postgres_mirror.rs`, add the `type_coverage` mirror. It holds the values Exasol reads: the lower-case `uuid` text, the stored `json` text, the `jsonb` output text, the widened `real`, each `numeric` as `DOUBLE`, the `boolean` values, and `happened_at` and `local_at` as `TIMESTAMP` UTC wall-clock values. Add `description`, `details`, `uid`, and `seen_at` (as its UTC wall-clock `TIMESTAMP`) to the `items` mirror.
- [ ] 2.6 Add the PostgreSQL E2E tests the Scenario Coverage table names to `e2e_postgres_test.rs`. Each compares rows with `assert_matches_mirror` (`:1376`) and checks delegation with `generated_statement` (`:1402`). The `timestamptz` test covers a comparison with a `TIMESTAMP` literal and a `DATE` literal, `MIN`, `MAX`, `COUNT(DISTINCT)`, `GROUP BY`, and an equi-join of `happened_at` with `local_at`. `sub_millisecond_timestamp_equality_follows_the_source_precision` asserts that a delegated equality of `precise_at` with `TIMESTAMP '2024-01-01 12:00:00.123'` selects no row. Update `unmapped_items_columns_are_recorded_and_all_unmapped_table_is_absent` (`:936`), since `items.seen_at` now maps, and `mirror_declares_the_same_columns_and_types_as_the_virtual_table` (`:121`).
- [ ] 2.7 Add `non_finite_double_fails_with_a_named_error` over `numeric_specials.unbounded_amount`, which now maps to `DOUBLE` and holds `Infinity`. Add `non_finite_values_filter_and_count_without_projection_follow_postgresql_ordering`: `SELECT COUNT(*) FROM PGVS.NUMERIC_SPECIALS WHERE UNBOUNDED_AMOUNT > 0` delegates, succeeds, and counts the non-finite rows PostgreSQL's ordering places above 0. Add `out_of_range_numeric_fails_with_postgresqls_error` over `numeric_out_of_range`.
- [ ] 2.8 Measure on the test Exasol whether `SELECT ID FROM PGVS.TYPE_COVERAGE WHERE FLAG = (ID > 5)` reaches the adapter as a `predicate_equal` with a predicate node as an operand. If it does, accept a predicate node as a comparison operand in `read_expression` (`pushdown_query.rs:851-880`), render it parenthesized in each dialect, add a unit test in `pushdown_query_tests.rs`, and add an AND step to the `vs-adapter/pushdown-filter` scenario "BOOLEAN column is a predicate on its own". If it does not, record the measured request shape in plan.md § Context.
- [ ] 2.9 If the `timestamptz` E2E test shows a delegated comparison, grouping, or join diverging from the mirror with the pin of task 4.4 in place, return `timestamp with time zone` to omission with a reason naming the measured divergence. Then replace the mapping scenario in the `postgresql-type-mapping` delta with an omission scenario, record the measured result in decision-log [4] and [5], and update task 5.1's page.

### 3. MySQL mapping and E2E

- [ ] 3.1 Extend `mysql_type_mapping.rs::map_data_type` (`:59-89`) per the `mysql-type-mapping` delta: signed and unsigned `TINYINT`, `SMALLINT`, `MEDIUMINT`, `INT`, and `BIGINT`, `FLOAT`, `TIMESTAMP` to `TIMESTAMP`, and the four text types, each mapped to `VARCHAR` of its `CHARACTER_MAXIMUM_LENGTH`, which MySQL reports as the type's byte limit, capped at `MAX_VARCHAR_SIZE`, and unmapped unless its character set is `utf8mb4`. Give `JSON`, `TIME`, and `YEAR` named reasons, and drop the "eight types" wording. Replace the test at `mysql_type_mapping_tests.rs:180` and update `:192`.
- [ ] 3.2 In `tests/common/mysql.rs::fixture_script` (`:93-115`), add a `type_coverage` table: each integer width signed and unsigned at its extreme values, a `TINYINT(1)` holding `5`, a `FLOAT` holding `0.1`, the four text types, and a `latin1` `TEXT`. Its `TIMESTAMP` column `happened_at` holds instants on both sides of the Europe/Berlin transitions, written as offset literals such as `'2024-10-27 00:30:00+00:00'` and `'2024-10-27 01:30:00+00:00'`, and a `DATETIME` column `local_at` holds the UTC wall-clock value of `happened_at` in some rows and a different value in others, as the join partner. Add a `MEDIUMTEXT` value of 2000001 characters in a separate table. Add a `divergent_types` table with `JSON`, `TIME`, and `YEAR` columns. Replace `unmapped_only.notes TEXT` with a `TIME` column.
- [ ] 3.3 Add the `type_coverage` mirror to `tests/common/mirror.rs` (`install`, `:99`), holding `BIGINT UNSIGNED` 18446744073709551615 as `DECIMAL(20,0)`, the `FLOAT` as `0.100000001490116119384765625`, and `happened_at` and `local_at` as `TIMESTAMP` UTC wall-clock values.
- [ ] 3.4 Add the MySQL E2E tests the Scenario Coverage table names to `e2e_mysql_test.rs`, comparing rows with `assert_matches_mirror` (`:1229`) and checking delegation with `pushdown_statement` (`:2100`). The `TIMESTAMP` test covers a comparison with a `TIMESTAMP` literal and a `DATE` literal, `MIN`, `MAX`, `COUNT(DISTINCT)`, `GROUP BY`, and an equi-join of `happened_at` with `local_at`. Update `fully_unmappable_table_omitted` (`:899`) and `latin1_column_omitted_and_recorded` (`:875`). If an E2E test shows that the value the driver returns for `FLOAT`, an unsigned integer, or `TINYINT(1)` differs from the value MySQL compares, unmap that type with a reason that names the difference, and update the `mysql-type-mapping` delta and docs task 5.1.
- [ ] 3.5 If the `TIMESTAMP` E2E test shows a delegated comparison, grouping, or join diverging from the mirror with the pin of task 4.5 in place, return MySQL `TIMESTAMP` to omission with a reason naming the measured divergence. Then replace the mapping scenario in the `mysql-type-mapping` delta with an omission scenario, record the measured result in decision-log [4] and [5], and update task 5.1's page.

### 4. Scan connection and coercion

- [ ] 4.1 In `crates/adbc-vs-scan/src/coerce.rs`, fail with an error naming the value and the declared type when a declared `DOUBLE` column holds `NaN` or an infinity, with a test in `coerce_tests.rs`.
- [ ] 4.2 In `coerce_tests.rs`, cover signed and unsigned 8-, 16-, 32-, and 64-bit integers coerced into their declared `DECIMAL`, including `u64::MAX` into `DECIMAL(20,0)`.
- [ ] 4.3 In `coerce_tests.rs`, cover a time-zone-aware Arrow timestamp in UTC (`Timestamp(Microsecond, Some("UTC"))` and `Some("+00:00")`) coerced into a declared `TIMESTAMP` as its UTC wall-clock value, in `utc_timestamp_coerces_to_its_wall_clock_value`. If `coerce.rs` shifts the value, convert it to UTC before the zone is dropped.
- [ ] 4.4 In `crates/adbc-vs-scan/src/connection.rs`, make the conninfo that `postgres_target` (`:169-177`) builds carry the `options` keyword with `-c TimeZone=UTC`, appended after the value of an `options` parameter the address carries. Update the exact-conninfo assertions in `connection_tests.rs` (`postgres_builds_a_conninfo_from_the_address_and_the_credential` `:298`, `postgres_carries_every_address_parameter_across` `:323`), and add `postgres_conninfo_pins_the_session_time_zone_to_utc` and `postgres_pin_keeps_the_addresss_own_options`. Add `session_time_zone_is_utc_although_the_address_sets_another` to `crates/adbc-vs-scan/tests/postgres_it_test.rs`: through `open`, an address whose `options` sets `-c TimeZone=Europe/Berlin` yields a session whose `SHOW TimeZone` is `UTC`.
- [ ] 4.5 In `connection.rs`, have `mysql_target` (`:145-160`) carry the session statement `SET time_zone = '+00:00'` in `DatabaseTarget`, and have `open` (`:465-494`) execute it on the new connection before returning it, naming and redacting a failure like the other open steps. The per-driver match in `database_target` stays the one place a driver's connection settings are decided, and the `uri` option stays unchanged. In `docker/mysql/init.sql`, set a non-UTC server default with `SET PERSIST time_zone = 'Europe/Berlin'`, or `'+05:00'` if the image loads no time zone tables, update the comment at `init.sql:7-8`, which says a `TIMESTAMP` column would be omitted, and recreate the volume with `docker compose rm -sfv mysql && make stack-up-mysql`. Add `mysql_target_pins_the_session_time_zone_without_changing_the_uri` to `connection_tests.rs`, and add `session_time_zone_is_utc_although_the_server_default_is_not` to `crates/adbc-vs-scan/tests/mysql_it_test.rs`: through `open`, `@@global.time_zone` is not `+00:00`, and a statement after the pin reads `@@session.time_zone` as `+00:00`. If the driver does not keep the setting for later statements, pass `time_zone` as a query parameter of the driver URI instead, and change the `uri` step of the `scan-udf/mysql-scan` delta.

### 5. Documentation

- [ ] 5.1 Write `docs/type-mapping.md` with one table per dialect (PostgreSQL, MySQL, Trino) and the columns source type, Exasol type, and notes or omission reason. The tables list PostgreSQL `timestamp with time zone` and MySQL `TIMESTAMP` as `TIMESTAMP` holding the UTC wall-clock value. A short section covers what delegation means for a mapped type: collation; sub-millisecond timestamp precision, where the source compares at its fractional-second precision while Exasol holds milliseconds; the UTC session pin and its effect on source views that depend on the session time zone; PostgreSQL's conversion and its index cost on filters, join keys, and grouping keys, with a `uuid` key as the main case; the 2000000-character limit; non-finite `DOUBLE` values, including a filter or count over them that does not project the column and an unconstrained `numeric` outside the double-precision range; and why no column maps to `TIMESTAMP WITH LOCAL TIME ZONE`. The index-cost paragraph names the remedy, a PostgreSQL expression index on the rendered form such as `CREATE INDEX ON t ((CAST(uid AS text)))`. Before documenting it, confirm with one `EXPLAIN` on the test stack that PostgreSQL uses that index for the generated statement of a `uuid` equality.
- [ ] 5.2 Link the page from `docs/index.md` (`:7-9`), from the Limits line in `docs/how-it-works.md` (`:34`), and from the omitted-columns paragraph in `docs/create-virtual-schema.md` (`:29`).

## Parallelization

| Group | Tasks | Depends on | Knowledge |
|-------|-------|------------|-----------|
| A: Type model and predicates | 1.1-1.5 | — | spec deltas `vs-adapter/type-mapping`, `vs-adapter/pushdown-dialect`, `vs-adapter/pushdown-filter`, `vs-adapter/trino-type-mapping`; `crates/adbc-vs-adapter/src/{type_mapping,exa_type,pushdown_query,postgres_dialect,mysql_dialect,trino_dialect,trino_type_mapping}.rs` and their `*_tests.rs`, `metadata_source_tests.rs` |
| D: Scan connection and coercion | 4.1-4.5 | — | spec deltas `scan-udf/arrow-scan`, `scan-udf/postgresql-scan`, `scan-udf/mysql-scan`; `crates/adbc-vs-scan/src/{coerce,connection}.rs`, `coerce_tests.rs`, `connection_tests.rs`, `crates/adbc-vs-scan/tests/{postgres,mysql}_it_test.rs`, `docker/mysql/init.sql` |
| B: PostgreSQL | 2.1-2.9 | A (shares `pushdown_query.rs`, `postgres_dialect.rs`, `MappedType::Boolean`), D (tasks 2.7 and 2.9 need the coercion and the pin) | spec deltas `vs-adapter/postgresql-type-mapping`, `vs-adapter/postgresql-pushdown`, `vs-adapter/postgresql-collation`, `vs-adapter/create-virtual-schema`; `postgres_type_mapping.rs`, `postgres_dialect.rs`, `capabilities_tests.rs`, `docker/postgres/init.sql`, `tests/common/postgres.rs`, `tests/common/postgres_mirror.rs`, `tests/e2e_postgres_test.rs` |
| C: MySQL | 3.1-3.5 | D (unsigned coercion and the pin) | spec delta `vs-adapter/mysql-type-mapping`; `mysql_type_mapping.rs`, `tests/common/mysql.rs`, `tests/common/mirror.rs`, `tests/e2e_mysql_test.rs` |
| E: Documentation | 5.1-5.2 | B, C | every `*-type-mapping` delta; `docs/` |

Groups B and C may write code in parallel. Their E2E runs (tasks 2.6, 2.7, 2.8, 2.9, 3.4, and 3.5) run one at a time, after the code of both groups compiles, because each run needs `make cross-udf-build` from the shared working tree and uses the one Exasol stack, whose shared setup installs the SLC and creates the virtual schemas.

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| Function | `crates/adbc-vs-adapter/src/postgres_type_mapping.rs::boolean_reason` (`:160`) | `boolean` maps |
| Function | `crates/adbc-vs-adapter/src/postgres_type_mapping.rs::unbounded_length_reason` (`:168`) | `text` and unbounded `character varying` map |
| Function | `crates/adbc-vs-adapter/src/mysql_type_mapping.rs::unsigned_reason` | Every unsigned integer width maps |
| Comment | `crates/adbc-vs-adapter/src/type_mapping.rs:27-33` | States that no source produces `BOOLEAN` |
| Test | `postgres_type_mapping_tests.rs::numeric_without_a_declared_precision_is_unmapped` (`:143`) | Replaced by the `DOUBLE` mapping test |
| Test | `postgres_type_mapping_tests.rs::character_varying_without_a_declared_length_is_unmapped` (`:173`) | Replaced by the maximum-`VARCHAR` test |
| Test | `postgres_type_mapping_tests.rs::boolean_is_unmapped_despite_exasol_having_one` (`:272`) | Replaced by `boolean_maps_to_boolean` |
| Test | `postgres_type_mapping_tests.rs::a_time_zone_aware_timestamp_is_unmapped_rather_than_mapped_to_timestamp` (`:286`) | Replaced by `timestamp_with_time_zone_maps_to_timestamp` |
| Code | `crates/adbc-vs-adapter/src/postgres_type_mapping.rs` `timestamp with time zone` omission reason (`:152-180`) | `timestamp with time zone` maps |
| Comment | `docker/mysql/init.sql:7-8` | States that a `TIMESTAMP` column would be omitted |
| Test | `mysql_type_mapping_tests.rs::unsigned_integers_unmapped` (`:180`) | Replaced by the unsigned mapping test |
| Fixture | `docker/postgres/init.sql` `all_unmapped` columns of newly mapped types (`:157-174`) | They no longer leave the table unmapped |
| Fixture | `tests/common/mysql.rs` `unmapped_only.notes TEXT` (`:106`) | `TEXT` maps |

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| type-mapping: A mapped column declares only an Exasol type this release produces | Unit | `crates/adbc-vs-adapter/src/metadata_source_tests.rs` | `every_mapped_type_is_in_the_declarable_set` |
| type-mapping: A source type whose delegated comparison differs from Exasol's is unmapped | Integration | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` | `divergent_columns_are_omitted_and_recorded` |
| type-mapping: A source type whose delegated comparison differs from Exasol's is unmapped | Integration | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `sub_millisecond_timestamp_equality_follows_the_source_precision` |
| postgresql-type-mapping: Integer types map to DECIMAL with the matching digit count | Integration | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `smallint_column_matches_the_mirror` |
| postgresql-type-mapping: Integer types map to DECIMAL with the matching digit count | Unit | `crates/adbc-vs-adapter/src/postgres_type_mapping_tests.rs` | `smallint_maps_to_decimal_with_five_digits` |
| postgresql-type-mapping: numeric without a declared precision maps to DOUBLE | Integration | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `unbounded_numeric_compares_and_groups_as_double_like_the_mirror` |
| postgresql-type-mapping: numeric without a declared precision maps to DOUBLE | Integration | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `non_finite_values_filter_and_count_without_projection_follow_postgresql_ordering` |
| postgresql-type-mapping: numeric without a declared precision maps to DOUBLE | Integration | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `out_of_range_numeric_fails_with_postgresqls_error` |
| postgresql-type-mapping: numeric without a declared precision maps to DOUBLE | Unit | `crates/adbc-vs-adapter/src/postgres_type_mapping_tests.rs` | `numeric_without_a_declared_precision_maps_to_double` |
| postgresql-type-mapping: character varying preserves the declared length and uses UTF8 | Integration | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `text_and_unbounded_varchar_match_the_mirror` |
| postgresql-type-mapping: character varying preserves the declared length and uses UTF8 | Unit | `crates/adbc-vs-adapter/src/postgres_type_mapping_tests.rs` | `character_varying_without_a_declared_length_maps_to_the_maximum_varchar` |
| postgresql-type-mapping: text, uuid, json, and jsonb map to VARCHAR and compare as the text the scan emits | Integration | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `uuid_json_and_jsonb_compare_as_the_emitted_text` |
| postgresql-type-mapping: text, uuid, json, and jsonb map to VARCHAR and compare as the text the scan emits | Integration | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `text_value_beyond_the_varchar_maximum_fails_the_query` |
| postgresql-type-mapping: text, uuid, json, and jsonb map to VARCHAR and compare as the text the scan emits | Unit | `crates/adbc-vs-adapter/src/postgres_type_mapping_tests.rs` | `text_uuid_json_and_jsonb_map_to_utf8_varchar` |
| postgresql-type-mapping: Temporal and floating-point types map to their Exasol counterparts | Integration | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `real_column_holds_the_widened_single_precision_value` |
| postgresql-type-mapping: Temporal and floating-point types map to their Exasol counterparts | Unit | `crates/adbc-vs-adapter/src/postgres_type_mapping_tests.rs` | `real_maps_to_double` |
| postgresql-type-mapping: boolean maps to BOOLEAN | Integration | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `boolean_column_filters_groups_and_aggregates_like_the_mirror` |
| postgresql-type-mapping: boolean maps to BOOLEAN | Unit | `crates/adbc-vs-adapter/src/postgres_type_mapping_tests.rs` | `boolean_maps_to_boolean` |
| postgresql-type-mapping: timestamp with time zone maps to TIMESTAMP holding the UTC wall-clock value | Unit | `crates/adbc-vs-adapter/src/postgres_type_mapping_tests.rs` | `timestamp_with_time_zone_maps_to_timestamp` |
| postgresql-type-mapping: timestamp with time zone maps to TIMESTAMP holding the UTC wall-clock value | Unit | `crates/adbc-vs-scan/src/coerce_tests.rs` | `utc_timestamp_coerces_to_its_wall_clock_value` |
| postgresql-type-mapping: timestamp with time zone maps to TIMESTAMP holding the UTC wall-clock value | Integration | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `timestamptz_values_on_both_sides_of_a_transition_match_the_mirror` |
| postgresql-type-mapping: Delegated operations over a timestamp with time zone column follow the UTC session | Integration | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `timestamptz_column_compares_groups_and_joins_in_utc_like_the_mirror` |
| postgresql-type-mapping: Non-core types are unmapped | Unit | `crates/adbc-vs-adapter/src/postgres_type_mapping_tests.rs` | `non_core_types_are_unmapped_with_the_reason_naming_the_type` |
| mysql-type-mapping: Signed integer types map to DECIMAL with the matching digit count | Integration | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` | `narrow_integer_columns_match_the_mirror` |
| mysql-type-mapping: Signed integer types map to DECIMAL with the matching digit count | Unit | `crates/adbc-vs-adapter/src/mysql_type_mapping_tests.rs` | `narrow_signed_integers_map_to_decimal_with_matching_digits` |
| mysql-type-mapping: Unsigned integer types map to DECIMAL with the digits of their unsigned range | Integration | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` | `unsigned_integer_columns_match_the_mirror` |
| mysql-type-mapping: Unsigned integer types map to DECIMAL with the digits of their unsigned range | Unit | `crates/adbc-vs-adapter/src/mysql_type_mapping_tests.rs` | `unsigned_integers_map_to_decimal_with_unsigned_digits` |
| mysql-type-mapping: Text types map to VARCHAR bounded by their storage limit | Integration | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` | `text_columns_match_the_mirror` |
| mysql-type-mapping: Text types map to VARCHAR bounded by their storage limit | Integration | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` | `text_value_beyond_the_varchar_maximum_fails_the_query` |
| mysql-type-mapping: Text types map to VARCHAR bounded by their storage limit | Unit | `crates/adbc-vs-adapter/src/mysql_type_mapping_tests.rs` | `text_types_map_to_varchar_bounded_by_their_storage_limit` |
| mysql-type-mapping: Character column of a non-utf8mb4 character set is unmapped | Integration | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` | `latin1_column_omitted_and_recorded` |
| mysql-type-mapping: Character column of a non-utf8mb4 character set is unmapped | Unit | `crates/adbc-vs-adapter/src/mysql_type_mapping_tests.rs` | `non_utf8mb4_character_and_text_columns_are_unmapped` |
| mysql-type-mapping: Temporal and floating-point types map to their Exasol counterparts | Integration | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` | `float_column_holds_the_widened_single_precision_value` |
| mysql-type-mapping: Temporal and floating-point types map to their Exasol counterparts | Unit | `crates/adbc-vs-adapter/src/mysql_type_mapping_tests.rs` | `float_maps_to_double` |
| mysql-type-mapping: TIMESTAMP maps to TIMESTAMP holding the UTC wall-clock value | Unit | `crates/adbc-vs-adapter/src/mysql_type_mapping_tests.rs` | `timestamp_maps_to_timestamp` |
| mysql-type-mapping: TIMESTAMP maps to TIMESTAMP holding the UTC wall-clock value | Integration | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` | `timestamp_values_on_both_sides_of_a_transition_match_the_mirror` |
| mysql-type-mapping: Delegated operations over a TIMESTAMP column follow the UTC session | Integration | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` | `timestamp_column_compares_groups_and_joins_in_utc_like_the_mirror` |
| mysql-type-mapping: JSON, TIME, and YEAR are unmapped for how MySQL compares them | Unit | `crates/adbc-vs-adapter/src/mysql_type_mapping_tests.rs` | `json_time_and_year_are_unmapped_naming_the_comparison` |
| mysql-type-mapping: JSON, TIME, and YEAR are unmapped for how MySQL compares them | Integration | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` | `divergent_columns_are_omitted_and_recorded` |
| mysql-type-mapping: Non-core MySQL types are unmapped | Unit | `crates/adbc-vs-adapter/src/mysql_type_mapping_tests.rs` | `non_core_types_unmapped` |
| mysql-type-mapping: Non-core MySQL types are unmapped | Integration | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` | `fully_unmappable_table_omitted` |
| trino-type-mapping: Non-core types are unmapped | Unit | `crates/adbc-vs-adapter/src/trino_type_mapping_tests.rs` | `boolean_is_unmapped_as_outside_the_trino_scope` |
| pushdown-dialect: Every advertised capability has a rendering path | Unit | `crates/adbc-vs-adapter/src/capabilities_tests.rs` | `every_advertised_capability_renders_per_dialect` |
| pushdown-dialect: Declared type outside the declarable set fails with a named error | Unit | `crates/adbc-vs-adapter/src/exa_type_tests.rs` | `local_time_zone_timestamp_fails_rather_than_reading_as_timestamp` |
| pushdown-filter: BOOLEAN column is a predicate on its own | Unit | `crates/adbc-vs-adapter/src/pushdown_query_tests.rs` | `boolean_column_is_a_predicate_in_every_position` |
| pushdown-filter: BOOLEAN column is a predicate on its own | Unit | `crates/adbc-vs-adapter/src/pushdown_query_tests.rs` | `non_boolean_column_in_predicate_position_fails_naming_its_type` |
| pushdown-filter: BOOLEAN column is a predicate on its own | Unit | `crates/adbc-vs-adapter/src/{postgres,mysql,trino}_dialect_tests.rs` | `boolean_column_predicate_renders_bare` |
| pushdown-filter: BOOLEAN column is a predicate on its own | Integration | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `boolean_column_filters_groups_and_aggregates_like_the_mirror` |
| postgresql-pushdown: Character and floating-point column references render as their declared type | Unit | `crates/adbc-vs-adapter/src/postgres_dialect_tests.rs` | `varchar_and_double_references_render_as_their_declared_type` |
| postgresql-pushdown: Character and floating-point column references render as their declared type | Unit | `crates/adbc-vs-adapter/src/postgres_dialect_tests.rs` | `converted_grouping_key_renders_identically_in_group_by_and_select_list` |
| postgresql-pushdown: Character and floating-point column references render as their declared type | Integration | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `typed_column_references_reach_postgresql_and_match_the_mirror` |
| postgresql-pushdown: MIN and MAX over a BOOLEAN column render as bool_and and bool_or | Unit | `crates/adbc-vs-adapter/src/postgres_dialect_tests.rs` | `min_and_max_over_boolean_render_as_bool_and_and_bool_or` |
| postgresql-pushdown: MIN and MAX over a BOOLEAN column render as bool_and and bool_or | Integration | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `boolean_column_filters_groups_and_aggregates_like_the_mirror` |
| postgresql-collation: A select-list predicate over the grouped character column is delegated | Integration | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `select_list_predicate_over_the_grouped_character_column_delegates_and_matches_the_mirror` |
| create-virtual-schema: Columns of unmappable type are omitted and recorded | Integration | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `unmapped_items_columns_are_recorded_and_all_unmapped_table_is_absent` |
| arrow-scan: Non-finite DOUBLE value fails with a named error | Unit | `crates/adbc-vs-scan/src/coerce_tests.rs` | `non_finite_double_fails_naming_the_value` |
| arrow-scan: Non-finite DOUBLE value fails with a named error | Integration | `crates/adbc-vs-adapter/tests/e2e_postgres_test.rs` | `non_finite_double_fails_with_a_named_error` |
| arrow-scan: Narrow and unsigned integers coerce into their declared DECIMAL | Unit | `crates/adbc-vs-scan/src/coerce_tests.rs` | `narrow_and_unsigned_integers_coerce_into_their_declared_decimal` |
| arrow-scan: Narrow and unsigned integers coerce into their declared DECIMAL | Integration | `crates/adbc-vs-adapter/tests/e2e_mysql_test.rs` | `unsigned_integer_columns_match_the_mirror` |
| postgresql-scan: Connection pins the session time zone to UTC | Unit | `crates/adbc-vs-scan/src/connection_tests.rs` | `postgres_conninfo_pins_the_session_time_zone_to_utc` |
| postgresql-scan: Connection pins the session time zone to UTC | Unit | `crates/adbc-vs-scan/src/connection_tests.rs` | `postgres_pin_keeps_the_addresss_own_options` |
| postgresql-scan: Connection pins the session time zone to UTC | Integration | `crates/adbc-vs-scan/tests/postgres_it_test.rs` | `session_time_zone_is_utc_although_the_address_sets_another` |
| mysql-scan: Connection pins the session time zone to UTC | Unit | `crates/adbc-vs-scan/src/connection_tests.rs` | `mysql_target_pins_the_session_time_zone_without_changing_the_uri` |
| mysql-scan: Connection pins the session time zone to UTC | Integration | `crates/adbc-vs-scan/tests/mysql_it_test.rs` | `session_time_zone_is_utc_although_the_server_default_is_not` |

- The existing `vs-adapter/postgresql-collation` E2E tests run unchanged and show that the `text` conversion keeps a column's collation.
- The `postgresql-scan` Background change adds no scenario to test. Its new scenario is covered above.
- Each differential E2E test checks a delegated fragment twice, per `AGENTS.md`: `EXPLAIN VIRTUAL` shows the fragment in the source statement, and the rows equal those of the same query over the local Exasol mirror table.

### Manual Testing

The commands run against the stack the E2E harness brings up, with `<dsn>` standing for the test Exasol's DSN.

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| type-mapping | `exapump sql --dsn "<dsn>" "SELECT DISTINCT COLUMN_TYPE FROM EXA_ALL_COLUMNS WHERE COLUMN_SCHEMA IN ('PGVS','MYSQLVS')"` | Only `DECIMAL`, `VARCHAR`, `DATE`, `TIMESTAMP`, `DOUBLE`, and `BOOLEAN` types |
| postgresql-type-mapping | `exapump sql --dsn "<dsn>" "SELECT COLUMN_NAME, COLUMN_TYPE FROM EXA_ALL_COLUMNS WHERE COLUMN_SCHEMA = 'PGVS' AND COLUMN_TABLE = 'TYPE_COVERAGE'"` | `smallint` as `DECIMAL(5,0)`, `real` and `numeric` as `DOUBLE`, `boolean` as `BOOLEAN`, `uuid` as `VARCHAR(36) UTF8`, `text`, `json`, and `jsonb` as `VARCHAR(2000000) UTF8`, `happened_at` as `TIMESTAMP` |
| mysql-type-mapping | `exapump sql --dsn "<dsn>" "SELECT COLUMN_NAME, COLUMN_TYPE FROM EXA_ALL_COLUMNS WHERE COLUMN_SCHEMA = 'MYSQLVS' AND COLUMN_TABLE = 'TYPE_COVERAGE'"` | `BIGINT UNSIGNED` as `DECIMAL(20,0)`, `FLOAT` as `DOUBLE`, `MEDIUMTEXT` as `VARCHAR(2000000) UTF8`, `happened_at` as `TIMESTAMP` |
| trino-type-mapping | `exapump sql --dsn "<dsn>" "SELECT ADAPTER_NOTES FROM EXA_ALL_VIRTUAL_SCHEMAS WHERE SCHEMA_NAME LIKE 'TRINO%'"` | A `boolean` omission whose reason names this release's Trino scope |
| pushdown-dialect | `exapump sql --dsn "<dsn>" "SELECT MIN(FLAG), MAX(FLAG) FROM PGVS.TYPE_COVERAGE"` | `FALSE` and `TRUE`, with no error |
| pushdown-filter | `exapump sql --dsn "<dsn>" "EXPLAIN VIRTUAL SELECT ID FROM PGVS.TYPE_COVERAGE WHERE FLAG AND NOT OTHER_FLAG"` | A `WHERE` clause carrying the bare `flag` column and `NOT` over the bare `other_flag` column, with no comparison added |
| postgresql-pushdown | `exapump sql --dsn "<dsn>" "EXPLAIN VIRTUAL SELECT ID FROM PGVS.TYPE_COVERAGE WHERE UID = 'A0EEBC99-9C0B-4EF8-BB6D-6BB9BD380A11'"` | A generated statement carrying `CAST("uid" AS TEXT)`, and the query itself returns no row |
| postgresql-collation | `exapump sql --dsn "<dsn>" "EXPLAIN VIRTUAL SELECT CATEGORY, COUNT(*) FROM PGVS.ITEMS GROUP BY CATEGORY"` | A generated statement carrying `CAST("category" AS TEXT)` in both `GROUP BY` and the select list, with no `COLLATE` keyword |
| create-virtual-schema | `exapump sql --dsn "<dsn>" "SELECT ADAPTER_NOTES FROM EXA_ALL_VIRTUAL_SCHEMAS WHERE SCHEMA_NAME = 'PGVS'"` | Omissions for `items.code` and `items.photo`, and none for `items.details`, `items.uid`, or `items.seen_at` |
| arrow-scan | `exapump sql --dsn "<dsn>" "SELECT UNBOUNDED_AMOUNT FROM PGVS.NUMERIC_SPECIALS"` | An error naming the infinite value and `DOUBLE` |
| postgresql-scan | `exapump sql --dsn "<dsn>" "SELECT UNBOUNDED_AMOUNT FROM PGVS.NUMERIC_SPECIALS WHERE ID = 1"` | `100` as a `DOUBLE` |
| postgresql-scan | `exapump sql --dsn "<dsn>" "SELECT HAPPENED_AT FROM PGVS.TYPE_COVERAGE WHERE HAPPENED_AT = TIMESTAMP '2024-10-27 00:30:00'"` | One row, `2024-10-27 00:30:00.000`, the UTC wall-clock value rather than the Europe/Berlin local time `02:30` |
| mysql-scan | `exapump sql --dsn "<dsn>" "SELECT HAPPENED_AT FROM MYSQLVS.TYPE_COVERAGE WHERE HAPPENED_AT = TIMESTAMP '2024-10-27 00:30:00'"` | One row, `2024-10-27 00:30:00.000`, the UTC wall-clock value rather than the server's local time |

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Build | `make cross-udf-build` | Exit 0 |
| Test | `make test` | 0 failures |
| Integration | `make test-it test-it-postgres test-it-trino` | 0 failures |
| E2E | `make test-e2e`, `make test-e2e-postgres`, `make test-e2e-trino` | 0 failures |
| Lint | `make lint` | 0 errors/warnings |
| Format | `make fmt-check` | No changes |
| Licenses | `make deny` | Exit 0 |
