# ADR 0023: M3D-18 composition evidence feasibility review

Status: Accepted for M3D-18 design.

M3D-18 is a design-only review of whether ZonKey can obtain trustworthy
IME/composition evidence for the VS Code cooperating-host path. No TSF
implementation, IPC, transport, mutation, or OS API selection is included, and
the M3D-17 `CompositionUnknown` fail-closed behavior is unchanged.

## VS Code capability

The public VS Code extension API exposes no composition state. Text document
change events do not distinguish composition (preedit) text from ordinary
typing, paste, snippets, or other extensions' edits; keyboard/composition DOM
events exist only inside webviews, not for the workbench editor; and no
context key or proposed API surfaces composition activity to extensions. In
practice, extension authors confirm they cannot observe intermediate
composition state (for example VSCodeVim issues 9668 and 9712). Heuristic
signals, such as rapid multi-character document changes, do not measure
composition state at all; they cannot qualify as evidence under the M3C-07
requirements and carry no binding semantics.

## TSF feasibility

TSF composition objects are in-process, per-thread COM. Microsoft documents
that an external reader must run inside the process that created the
composition — in practice DLL injection — to reach
`ITfContextComposition::EnumCompositions`. A registered TSF text service
loads into every TSF-enabled process system-wide; it is not a narrow
cooperating-host adapter, requires system registration, and cannot be scoped
to ZonKey sessions or documents. No supported mutation-free cross-process TSF
observer exists. Inside the host process, TSF evidence is exact (composition
ranges and views), but that requires code running inside VS Code's process
beyond the approved extension surface, which is out of scope.

UI Automation's TextEdit control pattern
(`ITextEditProvider::GetActiveComposition`) is the only documented
cross-process composition mechanism. A read-only, sanitized probe of a real
VS Code 1.133 desktop window (see `vscode_capability.rs` in `zonkey-win`)
measured that the Chromium UIA provider does expose the TextEdit pattern:
the root web area (ControlType Document) and every measured Edit element
under the editor host window support `TextEditPattern` and
`TextPattern`. With no IME active, `GetActiveComposition` returns `S_OK`
with a null range on all measured elements — a measurable NONE state that is
distinguishable from a genuine provider error (non-zero HRESULT). An active
IME composition returning a usable range was not measured, because producing
one requires real IME input, which this project forbids injecting.

## Binding and coherence limits

The measured TextEdit signal exists but cannot be bound or made coherent.
None of the six elements that expose TextEdit carried an identity usable for
binding: the Edit elements reported empty automation ids and the signal is
not attributable to one specific editor, document, or editor instance of the
contract's document URI + open-instance epoch + editor identity + session
id, so composition evidence cannot be proven to describe the exact targeted
editor. Composition evidence read at time t0 and a host snapshot captured at
t1 are independent reads from independent processes with no atomic primitive
spanning them; composition may begin between the two reads, so the
check-then-act race cannot be closed from outside the host transaction.
Cross-process reads also depend on the Chromium accessibility tree being
enabled, which happened asynchronously after first contact during the probe.
Only composition evidence supplied by the host inside the same atomic
transaction as validation could close the race, and VS Code does not provide
such a transaction.

## Verdict

**NO_TRUSTWORTHY_COMPOSITION_BINDING.**

- VS Code API: no composition evidence exists; weak heuristics do not qualify.
- TSF outside the host process: no supported mutation-free observer.
- UIA TextEdit: measured as implemented by the real VS Code Chromium
  provider, with a measurable no-composition state, but the signal carries no
  element identity that binds to the targeted editor/document, an active
  composition returning a usable range is unproven, and read-to-snapshot
  coherence remains unresolvable from outside the host transaction.

`CompositionUnknown` therefore remains fail-closed for the VS Code path, and
host execution cannot be authorized on any currently available external
composition signal. Reopening this verdict requires a new ADR backed by one
of: (a) an upstream VS Code API exposing composition state inside the edit
transaction, (b) a Chromium/Electron UIA TextEdit provider plus a proven
identity and coherence binding, or (c) a separately approved in-host evidence
mechanism.

## Measured evidence

The capability probe is a read-only, sanitized, `#[ignore]`d test in
`crates/zonkey-win/src/vscode_capability.rs`; it never reads element names or
document text. Measured against real VS Code 1.133 desktop with one ordinary
local file editor open: the editor host window element is a `Pane`
(`Chrome_RenderWidgetHostHWND`); after Chromium accessibility enabled, a
bounded walk found six TextEdit-capable elements (one `Document`
root web area and five `Edit` elements), all also exposing `TextPattern`,
and all returning `S_OK` with a null range from `GetActiveComposition` with
no IME active. `TextPattern2` was not measured (no windows-rs 0.62 binding);
it is a caret signal, not a composition signal.

## References

- VS Code API reference (no composition surface): code.visualstudio.com/api/references/vscode-api
- VSCodeVim 9668, 9712 (extensions cannot observe composition state)
- Microsoft Learn, "Input Method Editor and Text Services Framework
  Accessibility" (TSF composition access is in-process)
- Microsoft Learn, TextEdit control pattern / `ITextEditProvider` (provider
  responsibility, `GetActiveComposition`)
- Chromium UI Automation documentation (TextPattern; no TextEdit/composition)
