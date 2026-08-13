# ADR 0018: M3D-01 production acquisition readiness review

Status: Accepted for M3D-01 readiness.

## Decision

`READY_FOR_NARROW_SPIKE` is approved at the design/readiness level for one
observe-only target/family pair:

- Target class: standard Win32 edit controls, excluding secure/password fields
- Acquisition family: UI Automation/accessibility text-provider family

This is not an implementation approval and no Windows API is selected here.

## Evidence to observe

The future spike may attempt to collect a single coherent, sanitized snapshot
of exact target text, the range immediately preceding the caret, zero/non-zero
selection, logical provider/control identity, and the provider's available
text-unit description. It must report evidence as partial unless the provider
explicitly proves the required predicate. HWND/PID alone is not identity.

## Unknown and fail-closed states

Composition/IME freshness, secure state, remote/session state, editor-native
operation-unit mapping, and validation-to-mutation atomicity remain UNKNOWN for
this family unless separately proven. Provider denial, protected/secure
controls, unsupported controls, elevated or cross-integrity targets, custom
providers, non-empty selection, ambiguous ranges, stale multi-query state, or
unknown session/composition state must reject and stop. The spike must not read
password contents to establish a veto.

## Freshness, coherence, and race

Separate provider queries are only a candidate snapshot and may describe
different target states. The family has no assumed atomic bind to later
mutation; generic read-then-act remains `CHECK_THEN_ACT_RACE`. The spike must
record whether one coherent snapshot was obtained and must treat any race or
revision uncertainty as a fail-closed result.

## Explicit stop conditions

Stop before producing an externally eligible result if any required evidence is
missing, unknown, denied, contradictory, non-exact, non-empty-selected,
provider-identity-only, operation-unit-unproven, composition-active, secure,
remote/unknown-session, stale, or not coherent. Stop on unsupported control,
privilege/integrity mismatch, unexpected native value, or any need for a
mutation API. No fallback to another family is implicit.

## Spike must not do

The future spike must not modify text, send input, suppress or replay keys,
replace clipboard contents, inspect foreground/window/process/session state
through an unapproved mechanism, or claim production safety. It must not use
UIA/TSF calls, native handles, unsafe code, new dependencies, persistence, or
diagnostic raw text logging in this readiness task. Production atomicity remains
unresolved.

This readiness decision is limited to observing a narrow candidate surface and
stopping safely. It does not authorize a production adapter, mutation, or
execution path.
