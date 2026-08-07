# ADR 0003: M3A observe-only boundary

- Status: Proposed design baseline
- Date: 2026-08-07
- Scope: Windows observe-only milestone M3A

## Context

Zonkey M1 and M2 are platform-independent. They maintain raw/rendered token
state, compose Telex text, classify lexical evidence, and return decisions or
abstract edit plans. They do not acquire global input or execute edits. A
future Windows adapter needs a safety boundary before any event source is
considered.

Global observation is sensitive even when it is nominally read-only. It can
cross elevated-application, secure-desktop, RDP, and VM-console boundaries;
focus can change while an event is queued; and diagnostics can accidentally
become keystroke collection.

## Decision

M3A is documentation and contract review only. It introduces no runtime,
Windows dependency, hook, FFI, unsafe block, queue, service loop, CLI command,
diagnostic writer, persistence, or context detector.

The future design separates observation from all controlled execution:

```text
future callback -> bounded loss-aware queue -> future service loop
                -> platform-neutral detection/policy
                -> opt-in aggregate/redacted diagnostics
```

This diagram is non-implemented. M3A has no public or internal capability to
alter user input. In particular, it cannot inject, replay, suppress, replace,
or block text or keyboard events.

A future hook callback must forward input immediately to the operating system
and report observation side-band. It must not wait for Zonkey, call core logic,
or perform a replacement. Any controlled editing, if ever approved, is a
separate milestone with separate contracts, tests, and owner review.

Diagnostics are opt-in and redacted by default. They must not contain plaintext
tokens, raw keystrokes, window titles, full executable paths, or network
telemetry.

## Alternatives considered

1. Implement the Windows hook and observe-only loop in M3A. Rejected: it would
   combine a sensitive event source with an unreviewed lifecycle and privacy
   boundary.
2. Let the callback invoke detection/policy synchronously. Rejected: callback
   latency, allocation, and shutdown behavior would become input-path risks.
3. Reuse an existing injection-oriented adapter design. Rejected: it violates
   the clean-room boundary and makes alteration capability too easy to expose.
4. Log raw events for debugging. Rejected: it creates a keystroke/privacy
   liability and is unnecessary for an aggregate observe-only baseline.
5. Use an unbounded channel to avoid event loss. Rejected: it permits memory
   growth and obscures overflow risk; bounded loss must be explicit instead.

## Consequences

Positive consequences:

- M1/M2 stay deterministic, testable, and free of Windows APIs and unsafe code.
- Observation and controlled editing receive separate threat-model review.
- Queue overflow, shutdown, integrity, and privacy behavior must be specified
  before implementation.
- The current repository remains incapable of changing user input.

Costs and risks:

- M3A does not provide a runnable observer.
- Some event semantics cannot be validated until a later isolated adapter exists.
- A future implementation must resolve elevated apps, secure desktop, RDP,
  focus changes, injected-event metadata, and privacy retention explicitly.

## Owner review gates

Before M3B, owners must approve M3A-01 event types, M3A-02 bounded queue and
lifecycle, M3A-03 conversion, M3A-04 context, M3A-05 diagnostics, and M3A-06
observe-only hook as separate deliverables. No approval is implied by this ADR.
