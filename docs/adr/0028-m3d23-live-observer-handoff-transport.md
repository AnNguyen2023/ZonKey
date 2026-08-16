# ADR 0028: M3D-23 live observer → handoff → transport wiring

Status: Accepted for M3D-23; owner live smoke PASSED (2026-08-16).

M3D-23 connects the real Windows observer to the existing handoff/transport
path without creating a second input pipeline. `zonkey-service::transport`
gains `SharedDecisionState` plus `SharedDecisionProcessor`: the processor
view forwards live `ObservedInputEvent`s into the one real
`DiagnosticDecisionProcessor` under a shared lock while the pipe endpoint
maps the *current* handoff through `build_host_request`. All decision
semantics — injected-event exclusion, shortcut isolation, discontinuity
resets, Keep/Ambiguous producing no plan, only-current-eligible-handoff,
deterministic `handoff-<generation>` identity — stay owned by the existing
service code and are re-verified by dedicated wiring tests.

`zonkey-win` adds `run_handoff_live`: it spawns the existing pipe endpoint
with a handoff provider over the shared state and then runs the unchanged
`WH_KEYBOARD_LL` observe path (`run_observe_with_processor`). The host-native
UTF-16 range stays owned by the VS Code adapter: the M3D-23 live entry
reuses the M3D-22 flow with assertions relaxed to whatever the owner's real
typing produced, and it refuses to run without a live handoff present.

Automated verification covers the wiring (86 service tests including six
live-wiring cases, 35 win tests) and the tooling (the probe script and the
live server boot were verified against the scripted endpoint and a boot
check; these are tool checks, not live evidence). The owner's first manual
run exposed a real harness race: the 3-terminal procedure required keystrokes
to launch the E2E step, and the global hook observed those keys and
invalidated the live handoff first. The fixed single-command harness
(`npm run smoke:m3d23`) pre-starts every process, polls the handoff itself,
and on the first live `HANDOFF_OBSERVED` closes its poll connection and runs
the real VS Code validation automatically — no keystrokes between the
RestoreCandidate and the query, no handoff copying, and unchanged lifecycle
semantics. A clearly labeled `--scripted-tooling-check` mode proves the
harness mechanics (`M3D23_SMOKE_TOOLING_OK`) without ever standing in for
live evidence.

No mutation exists anywhere in the chain; the final result must remain
`rejected:CompositionUnknown`. Security claims are unchanged (ADR 0025).

## Owner live-smoke evidence (2026-08-16)

Executed via the single-command harness after the race fix. Real physical
keyboard input through the live `WH_KEYBOARD_LL` observer produced a
`RestoreCandidate`; the handoff stayed current with no keystrokes between
observation and query; the real pipe plus real VS Code harness completed
with final marker `M3D23_LIVE_SMOKE_OK` and exit code 0. No SendInput and
no scripted feed were used as live evidence. The observed handoff payload
was `handoff:handoff-1|réume|resume|5|6|1` (Telex rendering of "resume"),
and the real host result remained `rejected:CompositionUnknown` with the
probe document's text and version unchanged.
