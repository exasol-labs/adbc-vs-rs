# Tasks: add-trino-support

## PR Lifecycle
- [x] resolved
- [ ] implemented
- [ ] version-bumped
- [ ] tested-green
- [ ] recorded
- [ ] pr-ready

## Phase 2: Implementation (Group A: Trino container and build surface)
- [x] 1.1 Add the `trino` service to `docker-compose.yml` with TLS + password auth, readiness poll, negative probe
- [x] 1.2 Add `docker/trino/init.sql` fixture, seeded via authenticated CLI from `stack-up-trino`
- [x] 1.3 Split `Makefile` stack targets (`stack-up-trino`, `stack-up-exasol`), add `test-it-trino`/`test-e2e-trino`, extend `make_targets_test.rs`

## Phase 2: Implementation (Group B: Measurement gates)
- [x] 2.0 Gate [expert]: Trino driver library loads inside Exasol UDF sandbox
- [x] 2.1 Gate [expert]: connection path — scheme, percent-encoded credential, SSL
- [x] 2.2 Gate: `information_schema` reads, `data_type` strings, CHAR LIKE/= disagreement
- [x] 2.3 Gate [expert]: rendering rules vs live Trino/Exasol (collation, literals, LIKE, FULL OUTER JOIN, GROUP BY, E'a', date/decimal literals)
- [x] 2.4 Gate: Arrow type / decimal width / timestamp unit coercion gaps
- [x] 2.5 Gate: `COLLATE` is not a Trino keyword in any syntactic position

## Phase 2: Implementation (Group C: Source qualifier segments)
- [x] 3.1 Generalize recorded qualifier from string to ordered segment list [expert]
- [x] 3.2 Give qualifier arity one owner (per-technology declared segment count)

## Phase 2: Implementation (Group D: Driver identity and package)
- [x] 4.1 Add `Driver::Trino` + `drivers/trino.toml`, rewrite provisioning pairwise-distinctness test to enumerate `Driver::ALL`

## Phase 2: Implementation (Group E: Trino connection and scan)
- [x] 5.1 Trino arm of `connection.rs` `database_target`: `uri` without userinfo plus separate `username`/`password` options [expert]
- [x] 5.2 Extend redaction to the database-creation failure path, stripping the password's verbatim bytes
- [x] 5.3 Pin the Arrow decimal widths and timestamp units gate 2.4 measured as handled
- [x] 5.4 Add `trino_it_test.rs` behind `trino-it` feature

## Phase 2: Implementation (Group F: Trino dialect)
- [x] 6.1 Add `trino_dialect.rs` — identifiers, qualifier, joins, GROUP BY, no collation suffix
- [x] 6.2 Render literals — string, DATE/TIMESTAMP, exact-numeric, `DOUBLE '…'`
- [x] 6.3 Render `LIKE` with explicit `ESCAPE` clause, named error on rejected escape sequence
- [x] 6.4 Wire `dialect_for` Trino arm, capability fixtures, `trino_capability_set_is_exact`, rewrite `SHARED_MAIN_CAPABILITIES` doc comment

## Phase 2: Implementation (Group G: Trino metadata source and type mapping)
- [x] 7.1 Add `trino_type_mapping.rs` — parse `data_type` string
- [x] 7.2 Add `trino_source.rs` — `information_schema` queries, namespace, precondition check
- [x] 7.3 Wire `metadata::source_for` Trino arm

## RESOLVED: Group E (Trino connection and scan) — ready to implement
Gate 2.1's two STOP conditions are answered. The requester chose the separate
`username`/`password` options with no credential in the URI, and chose to drop
the multi-byte password case. Decision `[17]` in decision-log.md records the
design, and the `[human-decision]` entry under § Review Findings records the
answers. `scan-udf/trino-scan/spec.md` and plan.md tasks 5.1, 5.2, 5.3, and 8.5
state the revised design. The three `[gate] 2.1 …` entries stay as the evidence
trail. Tasks 5.1–5.4 are NOT started; Group H depends on Group E and Group I on
Group H, so both remain unstarted. Resume with
`/speq:implement-pr add-trino-support`.

## Phase 2: Implementation (Group H: End-to-end tier)
- [x] 8.1 Grow `docker/trino/init.sql` into differential fixture
- [x] 8.2 Add `tests/common/trino.rs`, `trino_stack.rs`, `trino_mirror.rs`, shared setup for `e2e_trino_test.rs`
- [x] 8.3 E2E pushdown: projection, filter, aggregation, inner join, full outer join
- [x] 8.4 E2E collation differential assertions
- [x] 8.5 E2E credential: wrong password (no host), unreachable host (host required), redaction, plain-HTTP refusal
- [x] 8.6 E2E discovery: case, view, REFRESH, unmapped columns, namespace, DRIVER_PATH
- [x] 8.7 E2E capability difference: FULL OUTER JOIN delegated vs withdrawn

## Phase 2: Implementation (Group I: CI and project documents)
- [x] 9.1 Add `integration-tests-trino`/`e2e-trino` CI jobs, extend `workflow_test.rs`
- [x] 9.2 Update `CLAUDE.md`
- [x] 9.3 Update `specs/mission.md`
- [x] 9.4 Update `README.md`

## Phase 4: Review Fixes
- [x] 4.2 Rewrite `trino_dialect.rs` module doc (lines 59-65) to state only the two reasons the code delivers, and record that `render_double_literal` refuses `Infinity`/`NaN` through its numeric-constant guard
- [x] 4.3 Delete `, or \`DOUBLE\`` from the `TEMPORAL_LITERAL_CHARACTERS` doc comment in `trino_dialect.rs`
- [x] 4.4 Port the six untested render-error branch tests (empty AND/OR, empty IN, empty GROUP BY, non-COUNT star aggregate, COUNT(DISTINCT *), no-table-reference alias, ambiguous alias) from `mysql_dialect_tests.rs` into `trino_dialect_tests.rs`
- [x] 4.5 Add a mixed-case `OrderId` column to the fixture `every_identifier_is_quoted_whatever_its_case` renders and assert its case survives quoting
- [x] 4.6 Replace the `clause.contains("ESCAPE")` assertion in `an_escape_character_followed_by_percent_underscore_or_itself_is_accepted` with an exact `assert_eq!` of the whole rendered clause for each of the three patterns
- [x] 4.7 Port the nested-join, self-join, request-alias, and join-with-filter-and-grouping clause-order tests from `mysql_dialect_tests.rs` into `trino_dialect_tests.rs`
- [x] 4.8 Add `(Driver::Trino, …, 1)` and `(Driver::Trino, …, 3)` rows to the qualifier-arity table in `create_schema_tests.rs` and correct the stale doc comment above it
- [x] 4.9 Add a `create_schema_tests.rs` test covering `SchemaError::ConnectionAddressQualifierSegment` for a blank trailing and a blank leading segment, and doc-comment the variant in `create_schema.rs` as reachable only for a multi-segment driver
- [x] 4.10 Add Trino to `metadata.rs`'s module doc sentence and extend `MetadataSource::namespace`'s doc with the Trino catalog/schema case
- [x] 4.11 Add an anti-vacuity assertion to `each_sources_precondition_reads_only_its_own_catalog` that an empty `subjects` implies an empty `queries`, naming Trino's deliberate no-query precondition
- [x] 4.12 Extend `a_namespace_deeper_than_its_driver_declares_is_refused_naming_both_counts` to assert the `declared - 1` direction whenever `declared > 1`, and rewrite its now-false doc comment
- [x] 4.13 Replace "neither dialect's" with "no dialect's" in `capabilities_tests.rs` line 246
- [x] 4.14 Delete `length_precision_and_scale_are_parsed_out_of_data_type` and `a_data_type_embedding_quotes_is_carried_verbatim_into_the_reason` from `trino_type_mapping_tests.rs`, drop the `map(varchar, integer)` half of the space-contrast test, and remove `row("a" integer)` from the non-core-type loop
- [x] 4.15 Extract `is_nullable_cell` as a private free function in `trino_source.rs` and rewrite `nullability_is_read_from_the_catalogs_yes_no_spelling` to assert on it for `"YES"`, `"NO"`, and one unexpected spelling
- [x] 4.16 Add a non-empty and expected-count assertion before the loop in `mirror_declares_the_same_columns_and_types_as_the_virtual_table`
- [x] 4.17 Replace the driver-internal-text disjunction in the plain-HTTP test with assertions on the durable spec clauses (attempt failed, host and port named, no part of the password present), keeping the driver text only in the failure message
- [x] 4.18 Replace `create_connection`'s three `&str` parameters with a `ConnectionSpec` struct, update every call site, and rewrite its doc comment to state it always authenticates as the hostile principal
- [x] 4.19 Rename `the_client_stops_on_the_first_rejected_statement` to `the_client_requests_an_unheaded_unquoted_result`
- [x] 4.20 Delete `the_published_endpoint_and_the_in_network_address_share_a_port` from `tests/common/trino.rs`
- [x] 4.21 Replace `assert_container_address_invariants`' five positional parameters with one `ContainerAddresses` struct and update every call site to construct it by field name
- [x] 4.22 Rename `every_supported_driver_creates_a_virtual_schema` to `every_driver_this_stack_provisions_creates_a_virtual_schema` in `e2e_postgres_test.rs`
- [x] 4.23 Move the six-line inline block in `provisioning_test.rs` into a `///` doc comment on the test and rename it to `two_drivers_install_side_by_side_and_every_driver_identity_is_pairwise_distinct`
- [x] 4.24 Add a one-line doc comment to each `Driver` variant in `driver.rs` naming its canonical `DRIVER` property value
- [x] 4.25 Delete `every_identity_declares_a_qualifier_of_at_least_one_segment` from `driver_tests.rs`
- [x] 4.26 Rewrite `build.rs`'s module doc to state what the script now refuses, and make the matching correction to the `CLAUDE.md` bullet
- [x] 4.27 Put `#[test]` on each of the twelve helpers in `driver_toml_test.rs`, delete the `malformed_driver_toml_fails_the_build` wrapper, and rename the two `package_only` fixture strings
- [x] 4.28 Fold the seven trailing inline annotations in `trino_it_test.rs` into their adjacent assertion messages and move the four-line block into the test's doc comment
- [x] 4.29 Rewrite the driver-load test's doc comment in `trino_it_test.rs` to claim only what it establishes, dropping the filename-heuristic contrast
- [x] 4.30 Add `assert_eq!(batch.num_rows(), 3, …)` after the `.next()` in the `trino_it_test.rs` coercion test, naming the single-batch assumption
- [x] 4.31 Change the Makefile's credential-free negative probe to capture combined output, fail when the command succeeds, and require the captured text to match an authentication refusal
- [x] 4.32 Interpolate `TRINO_HOSTILE_PASSWORD` into `docker-compose.yml` and add a Makefile comment stating the three Rust constants must be kept in step with it
- [x] 4.33 Add `POLL_INTERVAL ?= 5` to the Makefile and use it in both bounded readiness loops
- [x] 4.34 Extract the keystore generation into its own real-file target `$(TRINO_KEYSTORE):`, make `stack-up-trino` depend on it, and drop the `openssl req` output redirect
- [x] 4.35 Rewrite `docker/trino/init.sql`'s header to describe the file as it stands
- [x] 4.36 Replace every plan-local task and gate identifier in shipped code comments with the durable spec feature that owns the claim
- [x] 4.37 Add a private userinfo-stripping helper in `create_schema.rs` and apply it at every site constructing `ConnectionAddressQualifierArity`, `ConnectionAddressQualifierSegment`, or `ConnectionAddressMissingScheme`, plus a redaction test in `dispatch_test.rs` [expert]
- [x] 4.38 Narrow `trino_target`'s doc comment in `connection.rs` to the userinfo the driver's own URL reader recognises, and pin the unencoded `/`, `?`, `#` pass-through shapes in `connection_tests.rs` [expert]
- [x] 4.39 Call `check_namespace_arity(driver, address)?` as the first statement of `source_for` and extend the guard's doc comment to say it now runs on both the address and the source's answer [expert]
- [x] 4.40 Change `TrinoSource::new` to take a `SourceQualifier`, store it in one field, drop the `expect` from `namespace()`, and update the construction site and tests [expert]
- [x] 4.41 Extract the triplicated query-model builders into a new `#[cfg(test)]` module `dialect_test_fixtures.rs` and replace the three copies with `use` of it [expert]

## Phase 3: Verification
- [x] 10.1 Automated checks (build, test, lint, format) — all green, see verification-report.md
- [x] 10.2 Scenario coverage audit — 53/53 rows mapped to a passing test
- [x] 10.3 Manual verification — exercised via equivalent automated coverage, see verification-report.md Notes
