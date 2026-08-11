# ADR 0009: M3C-01 restore simulation boundary

Status: Accepted for M3C-01 implementation.

## Decision

An approved `RestoreCandidate` is converted to a bounded, immutable
platform-neutral `RestorePlan`. The plan describes the rendered token,
intended replacement, logical unit counts, and existing policy evidence. It
contains no execution method and its `execution_allowed` state is always
false.

M3C-01 is simulation/data only. It deliberately precedes any future editing
milestone so that restore intent can be inspected and tested without changing
user text.

## Boundary and ownership

`zonkey-service` owns the restore-plan value and the diagnostic processor
integration. Detector and policy remain the source of the decision; the
Windows crate may display sanitized metadata but does not compute intent,
inspect foreground text, or execute a plan.

Plans are built only for `RestoreCandidate`. Keep, Ambiguous, Unsupported,
empty boundaries, injected events, and discontinuity produce no plan or clear
the latest bounded plan as appropriate.

## Units and lifetime

`rendered_units_to_replace` and `replacement_units` count Unicode scalar
values (`str::chars().count()`). These logical counts do not promise a future
Backspace sequence or UTF-16 caret operation. A plan lives only as the latest
completed-token diagnostic value; it is not a session history and is never
persisted.

## Safety boundary

The plan contains no HWND, process identity, window title, clipboard,
keystroke history, caret/selection state, native handle, or mutable Windows
state. M3C-01 adds no SendInput, key replay, suppression, clipboard, UI
Automation, IME mutation, foreground inspection, or other text-editing path.
Any execution mechanism requires a separate explicitly approved milestone.
