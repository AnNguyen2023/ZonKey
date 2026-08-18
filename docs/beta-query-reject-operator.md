# ZonKey internal query/reject beta operator guide

Scope: Windows 11 x64 only (`x86_64-pc-windows-msvc`). This beta observes
physical keyboard input and exercises the read-only handoff/request path. It
never applies an edit.

## Package

The beta directory contains:

- `zonkey-cli.exe`
- `zonkey-vscode-spike-0.0.1.vsix`
- `release-manifest.json`
- `SHA256SUMS.txt`
- this guide as `OPERATOR.md`

Verify `SHA256SUMS.txt` before starting. The manifest pins the Git commit,
platform, artifact hashes, transport protocol, host protocol, recovery format,
and UTF-16 unit schema.

## Run

1. Install the VSIX from the VS Code Extensions view.
2. Open one ordinary local `file:` document with one empty caret.
3. Start `zonkey-cli.exe handoff-live --pipe auto`.
4. Use the physical keyboard to create the current RestoreCandidate. Do not
   use scripted input as live evidence.
5. In VS Code run **Zonkey spike: connect to discovered endpoint**, then run
   **Zonkey spike: check current handoff**.
6. Expected real-host result: `Rejected(CompositionUnknown)`. Verify the
   active document text and version are unchanged.

The observer does not transform or insert editor text. If the active document
does not already contain the handoff's rendered candidate immediately before
the caret, the command refuses before transport; this is a safe failure.

## Stop and clean up

Stop the CLI with Ctrl+C. Uninstall the VSIX through VS Code. After the CLI
has stopped, remove a stale `%LOCALAPPDATA%\ZonKey\endpoint.txt` if present.
Do not delete `recovery-state.bin` unless the owner explicitly wants to reset
recovery state. No service, auto-start, or background process is installed.

## Safety and limitations

`CompositionUnknown` is fail-closed. There is no Applied result, mutation,
`TextEditor.edit` runtime path, SendInput, clipboard, suppression, or
automatic retry. One active client is supported per endpoint; a second client
fails boundedly. Remote, secure, unsupported, and ambiguous contexts fail
closed.
