# M3D-23 live smoke test — single-command procedure

Status: awaiting owner execution. This is the only accepted live-keyboard
evidence for M3D-23; nothing scripted substitutes for it.

## Why one command

The first 3-terminal procedure was self-invalidating: after typing
`resume`+Space, the owner had to type more keys to launch the E2E step, and
the global `WH_KEYBOARD_LL` observer saw those keys and invalidated the live
handoff before it could be queried. The fixed harness pre-starts every
process, polls the read-only handoff query itself, and the moment a live
RestoreCandidate appears it closes the poll connection and automatically
runs the real VS Code validation — zero keystrokes in between. The handoff
is never copied or frozen; the E2E reads the current live handoff.

## What this proves

`WH_KEYBOARD_LL → ObservedInputEvent → ObserveService →
DiagnosticDecisionProcessor → Telex/detect/policy → RestorePlanHandoff →
internal gate → host request → named pipe → VS Code extension →
Rejected(CompositionUnknown)`, observe/query/reject only, no mutation.

## Owner procedure

```powershell
cargo build -p zonkey-cli
cd D:\ZonKey\hosts\vscode-spike
npm run smoke:m3d23
```

Wait for `LIVE_SMOKE NOW TYPE ON THE PHYSICAL KEYBOARD`, then type into any
app (Notepad is fine), on the physical keyboard only:

1. `dungf` then **Space** — expect `LIVE_SMOKE poll=N no-handoff
   handoff-rejected:NoCurrentPlan` (negative Keep case).
2. `resume` then **Space** — expect `LIVE_SMOKE poll=N HANDOFF_OBSERVED
   handoff:handoff-1|réume|resume|5|6|1`.
3. **Stop typing.** The harness closes its poll connection and launches the
   real VS Code validation automatically. Do not touch the keyboard until it
   finishes.

Expected final marker: `M3D23_LIVE_SMOKE_OK`, meaning the entry asserted
`DEFINITE|rejected:CompositionUnknown`, duplicate replay, and an untouched
probe document. The harness prints `negative_seen=true` when the `dungf`
step was captured (rerun with `dungf` first if it says false). The runner
stops its endpoint by itself when done.

If nothing is typed within 300 seconds the harness exits with instructions
(`ZONKEY_M3D23_WAIT_SECONDS=<n>` overrides).

## Tooling self-check (not live evidence)

`npm run smoke:m3d23-tooling` runs the same automatic flow against the
scripted `--handoff-token` endpoint, labeled `SCRIPTED_TOOLING_CHECK`, to
prove the harness mechanics without keyboard input. It never substitutes for
the live marker above.

## Evidence to attach to the milestone

- The full `LIVE_SMOKE` console output (negative + `HANDOFF_OBSERVED` +
  `M3D23_LIVE_SMOKE_OK`).

## Non-goals

No `TextEditor.edit`, no Applied path, no SendInput, no scripted feed as
live evidence, no suppression/replay, no Win32 or editor mutation, no
composition bypass. Failing any expectation is a finding, not something to
work around.
