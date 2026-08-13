# ADR 0017: M3C-09 controlled external-validation test surface

Status: Accepted for M3C-09 design.

M3C-09 defines a deterministic, platform-neutral test surface for exercising
the M3C-07 external-validation contract. It is test infrastructure, not a
production editor and not evidence that any production editor is safe.

## State model

The owned surface contains text, a caret, an optional selection, a composite
logical `SurfaceId`, an explicit operation-unit model, composition state,
secure state, session state, and a monotonic revision. It has no native handle,
window, process, or platform value.

Text is owned and compared exactly. No normalization is implicit. The surface
uses an abstract editor-native `TextUnit` coordinate; the model is deliberately
distinct from Rust byte offsets and Unicode scalar counts. A selected unit model
defines deterministic boundaries for the test surface, while a future adapter
must prove its own mapping into that model.

## Caret and selection

The caret is an insertion position in `TextUnit` coordinates from zero through
the buffer end. `NoSelection` is the initial state; `Selection { start, end }`
is representable for deterministic rejection tests. Any non-empty selection
fails closed. Only the exact range immediately preceding the caret is eligible;
start/end positions, a caret in the middle of a token, an overlong token, or an
ambiguous unit conversion reject.

`SurfaceId` is a deterministic test-provided composite identity (for example,
surface, document, and control IDs). It distinguishes identical content on
different surfaces and a recycled surface with identical text. It is not a PID,
HWND, UUID, or production identity claim.

Composition is `Inactive`, `Active`, or `Unknown`; active and unknown reject.
Secure state is `KnownNonSecure`, `Secure`, or `Unknown`; only the first may
proceed. Session state is `SupportedLocal`, `UnsupportedRemote`, or `Unknown`;
only supported local may proceed. Secure tests never require reading or logging
secret content.

## Revision and snapshots

Revision is a checked `u64` starting at one. Every text, caret, selection,
identity, operation-unit, composition, secure, or session change advances it.
At `u64::MAX`, a change requiring a new revision fails closed; it never wraps
or reuses a revision.

`ControlledEvidenceSnapshot` is immutable historical data containing identity,
revision, exact target-range evidence, caret, selection, operation-unit proof,
composition, secure, and session state. It has no mutation functions.

A handoff may be compared with a snapshot only when service-current handoff
state is already current. Binding then requires matching identity and revision,
exact rendered text in the range immediately before the caret, empty selection,
inactive composition, known non-secure state, supported session, and proven
operation units.

## Race and compare-and-replace oracle

The surface models text, caret, selection, identity, secure, composition, and
session races. Each mutation advances revision, so an old snapshot rejects even
when content later becomes identical. A conceptual test-only
`compare_revision_and_replace(expected_surface_id, expected_revision,
expected_range, replacement)` succeeds only when identity, revision, exact
text, range, and required state predicates still match; otherwise it returns a
rejection and performs no mutation. This is a deterministic CAS-equivalent test
oracle, not SendInput or real editing.

Unconditional snapshot-then-later-mutation remains `CHECK_THEN_ACT_RACE`.
The compare-and-replace oracle demonstrates only that the contract can express
a closed race in a controlled model; it does not prove a production API can do
so.

Failure vocabulary includes `TargetIdentityMismatch`, `RevisionMismatch`,
`TextMismatch`, `CaretMismatch`, `SelectionNotEmpty`, `CompositionActive`,
`CompositionUnknown`, `SecureTarget`, `SecureUnknown`, `UnsupportedSession`,
`SessionUnknown`, `OperationUnitsUnproven`, and `RangeInvalid`.

## Required future tests

The implementation must cover a matching happy path; text, caret, selection,
identity, secure, composition, session, and same-content-new-revision races;
two identical surfaces with different identities; invalid ranges; and checked
revision overflow. Secure rejection must depend on metadata, not secret text.

## Production non-equivalence and ownership

Passing these tests proves only the logical evidence model, state coherence,
and deterministic race contract. It does not prove UIA, TSF, real editor
operation units, production atomicity, or safe mutation.

The future implementation should begin as a test-only module in
`zonkey-service`: it reuses service-side handoff types while keeping the model
platform-neutral and isolated. A dedicated crate can be reconsidered only if
reuse or dependency isolation later requires it. No implementation is part of
M3C-09.
