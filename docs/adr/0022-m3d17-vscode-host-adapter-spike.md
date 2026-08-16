# ADR 0022: M3D-17 VS Code host adapter spike

Status: Accepted

The smallest VS Code extension-side adapter that maps the cooperating-host
contract onto VS Code APIs lives in `hosts/vscode-spike`. Scope is one local
ordinary `TextDocument`/`TextEditor` with a single caret; notebooks, custom
editors, remote workspaces, multi-cursor, and password-like surfaces are out
of scope and fail closed. The core is transport-free TypeScript over narrow
ports; the real `vscode` binding and a one-command manual probe are separate
and are never loaded by the automated tests, which run the same core against
an in-memory dummy harness with deterministic race injection.

Snapshot mapping: `TextDocument.version` becomes the revision; `env.sessionId`
and a host id derived from `env.appName`/`appHost`/`version` become the
session/host identity; document identity is URI plus an open-instance epoch
counter keyed by document object identity, so a closed and reopened document
gets a new epoch; editor identity is a per-instance counter. Ranges, carets,
and lengths are UTF-16 code units throughout, matching `offsetAt`/`positionAt`
and the service-side UTF-16 boundary validator. Snapshot reads are
synchronous within one extension-host turn, which is coherent for that turn;
no cross-turn atomicity is claimed.

Apply mapping: validation order mirrors the `dummy_host` contract, then the
expected text is re-read immediately before queuing exactly one replace inside
a single `TextEditor.edit` transaction. After commit the adapter requires the
next version to be exactly `revision + 1` and the resulting text to be exactly
`prefix + replacement + suffix`; otherwise the outcome is
`Indeterminate(AmbiguousCommit)`. A transaction resolving `false` is
`Rejected(EditTransactionRefused)`, one spike-specific addition to the service
rejection vocabulary; a thrown or lost result is
`Indeterminate(EditOutcomeLost)`. `Indeterminate` is never auto-retried.
Request ids are idempotent: exact duplicates replay the recorded result
without reapplying, conflicting reuse rejects, and a session restart clears
the ledger.

Honest limits: VS Code exposes no IME composition state, so the real binding
reports composition `Unknown` and every real-VS-Code apply fails closed with
`CompositionUnknown`; no composition proof is claimed and the `Applied` path
is dummy-harness-proven only. Secure state is proven non-secure only for
`file:` documents; sessions are local-desktop only. The in-session request
ledger is unbounded (spike grade). This spike claims no generic editor
support, no generic Windows execution safety, and no production auto-restore
wiring. SendInput, generic Win32 mutation, clipboard, suppression/replay, and
multi-host support are absent by design.
