# ADR 0031: M3D-26 controlled mutation contract harness

Status: Proposed for M3D-26 owner review.

M3D-26 implements and tests the ADR 0030 semantics on a deterministic dummy
cooperating host only. `zonkey-service` gains a `#[cfg(test)]` harness
(`mutation_harness.rs` + `mutation_harness_tests.rs`): a fixed-order
eligibility validator (protocol, host/session/document/editor identity,
caret/selection, host-owned UTF-16 range with surrogate-split rejection,
exact expected text, version match, secure/session state, composition,
capabilities, idempotent request id), one atomic host-owned
compare-and-replace whose in-transaction re-read is the only authorization
window, post-commit verification requiring exactly `revision + 1` and the
exact intended text, the reused bounded `BoundedRequestLedger` for
idempotency, and an Indeterminate workflow that blocks the logical target
(document URI + rendered token) until a reconciliation readback
(`AppliedAcknowledged` / `NotApplied` / `ConflictHumanReview`) plus an
explicit owner acknowledgement.

Synthetic evidence lives only in this harness: the test host may set
`CompositionState::Inactive` and `CAP_COMPOSITION_PROOF`, and an `Inactive`
state without the proof capability still fails closed as
`CompositionUnknown` — proof, not assertion, is required. The harness is
test-only code compiled behind `#[cfg(test)]`; nothing reaches the real VS
Code binding, which still reports `CompositionUnknown`, and no
`TextEditor.edit` wiring, Applied switch, SendInput, Win32 mutation,
clipboard, or suppression/replay exists anywhere.

Twenty deterministic tests cover: the happy Applied path (including a
non-BMP surrogate-safe range), every major eligibility rejection without
mutation, stale version, in-transaction text mismatch, composition
Unknown/Active rejection, missing `CAP_COMPOSITION_PROOF`, duplicate
Applied replay without reapplication, conflicting request-id reuse, invalid
request id, refused transactions, lost outcomes replaying as recorded
Indeterminate, commit-then-lost reconciling as `AppliedAcknowledged`,
lost-before-commit as `NotApplied`, contradictory live state as
`ConflictHumanReview`, the blocked-target lifecycle requiring reconciliation
then acknowledgement, revision overflow, and no-partial-mutation on every
rejection path.
