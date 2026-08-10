# ADR 0004: M3A-03 observe adapter boundary

- Status: Accepted
- Date: 2026-08-10

## Context

M3A-01 defines validated, platform-neutral `ObservedInputEvent` contracts in
`zonkey-types`. M3A-02 provides a bounded FIFO `ObserveQueue` and a
synchronous mock `EventSource`/`EventProcessor` pipeline in `zonkey-service`.
The next step must define how a future platform producer hands already
validated events to that pipeline without introducing Windows runtime behavior.

## Decision

M3A-03 is the platform-adapter contract only (Option A). It defines the
boundary between a future platform-specific producer and the existing
observe-only pipeline:

```text
future platform producer (zonkey-win)
        -> ObservedInputEvent
        -> ObserveQueue / ObserveService
        -> EventProcessor
```

The producer owns platform translation and validation. Once handed across the
boundary, an `ObservedInputEvent` is an owned immutable value. The queue stores
owned values; processors receive shared immutable references and may not mutate
or replace events. No new runtime harness, thread, async runtime, OS channel,
or cancellation primitive is introduced by M3A-03.

## Alternatives considered

### Option B: deterministic platform-neutral runtime harness

Rejected for M3A-03. A producer/consumer runtime with cancellation and
backpressure separation would enlarge the lifecycle and concurrency surface
without providing a platform adapter. It can be considered only after a
separate owner-approved milestone.

### Windows hook implementation

Deferred. It would require a new Windows safety/runtime decision and is outside
the current observe-only foundation task.

## Safety boundary

M3A-03 accepts only validated `ObservedInputEvent` values. It must not:

- capture real keyboard input in this milestone;
- consume input in a way that prevents another application receiving it;
- edit, suppress, replay, inject, replace, or block events;
- inspect foreground windows, processes, secrets, handles, or native values;
- persist raw input outside explicit in-memory test fixtures.

No Win32 dependency, FFI, `unsafe`, callback registration, clipboard access,
GUI, installer, startup registration, telemetry, or diagnostics persistence is
part of this boundary.

## Lifecycle semantics

The existing M3A-02 lifecycle remains authoritative:

- `Starting -> Running` when the synchronous service begins;
- an explicit stop request prevents further source polling;
- `Stopping` drains accepted queue entries FIFO exactly once;
- normal source exhaustion follows drain to `Stopped`;
- repeated stop requests are idempotent;
- a terminal `Stopped` or `Failed` service cannot resume.

Producer and processor failures are typed aggregate outcomes. A source failure
transitions to `Failed` and does not drain unprocessed queue entries. Processor
classification remains aggregate-only; a future processor failure must have a
separate approved error contract before implementation.

## Backpressure / overflow semantics

The M3A-02 queue semantics are retained unchanged: capacity is bounded (256 by
default), `try_enqueue` never blocks, and a full queue rejects the newest
incoming event. Older accepted events remain FIFO-intact. Each rejected event
increments the monotonic dropped counter and creates a pending discontinuity;
the first later dequeued event carries the marker exactly once. No event is
reconstructed or silently evicted.

## Error semantics

- Queue-full is a normal, observable drop outcome, not a panic or blocking
  condition.
- Source failure is fatal to the current service run and increments the typed
  failure counter once.
- Invalid events are rejected before processing and represented only by typed
  aggregate classification.
- Stop and normal exhaustion are non-error lifecycle outcomes.
- No logging framework or raw error payload is added by this ADR.

## Non-goals

M3A-03 does not implement Windows keyboard hooks, `SetWindowsHookEx`, Raw
Input, foreground inspection, UI Automation, clipboard access, `SendInput`,
injection, replay, suppression, IME replacement, text editing, EditPlan
execution, diagnostics files, telemetry, GUI, installer, startup registration,
or any persistence.

## Acceptance criteria

An implementation of this ADR must demonstrate through public APIs and tests:

1. A future producer can supply only validated `ObservedInputEvent` values.
2. Events are owned at the queue boundary and processed in FIFO order.
3. Processor code receives immutable event references and aggregate
   classifications only; it cannot return text or edit commands.
4. Queue capacity never grows beyond its configured bound.
5. Full-queue behavior is newest-event rejection with an observable drop and
   discontinuity marker.
6. Stop, exhaustion, source failure, and queue drain have deterministic
   terminal semantics with no deadlock or blocking producer.
7. The platform-neutral implementation adds no Windows dependency, FFI, or
   `unsafe` code.

## Consequences

This keeps the next integration seam small and testable while preserving the
strict observe-only boundary. It does not provide real observation or
concurrent producer/consumer behavior; those remain explicit future work.
The adapter contract can be implemented later by `zonkey-win` only after a
separate owner-approved Windows runtime decision.

## Follow-up milestone

The smallest implementation follow-up is to add a platform-neutral adapter
contract test using a mock producer that constructs existing validated events
and feeds the existing `ObserveService`. No Windows source or runtime harness
should be added in that task.
