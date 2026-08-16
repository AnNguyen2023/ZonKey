# ADR 0033: M3D-28 production reconciliation and recovery tooling

Status: Proposed for M3D-28 owner review.

M3D-28 turns the harness-proven ADR 0030 recovery model into operator
tooling over the existing transport, with no mutation and no Applied path.
`zonkey-service::transport` gains a `RecoveryRegistry`: a bounded (FIFO
eviction), in-session registry where an Indeterminate outcome blocks a
logical target (document URI + rendered token); only an explicit
reconciliation readback — comparing the live host range text against the
recorded replacement and expected token — followed by an explicit operator
acknowledgement unblocks it. Reconciliation is idempotent, ack-before-
reconcile is rejected, wrong sessions are rejected, and conflicting or
unknown readbacks keep the target blocked (`ConflictHumanReview`).

The pipe protocol reuses the existing session-bound framing with one new
command family, `RECOVERY|<session>|LIST|BLOCK|RECONCILE|ACK`; answers are
definitive transport results (`recovery-list…`, `recovery-blocked`,
`recovery-verdict:<verdict>`, `recovery-acked`, or
`recovery-error:<reason>`). `zonkey-cli recovery --pipe <name> <command>`
exposes the operator surface; the `serve-host-validation` and `handoff-live`
endpoints carry a shared registry.

Persistence is deliberately not implemented: the registry is bounded and
in-session, and a crash or restart empties it. This is a stated limit, not a
durable-recovery claim — after a restart the operator re-establishes state
from audit output; durable persistence stays release-gated (ADR 0032). No
automatic retry or reapplication exists anywhere.

Verification: registry unit tests (full lifecycle, all verdicts,
ack-before-reconcile, wrong session, bounded eviction, restart emptiness),
pipe tests (LIST/BLOCK/RECONCILE/ACK over a real pipe, idempotent reconcile,
wrong-session rejection), and a real-VS-Code E2E
(`npm run test:recovery`) that blocks a logical target derived from a real
editor snapshot, rejects ack-before-reconcile, reconciles a contradictory
readback to `ConflictHumanReview` and the exact host readback to
`NotApplied`, acknowledges both cycles, and asserts the document text and
version never change. The real VS Code binding is untouched;
`CompositionUnknown` remains fail-closed and `TextEditor.edit` remains
forbidden.
