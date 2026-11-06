# Decision Log: change-pushdown-aggregation-sum-avg-having

## Interview

- Q: Which SUM/AVG option from issue #19? → A: Option 1, advertise SUM/AVG and document floating-point order dependence.
- Q: Scope? → A: One plan covering HAVING and SUM/AVG (all of issue #19).

## Design Decisions

### [1] Delegate SUM and AVG, and let a floating-point sum follow the source's order

- **Decision:** Every dialect advertises `FN_AGG_SUM`, `FN_AGG_SUM_DISTINCT`, `FN_AGG_AVG`, and `FN_AGG_AVG_DISTINCT`. A delegated sum over a floating-point operand is the source's double-precision sum in the order the source chooses, and the user documentation states that the result can differ from Exasol's. A capability stays advertised only when its dialect renders it for every operand type it admits: no renderer refuses one operand type of an advertised capability.
- **Rationale:** Rule-2 criteria 2 and 4: the rule binds every dialect and every later aggregate, and it rejects two plausible alternatives. ADR digest: `capability-negotiation-all-or-nothing` withholds the four capabilities; this entry supersedes it and keeps its rule against a render-time refusal. The interview chose option 1 of issue #19. Over an exact-numeric operand the delegated value equals Exasol's, so the accepted difference is limited to floating-point operands.
- **Alternatives:** Keep the four capabilities withheld (issue option 3): every `SUM` and `AVG` keeps fetching every row, 1,000,000 rows for a five-group `SUM` in the issue's evaluation. Delegate only an exact-numeric operand (issue option 2): Exasol decides delegation by capability alone, so the adapter would have to refuse a `DOUBLE` operand at render time, and Exasol does not compute a fragment itself after the adapter errors.
- **Consequences:**
  - `AVG` renders as a quotient of `SUM` and `COUNT` ([2]), and two dialects widen the `SUM` operand ([3]).
  - A `SUM` or `AVG` over a `DOUBLE` column can return another value than the previous release returned, and two runs can differ.
  - A `NaN` in a PostgreSQL or Trino floating-point column makes the delegated `SUM` and `AVG` null.
  - Each dialect converts a `BOOLEAN` or character `SUM` and `AVG` operand as Exasol converts it (Review Findings, `[plan-review] Exasol delegates SUM and AVG over BOOLEAN and character operands`).
- **Supersedes:** capability-negotiation-all-or-nothing
- **Architecture:** § Constraints
- **Promotes to ADR:** yes

### [2] Render AVG as the double-precision quotient of SUM and COUNT

- **Decision:** Every dialect renders `AVG(x)` as `(CAST(SUM(x) AS <double type>) / COUNT(x))`, with `DISTINCT` inside both `SUM` and `COUNT` for `AVG(DISTINCT x)`.
- **Rationale:** Exasol declares `AVG` as `DOUBLE`. The quotient reproduced Exasol's `AVG` in every exact-numeric case measured for the `add-pushdown-parity` plan (`specs/_decision/001-add-pushdown-parity.md`). One form serves all three dialects, so the dialects differ only in the spelling of the double type.
- **Alternatives:** The target's own `AVG`: MySQL's returns a `DECIMAL` at the operand's scale plus 4 digits (measured), and Trino's `avg(decimal)` returns a `decimal` (Trino documentation). `AVG(CAST(x AS DOUBLE))`: measured 47 per cent wrong for `AVG(DISTINCT)` over a `DECIMAL(19,0)` column, because the cast merges distinct wide values before `DISTINCT` applies.
- **Consequences:**
  - MySQL's `CAST(... AS DOUBLE)` needs MySQL 8.0.17 or later, so a delegated `AVG` fails on an older MySQL server.
  - Each dialect states the quotient in its own `render_aggregate`, as it states `COUNT`, `MIN`, and `MAX`. The `pushdown-aggregation` scenario and one unit test per dialect hold the three copies to one form.
- **Promotes to ADR:** no

### [3] Widen the SUM operand where the target sums in a narrower type

- **Decision:** The PostgreSQL dialect casts a `SUM` operand declared `DOUBLE` to `DOUBLE PRECISION`. The Trino dialect casts a `SUM` operand declared `DECIMAL(p,s)` to `DECIMAL(38,s)`. Each rule keys on the operand's declared Exasol type, which the query model carries on every column reference.
- **Rationale:** PostgreSQL's `sum(real)` returns `real` and adds in single precision (PostgreSQL documentation), which loses digits that Exasol's double-precision sum keeps, independent of row order. Trino's `sum` returns its input's type (Trino documentation), so a sum over `bigint` cannot exceed the `bigint` range, where Exasol declares the sum `DECIMAL(30,0)`. The cast is a no-op for a `double precision` operand in PostgreSQL and for a `decimal` operand in Trino.
- **Alternatives:** Render the target's plain `SUM` and list both cases as known differences: the `real` case returns a less exact value than option 1 accepts, and the `bigint` case turns a query Exasol answers into a source error. Key the cast on the recorded source type: it would put type-mapping knowledge in the aggregate renderer, while the declared type already selects every affected operand.
- **Promotes to ADR:** no

## Review Findings

### [plan-review] Exasol delegates SUM and AVG over BOOLEAN and character operands

- **Finding:** Decision [1] requires each dialect to render `SUM` and `AVG` for every operand type Exasol admits. The plan covered only `DECIMAL` and `DOUBLE` operands. Exasol also computes both over `BOOLEAN`, `VARCHAR`, and `CHAR` operands, and the plan did not check whether Exasol delegates them.
- **Direction change:** Measured on the test Exasol, Exasol delegates `SUM` and `AVG` over a `BOOLEAN` or character column, a predicate, and a string literal as the unconverted operand. Each dialect now converts such an operand as Exasol does before the source sums it: a `BOOLEAN` to 1 or 0, and a character value to a double with an empty value read as null. The `COUNT` inside `AVG` takes the operand before the conversion, which reproduces Exasol's `AVG(DISTINCT)`. The query model classifies the operand once, and each dialect spells the conversion. The conversion keys on the operand's Exasol type, because it reproduces Exasol's conversion of the value the scan emits. It is therefore not a read expression under `column-records-source-type-and-type-mapping-names-read-expression`. MySQL's string-to-number conversion reads a value that is not a number as its numeric prefix or as 0, where Exasol fails the query. MySQL has no conversion that fails, so the user documentation lists the difference.
- **Promotes to ADR:** no
