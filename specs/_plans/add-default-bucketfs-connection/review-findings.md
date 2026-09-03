# Code Review Findings: add-default-bucketfs-connection

## Summary
- Files reviewed: 14
- Total findings: 11 (standard: 8, expert: 3)

## Standard fixes

### crates/adbc-vs-adapter/src/create_schema.rs

#### [DEAD_FLEXIBILITY] `driver_source` returns a `Result` that can never be `Err`

- Location: line 339
- Issue: after the `required_property` call was replaced with `optional_property`, every path through `driver_source` returns `Ok` — the `DRIVER_PATH` arm, the `SelfInstall` arm, and the `DefaultBucketFs` arm. The `Result<DriverSource, SchemaError>` return type now promises a failure mode that no longer exists, and it forces a `?` at the one production call site (`open`, line 254) and an `.expect(...)` in six tests (`create_schema_tests.rs:491, 505, 523, 540, 554, 575`) for an error the function cannot produce.
- Fix: In crates/adbc-vs-adapter/src/create_schema.rs, change `driver_source`'s signature to `fn driver_source(properties: &serde_json::Map<String, Value>) -> DriverSource`, drop the `Ok(...)` wrappers from its two return paths, drop the `?` at its call site in `open` (line 254), and remove the now-unnecessary `.expect(...)`/`.unwrap()` at every `driver_source(` call site in crates/adbc-vs-adapter/src/create_schema_tests.rs (lines 491, 505, 523, 540, 554, 575), keeping each test's assertions unchanged.

### crates/adbc-vs-adapter/src/provisioning_tests.rs

#### [UNTESTED_ERROR_PATH] `provisioning_error`'s leg × point-of-failure table has no direct test

- Location: crates/adbc-vs-adapter/src/provisioning.rs line 1059 (`provisioning_error`); no test file covers it
- Issue: the plan's four-row mapping table is the whole reason `RegistryLeg` exists, but nothing asserts the mapping itself. `PackageUnreachable` and `TransferInterrupted` appear in the repository only as hand-constructed values (`provisioning_tests.rs:457, 462`; `create_schema_tests.rs:844, 853, 861`), never as the output of `provisioning_error`. The one live test that touches the mapping, `provisioning_test.rs::registry_unreachability_is_told_from_a_registry_that_answered`, exercises the index leg alone. A regression that dropped the `leg` argument and mapped both legs' `DownloadFailure::Unreachable` back to `RegistryUnreachable` would leave the entire suite green while re-enabling the fallback on a package download that reached no host — the precise defect the split exists to prevent. `provisioning_tests.rs` is included with `use super::*`, so `provisioning_error`, `DownloadFailure`, and `RegistryLeg` are all reachable from it.
- Fix: In crates/adbc-vs-adapter/src/provisioning_tests.rs, add a table-driven test `each_download_failure_maps_to_one_variant_per_leg` that calls `provisioning_error` for all four combinations of `DownloadFailure::Unreachable("cause".to_owned())`/`DownloadFailure::Interrupted("cause".to_owned())` against `RegistryLeg::Index`/`RegistryLeg::Package` with `Driver::MySql`, and asserts with `matches!` that they produce `ProvisioningError::RegistryUnreachable`, `ProvisioningError::TransferInterrupted`, `ProvisioningError::PackageUnreachable`, and `ProvisioningError::TransferInterrupted` respectively, and that `registry_unreachable()` is `Some` only for the first.

### crates/adbc-vs-adapter/tests/provisioning_test.rs

#### [MISSING_BOUNDARY_TEST] The two mid-transfer tests assert only `is_none()`, which most variants satisfy

- Location: lines 872 and 909
- Issue: `a_package_download_that_failed_mid_transfer_is_not_an_unreachable_registry` and `an_index_read_that_failed_mid_transfer_is_not_an_unreachable_registry` assert only `error.registry_unreachable().is_none()`. Nineteen of the twenty-one `ProvisioningError` variants satisfy that, so both tests pass whether or not the truncated body actually reached `DownloadFailure::Interrupted`. If the truncated `index.yaml` were instead read to completion and rejected as unparseable, or the truncated package rejected as a bad gzip stream, both tests would still pass while `TransferInterrupted` went entirely unexercised — and these two tests are the only claimed coverage of rows 2 and 4 of the plan's mapping table.
- Fix: In crates/adbc-vs-adapter/tests/provisioning_test.rs, add to both `a_package_download_that_failed_mid_transfer_is_not_an_unreachable_registry` (line 872) and `an_index_read_that_failed_mid_transfer_is_not_an_unreachable_registry` (line 909) an assertion that the error is the interrupted-transfer variant, by checking that `error.to_string()` contains `stopped arriving while provisioning` — the fixed wording of `ProvisioningError::TransferInterrupted`'s `Display` — with a failure message stating that a truncated body must reach the interrupted-transfer variant rather than any other refusal.

#### [INLINE_COMMENT] Three-line `//` comment inside `the_default_location_route_makes_no_registry_request`

- Location: lines 773-775
- Issue: the new test carries a bare inline comment ("Prove the stub registry is actually reachable -- self-install hits it and installs successfully -- before trusting the request count below as evidence the default-location route skipped it"). `/speq:code-guardrails` bans inline comments, and every other explanation in the changed files is a doc comment. The same file already carries one pre-existing sibling at line 657 that the Boy Scout rule covers.
- Fix: In crates/adbc-vs-adapter/tests/provisioning_test.rs, delete the inline `//` comment block at lines 773-775 and fold its content into a `///` doc comment on `the_default_location_route_makes_no_registry_request`; do the same for the pre-existing inline `//` comment at line 657 by folding it into `the_driver_path_route_makes_no_registry_request`'s doc comment.

### crates/adbc-vs-adapter/tests/dispatch_test.rs

#### [MISSING_BOUNDARY_TEST] The new dispatch test cannot distinguish the new route from the behavior it replaced

- Location: line 311
- Issue: `an_unnamed_bucketfs_connection_falls_back_to_the_unresolved_reserved_one` asserts only `error.contains("BUCKETFS_CONNECTION")`. The removed behavior — `SchemaError::MissingProperty("BUCKETFS_CONNECTION")`, rendered as "missing required property `BUCKETFS_CONNECTION`" — satisfies that assertion exactly as well as the new `NoDriverRoute` message does. The test therefore cannot fail if `driver_source` is reverted to `required_property`, which is the one regression it was added to guard. It also depends on the host filesystem: the call reaches `open_driver_library` with the compiled-in `BUCKETFS_MOUNT`, so it takes the intended branch only while `/buckets/bfsdefault/default/adbc-drivers/mysql/supplied/libadbc_driver_mysql.so` does not exist on the machine running `make test`.
- Fix: In crates/adbc-vs-adapter/tests/dispatch_test.rs, add to `an_unnamed_bucketfs_connection_falls_back_to_the_unresolved_reserved_one` an assertion that the error contains `ADBC_VS_BUCKETFS` — which the removed missing-property message never named — and an assertion that it does not contain `missing required property`; extend the test's doc comment to state that it reads the real `/buckets/bfsdefault/default` mount because the dispatch tier cannot inject one, and so assumes no pre-placed library exists on the host.

### crates/adbc-vs-adapter/tests/common/exasol.rs

#### [NONDETERMINISTIC_TEST] `copy_bucket_object`'s download has no time bound

- Location: line 235
- Issue: the new `copy_bucket_object` runs `curl` with `--fail --silent --show-error --insecure --user --output`, and no `--max-time`. Every other `curl` invocation in this file bounds itself (`put_file` at line 186, `delete_file` at line 274, both `--max-time 600`), and the harness contract in CLAUDE.md is that every wait is bounded and panics naming its subject. A BucketFS read that accepts the connection and then stalls hangs the whole `--test-threads=1` E2E binary until the CI job timeout, with no diagnosis.
- Fix: In crates/adbc-vs-adapter/tests/common/exasol.rs, add `"--max-time", "600",` to the `curl` argument list in `copy_bucket_object` (line 235), matching the bound `put_file` and `delete_file` already use.

### crates/adbc-vs-adapter/tests/common/driver.rs

#### [INFORMATION_LEAKAGE] Which path segment holds the version is still decided in both E2E binaries

- Location: crates/adbc-vs-adapter/tests/e2e_mysql_test.rs line 1721; crates/adbc-vs-adapter/tests/e2e_postgres_test.rs line 1053
- Issue: `common/driver.rs` exists, by its own doc comment, so that "this is the one place either binary may decide" whether a BucketFS path segment is a driver version. Only the parsing half moved there. The extraction half — strip the `adbc-drivers/<driver>/` prefix, take the first remaining path segment — is written out identically in `installed_driver_entries` (e2e_mysql_test.rs:1721) and `installed_driver` (e2e_postgres_test.rs:1053). The two binaries share one BucketFS, so if the install layout ever gained or lost a segment, one binary could be updated and the other not, and the disagreement would surface as a `supplied` object being returned as "the" installed driver — exactly the failure `common/driver.rs` was created to rule out.
- Fix: In crates/adbc-vs-adapter/tests/common/driver.rs, add `pub fn installed_driver_version(entry: &str, prefix: &str) -> Option<semver::Version>` that strips `prefix` from `entry`, takes the first `/`-separated segment of the remainder, and returns `parse_driver_semver` of it; document that it owns both halves of the decision. Then replace the inline closure in `installed_driver_entries` (crates/adbc-vs-adapter/tests/e2e_mysql_test.rs line 1721) and in `installed_driver` (crates/adbc-vs-adapter/tests/e2e_postgres_test.rs line 1053) with a call to it, and extend `the_supplied_object_is_not_an_installed_driver` to keep asserting both versioned entries survive.

### CLAUDE.md

#### [OUTDATED_COMMENT] The 423-lock bullet names the wrong pair of writers

- Location: line 81
- Issue: the new bullet states that "`e2e_mysql_test.rs` and `e2e_postgres_test.rs` both write `adbc-drivers/mysql/supplied/libadbc_driver_mysql.so`". `e2e_postgres_test.rs` writes no supplied object at all — it only *reads* the listing and filters such an object out (line 1053). The two writers are both inside `e2e_mysql_test.rs`: `a_supplied_library_does_not_short_circuit_self_install` and `a_preplaced_library_answers_when_no_bucketfs_connection_resolves`, which write and delete the same path within one `--test-threads=1` run. As written the bullet sends a future reader to look for a collision in the wrong binary and understates that the collision happens inside a single one.
- Fix: In CLAUDE.md line 81, rewrite the bullet to state that two tests inside `e2e_mysql_test.rs` — `a_supplied_library_does_not_short_circuit_self_install` and `a_preplaced_library_answers_when_no_bucketfs_connection_resolves` — each write and then delete `adbc-drivers/mysql/supplied/libadbc_driver_mysql.so` within one `--test-threads=1` run, so the second write lands inside the first's post-delete lock window, and that this is safe only because `tests/common/exasol.rs::put_file` retries a `423`; keep the note that `e2e_postgres_test.rs` shares the BucketFS and therefore filters non-version segments out of its own listing.

## Expert fixes

### crates/adbc-vs-adapter/src/provisioning.rs

#### [BROAD_CATCH] Every `send()` error becomes `Unreachable`, so a registry that answered can still fire the fallback

- Location: line 1026 (inside `download`)
- Issue: `download` maps every `reqwest::Error` returned by `send()` to `DownloadFailure::Unreachable`, whose own doc comment claims "The request reached no host: a connect-, name-resolution-, or TLS-level failure, with no status line ever received." `send()` also returns `Err` when the redirect policy is exceeded (`Error::is_redirect()`) and when the request could not be built (`Error::is_builder()`). A CDN misconfigured into a redirect loop answers HTTP 3xx repeatedly — a host demonstrably answered — yet on the index leg that produces `RegistryUnreachable`, `registry_unreachable()` returns `Some`, and `self_install_fallback` pins the cluster to whatever pre-placed library sits at the default location for as long as the misconfiguration lasts. That is exactly the outcome `ProvisioningError::registry_unreachable`'s doc comment says cannot happen ("widening any of them would pin a cluster to a pre-placed library on a transient outage") and that decision `[15]` of the plan's log rules out. The taxonomy split in task 2.2 closed three of the four holes; this is the fourth, and it is closable without widening the boundary the plan settled, because these two cases belong on the already-existing "a host answered / this says nothing about egress" side.
- Fix: In crates/adbc-vs-adapter/src/provisioning.rs, add a fifth case `DownloadFailure::Answered(String)` documented as a request the host answered in a way the client could not follow; in `download` (line 1026) replace the single `send()` mapping with a classification that raises `DownloadFailure::Answered(error.to_string())` when `error.is_redirect() || error.is_builder()` and `DownloadFailure::Unreachable(error.to_string())` otherwise; in `provisioning_error` (line 1059) map `Answered` leg-independently — the same way `Interrupted` is mapped — to a new `ProvisioningError::RegistryMisdirected { url, driver, cause }` whose `Display` names the URL, the driver, the cause, and `DRIVER_PATH` as the remedy and denies no pre-load location; add its arm to `registry_unreachable`'s wildcard-free `None` list; add a constructed value of it to `every_provisioning_error()` in crates/adbc-vs-adapter/src/provisioning_tests.rs; and extend the `provisioning_error` mapping test to assert `Answered` yields `registry_unreachable() == None` on both `RegistryLeg::Index` and `RegistryLeg::Package`.

### crates/adbc-vs-adapter/src/create_schema.rs

#### [SWALLOWED_ERROR] `default_location_library` discards the cause `readable_library` was extracted to provide

- Location: line 431
- Issue: `default_location_library` tests the pre-placed library with `provisioning::readable_library(&path).is_ok()` and throws the `Err(String)` away. `readable_library`'s own doc comment states its contract as "The `Err` carries only the cause, never a message, so each route phrases its own error — naming its own property and its own remedy — over one shared answer", and the `DRIVER_PATH` route honours that by folding the cause into `SuppliedDriverUnreadable`. The default-location route does not, so `NoDriverRoute`'s `Display` asserts "No readable file is at the default driver location {path} either" even when a file *is* there and is unreadable for a stated reason — a directory at that path, or a permission error. That is a false statement in the one message the plan calls "the whole diagnosis they have to act on", in a module that went out of its way to delete a different false clause ("no path on the mount to pre-load it at") from every provisioning message. The operator most likely to hit it is precisely the air-gapped one who followed `README.md` and got the mode bits or the path shape slightly wrong, and the message tells them nothing is there.
- Fix: In crates/adbc-vs-adapter/src/create_schema.rs, change `default_location_library` (line 431) to bind `readable_library`'s `Err(String)` and carry it into a new `unreadable: Option<String>` field on `SchemaError::NoDriverRoute`; in `write_no_driver_route`, render the final sentence as the current wording when the field is `None` and as a sentence naming the path and the cause when it is `Some` — this is not the pairing the plan's three-field decision rules out, because it reports why the *location* failed and never a second reason self-install failed, which `FallbackCause` still owns alone. Add a test `no_driver_route_names_why_the_default_location_was_unusable` to crates/adbc-vs-adapter/src/create_schema_tests.rs that creates a directory at `provisioning::default_driver_path(mount, Driver::MySql)` and asserts the message names both the path and a not-a-regular-file cause, and update `no_driver_route_names_the_location_the_connection_and_both_properties` and `no_driver_route_names_the_registry_it_could_not_reach` to construct the new field as `None`.

### crates/adbc-vs-adapter/tests/e2e_mysql_test.rs

#### [TACTICAL_SHORTCUT] Both Drop guards panic on a failed cleanup, which aborts the run before the second guard restores anything

- Location: lines 218 and 297
- Issue: the guard placement and drop order are correct — `ReservedConnectionGuard` is constructed before the `DROP CONNECTION`, `BucketObjectGuard` after the copy, and reverse-declaration order drops the object guard first — so the happy path and the ordinary unwind path both restore the stack. The failure path does not. `BucketObjectGuard::drop` calls `Exasol::delete_file`, and `ReservedConnectionGuard::drop` calls `schema::create_bucketfs_connection`; both reach `common::run`, which panics on a non-zero `curl`/`exapump` exit. A panic raised inside a `Drop` that is itself running during an unwind is a double panic, which Rust turns into `std::process::abort()`. In `a_preplaced_library_answers_when_no_bucketfs_connection_resolves` that abort happens while unwinding through `BucketObjectGuard`, which drops *first* — so `ReservedConnectionGuard` never runs and `ADBC_VS_BUCKETFS` stays dropped in the container, killing every remaining test in the binary and the next `make test-e2e` run alike. That is the exact cascade the guard's own doc comment says it exists to prevent, and the comment's claim that a failed restore "aborts the run beside the original panic, which is the honest outcome" understates it: the abort also cancels the restore that had not run yet. The verification the implementer reports — inducing a panic in the guarded body and seeing state restored — exercises only the path where both cleanups succeed, so it cannot have covered this.
- Fix: In crates/adbc-vs-adapter/tests/e2e_mysql_test.rs, wrap each guard's cleanup so it can never panic during an unwind: in `BucketObjectGuard::drop` (line 218) and `ReservedConnectionGuard::drop` (line 297), run the cleanup call inside `std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| ...))`, and on `Err` either re-panic when `!std::thread::panicking()` or `eprintln!` a message naming the guard, the subject it failed to restore, and the instruction to reset the stack with `make stack-down && make stack-up` when `std::thread::panicking()` is true. Correct `ReservedConnectionGuard`'s doc comment to state that a failed restore during an unwind is reported rather than raised, because raising it would abort before the remaining guards run.
