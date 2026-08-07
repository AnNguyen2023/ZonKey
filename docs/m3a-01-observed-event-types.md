# M3A-01 — observed-input value types

Status: implemented as platform-neutral value types only. No observer runtime
exists.

## Type inventory

The `zonkey-types` crate now owns the neutral vocabulary for a future adapter:

- `KeyEventKind`: `KeyDown`, `KeyUp`, `SystemKeyDown`, `SystemKeyUp`.
- `ModifierKey`: `Shift`, `Control`, `Alt`, `Meta`.
- `ObservedKey`: validated letters, digits, space, enter, tab, backspace,
  escape, ASCII punctuation, modifiers, and `Other`.
- `ModifierState`: immutable four-modifier state with explicit accessors and
  builder methods; no public bitmask.
- `InjectionOrigin`: observation metadata only (`Unknown`,
  `PhysicalOrUnmarked`, `MarkedInjected`, `LowerIntegrityInjected`).
- `EventSequence`: non-zero newtype; zero construction is rejected.
- `ObservedInputEvent`: key, kind, modifiers, injection origin, and sequence.
- `IntegrityRelation`: future best-effort metadata (`SameOrUnknown`, `Lower`,
  `Higher`, `Unavailable`).
- `ForegroundContextSnapshot`: existing `InputContext`, optional bounded
  sanitized labels, integrity relation, and secure-desktop flag.
- `ObserverStatus`: lifecycle vocabulary only.
- `ObserverError`: redacted error categories without native OS payloads.

`TokenBoundary` remains the existing shared type and is not duplicated.

## Invariants

`ObservedKey::letter` accepts only ASCII alphabetic characters. The punctuation
constructor accepts only ASCII punctuation. `ObservedKey::digit` accepts only
values `0..=9`; native virtual-key numbers are not modeled. The enum-like
categories are represented behind a private validated representation, so
callers cannot construct arbitrary Unicode values through a public variant.

`ModifierState` has private fields and defaults to all modifiers clear.
`EventSequence` cannot contain zero. Context labels are optional, ASCII-only,
restricted to safe basename/class characters, and bounded to 64 bytes.
`ForegroundContextSnapshot::unknown()` contains no identifying metadata.
Secure desktop or `InputContext::Secure` makes token diagnostics ineligible.

## Privacy restrictions

Observed events contain no token text, key history, timestamp, window title,
full executable path, command line, PID, HWND, handle, native event object, or
user text. Injection origin is best-effort observation metadata and is not a
trust/security decision. Integrity relation is future best-effort platform
metadata only.

## Intentional non-capabilities

These types cannot observe input, start a thread, allocate a queue, log text,
write diagnostics, inspect applications, inject, replay, suppress, block,
replace, or edit text. They contain no OS dependency, FFI, Windows type, or
unsafe code. `zonkey-types` remains dependency-free.

## Future `zonkey-win` responsibilities

A later adapter may translate native observations into these values, while
forwarding input immediately. It must handle native lifecycle and conversion,
but must not leak native handles or payloads into this crate. Context discovery,
queueing, service lifecycle, diagnostics, and hook review are separate work.
The future dependency direction is `zonkey-win -> zonkey-types` only.

## Deferred to M3A-02

M3A-02 must decide bounded queue capacity, non-blocking producer behavior,
loss/overflow reporting, shutdown and drain semantics, lifecycle ownership, and
whether any service-loop abstraction is appropriate. No queue, channel, thread,
callback, or service loop is part of M3A-01.
