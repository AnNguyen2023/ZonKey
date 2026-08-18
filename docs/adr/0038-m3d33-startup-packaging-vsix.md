# ADR 0038: M3D-33 startup / packaging / VSIX release-path spike

Status: Proposed for M3D-33 owner review.

M3D-33 proves the release path on Windows 11 x64: release Rust artifacts,
a production VSIX, an explicit manual startup model with per-lifecycle
endpoint discovery, and a clean-profile install/activation validation in
real VS Code. Real Applied stays disabled, `CompositionUnknown` stays
fail-closed, no `TextEditor.edit`, no mutation.

## Release artifacts

`cargo build --release --workspace --all-targets --target
x86_64-pc-windows-msvc` produces exactly one runtime executable:
`target/x86_64-pc-windows-msvc/release/zonkey-cli.exe` (~584 KB). All
other workspace members are libraries. No 32-bit artifacts exist or are
supported.

## VSIX packaging

`hosts/vscode-spike` packages via `npm run package` (esbuild `--minify`
production bundle + `vsce package --no-dependencies`): `zonkey-vscode-
spike-0.0.1.vsix`, 5 files / ~9 KB — manifest, README, and
`out/extension.cjs` only. `.vscodeignore` excludes sources, tests,
integration entries, scripts, `node_modules`, `.vscode-test`, and
non-production `out/` artifacts; `*.vsix` is gitignored. The manifest
declares version 0.0.1, `engines.vscode ^1.90.0`, activation
`onStartupFinished`, and documents protocol compatibility
(`zonkey.host-transport/1`, recovery-state v2) in a `zonkey` metadata
section. Dependency versions and the lockfile are unchanged.

## Startup model (explicit and manual)

The approved model: the operator starts the endpoint —
`zonkey-cli.exe serve-host-validation --pipe auto` — which generates the
per-lifecycle nonce pipe name (M3D-29), prints `endpoint_pipe=…`, and
writes a discovery record at `%LOCALAPPDATA%\ZonKey\endpoint.txt`
(key=value lines: protocol, pipe, pid, started_unix_ms) with the
current-user-only ACL via the durable replace flow. Clean shutdown removes
the record, but only when it still names this endpoint — a newer endpoint
is never deregistered by an older one. A crash leaves a stale record;
consumers verify by connecting, and because the pipe identity is a
per-lifecycle nonce a stale name never authorizes anything. Duplicate
startup: the newest record wins discovery; each endpoint keeps its own
ACL'd pipe and session. No Windows service, no auto-start, no background
persistence was added. The extension reads the record at activation,
connects once, and offers an explicit `endpointConnect` command for
reconnect (reconnect within a lifecycle keeps the session; a restarted
endpoint yields a new session identity); `ZONKEY_ENDPOINT_DIR` overrides
the directory for isolated profiles.

## Clean-profile validation (real VS Code)

`npm run test:endpoint` (`scripts/run-clean-profile.mjs` +
`integration/real-vscode-endpoint.entry.ts`): builds the VSIX, installs it
into an isolated `--user-data-dir`/`--extensions-dir` profile via the VS
Code CLI, and runs the entry against the *installed* extension only (an
inert manifest-only dummy satisfies VS Code 1.133's requirement that a
development extension be present for test dispatch). Validated end to
end: installation, activation (`activate` now returns the endpoint state
so `extension.exports` is meaningful), discovery of the current endpoint,
the explicit reconnect command, recovery LIST and a real request
(`rejected:CompositionUnknown` — final host behavior unchanged, document
text/version unchanged), duplicate-endpoint discovery semantics
(last-writer-wins, older shutdown never deregisters), stale-identity
fail-closed connect, restart producing a new pipe and session identity,
and unknown-protocol discovery records failing closed. Verdict lines:
`M3D33_ENDPOINT_PROFILE_OK`, `M3D33_CLEAN_PROFILE_OK`.

## Honest engineering notes

Three real integration defects were found and fixed by the validation
itself: the CLI's argument validation rejected `--pipe auto` before the
resolver could handle it; the extension exported no activation result so
`extension.exports` was undefined for consumers; and Node's `child.kill()`
does not reliably terminate the CLI on Windows (the validation now uses
`taskkill /T /F`). The pipe-client also dropped a TypeScript parameter
property so `node --test` could load it.

## Verification

Rust: fmt, clippy `-D warnings`, 239 workspace tests (5 new discovery
tests: roundtrip/removal, newer-endpoint protection, malformed/unknown
protocol fail-closed, no secrets in the record, discovery-file DACL
inspection), release + msvc check builds. Host: npm 43/43 (4 new
discovery parser tests), typecheck, real `M3D28_RECOVERY_VALIDATION_OK`
regression, and the clean-profile E2E above. `git diff --check` clean.

## Verdict

**RELEASE_PATH_PACKAGING_VALIDATED** for the single-user development
posture: one release executable, a minimal production VSIX, an explicit
manual startup model with per-lifecycle discovery that can never leak
authorization across lifecycles, and real clean-profile install and
operation evidence. No mutation, no Applied, no composition change.
