# Code Review Findings: add-trino-support

## Summary
- Files reviewed: 58
- Total findings: 40 (standard: 35, expert: 5)

Baseline established before review: `cargo fmt --all -- --check` clean, `cargo clippy --workspace --all-targets` clean, `cargo test --workspace` 761 passed / 0 failed / 0 ignored. No finding below is a compiler- or lint-visible defect.

Verified against the **final** design (not the superseded one) and found correct, so not raised as findings: the credential travels in `OptionDatabase::Username`/`Password` with the URI carrying no userinfo and nothing percent-encoded (`connection.rs:199-216`); `drivers/trino.toml` states exactly PostgreSQL's 35 capabilities plus `AGGREGATE_GROUP_BY_TUPLE` (36, diffed programmatically) and `capabilities_tests.rs::trino_capability_set_is_exact` pins them against a literal, independently written 36-entry constant; `adapter_notes_tests.rs::a_legacy_single_segment_qualifier_deserializes_beside_an_array` is a genuine byte-identity round-trip, not a shape check; `trino_type_mapping.rs::parse_data_type` mis-parses none of `array(...)`, `row(...)`, `map(varchar, integer)`, `map(varchar,integer)`, `timestamp(p) with time zone`, `decimal(38,0)`, or an overflowing parameter — every one falls to a named unmapped reason; MySQL and PostgreSQL rendering and recorded notes are byte-identical after the qualifier refactor; `provisioning_test.rs::two_drivers_install_side_by_side_at_distinct_driver_and_version_segments` now enumerates `Driver::ALL` for both pairwise-distinctness claims while keeping the original two-driver install assertions; `make stack-up-trino` uses a bounded poll with a named timeout rather than `--wait` or the image `HEALTHCHECK`, is HTTPS-only, and writes its keystore to a gitignored path; `init.sql` carries a decimal at each of the three Arrow widths and all four timestamp precisions; `README.md` and `specs/mission.md` carry no stale percent-encoding claim.

Per-dialect duplication of `qualified_table` across `mysql_dialect.rs`, `postgres_dialect.rs`, and `trino_dialect.rs` was considered and is **not** raised: `SourceQualifier`'s own doc comment places the separator and the quoting in the dialect deliberately, so that triplication is an authorized project decision rather than a DRY violation.

## Standard fixes

### crates/adbc-vs-adapter/src/trino_dialect.rs

#### [OUTDATED_COMMENT] Module doc claims a totality the double-literal guard does not deliver
- Location: lines 59-65, contradicted by `render_double_literal` at lines 752-763
- Issue: the module doc states "`Infinity` and `NaN` have no bare spelling at all ... `DOUBLE '<value>'` is total over every value Exasol can send". `render_double_literal` guards its value with `is_numeric_constant`, which rejects both `Infinity` and `NaN` (neither is an optional sign plus digits plus an optional `e` exponent), so this dialect refuses exactly the two values the comment cites as the reason for choosing the typed form. `notes/B.md` § Gate 2.3(b) measured that `DOUBLE 'Infinity'` and `DOUBLE 'NaN'` both execute on Trino, so the guard is narrower than the form it guards, and `vs-adapter/trino-pushdown`'s own scenario clause ("a literal value not spelled as a bare numeric constant MUST fail with a named error") requires that guard — the comment is what is wrong, not the code.
- Fix: In crates/adbc-vs-adapter/src/trino_dialect.rs, rewrite the module doc sentence at lines 59-65 so it states only the two reasons the code delivers — a value spelled with an exponent is already typed `double`, and a bare constant above 38 significant digits is rejected outright — and add one sentence recording that `render_double_literal` also refuses `Infinity` and `NaN` through its numeric-constant guard although Trino would accept them in the typed form. Do not widen the guard.

#### [OUTDATED_COMMENT] TEMPORAL_LITERAL_CHARACTERS doc names DOUBLE, which does not use it
- Location: lines 151-155
- Issue: the constant's doc reads "The characters a `DATE`, `TIMESTAMP`, or `DOUBLE` typed literal's value may carry", but `TEMPORAL_LITERAL_CHARACTERS` is referenced only by `render_temporal_literal` (lines 793 and 797). `render_double_literal` validates through `is_numeric_constant` instead, which admits `e` and `E` — characters this set excludes — so a reader following the comment would conclude `DOUBLE '1e308'` is refused when it is rendered. The PostgreSQL twin at `postgres_dialect.rs:162` says "a `DATE` or `TIMESTAMP` literal" and is correct; the Trino copy added `or DOUBLE`.
- Fix: In crates/adbc-vs-adapter/src/trino_dialect.rs, delete `, or \`DOUBLE\`` from the doc comment on `TEMPORAL_LITERAL_CHARACTERS` at lines 151-152 so it names only `DATE` and `TIMESTAMP`.

### crates/adbc-vs-adapter/src/trino_dialect_tests.rs

#### [UNTESTED_ERROR_PATH] Six render-error branches of the new dialect have no test
- Location: whole file; the unreached branches are `trino_dialect.rs:378` (empty `AND`/`OR`), `:465` (empty `IN` list), `:339` (empty `GROUP BY`), `:644` and `:650` (`render_star_aggregate`'s non-`COUNT` and `COUNT(DISTINCT *)` arms), and `:579` and `:597` (`resolve_alias`'s "no table reference at all" and "more than one table answers" arms)
- Issue: `trino_dialect.rs:188-192` states that this dialect re-implements alias resolution and every empty-form guard independently of `postgres_dialect`, so the MySQL and PostgreSQL coverage does not carry over. Both sibling dialects test all seven branches; the Trino file tests none of them. Its only alias-resolution test, `unresolvable_column_named_in_error` at line 486, covers the "no table named X" arm alone.
- Fix: In crates/adbc-vs-adapter/src/trino_dialect_tests.rs, port these tests from their MySQL twins, adapting the fixture to `namespace()` and `TrinoDialect`: `a_predicate_with_no_operand_fails_rather_than_rendering_an_empty_group` (mysql_dialect_tests.rs:326), `an_aggregation_grouping_by_no_column_fails_rather_than_rendering_an_empty_clause` (mysql_dialect_tests.rs:873), `an_aggregate_carrying_no_argument_fails_unless_it_is_count_star` (mysql_dialect_tests.rs:845), `a_join_column_carrying_no_table_reference_at_all_fails_naming_it` (mysql_dialect_tests.rs:1830), `a_reference_more_than_one_joined_table_answers_to_fails_naming_it` (mysql_dialect_tests.rs:1847), and a case covering the empty `IN` list.

#### [MISSING_BOUNDARY_TEST] The identifier-case test carries no mixed-case identifier
- Location: line 133, `every_identifier_is_quoted_whatever_its_case`
- Issue: the fixture the test renders through is `orders_columns()` (line 35), whose columns are `id`, `total`, and `note`, over the table `orders` in the namespace `memory`/`adbc_vs`. Every identifier in it is already lower case, so the "whatever its case" claim in the name is never exercised. Trino folds an unquoted identifier to lower case exactly as PostgreSQL does, and the PostgreSQL twin at `postgres_dialect_tests.rs:590` correctly uses `"OrderId"`.
- Fix: In crates/adbc-vs-adapter/src/trino_dialect_tests.rs, add a mixed-case column named `OrderId` to the fixture `every_identifier_is_quoted_whatever_its_case` renders, and assert the rendered statement contains `"OrderId"` with its case intact.

#### [MISSING_BOUNDARY_TEST] The accepted-escape test asserts only that the word ESCAPE appears
- Location: line 368, `an_escape_character_followed_by_percent_underscore_or_itself_is_accepted`
- Issue: for the patterns `a\%b`, `a\_b`, and `a\\b` the test asserts only `clause.contains("ESCAPE")`. The pattern body could be mangled, the escape character could be rendered as something other than a backslash, and the operand could be dropped, and the assertion would still pass. Every neighbouring test in this file pins an exact string. Since `vs-adapter/trino-pushdown` makes the `ESCAPE` clause mandatory precisely because a mis-rendered pattern returns different rows rather than an error, a substring check is the weakest form this assertion can take.
- Fix: In crates/adbc-vs-adapter/src/trino_dialect_tests.rs, replace the `clause.contains("ESCAPE")` assertion in `an_escape_character_followed_by_percent_underscore_or_itself_is_accepted` with an exact `assert_eq!` of the whole rendered clause for each of the three patterns, in the shape `("note" LIKE '<pattern>' ESCAPE '\')`.

#### [MISSING_BOUNDARY_TEST] No test for a nested join, a self-join, or clause ordering in the new dialect
- Location: whole file; the untested code is `trino_dialect.rs:288` (`render_join_tree` recursion) and `AliasedFrom::assign` at `:212`
- Issue: this dialect owns its own alias counter and its own clause assembly (`render` at `trino_dialect.rs:170-184`). Four shapes that both sibling dialects test are absent here: a nested join rendering the inner join parenthesized inside the outer, a self-join told apart only by the request's aliases, a request-assigned alias that must not reach the statement, and a join composed with `filter` and `groupBy` in clause order. A regression in the alias counter or the clause order would be caught by nothing.
- Fix: In crates/adbc-vs-adapter/src/trino_dialect_tests.rs, port `a_nested_join_renders_the_inner_join_parenthesized_inside_the_outer` (mysql_dialect_tests.rs:1608), the self-join test (mysql_dialect_tests.rs:1776), the request-alias test (mysql_dialect_tests.rs:1749), and the join-with-filter-and-grouping clause-order test (mysql_dialect_tests.rs:1697), adapting each to `namespace()` and `TrinoDialect`.

### crates/adbc-vs-adapter/src/create_schema_tests.rs

#### [MISSING_BOUNDARY_TEST] The qualifier-arity table omits the only driver with a two-segment qualifier
- Location: lines 360-371
- Issue: `an_address_not_spelling_its_drivers_qualifier_is_refused_naming_the_driver_and_both_counts` iterates `(MySql, 0)`, `(MySql, 0)`, `(MySql, 2)`, `(Postgres, 0)`, `(Postgres, 2)`. There is no Trino row. Trino is the only driver whose `segment_count` is 2 — the driver the whole arity check exists for — so neither the too-few direction (one path element) nor the too-many direction (three) is exercised for it, and "too few" is only ever seen as the degenerate zero-element case. plan.md § Verification maps `create-virtual-schema / Connection address without the qualifier its driver needs fails with a named error` to this test.
- Fix: In crates/adbc-vs-adapter/src/create_schema_tests.rs, add `(Driver::Trino, "trino://172.28.5.12:8443/memory", 1)` and `(Driver::Trino, "trino://172.28.5.12:8443/memory/adbc_vs/extra", 3)` to the table at line 360, and correct the stale doc comment at lines 335-337, which reads "no identity this release ships qualifies with more than one segment" although Trino declares two and the test directly below it already uses `trino://` addresses.

#### [UNTESTED_ERROR_PATH] ConnectionAddressQualifierSegment is constructed by no test
- Location: `SchemaError::ConnectionAddressQualifierSegment` at create_schema.rs:116, raised from `parse_qualifier` at create_schema.rs:689
- Issue: grepping `SchemaError::` across create_schema_tests.rs returns only `ConnectionAddressQualifierArity`, `ConnectionSchemeMismatch`, `ConnectionAddressMissingScheme`, `MissingProperty`, and `UnsupportedDriver`. The segment variant is newly reachable for Trino and only for Trino: `address_path_elements("trino://host:8443/memory/")` yields `["memory", ""]`, which passes the arity check (2 == 2) and then fails `SourceQualifier::of`'s blank-segment refusal. A one-segment driver cannot reach it, because a single non-empty path element cannot be blank.
- Fix: In crates/adbc-vs-adapter/src/create_schema_tests.rs, add a test asserting that `parse_qualifier("trino://172.28.5.12:8443/memory/", Driver::Trino)` and `parse_qualifier("trino://172.28.5.12:8443//adbc_vs", Driver::Trino)` each fail with `SchemaError::ConnectionAddressQualifierSegment` and that the message names the empty segment's position. Add a doc comment on `ConnectionAddressQualifierSegment` in create_schema.rs stating that it is reachable only for a driver whose `segment_count` exceeds one, so a later reader does not mistake it for dead code.

### crates/adbc-vs-adapter/src/metadata.rs

#### [OUTDATED_COMMENT] Module doc and namespace doc still describe two technologies
- Location: lines 29-31 and 131-139
- Issue: line 29 reads "MySQL's is `mysql_source` and PostgreSQL's is `postgres_source`; each is reached only through `source_for`", but `source_for` now has a third arm returning `TrinoSource`. `MetadataSource::namespace`'s doc at lines 131-139 likewise explains only MySQL's and PostgreSQL's answers, although a multi-segment namespace is the entire reason that method's return type changed in this plan.
- Fix: In crates/adbc-vs-adapter/src/metadata.rs, add Trino to the module doc sentence at lines 29-31, and extend `MetadataSource::namespace`'s doc at lines 131-139 with the Trino case, stating that its namespace is the catalog and the schema the `CONNECTION` address's path spells.

### crates/adbc-vs-adapter/src/metadata_source_tests.rs

#### [ASSERTION_FREE_TEST] The Trino iteration of the precondition invariant asserts nothing
- Location: lines 105-108 and 122-145
- Issue: `precondition(Driver::Trino)` returns `Precondition { queries: vec![], subjects: vec![] }`. In `each_sources_precondition_reads_only_its_own_catalog` the first inner loop iterates an empty `subjects`, and the second evaluates `!own.queries.iter().any(...)` over an empty `queries`, which is trivially true for every other driver's subject. The Trino pass of the loop therefore makes no assertion at all, so a Trino precondition that later started reading MySQL's `information_schema.COLLATIONS` would not fail this test. `capabilities_tests.rs:53` already carries the anti-vacuity guard this loop lacks.
- Fix: In crates/adbc-vs-adapter/src/metadata_source_tests.rs, add an assertion before the two inner loops in `each_sources_precondition_reads_only_its_own_catalog` that a driver whose `subjects` is empty has an empty `queries` too, with a message naming that Trino deliberately runs no precondition query, so the empty case is asserted as a fact rather than passing vacuously.

#### [MISSING_BOUNDARY_TEST] Only the too-deep direction of the namespace-arity guard is tested
- Location: lines 157-191
- Issue: `a_namespace_deeper_than_its_driver_declares_is_refused_naming_both_counts` exercises `declared + 1` only, and its doc comment at lines 157-159 justifies that with "every identity this release ships declares one segment and a qualifier of none cannot be built". Trino declares two, so a one-segment qualifier is now constructible and `check_namespace_arity` (metadata.rs:208) refuses it — a direction nothing tests, and the same direction that panics before the guard runs in `source_for` (see the Expert finding on metadata.rs).
- Fix: In crates/adbc-vs-adapter/src/metadata_source_tests.rs, extend that test to also assert `check_namespace_arity` fails for `declared - 1` segments whenever `declared > 1`, and rewrite the doc comment at lines 157-159, which is now false.

### crates/adbc-vs-adapter/src/capabilities_tests.rs

#### [OUTDATED_COMMENT] A comment counts two dialects where the release ships three
- Location: line 246
- Issue: the comment reads "neither dialect's `render_predicate` or `render_literal` has a rendering path for them". This release ships three dialects, and the statement is about all of them.
- Fix: In crates/adbc-vs-adapter/src/capabilities_tests.rs line 246, replace "neither dialect's" with "no dialect's".

### crates/adbc-vs-adapter/src/trino_type_mapping_tests.rs

#### [DUPLICATE_TEST] Three tests re-assert what the scenario-named tests already cover
- Location: lines 31, 75, and 85
- Issue: every assertion in `length_precision_and_scale_are_parsed_out_of_data_type` (line 31) reappears verbatim later in the file — `varchar(100)` at line 138, `decimal(12,4)` at line 118, `timestamp(6)` at line 172, and `integer` at line 98. The `map(varchar, integer)` half of the test at line 75 is re-asserted by the non-core-type loop at line 198, and `a_data_type_embedding_quotes_is_carried_verbatim_into_the_reason` at line 85 asserts `row("a" integer)` carries itself, which the same loop asserts for the same string at line 199.
- Fix: In crates/adbc-vs-adapter/src/trino_type_mapping_tests.rs, delete `length_precision_and_scale_are_parsed_out_of_data_type` at line 31, drop the `map(varchar, integer)` half of the test at line 75 while keeping its space-versus-no-space contrast, and delete `a_data_type_embedding_quotes_is_carried_verbatim_into_the_reason` at line 85 after removing `row("a" integer)` from the non-core-type loop's list so the quote-carrying case keeps one owner.

### crates/adbc-vs-adapter/src/trino_source_tests.rs

#### [IMPLEMENTATION_COUPLED_TEST] The nullability test performs the conversion it claims to verify
- Location: line 126, `nullability_is_read_from_the_catalogs_yes_no_spelling`
- Issue: the test computes `is_nullable: spelling == NULLABLE` in its own body. The production conversion lives at trino_source.rs:235 (`required_string_cell(...)? == NULLABLE`) and is never executed by this test, so the test asserts that the test's own expression agrees with itself. A production change from `== NULLABLE` to, say, `!= "NO"` would not fail it.
- Fix: In crates/adbc-vs-adapter/src/trino_source.rs, extract the `== NULLABLE` comparison into a private free function `fn is_nullable_cell(value: &str) -> bool` and call it from the existing site; in crates/adbc-vs-adapter/src/trino_source_tests.rs, rewrite `nullability_is_read_from_the_catalogs_yes_no_spelling` to assert on that function for `"YES"`, `"NO"`, and one unexpected spelling.

### crates/adbc-vs-adapter/tests/e2e_trino_test.rs

#### [ASSERTION_FREE_TEST] The mirror-shape test passes vacuously when the mirror has no columns
- Location: lines 110-125, `mirror_declares_the_same_columns_and_types_as_the_virtual_table`
- Issue: the test body is a bare `for` over `declared_columns(trino_mirror::SCHEMA, "items")` with no assertion outside the loop. If the mirror schema were mis-seeded, renamed, or not created, `declared_columns` returns an empty `Vec` and the test passes having asserted nothing — and this test is the premise every differential assertion in the file rests on, because it is what establishes that the mirror and the virtual table are comparable.
- Fix: In crates/adbc-vs-adapter/tests/e2e_trino_test.rs, add an assertion before the loop in `mirror_declares_the_same_columns_and_types_as_the_virtual_table` that the mirror's declared-column list is non-empty and holds the expected column count, with a message naming the mirror schema and table.

#### [IMPLEMENTATION_COUPLED_TEST] The plain-HTTP test matches a third-party driver's internal message text
- Location: lines 706-710
- Issue: the assertion is `error.contains("HTTP response to HTTPS client") || error.contains("tls")`. The first operand is an internal string of the ADBC Trino driver's Go HTTP stack, which a driver release within the declared `>=0.5.3, <0.6.0` range may reword; the second is loose enough to pass on an unrelated TLS failure, so the fallback silently converts a broken assertion into a passing one. `scan-udf/trino-scan`'s scenario requires only that the attempt fail, that the surfaced error name the host, and that it carry no password.
- Fix: In crates/adbc-vs-adapter/tests/e2e_trino_test.rs, replace the disjunction at lines 706-710 with assertions on the durable clauses the spec states — that the attempt failed, that the message names the host and port, and that it contains no part of the password — and keep the driver's own text only as context in the failure message, never as the condition.

#### [TOO_MANY_ARGUMENTS] create_connection takes four positional arguments, three of them strings
- Location: line 724, and the doc comment at lines 717-723
- Issue: `fn create_connection(exasol, name, address, password)` takes three adjacent `&str` parameters that are trivially swappable at a call site with no compiler error. The doc comment above it also states the helper authenticates "as `user`/`password`" although there is no `user` parameter, and closes with "none accepts an arbitrary user, which every credential test above needs" — which the body contradicts by hardcoding `trino::HOSTILE_USER` at line 729.
- Fix: In crates/adbc-vs-adapter/tests/e2e_trino_test.rs, replace `create_connection`'s three `&str` parameters with a small `ConnectionSpec { name, address, password }` struct in the shape `schema::VirtualSchemaSpec` already used at line 934, update every call site, and rewrite the doc comment at lines 717-723 to state that the helper always authenticates as the hostile principal.

### crates/adbc-vs-adapter/tests/common/trino.rs

#### [VAGUE_TEST_NAME] A test named for stopping on rejection asserts the output format
- Location: lines 330-337, `the_client_stops_on_the_first_rejected_statement`
- Issue: the body asserts only `arguments.contains(&"--output-format".to_string())`. It tests neither stopping nor rejection, and nothing else in the file verifies the claim the name makes.
- Fix: In crates/adbc-vs-adapter/tests/common/trino.rs, rename `the_client_stops_on_the_first_rejected_statement` to `the_client_requests_an_unheaded_unquoted_result`, matching what it asserts.

#### [DUPLICATE_TEST] A port-sharing assertion is made twice over the same values
- Location: lines 300-303, `the_published_endpoint_and_the_in_network_address_share_a_port`
- Issue: it re-asserts, verbatim, the third assertion inside `assert_container_address_invariants` (tests/common/mod.rs:97-101), which `container_address_invariants_hold` at line 278 of this same file already runs.
- Fix: In crates/adbc-vs-adapter/tests/common/trino.rs, delete `the_published_endpoint_and_the_in_network_address_share_a_port` at lines 300-303.

### crates/adbc-vs-adapter/tests/common/mod.rs

#### [TOO_MANY_ARGUMENTS] assert_container_address_invariants takes five positional arguments
- Location: lines 81-87
- Issue: `assert_container_address_invariants(driver_uri, unreachable_uri, published_endpoint, port, password)` takes four `&str` and one `u16`, with two adjacent URI strings that a call site can swap with no compiler error and a misleading pass as the result.
- Fix: In crates/adbc-vs-adapter/tests/common/mod.rs, replace the five parameters of `assert_container_address_invariants` with one `ContainerAddresses` struct carrying the same fields, and update the call sites in tests/common/trino.rs and its siblings to construct it by field name.

### crates/adbc-vs-adapter/tests/e2e_postgres_test.rs

#### [VAGUE_TEST_NAME] A test named for every supported driver skips one
- Location: lines 739 and 746
- Issue: `every_supported_driver_creates_a_virtual_schema` now carries `Driver::Trino => continue`, so it no longer covers every supported driver. The Trino binary's counterpart was given the accurate name `every_driver_this_stack_provisions_creates_a_virtual_schema` (e2e_trino_test.rs:968). In a project whose stated rule is "fail, never skip", a match arm that skips a driver must not sit under a name claiming exhaustiveness, even with a doc comment explaining it.
- Fix: In crates/adbc-vs-adapter/tests/e2e_postgres_test.rs, rename `every_supported_driver_creates_a_virtual_schema` to `every_driver_this_stack_provisions_creates_a_virtual_schema`, matching the Trino binary's name.

### crates/adbc-vs-adapter/tests/provisioning_test.rs

#### [INLINE_COMMENT] A six-line inline block stands in for the test's missing doc comment
- Location: lines 637-642, on the test at lines 574-575
- Issue: a six-line `//` block sits inside the test body explaining the enumeration, while the file's convention is a `///` doc comment on the item. `two_drivers_install_side_by_side_at_distinct_driver_and_version_segments` has no doc comment at all, unlike its neighbours at lines 595 and 675, although it now proves two distinct claims, and its name still describes only the first.
- Fix: In crates/adbc-vs-adapter/tests/provisioning_test.rs, delete the inline block at lines 637-642 and move its content into a new `///` doc comment on the test, and rename the test to `two_drivers_install_side_by_side_and_every_driver_identity_is_pairwise_distinct`.

### crates/adbc-vs-scan/src/driver.rs

#### [MISSING_DOC_COMMENT] The public Driver variants carry no doc comments
- Location: lines 62-64
- Issue: `MySql`, `Postgres`, and `Trino` are public variants of a public enum and are the only items in the file without doc comments; every other public item has one. This change added the third undocumented variant rather than closing the gap.
- Fix: In crates/adbc-vs-scan/src/driver.rs, add a one-line doc comment to each of the three variants at lines 62-64 naming the canonical `DRIVER` property value it corresponds to.

### crates/adbc-vs-scan/src/driver_tests.rs

#### [ASSERTION_FREE_TEST] A test asserts a bound the build already makes unreachable
- Location: lines 93-102, `every_identity_declares_a_qualifier_of_at_least_one_segment`
- Issue: it asserts `qualifier_segment_count() >= 1` on a `usize`, so the only value that could fail it is 0 — and `build.rs:183-189` refuses a declaration whose `segment_count` is 0 before the crate compiles. No reachable state can turn this test red. The claim is genuinely established by `driver_toml_test.rs::a_declaration_qualifying_with_no_segment_names_the_file_and_the_field`.
- Fix: In crates/adbc-vs-scan/src/driver_tests.rs, delete `every_identity_declares_a_qualifier_of_at_least_one_segment` at lines 93-102.

### crates/adbc-vs-scan/build.rs

#### [OUTDATED_COMMENT] The module doc understates what the build script now refuses
- Location: lines 17-18
- Issue: the doc states "What it refuses is the `[package]` table's fields and nothing else", but the script now also refuses a missing or zero `[qualifier] segment_count` (lines 183-189), an empty capability list (lines 191-197), and a blank capability entry (lines 198-206). The same claim is repeated at CLAUDE.md:97 ("`build.rs` validates only the `[package]` table"), and this repo's own Spec-governance rule requires that bullet to change in the same commit as the file it summarizes.
- Fix: In crates/adbc-vs-scan/build.rs, rewrite the sentence at lines 17-18 to state that the script refuses the `[package]` table's fields, a `[qualifier] segment_count` of zero, and an empty or blank-entried capability list, while still checking no capability *name*. Make the matching correction to the bullet at CLAUDE.md:97.

### crates/adbc-vs-scan/tests/driver_toml_test.rs

#### [VAGUE_TEST_NAME] Twelve refusal cases run under one test name
- Location: lines 150-164, `malformed_driver_toml_fails_the_build`
- Issue: one `#[test]` calls twelve plain helper functions in sequence, so the name states no condition and no expected behaviour, the first refusal that regresses masks the other eleven, cargo reports one test rather than twelve, and none of them run in parallel.
- Fix: In crates/adbc-vs-scan/tests/driver_toml_test.rs, put `#[test]` on each of the twelve helper functions called from `malformed_driver_toml_fails_the_build` and delete that wrapper; they already take no arguments and share only free functions. While there, rename the fixture strings at lines 344 and 363 from `package_only` to `empty_capability_list` and `blank_capability_entry`, which is what each case is about.

### crates/adbc-vs-scan/tests/trino_it_test.rs

#### [INLINE_COMMENT] Eight inline comments annotate values inside a test body
- Location: lines 415, 426, 437, 448, 452-456, 464, 469, and 474
- Issue: seven trailing value annotations and one four-line block sit inside the function body, where the guardrails forbid inline comments. A reader sees none of them when the assertion actually fails, because they are not in the assertion messages.
- Fix: In crates/adbc-vs-scan/tests/trino_it_test.rs, fold each trailing annotation at lines 415, 426, 437, 448, 464, 469, and 474 into the message argument of the adjacent `assert_eq!`, and move the four-line block at lines 452-456 into the test's doc comment beside the truncation discussion already at line 288.

#### [OUTDATED_COMMENT] The driver-load test claims to prove the declared entry point is used
- Location: lines 180-205
- Issue: the doc states the test proves the driver loads "through the entry point `trino.toml` declares (`AdbcDriverTrinoInit`), rather than through the driver manager's own filename-derivation heuristic". The test opens a connection and reads a row. The manager's heuristic derives the same symbol from `libadbc_driver_trino.so`, so the test passes identically whether the declared entry point is honoured or ignored, and it cannot distinguish the two.
- Fix: In crates/adbc-vs-scan/tests/trino_it_test.rs, rewrite the doc comment at lines 180-205 to claim only what the test establishes — that the declared entry point loads and a statement executes — removing the contrast with the filename heuristic.

#### [MISSING_BOUNDARY_TEST] The coercion test assumes the driver returns all fixture rows in one batch
- Location: lines 314-324
- Issue: the test takes only the first batch via `.next()` and then indexes `value(1)` and `value(2)`, silently assuming all three fixture rows arrive together. A driver that split them would panic with an index-out-of-bounds rather than fail with a named message, which is the hardest failure of all to diagnose in this tier.
- Fix: In crates/adbc-vs-scan/tests/trino_it_test.rs, add `assert_eq!(batch.num_rows(), 3, ...)` immediately after the `.next()` at line 314, with a message naming the assumption that the driver returns the three fixture rows in one batch.

### Makefile

#### [BROAD_CATCH] The credential-free negative probe passes on any non-zero exit
- Location: lines 236-240
- Issue: the probe exists so that "a container that answers it is silently unauthenticated and fails stack-up rather than passing" (decision `[14]`, task 1.1). As written it treats every non-zero exit as proof of rejection and discards both streams with `>/dev/null 2>&1`, so a missing CLI binary, a stopped container, a TLS handshake error, and a genuine 401 are indistinguishable. The one condition the probe exists to detect — an unauthenticated container answering `SELECT 1` — is the only case it would catch, but every unrelated breakage also makes it "pass", so its signal degrades silently.
- Fix: In the Makefile, change the negative probe at lines 236-240 to capture the CLI's combined output into a variable, fail when the command succeeds, and additionally require the captured text to match the authentication refusal (for example `grep -qiE "401|[Aa]uthentication"`), so a probe that failed for any other reason fails stack-up with the captured message rather than passing.

#### [DEAD_FLEXIBILITY] TRINO_HOSTILE_PASSWORD is exported but read by nothing
- Location: lines 203-204
- Issue: `TRINO_HOSTILE_PASSWORD ?= p@ss/w:o?r#d&x% y` is declared and `export`ed, but no recipe forwards it and docker-compose.yml:99 hardcodes the same literal instead of interpolating `${TRINO_HOSTILE_PASSWORD}`, so `make test-it-trino TRINO_HOSTILE_PASSWORD=x` silently changes nothing. The plaintext is spelled five times with no owner and no check that the copies agree: Makefile:203, docker-compose.yml:99, crates/adbc-vs-scan/tests/trino_it_test.rs:71, crates/adbc-vs-scan/src/connection_tests.rs:33, and crates/adbc-vs-adapter/tests/common/trino.rs:68.
- Fix: In the Makefile and docker-compose.yml, make the variable load-bearing: change docker-compose.yml:99 to `TRINO_HOSTILE_PASSWORD: "${TRINO_HOSTILE_PASSWORD:-p@ss/w:o?r#d&x% y}"` so the Make variable reaches the container, and add a comment at Makefile:203 stating that the three Rust constants must be kept in step with it because the bcrypt hash in docker/trino/password.db is fixed.

#### [MAGIC_NUMBER] The poll interval is a bare literal beside a named deadline
- Location: lines 187 and 234
- Issue: both bounded readiness loops use a literal `sleep 5` while the deadline each is paired with is a named variable (`EXASOL_WAIT_TIMEOUT`, `TRINO_WAIT_TIMEOUT`).
- Fix: In the Makefile, add `POLL_INTERVAL ?= 5` beside the timeout variables and replace the literal in both loops at lines 187 and 234 with `sleep $(POLL_INTERVAL)`.

### docker-compose.yml

#### [TACTICAL_SHORTCUT] The keystore bind mount has no ordering contract with the recipe that creates it
- Location: line 108
- Issue: `./docker/trino/generated/keystore.pem:/etc/trino/keystore.pem:ro` names a path that does not exist on a fresh clone; it is created only by the `stack-up-trino` recipe at Makefile:219-224. Any `docker compose up` or `docker compose create` touching the `trino` service before that recipe runs makes Docker create a *directory* at that path, after which the recipe's `cat ... > $(TRINO_KEYSTORE)` fails with "Is a directory" and the only recovery is a manual `rm -rf`. The ordering is enforced by convention rather than by the build graph.
- Fix: In the Makefile, extract the keystore generation from the `stack-up-trino` recipe into its own real-file target `$(TRINO_KEYSTORE):` and make `stack-up-trino` depend on it, so the file exists before any compose invocation can create a directory in its place. Drop the `>/dev/null 2>&1` redirect on the `openssl req` invocation so a generation failure names itself.

### docker/trino/init.sql

#### [OUTDATED_COMMENT] The header says the fixture stays minimal, and it no longer does
- Location: lines 4-6
- Issue: the header states "Kept minimal on purpose: task 8.1 (a later group, not this one) grows this into the differential E2E fixture ... this file does not pre-empt that". Lines 62-186 *are* that differential fixture — `items`, `item_notes`, `all_unmapped`, `refresh_target`, and `orders_view` — so the two halves of the file contradict each other.
- Fix: In docker/trino/init.sql, rewrite the last two sentences of the header at lines 4-6 to describe the file as it stands: the core-type fixture plus the differential collation, join, view, and unmapped-column objects, and the four numeric-width and timestamp-precision columns the coercion tier binds to.

### Cross-file

#### [OUTDATED_COMMENT] Plan-local task and gate numbers are cited in shipped code comments
- Location: crates/adbc-vs-adapter/tests/e2e_trino_test.rs lines 125, 246, 300, 397, 514, 522, 644, 684, 733, 994; crates/adbc-vs-adapter/tests/common/trino.rs lines 42, 62, 234; crates/adbc-vs-adapter/tests/common/trino_stack.rs line 10; crates/adbc-vs-adapter/tests/common/trino_mirror.rs line 176; crates/adbc-vs-adapter/src/trino_dialect.rs (nine sites)
- Issue: roughly two dozen comments cite plan-local identifiers — "task 8.3", "task 5.4, group E", "gate 2.1f", "gate 2.3(a)", "gate 2.3(c)", "gate 2.3(d)", "gate 2.3(i)". `/speq:record` archives `specs/_plans/add-trino-support/` into `specs/_recorded/`, which CLAUDE.md states is gitignored, so every one of these references becomes unresolvable for a later reader. The PostgreSQL equivalents (`e2e_postgres_test.rs`, `tests/common/postgres.rs`, `tests/common/postgres_stack.rs`) contain none, so this is new rather than house style.
- Fix: In each of the files and lines listed above, replace the plan-local task or gate identifier with the durable spec feature that owns the claim — `vs-adapter/trino-pushdown`, `vs-adapter/trino-collation`, `vs-adapter/trino-type-mapping`, or `scan-udf/trino-scan` — keeping the measured fact the comment states and dropping only the number. Where a comment already names the spec feature beside the gate number, delete the number alone.

## Expert fixes

### crates/adbc-vs-adapter/src/create_schema.rs

#### [INFORMATION_LEAKAGE] The CONNECTION address is echoed verbatim into operator-visible errors, userinfo included
- Location: lines 110-127 (variant definitions) and 151-177 (their `Display`), reached from `parse_qualifier` at line 680 and `check_connection_scheme` at line 652; surfaced by dispatch.rs:97
- Issue: `ConnectionAddressQualifierArity` and `ConnectionAddressQualifierSegment` — both added by this change — and the pre-existing `ConnectionAddressMissingScheme` each store `address: address.to_string()` and print it inside backticks. `dispatch.rs:97` passes `SchemaError`'s `Display` straight into `UdfError::User` with no redaction, and `connection.rs::redact` never sees it because it only knows the password resolved from the `CONNECTION` object's own fields. So `CREATE VIRTUAL SCHEMA` over a `CONNECTION` whose address is `trino://user:s3cret@host:8443/memory` (one path element, two needed) returns `s3cret` in clear text to the querying session. The scan crate strips exactly this component for exactly this reason — `connection.rs:222-231` states that keeping a userinfo would "put that secret into every message the open produces" — and `SchemaError::ConnectionSchemeMismatch` at line 126 already stores only the scheme rather than the address, which shows the hazard was recognised here before. The two new variants regressed that. `scan-udf/trino-scan`'s Background requires that the credential reach "no log line or error message".
- Fix: In crates/adbc-vs-adapter/src/create_schema.rs, add a private function that returns the address with its authority's userinfo component removed, mirroring `adbc-vs-scan/src/connection.rs::without_userinfo` (authority runs from the scheme separator to the first `/`, `?`, or `#`; the userinfo ends at the last `@` inside it), and apply it at every site that constructs `ConnectionAddressQualifierArity`, `ConnectionAddressQualifierSegment`, or `ConnectionAddressMissingScheme` so no variant ever stores a raw address. Add a test in crates/adbc-vs-adapter/tests/dispatch_test.rs asserting that a `createVirtualSchema` failing on each of those three variants, over an address carrying `user:s3cret@`, surfaces a message containing neither `s3cret` nor `user:`.

### crates/adbc-vs-scan/src/connection.rs

#### [MISSING_BOUNDARY_TEST] A userinfo containing an unencoded `/`, `?`, or `#` survives the strip
- Location: `without_userinfo` at lines 232-250, and its callers' doc claims at lines 191-193 and 226-231
- Issue: `without_userinfo` ends the authority at the first `/`, `?`, or `#`, then looks for the last `@` inside it. When the operator writes one of those three characters unencoded into the userinfo itself, the authority span ends inside the credential, no `@` is found, and the address is returned unchanged. I confirmed this by executing the function: `trino://u:p/w@host:8443/cat/sch`, `trino://u:p?w@host:8443/cat/sch`, and `trino://u:p#w@host:8443/cat/sch` all pass through untouched, so a fragment of an operator-written secret reaches both the `uri` option value and the failure subject. This contradicts `trino_target`'s doc at lines 191-193, which claims the strip leaves the subject carrying no credential "neither the resolved one, which never enters it, nor one an operator wrote into the address itself", and `redact` cannot help because it only strips the password resolved from the `CONNECTION` object. `notes/B.md` line 162 records that exactly this shape — an unencoded `/` in a password — is what an operator hits. The strip boundary itself is right: it matches the Go `net/url` parse the driver uses, which gate 2.1d's error text confirms, and it correctly preserves an `@` in a path element or a query parameter, multiple `@` in the userinfo, an empty userinfo, and a bracketed IPv6 authority — all of which I executed. Only the absolute claim and the missing coverage are wrong.
- Fix: In crates/adbc-vs-scan/src/connection.rs, narrow `trino_target`'s doc comment at lines 191-193 to state that the strip removes a userinfo the driver's own URL reader would recognise, and that an address whose userinfo carries an unencoded `/`, `?`, or `#` is malformed, is rejected by the driver, and is passed through rather than stripped. Add cases to `crates/adbc-vs-scan/src/connection_tests.rs::trino_drops_the_authoritys_userinfo_and_changes_nothing_else` pinning that documented behaviour for those three shapes, so a later change to the boundary fails rather than silently altering it.

### crates/adbc-vs-adapter/src/metadata.rs

#### [TACTICAL_SHORTCUT] The Trino arm indexes the qualifier before the guard that would refuse it
- Location: lines 188-195, with `check_namespace_arity` at lines 208-219
- Issue: `Driver::Trino => { let segments = address.segments(); Box::new(TrinoSource::new(connection, &segments[0], &segments[1])) }` indexes element 1 unconditionally. A one-segment qualifier reaching `pub fn source_for` panics with an index-out-of-bounds, which inside an Exasol UDF surfaces as SQL state 22002 "VM crashed" — the least diagnosable outcome this codebase has, and the one CLAUDE.md singles out. The guard two lines later cannot catch it, and it checks the wrong value besides: it validates `source.namespace()`, which every one of the three shipped sources produces at its declared arity by construction, so `MetadataError::NamespaceArity` is unreachable for any real source while the operator-supplied `address` — the only value at this point that can carry the wrong count — is never checked here at all. The literal indices are also a second structural owner of "Trino is 2" beside `drivers/trino.toml`, which plan.md § Patterns row 2 bars: changing the declaration to 3 would compile and silently drop the third segment.
- Fix: In crates/adbc-vs-adapter/src/metadata.rs, call `check_namespace_arity(driver, address)?` as the first statement of `source_for`, before the `match driver`, so the declared count is enforced against the address before any segment is indexed and a short qualifier returns `MetadataError::NamespaceArity` instead of panicking. Keep the existing post-construction call. Extend the doc comment at lines 199-207 to say that the guard now runs on both the address and the source's answer, and why.

### crates/adbc-vs-adapter/src/trino_source.rs

#### [TACTICAL_SHORTCUT] A public constructor validates nothing and its accessor panics on the consequence
- Location: `TrinoSource::new` at lines 58-64 and `namespace` at lines 68-74
- Issue: `pub fn new(connection, catalog: &str, schema: &str)` accepts any two strings without validation, and `namespace()` then calls `SourceQualifier::of(vec![catalog, schema]).expect("the catalog and the schema were already validated as non-blank segments when the CONNECTION address's path was parsed into a qualifier, before this source was constructed")`. The invariant is enforced in a different module of a different crate's call chain (`create_schema.rs::parse_qualifier`) and held here only by that prose, while `MetadataSource::namespace` returns `SourceQualifier` rather than `Result`, so there is no non-panicking path once a blank name gets in. `SourceQualifier`'s own doc at adapter_notes.rs:25-26 promises every reader that no segment is blank; this constructor is where that promise can be broken. Storing the catalog and the schema as two separate `String` fields and re-assembling them on every `namespace()` call is also the second in-memory shape for one value.
- Fix: In crates/adbc-vs-adapter/src/trino_source.rs, change `TrinoSource::new` to take a `SourceQualifier` instead of two `&str`, store it in one field, and have `namespace()` return a clone of it with no `expect`; read the catalog and the schema from `segments()` locally inside `discover_tables` and the `information_schema` query builders. Update the construction site in crates/adbc-vs-adapter/src/metadata.rs to pass `address.clone()`, and update crates/adbc-vs-adapter/src/trino_source_tests.rs accordingly.

### crates/adbc-vs-adapter/src/trino_dialect_tests.rs

#### [SHRINKABLE] The query-model builders are triplicated byte for byte across three dialect test files
- Location: crates/adbc-vs-adapter/src/trino_dialect_tests.rs lines 19-118 and 619-740, with byte-identical copies at crates/adbc-vs-adapter/src/mysql_dialect_tests.rs lines 13-114 and 1128-1250 and crates/adbc-vs-adapter/src/postgres_dialect_tests.rs lines 16-118 and 1543-1665
- Issue: roughly 360 lines of helpers that build the *dialect-agnostic query model* — `request`, `orders_columns`, `column`, `filtered_request`, `numeric_literal`, `string_literal`, `equal`, `less`, and the nested builders inside `query_exercising` (`decimal_column`, `varchar_column`, `date_column`, `timestamp_column`, `select_list`, `from`, `filtered`, `id_equals`, `numeric`, `aggregated`, `aggregate_item`, `count_star`, `aggregate_call`, `joined_column_ref`, `joined_query`) exist three times, differing only in the dialect value and the namespace expression. This change added the third copy, which is where the Rule of Three applies. The project's per-dialect-independence convention covers what each dialect *renders* — the `match capability` arms and the expected fragments, which `capabilities_tests.rs:8-14` argues must stay separate — not the construction of the request each is driven with, which carries no dialect-specific content at all.
- Fix: Add a new `#[cfg(test)]` module `crates/adbc-vs-adapter/src/dialect_test_fixtures.rs`, declared from lib.rs beside the other test-only modules, holding the query-model builders listed above, each taking the namespace as a `&SourceQualifier` parameter and the render helpers taking a `&dyn SqlDialect`. Replace the three copies in trino_dialect_tests.rs, mysql_dialect_tests.rs, and postgres_dialect_tests.rs with `use` of that module, leaving in each file only its own `namespace()`/`DATABASE` constant, its `query_exercising` `match capability` arms, and its expected-fragment assertions. Run `cargo test --workspace` afterwards and confirm the count is unchanged at 761 passing, because the byte-identity assertions that guard MySQL's and PostgreSQL's released rendering run through these helpers.
