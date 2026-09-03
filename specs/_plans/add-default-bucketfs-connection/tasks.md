# Tasks: add-default-bucketfs-connection

## Phase 2: Implementation (Group A)
- [x] 1.1 Prove a CONNECTION the adapter cannot resolve surfaces as the adapter's own error rather than killing the statement (e2e_mysql_test.rs, tests/common/schema.rs)

## Phase 2: Implementation (Group B — sequential, one module pair)
- [x] 2.1 Default the BucketFS write credential to the reserved connection ADBC_VS_BUCKETFS (create_schema.rs, create_schema_tests.rs, tests/common/schema.rs)
- [x] 2.2 Add the default driver location and both fallback paths into the last-resort route (provisioning.rs, create_schema.rs, provisioning_tests.rs, create_schema_tests.rs) [expert]
- [x] 2.3 Prove at the provisioning tier that self-install ignores/preserves a pre-placed library and reaches no registry unnecessarily (provisioning_test.rs)

## Phase 2: Implementation (Group C — sequential, one test binary, --test-threads=1)
- [x] 3.0 Prepare the MySQL E2E harness for a non-installed object under the driver prefix (e2e_mysql_test.rs, e2e_postgres_test.rs, tests/common/driver.rs)
- [x] 3.1 Prove end to end that a statement naming no BUCKETFS_CONNECTION self-installs through the reserved connection (e2e_mysql_test.rs)
- [x] 3.2 Prove end to end that a pre-placed library does not short-circuit self-install while the registry is reachable (e2e_mysql_test.rs, tests/common/exasol.rs)
- [x] 3.3 Prove end to end that the default driver location answers when no BucketFS connection resolves, and an empty one fails naming every route (e2e_mysql_test.rs) [expert]

## Phase 2: Implementation (Group D)
- [x] 4.1 Update README.md and CLAUDE.md to state the new properties/precedence

## Phase 4: Review Fixes
- [x] 4.1 Classify a redirect-loop or unbuildable registry request as answered rather than unreachable, so it cannot fire the last-resort fallback (provisioning.rs, provisioning_tests.rs) [expert]
- [x] 4.2 Carry why the default driver location was unusable into the no-route refusal (create_schema.rs, create_schema_tests.rs) [expert]
- [x] 4.3 Report rather than raise a failed guard restore during an unwind, so the first guard's failure cannot abort before the second restores (e2e_mysql_test.rs) [expert]
- [x] 4.4 Drop the dead `Result` from `driver_source`, its `?` call site, and every now-unnecessary `.expect`/`.unwrap()` at its test call sites (create_schema.rs, create_schema_tests.rs)
- [x] 4.5 Assert the mid-transfer tests reach the interrupted-transfer variant specifically, not just a non-unreachable one (provisioning_test.rs)
- [x] 4.6 Fold the inline `//` comments in the driver-path and default-location no-registry-request tests into doc comments (provisioning_test.rs)
- [x] 4.7 Assert the unnamed-BucketFS-connection dispatch test names `ADBC_VS_BUCKETFS` and not the removed missing-property wording, and document its real-mount dependency (dispatch_test.rs)
- [x] 4.8 Bound `copy_bucket_object`'s curl with `--max-time 600`, matching `put_file`/`delete_file` (tests/common/exasol.rs)
- [x] 4.9 Add `installed_driver_version` to own both the extraction and parsing of a driver's version segment, and call it from both E2E binaries instead of each duplicating the extraction (tests/common/driver.rs, e2e_mysql_test.rs, e2e_postgres_test.rs)
- [x] 4.10 Correct the 423-lock bullet to name both writers as tests inside e2e_mysql_test.rs, not one per binary (CLAUDE.md)

## Phase 3: Verification
- [x] 5a.1 Run make cross-udf-build (exit 0)
- [x] 5a.2 Run make fmt-check (no changes)
- [x] 5a.3 Run make lint (0 warnings)
- [x] 5a.4 Run make test (0 failures)
- [x] 5a.5 Run make test-artifact (0 failures)
- [x] 5a.6 Run make test-it (0 failures)
- [x] 5a.7 Run make test-it-postgres (0 failures)
- [x] 5a.8 Run make test-e2e (0 failures)
- [x] 5a.9 Run make test-e2e-postgres (0 failures)
- [x] 5a.10 Run make deny (0 findings)
- [x] 5b Scenario coverage audit against plan.md Verification/Scenario Coverage table
- [x] 5c Manual testing per plan.md Verification/Manual Testing table
- [x] 6 Generate verification-report.md
