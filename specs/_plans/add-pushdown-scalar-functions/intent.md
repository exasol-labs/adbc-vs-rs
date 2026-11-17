# Intent: add-pushdown-scalar-functions

## Interview

- Q: Split issue #21 into separate plans? → A: All four groups in one plan
- Q: CAST pushdown? → A: Defer CAST to a follow-up plan
- Q: Advertise AGGREGATE_GROUP_BY_EXPRESSION? → A: Yes, all dialects
- Q: Keep the `+ 0` trick that makes Exasol compute an operand? → A: Drop it, push arithmetic down, fix the docs
- Q: Arithmetic semantics? → A: Match Exasol per operand type (date plus a number is day arithmetic, no early overflow)
- Q: String functions on CHAR padding and non-ASCII case? → A: Follow the source
- Q: MySQL string functions, which the MySQL JDBC VS withholds? → A: Push down, mapped to character semantics (LENGTH as CHAR_LENGTH)
- Q: MySQL arithmetic and DAY, which the MySQL JDBC VS withholds? → A: Push down with per-type rendering
- Q: TIMESTAMP minus TIMESTAMP yields INTERVAL, which EMITS cannot declare? → A: Advertise FN_SUB, fail the select-list case with a named error, new ADR
- Q: Two-key GROUP BY on MySQL and PostgreSQL, which withhold AGGREGATE_GROUP_BY_TUPLE? → A: Follow-up plan
- Q: Empty string from a delegated string function, which Exasol reads as NULL? → A: Map it to NULL at the source
- Q: MySQL two-argument TRIM? → A: Mirror the MySQL JDBC VS, withhold FN_TRIM on MySQL
- Q: Revisit the select-list TIMESTAMP difference error? → A: Compute it in Exasol around the scan, no breaking change, no ADR

## Directions

- [D1] The query model derives a result type class (exact numeric with its scale, double, character, boolean, date, timestamp, interval) for every scalar expression from its function and arguments, using Exasol's scale rule (`+` and `-` take the larger scale, `*` adds them); the request carries no type for a nested node, and per-type rendering needs one.
- [D2] Exact-numeric `+`, `-`, and `*` render through the target's widest exact decimal (MySQL `DECIMAL(65,s)` with `s` capped at MySQL's 30, PostgreSQL `numeric`, Trino `DECIMAL(38,s)`); an integer-typed source operation overflows where Exasol returns a value, and an operand whose scale exceeds the cap renders without the cast.
- [D3] An operand Exasol converts implicitly converts at the source as Exasol converts it under its default formats: a character operand of arithmetic as the `SUM` operand conversion does, a number or date operand of a string function to its text form, and a character operand of a date part to a date; a session that changes `NLS_DATE_FORMAT` or `NLS_TIMESTAMP_FORMAT` is not followed, which `docs/pushdown.md` states.
- [D4] `SUBSTR` position and length are normalized to Exasol's rule in every dialect (position 0, negative, beyond the end); "follow the source" covers padding and locale, not position arithmetic.
- [D5] A `TIMESTAMP` difference renders as each target's own interval (MySQL a microsecond count through `TIMESTAMPDIFF`), a `DATE` operand of a mixed subtraction reads as midnight of that day, and an interval operand of `ADD` or `SUB` adds that interval; Exasol delegates each once `FN_ADD` and `FN_SUB` are advertised.
- [D6] The two-key `GROUP BY YEAR(), MONTH()` of issue #21 stays undelegated for MySQL and PostgreSQL in this plan, because both keep withholding `AGGREGATE_GROUP_BY_TUPLE`; the follow-up plan `change-pushdown-group-by-tuple` widens it.
- [D7] A select-list entry declared `INTERVAL DAY TO SECOND` is emitted by the scan as a `DECIMAL` count of seconds and converted in an outer Exasol `SELECT` through `NUMTODSINTERVAL`, cast to the declared type; one rule covers a bare, nested, aggregated, or grouped interval, where an outer subtraction of emitted timestamps covers only the bare one.
- [D8] `docs/pushdown.md` names `SUM(CAST("price" AS DOUBLE))` as the way to make Exasol compute a floating-point sum, replacing `+ 0`, because no dialect advertises `FN_CAST`; measured on the test Exasol, `SUM("price" / 1)` is simplified to `SUM("price")` and delegated, while the `CAST` stays in Exasol.
- [D9] The pushdown component owns the interval transport: a select-list entry declared `INTERVAL DAY(p) TO SECOND(f)` renders, outermost, as the source's seconds count of the interval expression, declared `DECIMAL(p + 5 + f, f)` in the `EMITS` clause, which holds every value of the declared interval, and the outer `SELECT` applies the inverse; each dialect supplies only its seconds syntax, because the Arrow coercion of the scan has no interval-to-decimal cast.
