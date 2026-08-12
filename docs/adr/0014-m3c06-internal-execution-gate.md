# ADR 0014: M3C-06 internal execution gate

Status: Accepted for M3C-06 design.

## Decision

`evaluate_internal_execution_gate()` composes existing plan eligibility,
current handoff capture, and handoff revalidation. A
`PassedForExternalValidation` result means only that service-owned evidence
is sufficient to stop at the boundary before a future external-validation
stage. It never means executable, authorized, safe to edit, or target
verified.

The gate rejects fail-closed when current plan evidence is absent or
ineligible, handoff creation/revalidation fails, generations differ, or the
simulation-only invariant is broken. It does not duplicate lifecycle, span,
generation, or policy logic.

## External barrier

The following remain UNKNOWN and require a separately approved external
boundary: foreground text match, caret, selection, surrounding text, focus
stability, window/process identity, IME/composition, secure/password state,
remote/session restrictions, editor operation-unit mapping, app mutation
behavior, and observation-to-edit races. A prior policy context is historical
logical evidence, not fresh secure/editor verification.

The gate therefore stops before any OS consumer. M3C-06 adds no execution
trait, SendInput, replay, suppression, clipboard, foreground inspection, or
text mutation. Future work must obtain external evidence without assuming the
service-side pass resolves the race between observation and mutation.
