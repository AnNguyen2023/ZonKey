# ADR 0019: M3D-02 narrow Win32 edit UIA spike

Status: Blocked by UIA environment.

The spike is limited to one read-only UI Automation probe for a focused,
standard Win32 `Edit` control. It obtains only sanitized evidence: provider
availability, control identity/class, empty selection, and exact candidate text
immediately preceding the caret. Any unknown or contradictory result rejects.

The probe excludes secure/password controls and does not claim composition/IME,
session, editor-native operation-unit, cross-query coherence, freshness, or
atomicity evidence. A generic read-then-act race remains unresolved.

UIA/COM and `unsafe` are isolated in `zonkey-win`; core and service remain
platform-neutral. No UIA mutation, text editing, SendInput, clipboard,
suppression, replay, persistence, or fallback acquisition family is present.
The standard Edit identity path resolves, but the current environment does
not expose a usable TextPattern; UIA validation is not claimed.
