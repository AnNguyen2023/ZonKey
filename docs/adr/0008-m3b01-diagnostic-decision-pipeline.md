# ADR 0008: M3B-01 diagnostic decision pipeline boundary

Status: Accepted for implementation evidence; M3B-01 remains in progress.

## Decision

Connect the existing Windows observe source to a small, stateful diagnostic
processor owned by `zonkey-service`. The processor consumes validated
`ObservedInputEvent` values and uses the existing Telex, detection, and policy
APIs. `zonkey-win` remains responsible only for native capture, mapping, and
bounded handoff.

The processor mutates diagnostic token state only for semantic key-down events
in the existing ASCII letter/digit, boundary, and backspace scope. Key-up,
CTRL/ALT/META shortcuts, injected-origin events, unsupported keys, and queue
discontinuities are ignored or reset conservatively. Boundaries evaluate the
current token before clearing it through the existing Telex semantics.

Policy outcomes are reported as sanitized categories. No `EditPlan` is
executed, no foreground text is changed, and no native/platform operation is
invoked. Default output contains lengths and decision categories only;
`--show-token` is a temporary, explicit development diagnostic with no
persistence.

## Consequences

- `cargo run -p zonkey-cli -- diagnose` provides owner-visible decision
  evidence while preserving the existing `observe-hook` and `observe-raw`
  modes.
- Detection thresholds, dictionaries, and policy rules are unchanged.
- This does not select a final editing architecture or add foreground/context
  inspection, injection, suppression, replay, or text replacement.
- Manual Windows validation is still required before closing M3B-01.
