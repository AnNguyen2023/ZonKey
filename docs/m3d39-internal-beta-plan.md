# M3D-39 internal beta distribution plan

Checkpoint: `c7cd538166e2f2c57eb5db5763673fd3181b0018`

Verdict: **INTERNAL_BETA_PLAN_READY**

This plan distributes the current query/reject-only beta. It does not add
mutation, Applied, or any composition bypass. `CompositionUnknown` remains
fail-closed.

## Audience and scope

- Internal testers only.
- Windows 11 x64 only (`x86_64-pc-windows-msvc`).
- VS Code desktop with ordinary local `file:` documents only.
- No remote, web, secure, elevated, or unsupported editor contexts.

The exact package is
`release/zonkey-query-reject-beta-0.0.1/`. Verify its
`SHA256SUMS.txt` and `release-manifest.json` before each test cycle. The
manifest must report the pinned commit, `git_worktree_clean: true`, platform,
protocol IDs, recovery format, and artifact hashes.

## Test matrix

Run each scenario with a fresh endpoint lifecycle when noted. Use physical
keyboard input only; do not use SendInput, paste, macros, or scripted feeds.

| Scenario | Physical example | Expected query/reject behavior |
|---|---|---|
| Install | Install the packaged VSIX | VSIX activates; no extra runtime files are required |
| Startup/discovery | Start `zonkey-cli.exe handoff-live --pipe auto` | VS Code discovers and connects through the current-user record |
| RestoreCandidate | `resume` then Space | Current handoff is observed; packaged command returns `Rejected(CompositionUnknown)`; text/version unchanged |
| Keep | `dungf` then Space | No current handoff; no request or document change |
| Ambiguous | `hello` then Space | No current handoff; fail closed with no request or document change |
| Reconnect | Reconnect without restarting the CLI | Same lifecycle remains authorized; no cross-session replay |
| Restart/stale identity | Stop and start the CLI again | New pipe/session identity; stale lifecycle/session rejects |
| Recovery/reconciliation | Run the existing approved recovery/clean-profile regression | LIST is aggregate; unresolved state stays fail-closed until the required reconcile/owner acknowledgment; no plaintext is emitted |
| Uninstall/cleanup | Stop CLI, uninstall VSIX, clean stale record | No ZonKey service or background process remains |

For the packaged manual surface, the active local document must contain the
live rendered candidate immediately before the caret. The observer is
observe-only and never edits the document. If that precondition is not met,
`SnapshotRefused` is the expected safe result. The owner one-window physical
smoke is the accepted end-to-end evidence for the full live flow; its harness
is not part of the five-file minimal beta package.

## Feedback capture

Capture one sanitized record per scenario with only:

```text
beta_version=0.0.1
git_commit=c7cd538166e2f2c57eb5db5763673fd3181b0018
platform=Windows 11 x64 / x86_64-pc-windows-msvc
vscode_version=<version>
scenario=<restore|keep|ambiguous|reconnect|restart|recovery|cleanup>
result=<typed reason enum or bounded operator result>
document_unchanged=<true|false|not_checked>
crash=<true|false>
transport_retries=<bounded integer>
notes=<sanitized operational note>
```

Do not record raw document text, token contents, URI/path, request/session
IDs, pipe names, recovery hashes, secrets, or clipboard contents. If a log
contains any of those values, stop collection, delete the leaked copy, and
report a privacy stop condition.

## Stop-the-line conditions

Stop the beta immediately and preserve the minimal sanitized evidence if any
of these occurs:

- document text or version changes unexpectedly;
- Applied, mutation, `TextEditor.edit`, SendInput, clipboard, suppression, or
  replay becomes reachable;
- a crash loses unresolved durable recovery state;
- a cross-user or cross-session connection succeeds;
- a stale pipe/session identity is accepted;
- raw document/token/path/URI/secret data appears in production diagnostics;
- repeated transport instability, unbounded waits, threads, handles, or
  connections appears.

Do not retry through a stop condition and do not “clean” the evidence by
raising timeouts or deleting recovery state.

## Install, stop, and rollback

1. Verify `SHA256SUMS.txt` and the manifest commit before installation.
2. Install `zonkey-vscode-spike-0.0.1.vsix` from the VS Code Extensions view.
3. Start `zonkey-cli.exe handoff-live --pipe auto` in a dedicated terminal.
4. Run **Zonkey spike: connect to discovered endpoint**, then
   **Zonkey spike: check current handoff**.
5. Stop the CLI with `Ctrl+C` before uninstalling the VSIX.
6. Uninstall the VSIX from VS Code.
7. Only after the CLI has stopped, remove a stale
   `%LOCALAPPDATA%\ZonKey\endpoint.txt` if it remains.
8. Preserve `%LOCALAPPDATA%\ZonKey\recovery-state.bin`; remove it only with
   explicit owner approval to reset recovery state.

To roll back, stop and uninstall the current pair, verify the previous beta
manifest/checksums, then install the previous VSIX and start its matching
`zonkey-cli.exe`. Never mix artifacts from different manifests.

## Safety boundary

The beta is query/reject-only. Real Applied remains blocked because composition
evidence is not trustworthy. `CompositionUnknown` must remain fail-closed.
