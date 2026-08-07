# M3A-02 — bounded observe-only event pipeline

Status: implemented as a synchronous, in-memory, mock-only foundation. It is
not a runtime observer and has no keyboard or operating-system event source.

## Queue contract

`zonkey-service` owns `ObserveQueue`. Its default capacity is exactly 256
events. A non-zero custom capacity is accepted for deterministic tests; zero is
rejected with a typed error. The fixed bound prevents event accumulation from
becoming an unbounded memory or privacy surface.

The queue is FIFO. `try_enqueue` never waits: when capacity is available it
appends, and when full it returns `DroppedFull` for the newest event. Existing
older events are never evicted. Each drop increments the monotonic dropped
counter, saturating at `u64::MAX` rather than panicking or wrapping. Queue
state exposes only capacity, current length, dropped count, and whether
continuity is pending.

## Loss and discontinuity

The queue does not infer or reconstruct missing events. One or more drops create
one pending loss episode. The first later successful dequeue carries
`discontinuity_before_event = true`; the queue clears the marker immediately,
so that episode is reported exactly once. A later overflow after the marker has
been consumed creates a new episode.

For example, with capacity two:

```text
enqueue A, enqueue B, drop C, drop D
dequeue A -> discontinuity_before_event = true
dequeue B -> discontinuity_before_event = false
```

The service must reset processor token state before processing the marked event.
It must not carry token state across a loss boundary.

After all accepted events are drained, the discontinuity marker has already
been consumed by the first dequeued event and is no longer pending. If a drop
occurs while no later event is dequeued, the marker remains pending until the
next successful dequeue; an empty dequeue never consumes it.

## Synchronous service

`EventSource` is a mock-only trait returning `Ok(Some(event))`, `Ok(None)` for
normal exhaustion, or a typed `ObserverError`. `EventProcessor` receives only a
validated `ObservedInputEvent` or a discontinuity reset notification and
returns aggregate classification (`Ignored`, `Observed`, `BoundaryObserved`,
`Unsupported`, or `Invalid`). It cannot return text, an edit command, or an
`EditPlan`.

`ObserveService` is deterministic and synchronous. Lifecycle is:

```text
Starting -> Running -> Stopping -> Stopped
                         \-> Failed
```

Normal source exhaustion and an explicit idempotent stop perform a graceful
drain of accepted queued events before `Stopped`. A source error increments the
source-failure counter and transitions to `Failed`; no missing event is
fabricated. The aggregate report contains only received, accepted, dropped,
processed, discontinuity, source-failure, unsupported, and invalid counters.

The current synchronous `run` first consumes the finite mock source into the
bounded queue and then drains it. Therefore a finite source larger than the
configured capacity can genuinely exercise service-level overflow; the report's
`dropped` counter reflects those queue drops. A future producer/consumer
separation is not implied by this test-only arrangement.

## Boundary and privacy

This pipeline is platform-neutral, in-memory, synchronous, and mock-only. It
does not spawn threads, use async or OS channels, inspect processes/windows,
write diagnostics, persist data, access networks, observe real keyboard input,
or modify user text. It never executes an edit plan, injects, replays,
suppresses, blocks, or replaces input.

## Deferred work

The following remain outside M3A-02 and require separate review:

- Windows key/event mapping;
- a real keyboard hook or callback registration;
- foreground context and integrity discovery;
- an aggregate/redacted diagnostics writer;
- every form of text editing or controlled execution.
