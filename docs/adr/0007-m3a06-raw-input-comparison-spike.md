# ADR 0007: M3A-06 Raw Input comparison spike

- Status: Accepted
- Date: 2026-08-11

## Scope

M3A-06 adds a parallel, Windows-only Raw Input observer for comparison with
the validated M3A-04 `WH_KEYBOARD_LL` spike. It does not replace the hook,
feed Telex/detection/policy, or modify user input.

## Registration and runtime

The spike creates a hidden message-only window on the observer thread and
registers keyboard Raw Input with:

```text
usage page: 0x01 (HID_USAGE_PAGE_GENERIC)
usage:      0x06 (HID_USAGE_GENERIC_KEYBOARD)
flags:      RIDEV_INPUTSINK
target:     hidden message-only HWND
```

`RIDEV_NOLEGACY` is deliberately absent. `WM_INPUT` is dispatched through the
window procedure, `GetRawInputData` copies only a bounded `RAWKEYBOARD` packet,
and the copied scalar metadata enters a separate capacity-256 bridge. Stop
posts `WM_QUIT`, stops intake, drains accepted events, and destroys the hidden
window.

## Mapping and parity

Raw make/break metadata and the supplied keyboard message map into the same
`ObservedInputEvent` contract used by the hook spike. Modifier tracking uses
the existing left/right-aware neutral `ModifierTracker`. Repeated make events
are delivered as repeated key-down observations. Unsupported key identities
remain valid `ObservedKey::other` events where the shared contract permits.

Raw Input does not expose a direct equivalent to `LLKHF_INJECTED` in the
metadata used here. The spike maps origin conservatively as unmarked/physical
and explicitly does not claim injected-event detection parity.

## Bounded handoff and diagnostics

The Raw Input bridge has capacity 256, rejects the newest event when full,
never blocks the window procedure, and reports separate sanitized counters:

```text
raw_messages
raw_keyboard_packets
mapping_attempts
mapping_success
mapping_rejected
bridge_enqueued
bridge_full_drops
received
processed
```

No raw packet, Unicode text, key history, clipboard, window context, or
persistent diagnostic is retained.

## Commands and comparison

The existing `observe` command remains a hook alias. Explicit commands are:

```powershell
cargo run -p zonkey-cli -- observe-hook
cargo run -p zonkey-cli -- observe-raw
```

The controlled manual comparison covered A, Shift+A, Ctrl+X, Alt+A, held K,
clean shutdown, and focused-application transparency for both paths.

## Manual evidence and current limitation

The owner demonstrated Raw Input with registration success, including a final
Alt+A trace, 36 raw messages/keyboard packets, 36 mapping attempts/successes,
36 bridge enqueues, 36 received events, and 36 processed events. Mapping and
bridge drops were zero. Earlier controlled evidence also demonstrated Shift+A,
K repeat, and zero bridge drops.

The final hook controlled run recorded 96 callbacks, supported messages,
mapping attempts/successes, bridge enqueues, received, accepted, and
processed events, with zero mapping, bridge, lock, or service drops. Manual
traces demonstrated Shift+A, Ctrl+X, Alt+A, K repeat, and focused-application
transparency. The earlier sustained-input run's 53 losses were fixed by
M3A-07 at the downstream `ObserveQueue`, not in the native hook bridge.

M3A-06 exposed that the prior `ObserveService::run` called
`source.next_event()` and only enqueued events until terminal shutdown. The
M3A-07 service loop now processes one queued event after each source event,
then resumes polling. Exhaustion and explicit stop still drain the remainder;
source failure still leaves pending events undrained. Capacity must not be
increased as the architectural fix.

M3A-03 remains valid: its finite/mock lifecycle contract was sufficient for
its stated scope. M3A-07 now supplies continuous runtime consumption before
decision-pipeline integration.

The next controlled owner comparison is intentionally short and identical for
both modes: `A`, `Shift+A`, `Ctrl+X`, `Alt+A`, hold `K` for about one second,
then stop. Record native count, mapping success, bridge drops, received,
service drops, and processed events, while confirming focused-application
transparency.

For manual parity, `observe-raw` prints the same sanitized event shape as
the hook spike (`seq`, `kind`, `key`, `mods`, and injected-origin category).
This is spike/debug output only and does not change the production diagnostics
policy.

## Comparison outcome

Both mechanisms passed the observe-only controlled comparison. `WH_KEYBOARD_LL`
is selected for the next production phase because it provides direct
`LLKHF_INJECTED` and `LLKHF_LOWER_IL_INJECTED` visibility, while mapping,
lifecycle, modifier behavior, repeat behavior, and transparency are already
validated. Raw Input remains a validated fallback and future option for device
identity, buffered high-rate input, and avoiding low-level hook timeout risk;
it is not rejected permanently.

## Safety boundary

No suppression, replay, injection, editing, `SendInput`, clipboard capture,
foreground inspection, text reconstruction, telemetry, network access, or
Telex/detection/policy integration is permitted.
