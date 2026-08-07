# M3A — Windows observe-only scope

Status: design baseline only. No M3A runtime exists in this repository.

## Scope and goals

M3A establishes the reviewed boundary for a future Windows adapter that can
observe keyboard-related events without changing user input. The milestone
defines platform-neutral contracts, trust boundaries, privacy defaults, queue
semantics, lifecycle/error categories, and acceptance criteria. It does not
implement event acquisition, a service loop, diagnostics writing, or any
Windows API.

The existing M1/M2 crates remain deterministic and platform-independent. M1
produces Telex rendering and abstract edit data; M2 produces lexical evidence,
policy decisions, and optional abstract plans. No current crate executes those
plans.

## Explicit non-goals

M3A does not implement or authorize:

- keyboard hooks, SendInput, injection, replay, suppression, or keyboard
  blocking;
- clipboard replacement, TSF editing, text replacement, or execution of an
  EditPlan;
- tray UI, GUI, autorun, installer, service registration, or persistence;
- foreground-process inspection, window-title capture, executable-path capture,
  elevation handling, RDP support, or secure-desktop interaction;
- persistent/global keyboard capture, background threads, a CLI observer
  command, a diagnostics writer, or any runtime behavior.

## Future trust boundary

This is a design diagram, not an implemented data path:

```text
future hook callback
        │  immediately forward; never suppress or mutate
        ▼
bounded queue (non-blocking producer, loss-aware)
        ▼
future service loop (lifecycle, shutdown, overflow accounting)
        ▼
core detection/policy (platform-neutral facts and decisions)
        ▼
aggregate/redacted diagnostics (opt-in sink only)
```

The callback must not call the Telex engine, policy, filesystem, network, or a
text-editing API. A future adapter must forward the observed input immediately
to the operating system and treat observation as side-band metadata only.

## Proposed contracts (not implemented)

The following Rust-like shapes are review targets for M3A-01. They are not
public APIs and must not be added during M3A.

```text
ObservedKeyboardEvent {
    kind: KeyEventKind,
    key: PlatformNeutralKey,
    modifiers: ModifierState,
    injected: InjectedEventMetadata,
    observed_at: MonotonicTimestamp,
}

KeyEventKind = Down | Up | Repeat | Cancel
PlatformNeutralKey = CharacterKey | NamedKey | Unknown
ModifierState { shift, control, alt, meta, caps_lock }
InjectedEventMetadata = NotMarked | Marked | Unknown

ForegroundContextSnapshot {
    context: InputContext,
    integrity: IntegrityRelationship,
    changed: bool,
}

IntegrityRelationship = Same | TargetHigher | TargetLower | Unknown

ObserverLifecycle = Starting | Running | Draining | Stopped
ObserverError = Unsupported | PermissionDenied | QueueClosed | Overflow | Shutdown
```

`TokenBoundary` remains the existing platform-neutral boundary type. A future
adapter may map an observed boundary into it, but M3A must not infer a window,
process, secure desktop, or remote session from an event alone. Timestamps are
monotonic metadata only and must not become a source of policy or text content.

## Queue and lifecycle requirements

- The producer path is bounded and non-blocking; it must never wait on a
  consumer, allocator, logger, or operating-system call.
- Capacity is fixed or explicitly bounded. There must be no unbounded queue or
  per-event accumulation of raw text.
- Overflow is loss-aware: expose an aggregate counter/state, never fabricate
  missing events, and fail closed for any future action that would require a
  complete sequence.
- Shutdown is explicit and idempotent. Producers observe closure and return a
  typed error; the consumer drains only the bounded contents before stopping.
- Lifecycle transitions are observable as redacted state, not as raw events.

## Privacy requirements

- Diagnostics are disabled by default and require explicit opt-in.
- Never log plaintext tokens, raw keystrokes, window titles, or full executable
  paths. Any future diagnostic identity must be a redacted category, hash, or
  length, with a documented retention limit.
- No network telemetry, cloud dictionary, remote logging, or runtime learning.
- Secure, terminal, remote, and unknown contexts are deny-by-default for any
  future automatic recovery. Observation itself still must not alter input.

## Safety defaults by context

| Context | M3A observation default | Future recovery default |
|---|---|---|
| Writing | Metadata only, opt-in adapter | High-threshold policy may be considered later |
| TechnicalEditor | Metadata only, opt-in adapter | Exact technical evidence only |
| Terminal | Disabled unless explicitly reviewed | Never automatic |
| Secure | Disabled | Never automatic; no editing |
| Remote | Disabled unless explicitly reviewed | Never automatic |
| Unknown | Disabled | Never automatic |

These are design defaults, not context detection. M3A does not discover any
of these contexts.

## Risks requiring explicit review

Global keyboard observation can expose sensitive behavior even without logging.
Elevated application boundaries may reject or reorder future adapter work.
Secure desktops, RDP, and VM consoles may have separate input/security
boundaries. Focus changes and app exit can invalidate a queued observation.
Injected events can be mislabeled or indistinguishable from user events.
Queue overflow can destroy ordering. A diagnostics feature can become a
privacy abuse surface if redaction, opt-in, or retention is weakened.

## Follow-on checklist

- M3A-01 — define and test event/value types (no platform bindings).
- M3A-02 — define a bounded queue and service lifecycle/error contract.
- M3A-03 — review Windows-to-neutral conversion in the isolated adapter.
- M3A-04 — review foreground-context and integrity snapshots.
- M3A-05 — define opt-in redacted diagnostics aggregation.
- M3A-06 — separately review an actual observe-only hook.

Each item requires a separate scope review. None is implemented by M3A.

## Acceptance criteria before M3B

M3A is accepted only when the contracts and threat model are reviewed, queue
overflow/shutdown semantics are specified, privacy defaults are testable,
platform-neutral crates remain free of Windows dependencies and unsafe code,
and a repository audit confirms there is no path to alter user input. M3B
controlled execution cannot begin until M3A-01 through M3A-06 have explicit
owner approval and separate tests/design records where applicable.
