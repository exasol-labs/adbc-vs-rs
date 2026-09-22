# Feature: postgresql_pushdown

States the capability set the PostgreSQL dialect advertises and the PostgreSQL SQL it renders for each delegated fragment.

<!-- DELTA:CHANGED -->
## Background

- The PostgreSQL dialect states its own advertised capability set and derives it from no other dialect's. `vs-adapter/pushdown-dialect` owns the all-or-nothing negotiation rule — Exasol delegates a fragment fully once it receives the capability and never re-applies it — and the invariant that every advertised capability has a rendering path in the dialect that advertises it.
- The set holds one capability the MySQL dialect withholds, `JOIN_TYPE_FULL_OUTER`, and that is the one capability those two targets genuinely disagree on. MySQL implements no `FULL OUTER JOIN` and emulating one requires the union rewrite mission.md places out of scope; PostgreSQL implements the keyword directly. Every other capability is advertised or withheld identically by the two dialects, and the two sets are stated independently rather than one derived from the other. The Trino dialect's set holds every entry of this one plus `AGGREGATE_GROUP_BY_TUPLE`, which this dialect withholds by the framework scope decision `vs-adapter/pushdown-aggregation` owns rather than by anything about PostgreSQL's SQL; that third set is an independent statement whose overlap with this one is an outcome rather than a rule binding either.
- Advertising `JOIN_TYPE_FULL_OUTER` rests on PostgreSQL accepting a full join whose condition is an equality. PostgreSQL rejects a full join whose condition is neither merge-joinable nor hash-joinable — measured, `ON "l"."k" < "r"."k"` is refused on 18.6 — but only `JOIN_CONDITION_EQUI` is advertised and an equality over one common type is both, so the restriction does not bind. The restriction is PostgreSQL's own: neither of the other targets has it.
- `FN_AGG_SUM`, `FN_AGG_SUM_DISTINCT`, `FN_AGG_AVG`, and `FN_AGG_AVG_DISTINCT` stay withheld, despite PostgreSQL's exact-numeric `sum` being exact and order-independent. A capability is advertised once per session and admits every numeric operand, including `double precision`, whose sum is order-dependent because floating-point addition is not associative and whose accumulation order is not even stable between two runs under a parallel plan. `vs-adapter/pushdown-aggregation` owns the withholding, for this dialect and for every other.
- Identifier quoting is the double quote, and an embedded double quote is escaped by doubling it. Every identifier is quoted unconditionally, because an unquoted PostgreSQL identifier folds to lower case and a mixed-case source name would then resolve to an object that does not exist.
- The capability list is data rather than code. `crates/adbc-vs-scan/drivers/postgresql.toml` states it beside the driver package row, and the build generates the table the dialect returns, so adding or withdrawing a capability is a data edit.
- A string literal renders as an escape-string constant, `E'<body>'`, with a backslash emitted as two backslashes and a single quote as two single quotes. The `E` prefix selects backslash-escape processing whatever `standard_conforming_strings` is set to, and doubling the quote rather than backslash-escaping it keeps the literal independent of `backslash_quote`. The form is rendered unconditionally rather than only for a value that needs it, so no literal can change the shape of the generated statement.
- The hex route the MySQL dialect uses is rejected here for typing rather than verbosity. A decoded hex literal — `convert_from(decode('<hex>','hex'),'UTF8')` — is typed `text`, and PostgreSQL applies no implicit coercion from `text` to a numeric or temporal column, so comparing one against a mapped `DECIMAL` or `DATE` column fails with an unknown-operator error where MySQL would silently coerce. `E'<body>'` yields an untyped literal PostgreSQL resolves from the other operand.
- Date and timestamp literals render as typed literals, `DATE '...'` and `TIMESTAMP '...'`, so the source coerces no string by its own rules. `predicate_notequal` renders as `<>` and every nested predicate is parenthesized, both of which are the standard forms every other dialect also renders.
- Collation placement is owned by `vs-adapter/postgresql-collation`: which operands carry `COLLATE "C"` and which carry nothing is that feature's rule, not this one's. The mapped source types every rendered operand can hold are owned by `vs-adapter/postgresql-type-mapping`.
<!-- /DELTA:CHANGED -->

## Scenarios

### Scenario: Join capabilities add the full outer join

* *GIVEN* a virtual schema whose driver is `postgresql`
* *WHEN* Exasol sends a `getCapabilities` request
* *THEN* the `capabilities` array MUST contain `JOIN`, `JOIN_TYPE_INNER`, `JOIN_TYPE_LEFT_OUTER`, `JOIN_TYPE_RIGHT_OUTER`, `JOIN_TYPE_FULL_OUTER`, and `JOIN_CONDITION_EQUI`
* *AND* it MUST NOT contain `JOIN_CONDITION_ALL`
* *AND* `JOIN_TYPE_FULL_OUTER` MUST be the only capability present here and absent from the MySQL dialect's set
