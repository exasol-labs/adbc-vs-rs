# Decisions: add-trino-support

## ADR: Trino's advertised set is PostgreSQL's 35 entries, and every absence is a framework decision

**ID:** trino-capability-set-postgresql-parity-floor
**Plan:** add-trino-support
**Status:** Accepted

### Context

Trino needed an advertised pushdown capability set. `drivers/postgresql.toml` already stated 35 entries independently rendered through `PostgreSqlDialect`, and every capability this release's vocabulary holds beyond that set is withheld by a framework or mission decision that binds every dialect identically, not by a limit in any one target's SQL. The question was how far Trino's own set should reach.

### Decision

`drivers/trino.toml` states the same 35 entries `drivers/postgresql.toml` states, written out independently and rendered entry by entry through `TrinoDialect`. Nothing is withheld from Trino for a reason about Trino's SQL. Every capability name this release's vocabulary holds that the set omits is withheld by a framework or mission decision the plan's design section names.

### Options Considered

| Option | Verdict |
|--------|---------|
| Match PostgreSQL's 35-entry set exactly | ✓ Chosen — states the floor "full pushdown coverage" requires, checkable rather than asserted |
| Advertise `JOIN_CONDITION_ALL` for Trino alone | ✗ Rejected — mission.md scopes non-equi joins out of the project for every dialect |
| Advertise `AGGREGATE_GROUP_BY_TUPLE` for Trino alone | ✗ Rejected here — needs no rendering change in any dialect, but lifting a release-wide boundary under cover of adding a driver was not this entry's call to make |
| Advertise `FN_AGG_SUM` for Trino alone | ✗ Rejected — `vs-adapter/pushdown-aggregation` withholds the summing aggregates for every dialect for reasons that are not about any target's SQL |

### Consequences

The 35 entries Trino and PostgreSQL share is an agreement between two independent statements rather than a rule, and a test asserts both the agreement and any deliberate difference, so a later change to either set fails rather than being absorbed. Treating "a framework decision withholds it" as settling the question rather than as naming a decision the requester could still be asked about was this entry's own limit. Decision `trino-exceeds-parity-with-group-by-tuple` corrects it for one entry.

## ADR: The recorded source qualifier becomes an ordered segment list

**ID:** qualifier-as-ordered-segment-list
**Plan:** add-trino-support
**Status:** Accepted

### Context

A Trino table is named `catalog.schema.table`, and its `information_schema` is scoped to one catalog. The existing `adapterNotes` recorded one namespace string, and every dialect rendered it as one quoted identifier, which cannot represent two segments without being split at a character a catalog or schema name may contain.

### Decision

`MetadataSource::namespace` returns an ordered list of name segments, `SourceTable` carries the list, and each dialect's table qualifier joins the quoted segments. `AdapterNotes` accepts a JSON string or a JSON array on deserialization, and serializes one segment as a string and more as an array, so MySQL and PostgreSQL's generated statements and recorded notes stay byte-identical.

### Options Considered

| Option | Verdict |
|--------|---------|
| Ordered segment list, one field serialized as a string or an array by count | ✓ Chosen — the smallest honest representation, and answers the next nested technology (Snowflake) as well as this one |
| Record `<catalog>.<schema>` as one string the Trino dialect splits on a dot | ✗ Rejected — a catalog or schema name may contain a dot, making the split unrecoverable |
| Qualify with the schema alone and rely on the connection's selected catalog | ✗ Rejected — puts state into name resolution the generated statement does not carry and `EXPLAIN VIRTUAL` does not show |
| Add a second `adapterNotes` field for the catalog | ✗ Rejected — two records of one fact can disagree |

### Consequences

How many segments a technology's qualifier carries becomes a declared value in that technology's own TOML file, generated into the compiled-in tables the way its capability list already is, because the address check runs before a connection exists while the metadata source answers after one does. The address's path element count is checked against that value, narrowing released behavior: before this release `mysql://host:3306/db/extra` took `db/extra` as the database and failed later at the source; both too few and too many elements now fail with a named error. Notes Exasol persisted before this release parse unchanged, so no virtual schema needs recreating.

## ADR: The Trino dialect renders no collation suffix

**ID:** trino-no-collation-suffix
**Plan:** add-trino-support
**Status:** Accepted

### Context

MySQL and PostgreSQL each render a collation suffix somewhere a target's default comparison disagrees with Exasol's. Trino needed the same question answered: where its default character comparison and ordering already match Exasol's, rendering a suffix would be needless, but the answer had to be measured rather than assumed.

### Decision

`TrinoDialect` renders no collation anywhere: not on an equality operand, an ordering operand, a `LIKE` operand, an `IN` operand, a `GROUP BY` key, a `DISTINCT` operand, a `MIN` or `MAX` operand, or a join key. `vs-adapter/trino-collation` states the rule as a measured claim about Trino's own comparison rather than as an absence.

### Options Considered

| Option | Verdict |
|--------|---------|
| Render no collation suffix | ✓ Chosen — Trino's documentation states `varchar` comparison and ordering are Unicode code-point order, which is how Exasol answers both questions, confirmed by gate 2.3(a) against a live Trino before the rule was advertised |
| Render an ordering collation, mirroring PostgreSQL | ✗ Rejected — expected unavailable: `COLLATE` appears in no part of Trino's documented SQL grammar |
| Refuse character operands outright | ✗ Rejected — same reason, and unnecessary given the rule Trino's own comparison already provides |

### Consequences

Trino is the only one of the three dialects whose grouping key and select-list occurrence need no coordination and whose operands keep the source's own execution options. MySQL's composed-shape exclusion has no analogue here, for the same reason it has none for PostgreSQL. A Trino deployment that does have a collation mechanism of its own re-decides this entry rather than shipping it unchanged.

## ADR: Trino's connector-pushdown condition is stated and not checked

**ID:** trino-connector-pushdown-condition-unchecked
**Plan:** add-trino-support
**Status:** Accepted

### Context

A Trino catalog whose connector pushes a string comparison down to a remote database applies that database's own collation rather than Trino's, which can disagree with the no-suffix rule this release advertises. Trino documents the hazard per connector and offers no engine-level guarantee, and no catalog Trino exposes reports the condition to a client.

### Decision

adbc-vs runs no check for this condition, refuses no catalog over it, and renders the same statement whatever catalog a virtual schema is over. `vs-adapter/trino-collation` states the condition and names who owns it, and mission.md's Constraints carries it beside the two existing per-technology entries.

### Options Considered

| Option | Verdict |
|--------|---------|
| State the condition without enforcing it | ✓ Chosen — no catalog Trino exposes reports which comparisons a connector pushes down or which collation a remote source applies, so no query a precondition check could run exists |
| Refuse a virtual schema over a catalog whose connector reaches a remote database | ✗ Rejected — refuses what Trino exists for |
| Withhold the affected predicate and aggregate capabilities from Trino entirely | ✗ Rejected — costs every Trino deployment the coverage this plan delivers for a condition the operator configures and can avoid |

### Consequences

Trino is the first supported technology whose source precondition is unenforceable, where MySQL's collation floor and PostgreSQL's encoding requirement are both checked. The asymmetry is stated in the spec rather than left to be inferred from an empty check. The measurements this plan records are taken against a catalog that reaches no remote system, so what they measure is Trino's own comparison alone.

## ADR: Every Trino LIKE renders an explicit ESCAPE clause, and an inadmissible escape sequence fails in the dialect

**ID:** trino-like-explicit-escape-clause
**Plan:** add-trino-support
**Status:** Accepted

### Context

Exasol's own default treats a backslash as escaping the character after it, and a delegated filter is never re-applied. MySQL and PostgreSQL apply that same default, so their dialects render a `LIKE` pattern unchanged. Trino's documentation states that a wildcard is matched literally only by specifying an `ESCAPE` character, so a dialect passing a pattern through unchanged would let `\%` match a backslash followed by anything, which is wrong rows rather than an error.

### Decision

The Trino dialect renders an `ESCAPE` clause for every `LIKE`, carrying the request's own `escapeChar` when it has one and a backslash when it has none. A pattern whose escape character precedes a character the target does not admit fails with a named error rather than reaching the source. `vs-adapter/pushdown-filter`'s framework rule is restated: reproducing Exasol's escape handling is per dialect, and two targets need no clause because their default already matches Exasol's.

### Options Considered

| Option | Verdict |
|--------|---------|
| Render an explicit `ESCAPE` clause on every `LIKE`, with a rejection for an inadmissible sequence | ✓ Chosen — Trino defines no default escape character, so a supplied clause is required rather than optional |
| Pass the pattern through unchanged, as the other two dialects do | ✗ Rejected — matches a different set of rows than Exasol asked for |
| Drop or rewrite the inadmissible escape sequence | ✗ Rejected — changes which rows come back |
| Emit the statement and let Trino reject it | ✗ Rejected — the failure would then reach an operator as the source's own message about a pattern the operator never typed |

### Consequences

`vs-adapter/pushdown-filter` gains a scenario for the exclusion, bound to the dialects whose targets have the restriction rather than to every dialect. Gate 2.3(c) measured that Trino is one of them, so the resulting rule is a spec correction rather than a code special case.

## ADR: Trino advertises AGGREGATE_GROUP_BY_TUPLE beyond PostgreSQL parity, and nothing else

**ID:** trino-exceeds-parity-with-group-by-tuple
**Plan:** add-trino-support
**Status:** Accepted
**Supersedes:** trino-capability-set-postgresql-parity-floor

### Context

The interview asked for a capability set "matching, and where Trino's SQL surface allows, exceeding, PostgreSQL's capability set." The plan initially answered only the matching half, declining the exceeding half by planner fiat under a Non-Goal, even though its own analysis had already found that `vs-adapter/pushdown-dialect` records `AGGREGATE_GROUP_BY_TUPLE` needing no rendering change in any dialect: each already renders every grouping column a request carries. Plan review escalated the dropped half as a scope-reduction blocker, because moving a release-wide scope boundary is the requester's judgement rather than the planner's.

### Decision

`drivers/trino.toml` states the 35 entries `drivers/postgresql.toml` states plus `AGGREGATE_GROUP_BY_TUPLE`, 36 in all. The MySQL and PostgreSQL dialects keep their sets unchanged. `AGGREGATE_GROUP_BY_EXPRESSION` and `AGGREGATE_HAVING` stay withheld for every dialect, Trino included, with no rendering work scheduled for either.

### Options Considered

| Option | Verdict |
|--------|---------|
| Lift `AGGREGATE_GROUP_BY_TUPLE` for Trino alone | ✓ Chosen by the requester — the one entry the reviewer found free: it costs no dialect a rendering change |
| Keep exact PostgreSQL parity | ✗ Rejected — this is what the superseded entry chose, and what the reviewer flagged as a dropped half of the requester's ask |
| Lift `AGGREGATE_GROUP_BY_EXPRESSION` and `AGGREGATE_HAVING` for Trino beside it | ✗ Rejected — neither is confirmed rendering-free; Trino executes both, but lifting either would buy rendering work in three dialects rather than none |
| Lift `AGGREGATE_GROUP_BY_TUPLE` for every dialect | ✗ Rejected — widening this release's shared floor is a scope change for MySQL and PostgreSQL that nothing about adding a Trino driver motivates |

### Consequences

A Trino-backed virtual schema delegates a multi-column `GROUP BY` where a MySQL- or PostgreSQL-backed one withdraws it and Exasol groups locally, a throughput difference rather than a correctness one, and the first capability difference in this release that is not about a target's SQL. `vs-adapter/trino-pushdown`'s comparison scenario asserts one named difference rather than equality, and `capabilities_tests.rs::trino_capability_set_is_exact` pins the 36-entry set against an independently written constant. `vs-adapter/pushdown-aggregation` and `vs-adapter/pushdown-dialect` now state Trino as the one exception to a withholding they previously stated for every dialect. Gate 2.3(i) confirmed Trino executes the rendered multi-column form before the entry shipped.

## ADR: The Trino credential travels in the driver's separate username and password options

**ID:** trino-credential-via-separate-username-password-options
**Plan:** add-trino-support
**Status:** Accepted

### Context

The plan's initial design assumed the ADBC Trino driver accepted one `uri` option and documented no separate credential option, so the credential was percent-encoded into the connection URI's userinfo. Gate 2.1 measured the driver itself, at version `0.5.3`, rather than trusting its documentation, and found that assumption false: the library's option table registers `uri`, `username`, `password`, `passwordSet`, `SSL`, `SSLCert`, and `SSLCertPath`, and the separate options override a userinfo the URI carries, field by field.

### Decision

The Trino arm of the shared open path sets three database options: `uri`, `username`, and `password`. It strips the userinfo component from the address and passes every other component through byte for byte. The resolved user and password travel as the two credential options, byte for byte, with no percent-encoding, escaping, or transcoding applied anywhere, the shape `scan-udf/mysql-scan` already states. Redaction strips the password's verbatim bytes and no second spelling, because no second spelling exists.

### Options Considered

| Option | Verdict |
|--------|---------|
| Separate `username`/`password` database options, URI stripped of userinfo | ✓ Chosen — the requester's own answer to the question gate 2.1 raised: "username / password are encoded in the connection object; they should not part of the URI and passed separately" |
| Percent-encode the whole credential into the URI userinfo | ✗ Rejected — this is what the superseded entry chose from the driver's documentation, and what gate 2.1 contradicted |
| Carry the credential in both the userinfo and the separate options with a stated precedence | ✗ Rejected — doubles the disclosure surface and buys no authentication result the separate options do not already give |
| Pass the credential as a query parameter | ✗ Rejected — not where the driver documents a credential |

### Consequences

The credential reaches no value any parser reads, so no password byte needs escaping and no password character can change where the URI's authority ends; the guarantee is structural rather than a property of an encoding. The one disclosure path gate 2.1 measured, a database-creation failure quoting the URI it was handed, is closed by the URI holding no credential; the strip stays as defense in depth because nothing binds the driver to omit a value it received as an option. The shared open path now carries two credential shapes rather than three: separate options for MySQL and Trino, a connection string for PostgreSQL. One corollary follows: Trino 483 authenticates no password holding a character outside the ASCII range through this mechanism, because the driver sends the password's UTF-8 bytes and the server decodes an HTTP Basic credential as ISO-8859-1, RFC 7617's default charset. `scan-udf/trino-scan` states the limit as a Background fact with the gate's evidence, and task 9.3 carries it into `specs/mission.md`.
