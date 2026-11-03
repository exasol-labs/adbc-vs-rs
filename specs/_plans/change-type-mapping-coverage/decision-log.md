# Decision Log: change-type-mapping-coverage

## Interview

- Q: Type-set scope (stay in five types / add BOOLEAN / add BOOLEAN and TIMESTAMP WITH LOCAL TIME ZONE)? → A: Map the 7 proposed types from the issue, so BOOLEAN and TIMESTAMP WITH LOCAL TIME ZONE join the set
- Q: MySQL scope? → A: Small ints + FLOAT, TEXT family, unsigned integers, TIMESTAMP/JSON/TIME/YEAR
- Q: Unconstrained numeric and character(n)? → A: numeric -> DOUBLE, character(n) stays omitted
- Q: Predicates for types with differing comparison semantics? → A: Map only types with safe predicates, no new per-column gating mechanism
- Q: Docs? → A: Tabular type-mapping overview per dialect (PostgreSQL, MySQL, Trino) in docs/
- Note: Issue is exasol-labs/adbc-vs-rs#17. "Only types with safe predicates" constrains the 7 issue types and the MySQL groups: the planner decides per type, with measured evidence, and omits a type whose delegated comparison cannot be made safe. Split check: one plan, per the request that bundles PostgreSQL, MySQL, and docs.

## Design Decisions

### [1] A source column may map to Exasol BOOLEAN

- **Decision:** The mapped-type set is `DECIMAL`, `VARCHAR`, `DATE`, `TIMESTAMP`, `DOUBLE`, and `BOOLEAN`, and PostgreSQL `boolean` maps to `BOOLEAN`. `SELECTLIST_EXPRESSIONS` stays advertised, and the set an `EMITS` column may declare is the mapped set plus Exasol `CHAR` for a virtual schema release 0.1 created.
- **Rationale:** The superseded ADR kept `BOOLEAN` out of the mapped set because no source column produced it. PostgreSQL `boolean` now does, and its delegated comparisons, grouping, and joins agree with Exasol's once `MIN` and `MAX` render as `bool_and` and `bool_or`. Criterion 1 applies: the set binds the type model, the query parser, every dialect, and the scan. Criterion 2 applies: the pushdown closure argument rests on the set. ADR digest: `advertise-selectlist-expressions-with-boolean-emits` states that the mapped type gains no `BOOLEAN` variant, so this entry supersedes it and keeps its `SELECTLIST_EXPRESSIONS` decision.
- **Supersedes:** advertise-selectlist-expressions-with-boolean-emits
- **Architecture:** no change: `specs/architecture.md` names no mapped-type set
- **Promotes to ADR:** yes

### [2] The PostgreSQL dialect renders VARCHAR and DOUBLE column references as their declared type

- **Decision:** The PostgreSQL dialect renders a column reference converted to the PostgreSQL type that holds its declared Exasol type's value, so the source compares the value the scan emits. The declared type selects the conversion, and a declared type whose comparison already agrees with Exasol's renders the column without one.
- **Rationale:** An unconstrained `numeric`, `uuid`, `json`, and `jsonb` compare differently from their declared Exasol type when PostgreSQL compares them as themselves. The conversion makes PostgreSQL compare the value the scan emits, so these types map without a per-column mechanism, which the interview excluded. For `double precision`, `text`, and `character varying` the conversion is a no-op or a binary-compatible relabel, so their source index use does not change. Criterion 2 applies: future PostgreSQL mappings rely on the rule. Criterion 4 applies: it rejects omission and per-column notes. ADR digest: `character-comparisons-follow-source-collation` still holds, because the text conversion keeps the column's collation and renders no `COLLATE`.
- **Alternatives:** Omitting the four types loses issue-requested types that the conversion makes safe. Per-column `adapterNotes` that drive a conversion only where needed are the per-column mechanism the interview excluded, and the declared type already carries the needed distinction. Converting every reference, including `DECIMAL`, `DATE`, and `TIMESTAMP`, costs integer columns their index use for no correctness gain.
- **Consequences:**
  - A delegated filter, join key, grouping key, or `MIN` or `MAX` operand on a `real`, unconstrained `numeric`, `uuid`, `json`, or `jsonb` column cannot use a source index on that column. The main cost case is a `uuid` primary or foreign key: a point lookup becomes a sequential scan, and a delegated join on `uuid` keys cannot use an index nested-loop join. A PostgreSQL expression index on the rendered form, such as `CREATE INDEX ON t ((CAST(uid AS text)))`, restores index use.
  - `vs-adapter/pushdown-filter`'s rule against compensating syntax narrows to comparison rules: collation and escape.
- **Architecture:** § Constraints
- **Promotes to ADR:** yes

### [3] MySQL text types map although PAD SPACE collations group by a prefix

- **Decision:** `TINYTEXT`, `TEXT`, `MEDIUMTEXT`, and `LONGTEXT` map to `VARCHAR`. The `max_sort_length` prefix grouping under a `PAD SPACE` collation is stated as a property of that collation.
- **Rationale:** The prefix grouping binds a long `VARCHAR` as well, and MySQL's default `utf8mb4_0900` collations are `NO PAD`, which `max_sort_length` does not affect (MySQL 8.4 reference, `max_sort_length`). Omitting the text types would not remove the behavior from `VARCHAR`.
- **Alternatives:** Mapping a text column only under a `NO PAD` collation treats text and `VARCHAR` differently for one collation property. Extending that check to `VARCHAR` would omit columns that existing virtual schemas expose.
- **Promotes to ADR:** no

### [4] No source column maps to TIMESTAMP WITH LOCAL TIME ZONE

- **Decision:** No source column maps to Exasol `TIMESTAMP WITH LOCAL TIME ZONE`. This departs from the interview answer that `TIMESTAMP WITH LOCAL TIME ZONE` joins the set.
- **Rationale:** Exasol reads and writes a `TIMESTAMP WITH LOCAL TIME ZONE` value in the Exasol session time zone and compares it against `TIMESTAMP` and `DATE` in that zone (Exasol data type details; changelog 15053). A source session follows that zone only if a mechanism carries the Exasol session time zone to the source for each query, and this release has none. How a UDF emits a `TIMESTAMP WITH LOCAL TIME ZONE` value is also not documented. The interview rule that an unsafe type stays omitted decides the target type. Entry [5] maps the time-zone-aware source types to `TIMESTAMP` instead.
- **Alternatives:** Mapping `timestamptz` to `TIMESTAMP WITH LOCAL TIME ZONE` leaves a comparison against a `TIMESTAMP` or `DATE` operand dependent on two session zones, and leaves the emission semantics unknown.
- **Consequences:** MySQL `JSON`, `TIME`, and `YEAR` also stay omitted, for the reasons the `vs-adapter/mysql-type-mapping` delta states. The evidence for those three is the MySQL 8.4 reference, not a measurement. Together with this entry they are the interview-scope types this plan does not deliver.
- **Promotes to ADR:** no

### [5] Time-zone-aware source timestamps map to TIMESTAMP in UTC under a pinned session

- **Decision:** PostgreSQL `timestamp with time zone` and MySQL `TIMESTAMP` map to Exasol `TIMESTAMP` holding the UTC wall-clock value, and every PostgreSQL and MySQL source connection pins its session time zone to UTC when it opens. This departs from the interview in the target type only: the interview named `TIMESTAMP WITH LOCAL TIME ZONE` as the type joining the set, and these source types map to `TIMESTAMP`.
- **Rationale:** With the session pinned to UTC, PostgreSQL reads a `timestamp` or `date` operand compared against a `timestamptz` in UTC, and MySQL converts a stored `TIMESTAMP` from UTC to UTC, which is one-to-one. A delegated comparison, grouping key, and join key therefore act on the UTC wall-clock value the scan emits. The pin is session state set at connect time, so it needs no per-column mechanism, which the interview excluded. Criterion 1 applies: the pin lives in `connection` in `adbc-vs-scan`, and the adapter's type mappings and the scan's coercion depend on it. Criterion 4 applies: it rejects omission and the `TIMESTAMP WITH LOCAL TIME ZONE` target. ADR digest: no accepted ADR covers source session settings or time zones.
- **Alternatives:** Omitting both types loses the `timestamptz` columns that Rails, Django, Prisma, and Hibernate create by default (issue #17). Leaving the source session at its server default makes PostgreSQL read a compared literal in that zone, and makes MySQL return one wall-clock value for two instants across a daylight-saving transition.
- **Consequences:**
  - A source view or expression that depends on the session time zone, such as PostgreSQL `current_date` or a `timestamptz` converted to `date`, evaluates in UTC when read through adbc-vs.
  - If a differential E2E test shows a pinned-UTC comparison diverging from Exasol, that type stays omitted with the measured divergence as its reason.
- **Architecture:** § Constraints
- **Promotes to ADR:** yes

## Review Findings

### [plan-review] Time-zone-aware timestamps map in UTC instead of staying omitted

- **Finding:** `plan-reviewer` flagged that [4] omitted PostgreSQL `timestamp with time zone` and MySQL `TIMESTAMP` on a cost argument without a measurement, although its own alternative, a UTC-pinned session, makes both comparisons agree with Exasol's.
- **Direction change:** Both types map to Exasol `TIMESTAMP` holding the UTC wall-clock value, and the `connection` component pins the PostgreSQL and MySQL session time zone to UTC. [4] keeps only the `TIMESTAMP WITH LOCAL TIME ZONE` omission, and [5] records the mapping. A type returns to omission only if a measurement on the test stack shows a divergence.
- **Promotes to ADR:** no
