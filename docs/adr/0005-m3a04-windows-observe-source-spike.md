# ADR 0005: M3A-04 Windows observe-source spike

- Status: Accepted
- Date: 2026-08-10

## Context

M3A-01 through M3A-03 established a platform-neutral
`ObservedInputEvent` contract and a bounded, loss-aware `ObserveService`.
M3A-04 must choose a Windows-side observation mechanism for a small manual
spike without changing that contract or modifying user input.

At the time of this decision, `zonkey-win` was only a validation placeholder.
This ADR records the spike boundary; the separately reviewed implementation
now validates it without making it the final production architecture.

## Decision

Use a future `WH_KEYBOARD_LL` low-level keyboard hook as the first Windows
observe-source spike. The future adapter will map the minimum metadata needed
for `ObservedInputEvent` and hand owned values to the existing bounded,
platform-neutral pipeline.

```text
WH_KEYBOARD_LL callback
        -> minimal native metadata copy
        -> non-blocking bounded handoff
        -> ObservedInputEvent
        -> ObserveService
```

This is an observation choice only. The callback must never suppress, modify,
replay, inject, edit, run Telex/detection/policy, perform I/O, or wait on a
lock. It must forward unprocessed events with `CallNextHookEx` and return
promptly.

## Spike acceptance evidence

The M3A-04 implementation validated this boundary with the runnable
`zonkey-cli observe` command and owner manual Windows acceptance. `Shift+A`
and `Ctrl+C` reached the service with correct modifier metadata; the final
run recorded 49 callbacks, supported messages, bridge enqueues, mapping
attempts/successes, service receives/accepts/processes, and zero rejects or
drops. The focused application continued receiving input normally.

This closes the spike only. Diagnostics are sanitized and spike-only, key
mapping is intentionally incomplete (`ObservedKey::other` remains possible),
and there is no Unicode reconstruction, foreground/context inspection,
production IME behavior, injection, or editing. `WH_KEYBOARD_LL` is validated
for this spike and is not permanently selected as the final runtime; Raw Input
remains eligible for later evaluation.

## Alternatives considered

### Raw Input (`RegisterRawInputDevices` / `WM_INPUT`)

Raw Input is not selected for the first spike. Microsoft documents that an
application must register the devices it wants, receive `WM_INPUT` through a
window procedure, and parse the `HRAWINPUT` with `GetRawInputData`. Background
delivery requires the appropriate input-sink registration. This gives better
device identity and can be preferable for high-rate asynchronous monitoring,
but adds a window/message and raw-device parsing surface before the basic
adapter boundary is validated.

References: [Raw Input overview](https://learn.microsoft.com/en-us/windows/win32/inputdev/about-raw-input),
[WM_INPUT](https://learn.microsoft.com/en-us/windows/win32/inputdev/wm-input),
[GetRawInputData](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getrawinputdata).

### Other mechanisms

No other mechanism is selected. Text services, UI Automation, foreground
inspection, and application-specific APIs would broaden the scope beyond a
keyboard event source and are not required to validate this boundary.

## Windows API choice

Microsoft documents `WH_KEYBOARD_LL` as a low-level keyboard hook installed
with `SetWindowsHookExW`. It is global for the desktop when the thread id is
zero and delivers `WM_KEYDOWN`, `WM_KEYUP`, `WM_SYSKEYDOWN`, and
`WM_SYSKEYUP` information to `LowLevelKeyboardProc`.

References: [SetWindowsHookExW](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwindowshookexw),
[LowLevelKeyboardProc](https://learn.microsoft.com/en-us/windows/win32/winmsg/lowlevelkeyboardproc).

The callback must run on a thread with a message loop. Microsoft also requires
short callback processing; the documented timeout is bounded and hooks can be
silently removed after timeout. Therefore the future spike uses a dedicated
observer thread only as a runtime design, with immediate bounded handoff to
the service and no heavy work in the callback.

## Safety boundary

M3A-04 remains observe-only:

- always call `CallNextHookEx`; the observe-only callback never consumes an
  event;
- copy only the minimum event metadata needed by the neutral contract;
- never suppress, replay, inject, replace, block, or delay application input;
- never run Telex, detection, policy, or text editing on the callback path;
- never perform filesystem/network I/O, diagnostics persistence, or heavy
  logging in the callback;
- never inspect foreground windows, processes, clipboard, passwords, or text;
- use a bounded non-blocking handoff and preserve M3A-03 drop/discontinuity
  semantics.

## Callback/message-loop constraints

The future implementation must use a dedicated observer thread with a native
message loop because the low-level hook callback is delivered in the context
of the installing thread. The callback does only:

1. accept an actionable hook notification;
2. copy validated scalar metadata;
3. attempt the bounded handoff;
4. call `CallNextHookEx` and return immediately.

Stop must unhook, end the message loop, and report a typed terminal outcome to
the platform-neutral service. No callback may wait for the consumer.

## Mapping to `ObservedInputEvent`

The adapter maps only existing contract fields:

| Native observation | Neutral mapping |
| --- | --- |
| `WM_KEYDOWN` / `WM_SYSKEYDOWN` | `KeyEventKind::KeyDown` / `SystemKeyDown` |
| `WM_KEYUP` / `WM_SYSKEYUP` | `KeyEventKind::KeyUp` / `SystemKeyUp` |
| representable letter key | `ObservedKey::letter` |
| representable decimal digit | `ObservedKey::digit` |
| representable punctuation | `ObservedKey::punctuation` |
| Space, Enter, Tab, Backspace, Escape | corresponding validated constructor |
| other key identity | `ObservedKey::other` or typed adapter rejection |
| tracked Shift/Control/Alt/Meta state | `ModifierState` builders |
| adapter sequence counter | non-zero `EventSequence` |

Virtual-key values, scan codes, timestamps, native handles, device identity,
Unicode text and window metadata remain adapter-local. A keyboard event is not
final Unicode text; M3A-04 does not reconstruct text or expand
`ObservedInputEvent` to hold every native field.

## Injected-event policy

Observe and mark injected events. `KBDLLHOOKSTRUCT` exposes
`LLKHF_INJECTED` and `LLKHF_LOWER_IL_INJECTED`; these map to the existing
`InjectionOrigin::MarkedInjected` and `LowerIntegrityInjected` values. The
adapter does not suppress or replay them. Downstream observe-only processing
may classify or ignore marked events, but that policy is outside the callback
and outside this spike.

Reference: [KBDLLHOOKSTRUCT](https://learn.microsoft.com/en-us/windows/win32/api/winuser/ns-winuser-kbdllhookstruct).

## Overflow / backpressure

Reuse M3A-03 semantics unchanged: bounded queue, reject newest when full,
observable dropped counter, one discontinuity marker per loss episode, and no
blocking of the native callback. There is no unbounded intermediate queue.

## Thread / lifecycle model

This ADR defines the future runtime model only:

```text
dedicated Windows observer thread
        -> hook callback and message loop
        -> bounded non-blocking handoff
        -> platform-neutral ObserveService
```

The future lifecycle must start the observer, reach `Running`, stop without
polling after a terminal request, unhook and drain according to the existing
service contract, then reach `Stopped` or typed `Failed`. Thread creation and
native lifecycle code are explicitly deferred to the follow-up implementation
spike.

## Dependency strategy

No dependency is added in M3A-04 design work. A future implementation should
prefer the maintained `windows` crate isolated in `zonkey-win`, subject to a
separate dependency review. Manual FFI is not approved; core crates remain
free of Windows dependencies and `unsafe`.

## Manual test strategy

The future Windows spike must provide a controlled manual observer command or
example only after implementation approval. The exact command is intentionally
deferred; it must not be added by this ADR. A candidate shape is:

```powershell
cargo run -p zonkey-cli -- observe
```

Expected redacted output is metadata only, for example:

```text
seq=12 kind=down key=A modifiers=SHIFT injected=false
```

The manual checklist must cover:

1. press/release `A`;
2. `Shift+A`;
3. `Ctrl+C`;
4. briefly hold a key;
5. type rapidly;
6. stop the observer cleanly;
7. confirm the focused application continues receiving all keyboard input.

The seventh item is mandatory. No accumulated text, clipboard contents,
secret history, or full typed buffer may appear in diagnostics.

## Non-goals

M3A-04 does not implement Raw Input, `SendInput`, suppression, replacement,
replay, text editing, foreground/process inspection, UI Automation, clipboard
access, password detection, persistence, telemetry, GUI, installer, startup
registration, or full IME integration.

## Acceptance criteria

The follow-up implementation spike must demonstrate:

1. A `WH_KEYBOARD_LL` source maps key transitions to existing validated
   `ObservedInputEvent` values without changing the contract.
2. The callback forwards every non-consumed event and never blocks the input
   path.
3. Injected flags are observed and marked, never suppressed or replayed.
4. Bounded handoff preserves M3A-03 newest-drop and discontinuity semantics.
5. Stop/unhook/message-loop shutdown is deterministic and does not resume after
   a terminal state.
6. Manual tests show A, Shift+A, Ctrl+C, held/rapid keys, clean stop, and
   continued focused-app input.
7. Diagnostics contain metadata only and no raw text or secret history.
8. Windows code remains isolated to `zonkey-win`; no core crate gains a
   Windows dependency, FFI, or `unsafe` beyond an explicitly reviewed adapter
   boundary.

## Consequences

`WH_KEYBOARD_LL` gives the first spike a direct global key-transition signal
and explicit injected-event flags with less device-registration machinery than
Raw Input. It also introduces callback timeout, message-loop, desktop-scope,
and native lifecycle risks. Those risks are contained by a dedicated thread,
minimal callback work, bounded handoff, and the mandatory focused-application
manual test.

Raw Input remains a viable future alternative if device identity, high-rate
buffering, or asynchronous monitoring becomes a demonstrated requirement.

## Follow-up implementation step

Implement only a Windows manual observe-source spike in `zonkey-win`, behind a
separate owner-reviewed change. It may add the approved Windows dependency and
native boundary then, but must not add suppression, injection, replay, text
editing, or foreground inspection.
