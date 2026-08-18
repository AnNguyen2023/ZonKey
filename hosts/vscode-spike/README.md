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

## Bounded ledger and transport (M3D-19)

The adapter ledger is bounded (default capacity 256, validated up front) with
deterministic FIFO eviction: the oldest inserted request id is dropped when
full, exact duplicates replay recorded results, conflicting reuse rejects,
and every outcome kind — including `Indeterminate` — is replayed without
retry. The matching platform-neutral transport contract (framing, session
binding, and ledger semantics) lives in `zonkey-service::transport`; the
Windows named-pipe binding is deliberately not implemented yet, and real VS
Code applies remain fail-closed at `CompositionUnknown`.

## Real VS Code transport integration (M3D-21)

`npm run test:transport` (after `cargo build -p zonkey-cli`) spawns a real
`serve-host-validation` pipe endpoint, launches real VS Code, and asserts the
full query/reject chain: activation, real snapshot, `HELLO`/`WELCOME`,
request over the pipe, `DEFINITE|rejected:CompositionUnknown`, duplicate
replay, disconnect-as-Indeterminate resolved by replay, and an untouched
document. Note: the extension host connects via the `\\?\pipe\` form and the
spike pipe serves one connection at a time.

## Non-claims

- No composition proof, no generic editor support beyond one local text
  document, no generic Windows execution safety, no auto-retry of
  `Indeterminate`, no production auto-restore wiring.
- No SendInput, no Win32 mutation, no clipboard, no suppression/replay, no
  multi-host support.
- The in-session request ledger is unbounded (spike grade).

## Packaging and startup (M3D-33)

Build the release endpoint binary (Windows 11 x64, msvc only):

    cargo build --release -p zonkey-cli --target x86_64-pc-windows-msvc
    # runtime artifact: target/x86_64-pc-windows-msvc/release/zonkey-cli.exe

Package the production VSIX (manifest + README + minified bundle only):

    npm run package          # -> zonkey-vscode-spike-0.0.1.vsix

Start the endpoint explicitly (manual, approved model; no service, no
auto-start):

    zonkey-cli.exe handoff-live --pipe auto

`--pipe auto` generates the per-lifecycle nonce pipe and writes the
current-user discovery record at `%LOCALAPPDATA%\ZonKey\endpoint.txt`.
The CLI prints only a sanitized readiness/protocol line; the extension reads
the pipe identity from the protected discovery record. Install the VSIX into
VS Code; it connects once at activation and offers **Zonkey spike: connect to
discovered endpoint** for explicit reconnect.

## Packaged query/reject command (M3D-37)

Open one ordinary local `file:` document with a single caret immediately after
the current rendered candidate, then run **Zonkey spike: check current
handoff**. The command queries the live `RestorePlanHandoff`, captures the
real host snapshot, sends the request over the discovered pipe, and displays
only a sanitized result. The expected real-host result is
`Rejected(CompositionUnknown)`; document text and version must remain
unchanged. A missing/mismatched candidate is refused before transport.

The observer is observe-only: it never inserts or replaces document text.
For the manual command, the active ordinary local document must already contain
the rendered candidate immediately before the caret. The one-window runner
creates its local host fixture only after it observes the current live handoff;
the packaged command queries that handoff again, so no stale handoff is frozen
or fabricated. Scripted input is tooling validation, not live evidence.

### One-window packaged physical smoke

Build the release CLI first, then run this one command from
`hosts/vscode-spike`:

    npm run smoke:m3d37

The runner starts the real `handoff-live` endpoint, installs the packaged VSIX
into a temporary isolated profile, and opens one VS Code window with one local
file editor. The installed VSIX owns the only endpoint client. Type only on the
physical keyboard when the window says it is ready:

1. `dungf` then **Space**.
2. `resume` then **Space**.
3. Stop typing immediately.

The harness observes the live `WH_KEYBOARD_LL` handoff, automatically invokes
**Zonkey spike: check current handoff**, and verifies
`Rejected(CompositionUnknown)` plus unchanged target document text/version.
No `SendInput`, scripted feed, `TextEditor.edit`, or second VS Code window is
used. The temporary endpoint, profile, workspace, and VSIX installation are
removed automatically.

Shutdown: stop `zonkey-cli.exe` with Ctrl+C. A hard crash leaves a stale
discovery record that fails closed; starting a new endpoint replaces it.
Uninstall the VSIX from VS Code Extensions, stop the CLI, and remove the
stale `%LOCALAPPDATA%\ZonKey\endpoint.txt` only after the endpoint is stopped.
Preserve `recovery-state.bin` unless intentionally resetting recovery state.
The endpoint protocol is `zonkey.host-transport/1`; unknown protocols and
unknown discovery/state schemas fail closed.

Known limitations: Windows 11 x64 only, manual startup, one active pipe
client, no service/auto-start, no installer, no automatic reconnect, and real
Applied remains disabled while composition is Unknown. The smallest beta
bundle is produced by `npm run beta:package` after a clean release build.

Clean-profile end-to-end validation (installs the VSIX into an isolated
VS Code profile and validates discovery, reconnect, recovery, restart
identity, and fail-closed behavior):

    npm run test:endpoint    # prints M3D33_CLEAN_PROFILE_OK on success
