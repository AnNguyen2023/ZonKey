# ADR 0013: M3C-05 handoff revalidation

Status: Accepted for M3C-05 implementation.

## Decision

`revalidate_restore_handoff()` compares an owned handoff with the current
service-held logical plan and returns `HandoffRevalidation::Current` or a
fail-closed `Stale` reason. `Current` means only that the snapshot still
matches service state; it does not verify an editor, target, caret, or any
execution precondition.

Structural equality alone is insufficient: two separate `resume` decisions
can have identical fields. Therefore each newly created RestorePlan receives
a service-local monotonic `u64` generation, captured in its handoff.

Generation starts at 1, increments only when a RestoreCandidate creates a
plan, and is unchanged by invalidation, key-up, modifier-only input, injected
input, or inspection. Increment uses checked arithmetic; after generation
`u64::MAX` is issued, allocation is exhausted and future candidates fail
closed rather than wrapping and risking identity reuse. The generation is
not a timestamp, UUID, OS sequence, global identity, or persisted value.

## Revalidation and stale snapshots

No current plan yields `Stale(NoCurrentPlan)`. A malformed handoff fails closed
as `Stale(MalformedSnapshot)`. A different generation or plan content yields
`Stale(DifferentGeneration)`. A captured handoff remains historical immutable
data after service invalidation or replacement; no round-trip execution or
external-state revalidation API exists.

Foreground text, caret, selection, focus, window/process identity,
IME/composition, secure-field state, actual editor units, and observation/edit
races remain unknown and out of scope. Execution requires a separately
approved boundary.
