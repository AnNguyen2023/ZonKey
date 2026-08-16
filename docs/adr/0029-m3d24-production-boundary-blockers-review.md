# ADR 0029: M3D-24 production boundary / remaining blockers review

Status: Proposed for M3D-24 owner review.

M3D-24 is a design-only review of the remaining blockers before any real
`Applied` path, now that physical keyboard → observer → decision → handoff →
internal gate → named pipe → real VS Code extension is validated end to end
with live evidence (ADR 0028). No mutation is designed or implemented here.

## Blocker matrix

| Blocker | Classification | Rationale |
|---|---|---|
| Composition evidence | MUST_SOLVE_BEFORE_APPLIED | ADR 0023 measured that VS Code cannot provide trustworthy composition state and no external source binds coherently; every real apply fails closed at `CompositionUnknown` today. A real Applied requires a proven-Inactive source (upstream VS Code API, an approved in-host mechanism, or an owner-accepted policy change) plus a new ADR. |
| Crash / Indeterminate recovery policy | MUST_SOLVE_BEFORE_APPLIED | A real commit can be lost between the edit transaction and the ledger record; replay-after-reconnect already resolves it idempotently, but a user-visible "state unknown" outcome needs an explicit owner-approved recovery policy and tooling before any Applied runs on real documents. |
| Pipe authentication / ACL | HARDEN_BEFORE_RELEASE | Default process DACL plus the session-id correlation token is honest and adequate for a single-user development machine; an explicit owner-only DACL, per-boot pipe naming, and a surface review are release work, and request validation already fails closed for foreign clients. |
| Request ledger production bounds | HARDEN_BEFORE_RELEASE | Capacity-bounded FIFO eviction with exact-duplicate replay and restart invalidation exists (M3D-19); production needs sizing/retention review and persistence decisions, not new semantics. |
| Host/session identity hardening | HARDEN_BEFORE_RELEASE | Document epoch + editor identity + session id bound correctly within one window session today; multi-window behavior, identity versioning, and reconnection re-binding need hardening before release. |
| Transport lifecycle / reconnect | HARDEN_BEFORE_RELEASE | One-connection-at-a-time with manual sequencing is spike-adequate; auto-reconnect, concurrent clients, and backoff are release engineering on the unchanged contract. |
| Packaging / service startup | HARDEN_BEFORE_RELEASE | Debug CLI plus manual launches are development-only; service install/startup, extension packaging, and version matching are release work. |
| Diagnostics / privacy | HARDEN_BEFORE_RELEASE | Outputs are sanitized today (token lengths, typed enums); a release audit must re-verify that no token text or editor content leaks into logs, crashes, or telemetry. |
| Real editor mutation eligibility | MUST_SOLVE_BEFORE_APPLIED (as design input) | The eligibility rule set for a controlled Applied — local file document, single empty caret, exact version/text/range match, one-transaction edit with post-commit verification, composition proven Inactive — must be written, reviewed, and approved before any implementation; the harness-proven mechanics from M3D-17 define its shape. |

## Verdict

**READY_FOR_CONTROLLED_MUTATION_DESIGN.**

The end-to-end evidence chain is complete and every remaining blocker is now
explicitly classified; none is an unexplored unknown. The controlled-mutation
design milestone may proceed under three standing constraints: (1) the
Applied path is specified to execute only with a proven-Inactive composition
source, which does not exist today, so the design remains unimplementable
until the composition blocker is resolved through one of the ADR-0023
reopening conditions; (2) the crash/Indeterminate recovery policy is approved
in the same design; (3) all HARDEN_BEFORE_RELEASE items stay fail-closed and
are scheduled before any release, not before the design.
