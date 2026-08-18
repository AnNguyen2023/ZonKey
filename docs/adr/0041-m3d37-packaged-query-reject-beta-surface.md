# ADR 0041: M3D-37 packaged query/reject beta surface

Status: Accepted; M3D-37 DONE.

## Decision

Expose the smallest production VSIX operator surface required to query the
current live `RestorePlanHandoff`, capture a real VS Code host snapshot, send
the query/reject request through the discovered named pipe, and display only a
sanitized result. The command is **Zonkey spike: check current handoff**.

The packaged physical smoke is a single runner:
`npm run smoke:m3d37`. It starts the real `handoff-live` endpoint, installs the
packaged VSIX into an isolated profile, opens one VS Code window, and waits for
real physical `WH_KEYBOARD_LL` input. After the live handoff appears, the
installed VSIX command runs automatically and verifies
`Rejected(CompositionUnknown)` and unchanged document text/version.

The runner never uses `SendInput`, scripted keyboard input, `TextEditor.edit`,
Applied, clipboard, suppression, or a composition bypass. The handoff is
queried again by the packaged command; it is not frozen or fabricated.

## Release artifacts

The beta lifecycle pack contains the release `zonkey-cli.exe`, the minimal
five-entry VSIX, `OPERATOR.md`, `release-manifest.json`, and
`SHA256SUMS.txt`. The manifest records the commit, Windows 11 x64 target,
protocol identifiers, recovery format, and artifact hashes. Packaging refuses
a dirty worktree by default.

## Verification

The owner completed the physical one-window smoke with exit code 0 and no
scripted input as final evidence. Automated verification covers the packaged
command path, clean-profile installation, VSIX allow-list, diagnostics tests,
Rust transport gates, npm tests, typecheck, and document text/version
invariance. Real Applied remains disabled because composition evidence is not
trustworthy; `CompositionUnknown` remains fail-closed.
