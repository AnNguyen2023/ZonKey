# ADR 0006: M3A-05 Windows observer production boundary

- Status: Accepted
- Date: 2026-08-11

## Context

M3A-04 validated an observe-only `WH_KEYBOARD_LL` spike. Owner manual
acceptance recorded 49 callbacks, supported messages, bridge enqueues,
mapping attempts/successes, service receives/accepts/processes, and zero
mapping or queue drops. `Shift+A` and `Ctrl+C` carried correct modifier state,
and the focused application continued receiving input normally.

That evidence validates the spike boundary, not a final production observer.
Before observed events can enter Telex, detection, or policy decisions, the
native boundary and its operational risks require an explicit decision.

## Decision

**Decision class C: keep `WH_KEYBOARD_LL` temporarily, but require a separate
Raw Input comparison spike before production release or decision-pipeline
integration.**

This preserves the validated M3A-04 evidence while avoiding premature commitment
to a hook whose callback timeout and message-loop risks are not yet compared
against a device-oriented input path. The owner accepted this comparison
requirement, and M3A-06 completed it. The next production phase selects the
hook path without permanently rejecting Raw Input.

## M3A-06 comparison status

The Raw Input comparison spike is now implemented as a separate
`observe-raw` command. It registers keyboard usage page `0x01`, usage `0x06`,
with `RIDEV_INPUTSINK` on a hidden message-only window. It does not use
`RIDEV_NOLEGACY`, so normal focused-application delivery remains enabled.
The owner comparison is complete. Both paths passed the controlled observe-only
scenarios; `WH_KEYBOARD_LL` is selected for the next production phase because
its injected-origin visibility is important for future feedback-loop safety.
Raw Input remains a validated fallback for device identity and buffered input,
and is not permanently rejected.

Raw Input `RAWINPUTHEADER` does not expose an equivalent to the hook's
`LLKHF_INJECTED` flags in this boundary. Injection visibility is therefore
**not directly available** in this spike; no equivalence is invented.

## Mechanism comparison

| Concern | `WH_KEYBOARD_LL` | Raw Input |
| --- | --- | --- |
| Global/background observation | Direct desktop-wide hook path; validated manually | Requires registered devices and a window/message target; background input uses input-sink registration |
| Reliability risk | Callback timeout, hook removal, and message-loop lifetime | Registration/window/message ownership and raw-data parsing |
| Callback timeout | Must remain below `LowLevelHooksTimeout`; no waiting or heavy work | No low-level hook callback; processing occurs from `WM_INPUT`/buffer reads |
| Device identity | Not a first-class contract field in the spike | Raw input can distinguish source devices |
| Injected-origin signal | `KBDLLHOOKSTRUCT` flags expose injected markers | Requires separate interpretation of raw input metadata; must be evaluated |
| Key transitions | Direct key transition messages; validated for A-Z and modifiers | Raw keyboard packets require explicit make/break mapping |
| Latency | Direct callback path, bounded handoff | Message delivery and raw packet parsing; buffered reads suit bursts |
| Burst/high-rate behavior | Fixed bounded bridge; newest-event drop remains observable | Buffered raw input may improve burst handling but needs bounded ownership design |
| Shutdown | Unhook, wake message, `WM_QUIT`, drain bridge | Unregister/close window loop and drain pending raw messages |
| Testability | Existing deterministic mapper and lifecycle tests | Requires a comparison spike with native raw packet fixtures and manual tests |
| Safety implications | Never block/suppress; callback timeout is a hard constraint | Native handles and registration lifecycle must stay outside core contracts |

Microsoft documents that `LowLevelKeyboardProc` must return within the bounded
hook timeout and that the installing thread must pump messages. Raw Input
requires registration and delivers `WM_INPUT` to a window; it can provide
device-level identity and buffered reads for high-frequency input.

References: [LowLevelKeyboardProc](https://learn.microsoft.com/en-us/windows/win32/winmsg/lowlevelkeyboardproc),
[SetWindowsHookExW](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwindowshookexw),
[Raw Input overview](https://learn.microsoft.com/en-us/windows/win32/inputdev/about-raw-input),
[WM_INPUT](https://learn.microsoft.com/en-us/windows/win32/inputdev/wm-input),
[RegisterRawInputDevices](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-registerrawinputdevices).

## Native production boundary

The native layer may:

- observe native key notifications;
- copy minimal scalar metadata;
- track modifiers needed for `ModifierState`;
- map injection-origin flags;
- assign a non-zero monotonic sequence;
- perform a non-blocking bounded handoff;
- signal shutdown and report typed terminal failure.

It must not:

- run Telex, dictionary lookup, detection, or policy;
- reconstruct Unicode or retain text/key history;
- inspect foreground windows, processes, titles, clipboard, or password fields;
- perform filesystem, network, telemetry, or persistence I/O;
- suppress, replay, inject, edit, delay, or block input;
- wait for a consumer or perform heavy logging in a callback.

## Runtime ownership model

The eventual observer owns one dedicated Windows runtime context containing
the native registration and message loop. The observer owns registration until
terminal shutdown, accepts a stop request idempotently, stops accepting new
events, drains already accepted bounded events in FIFO order, releases native
resources, and reports `Stopped` or typed `Failed`. Core crates own neither
native handles nor the message loop. No new runtime implementation is part of
this ADR.

## Overflow and backpressure

Retain the M3A-04/M3A-02 policy:

- native bridge capacity is 256;
- `try_enqueue` never blocks;
- newest incoming event is rejected when full;
- older accepted events remain FIFO;
- bridge drops and `ObserveQueue` drops remain separate aggregate counters;
- every loss remains observable and can create a discontinuity boundary;
- no unbounded intermediate queue is permitted.

Capacity configurability remains a later design choice; production integration
must not silently change the current loss semantics.

## Diagnostics policy

The console diagnostics are spike-only. A production observer may expose only
aggregate counters, lifecycle state, sanitized key categories, mapping status,
and drop/discontinuity counts. Per-key event output is debug-only and must be
feature-gated or removed from normal production runtime. Raw accumulated text,
command reconstruction, and secret history are never diagnostics payloads.

## Privacy and security boundary

There is no persistent raw key history, command-line reconstruction, clipboard
capture, window-title/process-name inspection, password-field inference, or
telemetry containing raw input. Any future contextual inspection requires a
separate milestone and ADR. Injected-origin classification remains metadata;
it does not authorize suppression or replay.

## Preconditions before decision-pipeline integration

All of the following must pass before real observed events feed Telex,
detection, or policy:

1. Native source stability and deterministic terminal cleanup.
2. No input suppression and no callback blocking.
3. Bounded memory with observable bridge and service loss counters.
4. Fixed injected-event policy and correct left/right modifier behavior.
5. Defined repeated-key, supported-key, and unsupported-key semantics.
6. Deterministic shutdown, drain, and failure behavior.
7. Automated mapping/lifecycle tests and manual focused-application
   transparency tests.
8. A Raw Input comparison spike with an owner-accepted mechanism decision.

## Consequences

The project keeps the working M3A-04 hook spike available for evidence and
testing, but does not treat it as production-ready or final. The next native
implementation work must be a bounded Raw Input comparison, not Telex or edit
integration. Core contracts and the observe-only safety boundary remain
unchanged.
