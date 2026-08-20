# ZonKey tooling backlog

Owner-visible tooling debt. No runtime, package, or checksum implications.

## TB-001 — M3D-37 smoke runner is not self-contained on a clean machine

Recorded: 2026-08-20 (after the machine #2 internal-beta pilot attempt).
Classification: BETA_TEST_TOOLING_DEPENDENCY (confirmed root cause; not a
runtime defect). Confirmed by the machine #2 rerun passing after only the
archive was supplied (see the M3D-41 record in
`docs/m3d40-internal-beta-pilot-results.md`).

Status: **IMPLEMENTED 2026-08-20** (uncommitted):
`run-m3d37-smoke.mjs` now downloads the VS Code test Electron archive via
`@vscode/test-electron`'s `downloadAndUnzipVSCode` when the
`.vscode-test` cache is missing (clean-machine self-bootstrap); bootstrap
failures map to typed markers `BOOTSTRAP_VSCODE_DOWNLOAD` /
`BOOTSTRAP_VSIX_INSTALL` instead of `OTHER_TYPED_FAILURE`; and
`ZONKEY_M3D37_BOOTSTRAP_ONLY=1` verifies the bootstrap without launching
the physical smoke. Remaining: confirm the download branch once on a
genuinely clean machine (machine #2's next natural rerun after deleting
its copied archive).

Original root cause (kept for the record): `locateCodeCli()` ran before
`runTests()` — but `runTests` is what downloads the archive. Machine #1
passed only because prior `test:*` runs had already seeded the cache.

## TB-002 — Standalone tester kit (assessment; not yet scheduled)

Target tester requirements: Windows 11 x64, VS Code Desktop, PowerShell,
the beta artifacts, a physical keyboard — ideally **no Node/npm, no Rust,
no source tree**. Feasibility (high, tooling-only): the physical smoke
needs (a) the release `zonkey-cli.exe` (self-contained), (b) the packaged
VSIX, (c) a VS Code instance with the extension and the compiled
`m3d37-physical-smoke.cjs` entry, (d) the PowerShell wrapper. Node today
serves two roles: `esbuild` compile (replaced by shipping the prebuilt
`out\m3d37-physical-smoke.cjs` in the kit) and `@vscode/test-electron`
launch (replaced by launching the vendored VS Code archive's `Code.exe`
directly with `--extensionDevelopmentPath` dummy + `--extensionTestsPath`
+ isolated profile args, which propagates the entry's exit code). The
kit would therefore be: PowerShell scripts + release CLI + VSIX + VS Code
portable archive + prebuilt smoke entry — no Node/Rust/source. Not
scheduled until the owner approves a TB-002 milestone.
