# Open Questions: add-default-bucketfs-connection

> **Status:** resolved — all five questions answered by the maintainer and folded into the plan. Nothing here blocks implementation.

The questions three review rounds could not settle headlessly, and their answers. `decision-log.md` carries the answers as decisions `[11]` to `[15]` and as the five `[human]` entries under Review Findings; this file is the audit trail of what was asked.

## Round 1

- [x] **Intent Fidelity (BLOCKER):** issue #4 point 3's "Why" names its purpose as a soft landing when an operator pre-places a driver but forgets `DRIVER_PATH`. The plan's pre-placed route fired only when *no* BucketFS connection resolved at all — but `README.md` tells every operator to create the reserved `ADBC_VS_BUCKETFS` connection, and decision `[1]`'s own rationale is that a cluster following the README needs no change. On such a cluster (connection present, no egress, library pre-placed, `DRIVER_PATH` forgotten), the statement still hard-failed with an unreachable-registry error instead of the soft landing the issue asked for. Does issue #4 point 3's soft landing need to also fire when `ADBC_VS_BUCKETFS` resolves but the registry is unreachable — which decision `[3]` forbade as a silent fallback — or is "no BucketFS connection at all" the correct, narrower trigger the plan chose?
  - **Answered:** it must fire in that case too. The widening applies to the reserved connection route alone; an explicit `BUCKETFS_CONNECTION` continues to never fall back, on a failed resolution and an unreachable registry alike. Recorded as decision `[11]`, bounded by decision `[12]`, and reaching all four spec deltas.
- [x] **Scope (ADVISORY, same section):** issue #4 point 4 asks how "look for the default" interacts with the presence check's existing BucketFS read. The plan declines any interaction (decision `[3]`) to avoid reintroducing the "answers for whichever version arrived first" defect the versioned install path was built to remove. Does that declination need explicit ratification, or does the plan's own rationale settle it?
  - **Answered:** the plan's own rationale settles it; no separate sign-off is needed. The declination stands, ratified rather than assumed, and decision `[11]`'s widening leaves it intact — the default location is still never consulted before an index read, only after one that reached nothing.

## Round 2

Both questions are about how round 1's widening is operationalized, not about the widening itself.

- [x] **Feasibility (BLOCKER):** the widened trigger fires on `ProvisioningError::RegistryUnreachable`, and that variant does not mean "the registry answered nothing at all". `provisioning_error` is its sole constructor and runs on both legs of `ensure_driver`, so a package download that reached no host produces it too, and `download` raises the same failure for a body-read error after the status line arrived. Both happen after the index was read, a version was resolved, and the versioned presence check ran, which is where decision `[11]`'s "nothing resolved, nothing installed" argument stops holding. A reset download, or the 600-second transfer timeout expiring on a 46 MB body, would pin the cluster to an unversioned pre-placed library. Narrow the trigger, or accept the wider one?
  - **Answered:** narrow it. Split the variant before the predicate is written: `PackageUnreachable { url, driver, cause }` for the package leg, `TransferInterrupted { url, driver, cause }` for a body that stopped arriving after its status did, and `RegistryUnreachable` for a connect-, DNS-, or TLS-level failure of the index request alone. Recorded as the narrowed boundary in decision `[12]`, with `a_package_download_that_failed_mid_transfer_is_not_an_unreachable_registry` added to task 2.3 and the boundary mirrored into two spec deltas.
- [x] **Requirement Quality (BLOCKER):** the last-resort error was to be built as `FallbackCause::RegistryUnreachable(error.to_string())`. That rendered text ends "there is no path on the mount to pre-load it at, because the install path carries a version only the registry could have resolved", which is the explicit route's remedy. The new route's error would then name the default driver location as the place it looked and, in the same message, tell the operator no such place exists. Split the message, or keep one rendered text?
  - **Answered:** split it. `ProvisioningError::registry_unreachable` returns `Option<(&str, &str)>` rather than a bool, `FallbackCause::RegistryUnreachable` carries the index URL and the transport cause, and `SchemaError::NoDriverRoute`'s `Display` composes its own sentence from them. No text from the explicit route's remedy reaches the new route's error. Recorded as decision `[15]`, with a MUST NOT on `no path on the mount to pre-load` in both the unit test and the `driver-path-override` scenario.

## Round 3

One question, about how far round 2's message fix has to reach.

- [x] **Requirement Quality (BLOCKER):** round 2's fix gave `SchemaError::NoDriverRoute` its own sentence, and `ProvisioningError::RegistryRejected`'s `Display` still ends with the clause "there is no path on the mount to pre-load it at, because the install path carries a version only the registry could have resolved". After this plan that variant is reachable on the derived route: a cluster holding `ADBC_VS_BUCKETFS` whose CDN answers HTTP 503 gets `None` from `registry_unreachable` — correctly, since a host answered — and surfaces the rendered message verbatim. An operator who followed task 4.1's documentation to `adbc-drivers/<driver>/supplied/` is then told no such location exists. Reword the clause on `RegistryRejected` alone, or on every route including the explicit one?
  - **Answered:** reword it everywhere. The old wording is no longer true anywhere in the product, because the path now exists, so keep the clause removed on every route rather than special-casing the explicit one. Recorded as decision `[15]`'s extension, with the deletion specified in task 2.2, `no_provisioning_message_denies_the_default_location` asserting it over every `ProvisioningError` variant, and a MUST NOT clause in both `driver-path-override`'s and `driver-version-resolution`'s scenarios.

## Note for the reviewer

Round 2's eight ADVISORY findings are addressed in the same pass as these two answers. One, `[AMBIGUOUS_REQUIREMENT]` on `SchemaError::NoDriverRoute`'s five fields, is part of the second answer's redesign and reduces the variant to three fields. The other seven are recorded in `decision-log.md` § Review Findings under `[plan-review] Round-2 advisory findings addressed in this pass`.

Round 3's six ADVISORY findings are addressed in the same pass as its answer, under `[plan-review] Round-3 advisory findings addressed in this pass`. The largest is a fourth case the round-2 variant split missed: `download` is shared by both legs, so the index request's own body can stop arriving too, and the exclusion's stated ground changes to the one that holds for every excluded case.

Round 1's three non-intent BLOCKERs were fixed alongside its two answers: the E2E harness collision (task 3.0), the unscoped `driver-version-resolution` scenario (new delta, decision `[13]`), and the missing no-registry-request test (task 2.3).

No further adversarial review round runs against this plan. It ships as a draft PR for human review.
