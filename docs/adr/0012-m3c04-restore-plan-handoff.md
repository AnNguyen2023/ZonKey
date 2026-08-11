# ADR 0012: M3C-04 restore-plan handoff

Status: Accepted for M3C-04 implementation.

## Decision

`RestorePlanHandoff` is an owned immutable snapshot created only from the
current plan when `PlanEligibility` is
`EligibleForFutureExecutionConsideration`. It contains rendered and
replacement token values, Unicode-scalar span lengths, policy reason, and a
simulation-only marker.

The snapshot records capture-time eligibility. It has no mutable link back to
the processor and is not a promise that the plan remains current, that a
target contains the token, or that any execution is authorized.

## Stale-after-handoff semantics

After capture, the service may invalidate or replace its current plan because
of semantic input, a new decision, or discontinuity. The previously returned
handoff remains a valid Rust value only as a historical simulation snapshot;
there is no API to present it back as current or to revalidate it against
future service state.

No generation, timestamp, UUID, persistence, or cross-component identity is
added. Capture-time-only semantics are explicit and sufficient until a future
execution boundary requires a separately approved revalidation contract.

## Safety boundary

No round-trip execution API or Windows consumer exists. The handoff contains
no HWND, process/window identity, caret, selection, clipboard, native handle,
or mutable callback. Future execution requires external validation and a new
approval; M3C-04 performs no OS mutation or foreground inspection.
