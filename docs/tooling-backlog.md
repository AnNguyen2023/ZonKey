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

## TB-002 CLOSED — 2026-08-21

Verdict **STANDALONE_KIT_READY**. Machine #2 passed the real physical
one-command smoke from the kit outside any source repo (markers through
PILOT_SMOKE_OK / ZONKEY_BETA_SMOKE_OK, Rejected(CompositionUnknown),
document unchanged, no Applied/mutation). Tester machine requires no
Node/npm/Rust/source tree; offline after copy; VS Code pinned 1.133.0;
Git absence is non-blocking (manifest + checksums carry provenance).
Canonical: scripts/beta-kit/run-beta-smoke.ps1 + scripts/package-beta-kit.mjs.
See ADR 0042.
## TB-003 — Transactional release packaging — CLOSED 2026-08-21

`scripts/package-beta-kit.mjs` is now transactional: build/download into a
unique staging directory; verify everything there (pinned VS Code 1.133.0
zip, all artifacts, manifest, SHA256SUMS, checksum re-verification);
publish only after full verification (old kit renamed aside, deleted only
after the new kit is in place, mid-publish failure rolls back). Failures
exit typed — `PACKAGING_DOWNLOAD` (network/DNS surfaced verbatim),
`PACKAGING_VERIFY`, `PACKAGING_PUBLISH` — and never touch the previous
known-good kit. Proven by injection: bogus URL → PACKAGING_DOWNLOAD;
missing artifact → PACKAGING_VERIFY; exclusive file handle inside the kit
directory → PACKAGING_PUBLISH (EPERM) — in all three the prior kit stayed
byte-for-byte identical and staging/retired directories were cleaned.
Successful rebuilds publish and pass 7/7 checksums. Manifest semantic made
explicit: `git_worktree_clean` renamed to `git_tracked_changes_clean`
(tracked-changes-only; untracked local files never affect kit provenance;
the source commit is recorded separately; the kit runner never read the
old field, so no compatibility break). Test hook `ZONKEY_KIT_VSCODE_URL`
exists for failure injection only.
