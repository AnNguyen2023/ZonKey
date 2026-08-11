# ADR 0011: M3C-03 restore-plan preconditions

Status: Accepted for M3C-03 implementation.

## Decision

`PlanEligibility` is a fail-closed, platform-neutral validation result. It
answers only whether the current in-memory plan is eligible for future
execution consideration; it never means executable or safe to edit.

Eligibility requires a current plan, simulation-only execution state, and
internally consistent Unicode-scalar span counts. The plan already exists only
because the current policy produced `RestoreCandidate`; detector and policy
are not rerun. Lifecycle invalidation removes stale plans before validation.

No plan identity, generation, timestamp, UUID, consumed flag, or persistence
is added. `Option<RestorePlan>` is sufficient because stale plans are eagerly
cleared and no cross-component handoff exists in this simulation boundary.

## Internal versus external state

The validator can check only service-owned values: plan presence,
`execution_allowed() == false`, non-empty token values, and stored scalar
lengths matching the token values. It does not validate whether the
foreground application still contains the rendered token, caret or selection
position, surrounding text, editor mutation, focus/window/process identity,
IME/composition state, password/secure state, remote session state, or races
between observation and future mutation. Those conditions remain unknown and
require a separately approved external boundary.

## Safety boundary

Validation is pure/read-only and has no I/O or execution method. It adds no
SendInput, suppression, replay, clipboard, foreground inspection, native
handles, or text mutation. Future execution consideration requires a new
explicitly approved milestone with external validation.
