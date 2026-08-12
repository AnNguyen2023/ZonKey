# ADR 0015: M3C-07 external-validation requirements

Status: Accepted for M3C-07 design.

This ADR specifies what a future platform layer must know before mutation
could be considered. It specifies no acquisition mechanism and performs no
inspection. Unknown required evidence fails closed.

| Evidence | Classification | Requirement |
|---|---|---|
| Target text at logical target | REQUIRED | Exact rendered snapshot match in proven editor units |
| Caret and selection | REQUIRED | Caret follows span; non-empty selection rejected initially |
| Target identity/focus | REQUIRED | Same logical text surface and stable focus; process/window alone insufficient |
| Composition/IME state | REQUIRED | No active mutation-sensitive composition; unknown rejects |
| Secure/password state | VETO-ONLY | Positive secure evidence vetoes; unknown rejects |
| Remote/session state | REQUIRED | Supported-session evidence; unknown remote state rejects |
| Editor operation-unit mapping | REQUIRED | Adapter proves mapping; scalar counts alone do not pass |
| Freshness/atomicity | REQUIRED | One coherent current bundle and race control |
| Extra diagnostics | OPTIONAL | Never substitutes for required evidence |

The target must exactly match the handoff rendered snapshot without assumed
normalization. Initial semantics require zero selection and a caret after the
matched span. Same target means the same logical text surface/document/control;
window or process identity alone is insufficient.

Unknown composition, secure, remote, or other safety-critical state fails
closed. Historical policy context is not fresh secure verification.

The future adapter must prove operation-unit mapping. Unicode scalar lengths
in `RestorePlan` are logical metadata, not Backspace counts, UTF-16,
grapheme, or editor-native range proof.

Freshness is logical (handoff generation, target snapshot, coherent bundle),
not a guessed time threshold. Evidence can stale between validation and
mutation; a future design needs atomic binding, an immediate recheck, or an
atomic compare-and-replace primitive. “Check, then SendInput” alone is
insufficient.

Acquisition mechanisms are NOT SELECTED and deferred. The flow stops at:

```text
RestoreCandidate -> RestorePlan -> PlanEligibility -> RestorePlanHandoff
-> HandoffRevalidation -> InternalExecutionGate
-> EXTERNAL VALIDATION REQUIREMENTS -> STOP
```

No OS mutation or evidence acquisition is part of M3C-07.
