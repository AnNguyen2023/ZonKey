# ADR 0042: TB-002 standalone internal beta tester kit

Status: Accepted — **STANDALONE_KIT_READY** (clean-machine physical
validation passed 2026-08-21).

## Design (implemented)

A self-contained kit requiring on the tester machine ONLY Windows 11 x64,
PowerShell, and a physical keyboard — no Node.js, no npm, no Rust, no
source tree, no `.vscode-test` cache, no internet.

Layout (`release/zonkey-beta-kit-0.1.0/`, 342.4 MB):

- `run-beta-smoke.ps1` — PowerShell orchestration (one command)
- `zonkey-cli.exe` — release endpoint binary (from the beta package)
- `zonkey-vscode-spike-0.0.1.vsix` — packaged extension
- `m3d37-physical-smoke.cjs` — prebuilt smoke entry, compiled from
  `integration/real-vscode-m3d37.entry.ts` by the same esbuild pipeline as
  `npm run compile:m3d37` (reproducible from repo source)
- `vscode-1.133.0-win32-x64-archive.zip` — pinned VS Code runtime,
  extracted locally on first run (offline thereafter)
- `OPERATOR.md`, `release-manifest.json`, `SHA256SUMS.txt`

Node-removal strategy: `esbuild`/`npm` are replaced by shipping the
prebuilt entry; `@vscode/test-electron` is replaced by launching the
bundled archive's `Code.exe` directly with `--extensionDevelopmentPath`
(manifest-only dummy), `--extensionTestsPath`, and isolated
`--user-data-dir`/`--extensions-dir` args — the same launch shape
test-electron uses, with the entry's exit code propagating. The VSIX
install uses the archive's `bin\code.cmd`, which runs Code.exe with its
own bundled node (`ELECTRON_RUN_AS_NODE=1` internally) — no external
Node anywhere.

Runner stages (typed, fail-closed): `PACKAGE_OR_CHECKSUM` (presence,
manifest kind/platform/version pin, SHA256 of every artifact),
`BOOTSTRAP_VSCODE_EXTRACT` (offline Expand-Archive, idempotent),
`BOOTSTRAP_VSIX_INSTALL` (isolated profile), `ENDPOINT_STARTUP`
(`handoff-live --pipe auto` + discovery poll, 20 s bound), then the
entry's own typed markers (`LIVE_HANDOFF_TIMEOUT`, `DOCUMENT_CHANGED`,
…) and the same sanitized transcript format as the repo runner.
Cleanup always kills the endpoint tree and removes all temp dirs.

Packaging: `scripts/package-beta-kit.mjs` (dev machine, needs Node once)
assembles the kit, downloads the pinned VS Code zip once, writes the
manifest (kit kind/version, platform, pinned VS Code version, source
commit, `git_worktree_clean`, expected result, safety block) and
SHA256SUMS over every artifact.

## Machine #1 evidence (2026-08-20)

Under a Node/npm-scrubbed PATH, from a kit copy outside any repo/source
tree, the kit runner proved: full checksum gate (`PILOT_PACKAGE_OK`) and
offline bundled-runtime extraction (`PILOT_VSCODE_RUNTIME_OK`, ~57 s).
The run then failed at `BOOTSTRAP_VSIX_INSTALL` — a real quoting bug
(`Start-Process cmd /c` with several quoted args mangles the `code.cmd`
path), root-caused and fixed: replace the Start-Process invocation with
the PowerShell call operator (`& $codeCli "--user-data-dir=…" …`), which
was verified working directly under the scrubbed PATH ("successfully
installed", exit 0). The fix is two lines; see "Owner unblock" below.

## Machine #1 evidence — final (2026-08-21, post AV unblock)

The owner added a Bitdefender exception covering `D:\Zonkey` (on-access,
on-demand, embedded scripts). The pre-existing quarantined files under
`scripts\kit\` remain kernel-locked (created before the exception); the
canonical runner therefore lives at `scripts\beta-kit\run-beta-smoke.ps1`
(the stale locked copies are inert local junk pending a Bitdefender
release/reboot and are the only contents of that legacy directory).

With the fix packaged (`release\zonkey-beta-kit-0.1.0`, runner byte-identical
to the canonical file), a full machine-#1 validation ran with Node and npm
**removed from PATH** (verified absent) from the kit directory itself:

```text
stage=PILOT_PACKAGE_OK
stage=PILOT_VSCODE_RUNTIME_OK        (offline extraction, ~56s, no network)
stage=PILOT_VSIX_INSTALLED           (call-operator fix works in-kit)
stage=PILOT_ENDPOINT_STARTED         (handoff-live --pipe auto + discovery)
stage=PILOT_VSCODE_READY             (bundled Code.exe + prebuilt entry)
stage=PILOT_NO_CURRENT_HANDOFF_OK    (extension connected + live query OK)
stage=PILOT_SMOKE_FAIL:LIVE_HANDOFF_TIMEOUT   (25s bound, no typing — expected)
exit_code=1 document_unchanged=unknown
```

A checksum-tamper test (one hex character flipped in SHA256SUMS.txt) failed
closed immediately with `PILOT_SMOKE_FAIL:PACKAGE_OR_CHECKSUM` before any
extraction or launch. The earlier machine-#1 section (quoting bug +
pre-fix transcript) is retained below for the record.

## Machine #1 environmental incident (2026-08-20, resolved)

Before the owner's Bitdefender exception, this machine content-quarantined
the runner script: every copy of that content — any file name, extension,
or directory — was locked or deleted within seconds, including the
corrected variant and a `.ps1.txt` twin; trivial and random content was
unaffected. Canonical rewrite and local `.ps1` execution were temporarily
impossible. This was machine security policy, not a kit defect; no
evasion was attempted, and the owner resolved it with the documented
exception. The pre-fix transcript (`PILOT_PACKAGE_OK` →
`PILOT_VSCODE_RUNTIME_OK` → `BOOTSTRAP_VSIX_INSTALL` failure under a
scrubbed PATH outside any source tree) first proved the checksum gate and
offline extraction, and surfaced the real `Start-Process cmd /c`
multi-quote bug in the VSIX-install step; the fix (PowerShell call
operator) was verified directly and then in-kit as above.

## Remaining validation before STANDALONE_KIT_READY

~~Clean machine #2 only~~ — **completed 2026-08-21**. Machine #2 ran the
kit from a directory outside any source repository and passed the real
physical one-command smoke: `M3D37_NO_CURRENT_HANDOFF_OBSERVED`,
`M3D37_LIVE_HANDOFF_OBSERVED`, `M3D37_PACKAGED_COMMAND_OK`,
`M3D37_DOCUMENT_UNCHANGED_OK`, `PILOT_SMOKE_OK`,
`ZONKEY_BETA_SMOKE_OK`, exit 0, result `Rejected(CompositionUnknown)`,
document unchanged, no Applied/mutation. Recorded facts for the ready
state: the tester machine needs no Node.js, npm, Rust, or source tree
(machine-#1 evidence additionally proved the full chain with Node/npm
removed from PATH); offline operation after copying the kit is supported
(the pinned VS Code runtime ships inside the kit and extracts locally);
the VS Code runtime is pinned to 1.133.0 win32-x64 (manifest-checked);
and Git is absent/non-blocking on this smoke path — nothing in the kit
or runner invokes Git; provenance is carried by the manifest and
checksums instead.

## Safety

No runtime semantics changed; `release/` beta package and checksums
untouched; the kit only orchestrates already-approved surfaces. No
SendInput, no clipboard, no suppression/replay, no TextEditor.edit, no
mutation; `CompositionUnknown` stays fail-closed; Applied stays
disabled.
