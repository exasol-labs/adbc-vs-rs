# Decisions: change-type-mapping-coverage

## ADR: One set of seven Exasol types serves mapping and EMITS

**ID:** one-exasol-type-set-serves-mapping-and-emits
**Plan:** change-type-mapping-coverage
**Status:** Accepted
**Supersedes:** advertise-selectlist-expressions-with-boolean-emits

### Context

PostgreSQL `boolean` and `bit` and MySQL `BIT` produce `BOOLEAN`, and PostgreSQL `character(n)` and MySQL `CHAR`, `ENUM`, and `SET` produce `CHAR`. The set a column may map to and the set an `EMITS` column may declare are therefore equal.

### Decision

A source column may map to `DECIMAL`, `VARCHAR`, `CHAR`, `DATE`, `TIMESTAMP(p)`, `DOUBLE`, or `BOOLEAN`, and an `EMITS` column may declare exactly the same set, modeled as one type. `SELECTLIST_EXPRESSIONS` stays advertised.

## ADR: A column records its source type, and the technology's type mapping names its read expression

**ID:** column-records-source-type-and-type-mapping-names-read-expression
**Plan:** change-type-mapping-coverage
**Status:** Accepted

### Context

Several source types share a declared Exasol type with a source type that needs no conversion, for example PostgreSQL `money` and `real`, or MySQL `YEAR` and `DATE`. Only the source type tells them apart.

### Decision

A column that needs a conversion is converted in every position the query references it, keyed on its recorded source type, rather than in the select list only or keyed on its declared Exasol type. The technology's type mapping owns both the declared type and the read expression, so the two cannot disagree.

### Options Considered

| Option | Verdict |
|--------|---------|
| Key the conversion on the declared Exasol type | Rejected: it cannot tell `money` from `real`, `bit` from `boolean`, `TIME` from `DATETIME`, or `YEAR` from `DATE` |
| Convert in the select list only, as the JDBC PostgreSQL dialect does | Rejected: a delegated equality over `json` or `xml` fails for a missing operator, and `numeric` and `interval` compare by source semantics |
| Omit the types that need a conversion | Rejected: it loses the coverage the Exasol JDBC Virtual Schemas provide |

### Consequences

- A delegated filter, join key, grouping key, or `MIN` or `MAX` operand on a converted column cannot use a source index on that column. A PostgreSQL `uuid` key lookup becomes a sequential scan until an expression index on the rendered form exists.
- A virtual schema created before this release gains column notes only after `ALTER VIRTUAL SCHEMA ... REFRESH`.
- The rule against compensating syntax in `vs-adapter/pushdown-filter` covers comparison rules, not the read expression.
