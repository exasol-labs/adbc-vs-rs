# Decision Log: change-type-mapping-coverage

## Interview

- Q: Type-set scope (stay in five types / add BOOLEAN / add BOOLEAN and TIMESTAMP WITH LOCAL TIME ZONE)? → A: Map the 7 proposed types from the issue, so BOOLEAN and TIMESTAMP WITH LOCAL TIME ZONE join the set
- Q: MySQL scope? → A: Small ints + FLOAT, TEXT family, unsigned integers, TIMESTAMP/JSON/TIME/YEAR
- Q: Unconstrained numeric and character(n)? → A: numeric -> DOUBLE, character(n) stays omitted
- Q: Predicates for types with differing comparison semantics? → A: Map only types with safe predicates, no new per-column gating mechanism
- Q: Docs? → A: Tabular type-mapping overview per dialect (PostgreSQL, MySQL, Trino) in docs/
- Note: Issue is exasol-labs/adbc-vs-rs#17. "Only types with safe predicates" constrains the 7 issue types and the MySQL groups: the planner decides per type, with measured evidence, and omits a type whose delegated comparison cannot be made safe. Split check: one plan, per the request that bundles PostgreSQL, MySQL, and docs.

## Design Decisions

### [1] One set of seven Exasol types serves mapping and EMITS

- **Decision:** A source column may map to `DECIMAL`, `VARCHAR`, `CHAR`, `DATE`, `TIMESTAMP(p)`, `DOUBLE`, or `BOOLEAN`, and an `EMITS` column may declare exactly the same set, modeled as one type. `SELECTLIST_EXPRESSIONS` stays advertised.
- **Rationale:** The superseded ADR modeled the mapped set and the declarable set as two types because no source column produced `BOOLEAN` or `CHAR`. PostgreSQL `boolean` and `bit` and MySQL `BIT` now produce `BOOLEAN`, and PostgreSQL `character(n)` and MySQL `CHAR`, `ENUM`, and `SET` produce `CHAR`, so the sets are equal and a second type would restate the first. Criterion 1 applies: the set binds the type model, the query parser, every dialect, and the scan. Criterion 2 applies: the pushdown closure argument rests on it. ADR digest: `advertise-selectlist-expressions-with-boolean-emits` states that the mapped type gains no `BOOLEAN` variant, so this entry supersedes it and keeps its `SELECTLIST_EXPRESSIONS` decision.
- **Supersedes:** advertise-selectlist-expressions-with-boolean-emits
- **Architecture:** § Interfaces
- **Promotes to ADR:** yes

### [2] The PostgreSQL dialect renders VARCHAR and DOUBLE column references as their declared type

- **Status:** Superseded by [6]
- **Decision:** The PostgreSQL dialect renders a column reference converted to the PostgreSQL type that holds its declared Exasol type's value, selected by the declared type alone.
- **Rationale:** The declared type cannot tell `money` from `real`, or `bit` from `boolean`, which share a declared type and need different conversions, so [6] keys the conversion on the recorded source type.
- **Promotes to ADR:** no

### [3] MySQL text types map although PAD SPACE collations group by a prefix

- **Decision:** `TINYTEXT`, `TEXT`, `MEDIUMTEXT`, and `LONGTEXT` map to `VARCHAR`, although MySQL groups and deduplicates the values of a `PAD SPACE` collation by their first `max_sort_length` bytes.
- **Rationale:** The prefix grouping binds a long `VARCHAR` as well, and MySQL's default `utf8mb4_0900` collations are `NO PAD`, which `max_sort_length` does not affect (MySQL 8.4 reference, `max_sort_length`). Omitting the text types would not remove the behavior from `VARCHAR`.
- **Alternatives:** Mapping a text column only under a `NO PAD` collation treats text and `VARCHAR` differently for one collation property. Extending that check to `VARCHAR` would omit columns that existing virtual schemas expose.
- **Promotes to ADR:** no

### [4] No source column maps to TIMESTAMP WITH LOCAL TIME ZONE

- **Decision:** No source column maps to Exasol `TIMESTAMP WITH LOCAL TIME ZONE`. PostgreSQL `timestamp with time zone` and MySQL `TIMESTAMP` map to `TIMESTAMP(p)`, as the Exasol JDBC Virtual Schemas map them. This departs from the interview answer that `TIMESTAMP WITH LOCAL TIME ZONE` joins the set.
- **Rationale:** Exasol reads, writes, and compares a `TIMESTAMP WITH LOCAL TIME ZONE` value in the Exasol session time zone (Exasol data type details; changelog 15053), and no mechanism carries that zone to the source session for each query. How a UDF emits such a value is not documented. The JDBC Virtual Schemas declare `withLocalTimeZone` false for every column.
- **Alternatives:** Mapping `timestamp with time zone` to `TIMESTAMP WITH LOCAL TIME ZONE` makes a delegated comparison depend on two session zones, the Exasol one and the source one.
- **Promotes to ADR:** no

### [5] Time-zone-aware source timestamps map to TIMESTAMP in UTC under a pinned session

- **Status:** Superseded by [7]
- **Decision:** Every PostgreSQL and MySQL connection pins its session time zone to UTC when it opens.
- **Rationale:** [7] removes the pin: the JDBC Virtual Schemas set no session time zone, and a pin changes every session-dependent expression a source view holds.
- **Promotes to ADR:** no

### [6] A column records its source type, and the technology's type mapping names its read expression

- **Decision:** Each mapped column's own `adapterNotes` record its source type. Each technology's type mapping names, for a source type and a declared type, the read expression that yields the value the scan emits, and the dialect renders every reference to the column through it: in the select list, a filter, a grouping key, an aggregate operand, and a join condition. A column without a recorded source type renders bare.
- **Rationale:** PostgreSQL `money`, `bit`, `uuid`, `json`, `xml`, and unconstrained `numeric`, and MySQL `TIME`, `YEAR`, `BIT`, `JSON`, and `DECIMAL` beyond precision 36, need a conversion to yield their declared Exasol value, and each shares its declared type with a source type that needs none. Only the source type tells them apart, and the JDBC Virtual Schemas record it in the same place (`{"jdbcDataType", "typeName"}` in column `adapterNotes`). One module per technology owns how a source type becomes its Exasol value, both as the declared type and as the read expression, so the two cannot disagree. The dialect depends on its own technology's type mapping, and no cycle results. Criterion 1 applies: `create_schema`, `adapter_notes`, `pushdown_query`, `type_mapping`, and `sql_dialect` change. Criterion 2 applies: every future mapping states its read expression here. Criterion 4 applies: it rejects the alternatives below. ADR digest: `character-comparisons-follow-source-collation` still holds, because no read expression renders a `COLLATE`, and `vs-adapter/postgresql-collation` states that a value read as text compares under the database default collation.
- **Alternatives:** Keying the conversion on the declared type ([2]) cannot tell `money` from `real`, `bit` from `boolean`, `TIME` from `DATETIME`, or `YEAR` from `DATE`. The JDBC PostgreSQL dialect casts in the select list only, so a delegated equality over `json` or `xml` fails for a missing operator, and `numeric` and `interval` compare by source semantics rather than as the emitted text. Omitting these types loses the coverage the PR feedback asks for.
- **Consequences:**
  - A delegated filter, join key, grouping key, or `MIN` or `MAX` operand on a converted column cannot use a source index on that column. The main cost case is a PostgreSQL `uuid` primary or foreign key: a point lookup becomes a sequential scan. A PostgreSQL expression index on the rendered form, such as `CREATE INDEX ON t ((CAST(uid AS text)))`, restores index use. Character columns other than a PostgreSQL `character` column wider than 2000 characters, and integer, constrained `numeric`, date, and timestamp columns, render bare and keep index use.
  - A virtual schema created before this release gains column notes only after `ALTER VIRTUAL SCHEMA ... REFRESH`.
  - `vs-adapter/pushdown-filter`'s rule against compensating syntax covers comparison rules, collation and escape, not the read expression.
- **Architecture:** § Components, § Data Flow, § Interfaces, § Constraints
- **Promotes to ADR:** yes

### [7] adbc-vs sets no source session time zone

- **Decision:** The adapter and the scan set no session time zone on a PostgreSQL or MySQL connection, and a time-zone-aware source timestamp renders bare. A delegated comparison of a PostgreSQL `timestamp with time zone` with a timestamp or a date follows the source session's `TimeZone`, and a MySQL `TIMESTAMP` is emitted and compared in the session `time_zone`.
- **Rationale:** The JDBC PostgreSQL and MySQL Virtual Schemas set no session time zone and no JDBC time-zone property (`BaseColumnMetadataReader`, the dialects, and both user guides). A session pin changes every session-dependent expression a source view holds, such as PostgreSQL `current_date`, for every query and not only for the mapped column. Criterion 2 applies: the absence binds every future connection setting. Criterion 4 applies: it rejects the alternatives below. ADR digest: no accepted ADR covers source session settings or time zones.
- **Alternatives:** Pinning the session to UTC at connect time is the pin this entry rejects. Rendering every `timestamp with time zone` reference as `"<column>" AT TIME ZONE 'UTC'` makes comparisons independent of the session zone, but costs index use on the timestamp columns that range filters use most.
- **Consequences:**
  - Under a PostgreSQL session `TimeZone` other than UTC, a delegated comparison of a `timestamp with time zone` column with a `TIMESTAMP` literal selects rows offset by the zone's UTC offset from the emitted UTC values. Setting the role's or the database's `TimeZone` to `UTC` removes the offset. `docs/type-mapping.md` states both.
  - In a MySQL session zone with daylight saving time, two instants in the hour the clocks repeat share one emitted wall-clock value, and a delegated comparison with that value selects one of the two instants.
- **Architecture:** § Constraints
- **Promotes to ADR:** yes

### [8] Every bit width maps to BOOLEAN

- **Decision:** PostgreSQL `bit(n)` and MySQL `BIT(n)` map to `BOOLEAN` for every n, read as true when any bit is set. PostgreSQL `bit varying` maps to `VARCHAR`, as the JDBC PostgreSQL Virtual Schema maps it.
- **Rationale:** The PR request says that `bit` and `BIT` map to `BOOLEAN`, and the JDBC Virtual Schemas map every width to `BOOLEAN`. Under [6], a delegated filter, grouping key, or aggregate over the column acts on its read expression, which is the boolean the scan emits, so the delegated result agrees with the emitted values.
- **Alternatives:** Mapping only width 1 to `BOOLEAN`, a wider PostgreSQL `bit(n)` to `VARCHAR(n)` holding its bit string, and a wider MySQL `BIT(n)` to the `DECIMAL` of its unsigned value. That mapping keeps every source value distinct, and the user rejected it in favor of the JDBC mapping.
- **Consequences:**
  - Two different values of a wider column read as one `true`: `BIT(6)` values 5 and 9 both emit `true`, as the JDBC MySQL integration test shows. `docs/type-mapping.md` lists this as a known limitation.
- **Promotes to ADR:** no

### [9] Constrained numeric keeps DECIMAL

- **Decision:** PostgreSQL `numeric(p,s)` with p at most 36 keeps mapping to `DECIMAL(p,s)`.
- **Rationale:** The PR request names the unconstrained `numeric` alone. `DECIMAL(p,s)` keeps numeric comparison, arithmetic, and the column types existing virtual schemas expose.
- **Alternatives:** The JDBC PostgreSQL Virtual Schema maps every `numeric` to `VARCHAR(2000000)`.
- **Promotes to ADR:** no

### [10] PostgreSQL arrays and MySQL binary, blob, and spatial columns stay omitted

- **Decision:** PostgreSQL `ARRAY` columns and MySQL `BINARY`, `VARBINARY`, `BLOB`-family, and spatial columns stay unmapped, as the JDBC Virtual Schemas leave them out.
- **Rationale:** The PR request names a target for each type it wants mapped, names none of these, and asks to align with the JDBC Virtual Schemas, which omit them. A binary value read as hex text doubles in length, so a `BLOB` value above 1000000 bytes exceeds Exasol's maximum `VARCHAR` of 2000000 characters and fails the whole query, and `MEDIUMBLOB` and `LONGBLOB` columns hold such values routinely.
- **Alternatives:** Mapping each to `VARCHAR(2000000)` read as `CAST(<column> AS TEXT)`, `HEX(<column>)`, or `ST_AsText(<column>)` through the read expressions of [6].
- **Consequences:**
  - Issue #17 lists arrays among the omitted PostgreSQL types, so closing it leaves arrays omitted. The implementing pull request's description states this.
  - Mapping these types later adds a read expression and needs no new mechanism.
- **Promotes to ADR:** no

## Review Findings

### [plan-review] Time-zone-aware timestamps map in UTC instead of staying omitted

- **Status:** Superseded by [pr-feedback] Align the type mapping with the JDBC Virtual Schemas
- **Finding:** `plan-reviewer` flagged that [4] omitted PostgreSQL `timestamp with time zone` and MySQL `TIMESTAMP` on a cost argument without a measurement.
- **Direction change:** Both types map to Exasol `TIMESTAMP`.
- **Promotes to ADR:** no

### [pr-feedback] Align the type mapping with the JDBC Virtual Schemas

- **Finding:** The PR conversation comment asks to align the PostgreSQL and MySQL mappings with the Exasol JDBC Virtual Schemas and to maximize type coverage, with named targets for each type, timestamp behavior and fractional-second precision as JDBC handles them, no MySQL character-set check, and an Exasol type reference in `specs/exasol-types.md`.
- **Direction change:** The interview rules "map only types with safe predicates" and "no per-column mechanism" yield to coverage: a type maps when it has an Exasol representation, a column records its source type for its read expression ([6]), and a remaining difference from Exasol's comparison is a stated limitation. The UTC session pin is removed ([7]). Timestamps declare their source precision, and `CHAR` joins the mapped set ([1]).
- **Promotes to ADR:** no

### [pr-feedback] Every bit width maps to BOOLEAN

- **Finding:** `plan-reviewer` flagged that [8] mapped only `bit(1)` and `BIT(1)` to `BOOLEAN`, against the PR request. Asked in this session, the user chose the JDBC mapping for every width and accepted that two source values can read as one `true`.
- **Direction change:** [8] maps every PostgreSQL `bit(n)` and MySQL `BIT(n)` to `BOOLEAN`, and `docs/type-mapping.md` states the lost distinction as a known limitation.
- **Promotes to ADR:** no

### [pr-feedback] MySQL CHAR maps to CHAR

- **Finding:** `plan-reviewer` flagged that [9] kept MySQL `CHAR(n)` as `VARCHAR(n)` on a reason that holds for `ENUM` and `SET` too. Asked in this session, the user chose `CHAR(n)`, as the JDBC MySQL Virtual Schema maps it and as this plan maps `ENUM` and `SET`.
- **Direction change:** MySQL `CHAR(n)` maps to `CHAR(n)`, and [9] covers constrained PostgreSQL `numeric` alone. An existing virtual schema keeps `VARCHAR(n)` for its MySQL `CHAR` columns until `ALTER VIRTUAL SCHEMA ... REFRESH`.
- **Promotes to ADR:** no

### [pr-feedback] A non-finite DOUBLE reaches Exasol unchanged

- **Finding:** `plan-reviewer` flagged that failing the scan on a `NaN` or infinite `DOUBLE` breaks queries over `double precision` columns that succeed today, and departs from a JDBC import. Asked in this session, the user chose to drop the failure.
- **Direction change:** The scan emits a `DOUBLE` value as it does today, and Exasol reads `NaN` as null, as in a JDBC import. The arrow-scan scenario, its coercion task, and its fixture column are removed.
- **Promotes to ADR:** no

### [plan-review] MySQL reads a grouped converted column through ANY_VALUE in the select list

- **Finding:** `plan-reviewer` flagged that MySQL rejects a select-list predicate over a key grouped by its read expression with error 1055, because MySQL matches a select-list expression against a `GROUP BY` expression only whole (ADR `fail-select-list-predicate-over-grouped-character-key`).
- **Direction change:** The MySQL dialect keeps grouping by the read expression and renders each select-list reference to a grouped column whose read expression is not the bare column, outside an aggregate, as `ANY_VALUE(<read expression>)`. Within a group that expression has the grouping value, and MySQL exempts `ANY_VALUE` from its full-group-by check. Grouping by the bare column was rejected, because a read expression that does not preserve equality, such as a multi-bit `BIT` read as `BOOLEAN` or `JSON` read as text, then groups differently from the emitted key. A derived table projecting the read expressions was rejected as a second statement shape for every grouped query. ADR `delegate-select-list-predicate-over-grouped-character-key` still holds: the predicate renders like every other select-list predicate, and only a converted column reference inside it gains `ANY_VALUE`.
- **Promotes to ADR:** no
