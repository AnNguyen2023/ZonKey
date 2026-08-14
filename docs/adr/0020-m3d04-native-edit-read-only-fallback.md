# ADR 0020: M3D-04 Native standard-EDIT read-only fallback

Status: Accepted

M3D-04 adds a diagnostic-only fallback for standard Win32 `EDIT` controls
inside `zonkey-win`. It reads the control class/style, text, and selection
using bounded read-only operations, then compares the UTF-16 range immediately
before an empty caret selection with the expected rendered token.

The fallback rejects non-`Edit` controls, password styles, non-empty or
contradictory selections, failed reads, and timed-out messages. `EM_GETSEL` is
issued through `SendMessageTimeoutW` with an explicit
bounded timeout; no unbounded native message send is used.

Text and selection are separate reads. The resulting evidence is therefore
non-atomic and may be stale or incoherent. It is suitable only for diagnostic
observation and testing; it cannot authorize execution or any later mutation.
Composition, session/security completeness, editor-native operation units,
and validation-to-mutation races remain unresolved.

No core/service API depends on this fallback, and no external application is
modified.
