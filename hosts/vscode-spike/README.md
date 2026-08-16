# M3D-17 VS Code host adapter spike

Smallest VS Code extension-side adapter mapping the Zonkey cooperating-host
contract (see the `dummy_host` tests in `crates/zonkey-service`) onto VS Code
APIs. Spike scope only: **one local ordinary `TextDocument`/`TextEditor`,
single caret, no notebooks/custom editors/remote workspaces/multi-cursor, and
fail-closed on unknown composition/session/security state.**

This is not an end-user IME and wires no production restore path.

## Layout

- `src/contract.ts` — JSON contract types mirroring the service vocabulary.
- `src/ports.ts` — narrow vscode-shaped ports (UTF-16 offsets everywhere).
- `src/adapter.ts` — pure core: snapshot capture, validation, one-transaction
  compare-and-replace, idempotency ledger.
- `src/host-policy.ts` — what the real host honestly reports.
- `src/vscode-binding.ts`, `src/extension.ts` — real `vscode` API binding and
  the one-command manual probe. Never loaded by Node tests.
- `test/` — in-memory dummy harness with race-injection hooks.

## Commands

```powershell
npm install
npm test          # node:test, 29 tests (dummy harness)
npm run typecheck # tsc --noEmit
npm run compile   # esbuild bundle (out/extension.cjs) for manual F5
npm run test:real # real VS Code desktop validation via @vscode/test-electron
```

## Real VS Code desktop validation

`npm run test:real` launches a real VS Code desktop instance via
`@vscode/test-electron`, loads this extension, opens one ordinary local file
in the opened workspace, and asserts: activation, the `zonkeySpike.probe`
command, snapshot mapping fields (URI + epoch, editor id, version, UTF-16
range, exact expected text, empty single caret, capability flags), the
fail-closed `Rejected(CompositionUnknown)` result, unchanged document text
and version, and a non-empty-selection refusal. First run downloads VS Code
into `%USERPROFILE%\.vscode-test`.

## Manual probe (optional, dummy-harness in real VS Code)

1. `npm run compile`.
2. Open this folder in VS Code; create `.vscode/launch.json` (git-ignored)
   with the standard "Run Extension" config pointing at this workspace.
3. F5 → Extension Development Host → open one ordinary local file.
4. Type the sentinel token `zonkey-spike-target`, leave the single caret
   immediately after it, run
   **"Zonkey spike: probe cooperating-host apply"**.

Expected real-VS-Code outcome: `Rejected (CompositionUnknown)` — VS Code
exposes no IME composition state, so the honest host reports `Unknown` and the
apply fails closed. The `Applied` path is proven only on the Node dummy
harness. See `docs/adr/0022-m3d17-vscode-host-adapter-spike.md`.

## Non-claims

- No composition proof, no generic editor support beyond one local text
  document, no generic Windows execution safety, no auto-retry of
  `Indeterminate`, no production auto-restore wiring.
- No SendInput, no Win32 mutation, no clipboard, no suppression/replay, no
  multi-host support.
- The in-session request ledger is unbounded (spike grade).
