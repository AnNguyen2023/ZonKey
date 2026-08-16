# ADR 0032: M3D-27 production hardening priority review

Status: Proposed for M3D-27 owner review.

M3D-27 is a design/planning-only review of the remaining hardening areas,
after the ADR 0030 contract and its ADR 0031 harness evidence. Real Applied
stays disabled and `CompositionUnknown` stays fail-closed; nothing is
implemented in this milestone.

## Classification

| Area | Classification | Rationale |
|---|---|---|
| Composition evidence | BLOCKS_REAL_APPLIED | No proven-Inactive source exists (ADR 0023); every real apply fails closed until one of the reopening conditions lands. Timeline is not fully in our control (upstream-dependent). |
| Recovery policy owner approval + tooling | BLOCKS_REAL_APPLIED | The ADR 0030 recovery policy is designed and harness-proven but needs formal owner approval and production-grade reconciliation/readback/ack tooling before any real Applied runs on real documents. Fully in our control. |
| Named-pipe ACL/authentication | BLOCKS_RELEASE | Default DACL plus session correlation is honest for single-user development; an explicit owner-only DACL and client-auth review gate any release. |
| Per-boot/session pipe identity | BLOCKS_RELEASE | Per-boot pipe naming and stale-handle/replay hardening belong to the same release gate as the ACL work. |
| Bounded production ledger sizing/persistence | BLOCKS_RELEASE | Semantics are proven; sizing, retention, and persistence decisions gate release, not design. |
| Host/session identity hardening | BLOCKS_RELEASE | Document epoch/editor identity binds correctly within one window session; multi-window behavior and identity versioning must be hardened before release. |
| Reconnect/multi-client transport | SHOULD_HARDEN | Single-client sequencing is proven end to end; auto-reconnect, concurrent clients, and backoff improve operability without safety impact. |
| Diagnostics/privacy | SHOULD_HARDEN | Outputs are sanitized today; a release audit re-verifies that no token text or editor content leaks. |
| Startup/packaging/VSIX | BLOCKS_RELEASE | Development-grade CLI launches only; service install, extension packaging, and version matching are prerequisites of any release. |
| Crash recovery/reconciliation UX | SHOULD_HARDEN | The blocking policy/tooling half is classified above; the polished owner-facing UX beyond the minimal tooling can follow once real Applied becomes reachable. |
| (deferred items) | DEFER | None currently deferred; every area maps to a gate above. |

## Next milestone — exactly one

**M3D-28: production reconciliation and recovery tooling.**

Rationale: it is the only remaining BLOCKS_REAL_APPLIED item that is fully
within our control (composition evidence is upstream-dependent). It converts
the harness-proven ADR 0030 recovery workflow into real transport-grade
tooling — a reconcile/readback query and an explicit owner-ack command over
the existing named-pipe protocol, plus the blocked-target lifecycle surfaced
to operators — with no mutation and no Applied path. Completing it leaves
composition evidence as the single remaining blocker for real Applied, and
it directly improves safety today: whenever anything fails closed or
Indeterminate (as already happens in the live E2E), the owner gains a
deterministic, auditable way to establish ground truth.

## Verdict

**HARDENING_PLAN_READY.**
