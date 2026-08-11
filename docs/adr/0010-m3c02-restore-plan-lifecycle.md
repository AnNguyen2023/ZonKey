# ADR 0010: M3C-02 restore-plan lifecycle

Status: Accepted for M3C-02 implementation.

## Decision

The existing optional latest-plan field is the lifecycle model: `None` means
`NoPlan`, and `Some(RestorePlan)` means `CurrentPlan`. A plan is valid only
for the most recently completed token decision that produced a
`RestoreCandidate`.

There is no generation, timestamp, UUID, or persistence. The plan's bounded
ownership and replacement semantics already identify the current decision;
adding an identity would not provide a consumer or detectability benefit in
this simulation-only boundary. Reading `last_restore_plan()` is immutable and
does not consume or execute anything.

## Invalidation rules

- A semantic character or digit key-down, or Backspace, clears the current
  plan before changing token state.
- A completed boundary clears any prior plan before evaluation. A new
  `RestoreCandidate` replaces it; Keep, Ambiguous, or Unsupported leaves no
  current plan.
- Escape, unsupported structural input, and discontinuity clear the plan.
- Key-up, modifier-only transitions, shortcut events, and injected events do
  not represent semantic token intent and preserve the current plan. Injected
  events cannot create one.

No simulated one-shot consumption method is added. A future execution
milestone must define its own explicitly approved consumption contract and
still may not reuse this API to mutate user text.

## Safety boundary

This lifecycle is in-memory and deterministic. It has no OS mutation, input
replay, suppression, clipboard, foreground inspection, window/process data,
native handles, or execution API. Any execution remains a separate milestone.
