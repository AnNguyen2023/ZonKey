# ADR 0030: M3D-25 controlled mutation design

Status: Proposed for M3D-25 owner review.

M3D-25 defines, design only, the exact eligibility and recovery contract
that must hold before a cooperating VS Code host may return `Applied`. No
`TextEditor.edit` wiring, no Applied path, no composition bypass, no
SendInput/Win32 mutation/clipboard/suppression exists or is added. The
design builds on measured mechanics: the one-transaction compare-and-replace
proven on the M3D-17 dummy harness, the idempotency ledger (M3D-19), the
transport and E2E chain (M3D-21–23), and the composition evidence limits
(M3D-18 / ADR 0023).

## Eligibility contract (ALL required; any unknown or mismatch => Rejected)

Checks run in a fixed order before any transaction starts; the first
failure rejects with its typed reason, and nothing is retried:

1. Protocol id equals `zonkey.vscode-host/1` (else `ProtocolMismatch`).
2. Transport session established and the request's session id matches the
   handshake-bound session (else `SessionMismatch`).
3. Host identity matches the binding (else `HostIdentityMismatch`).
4. Document identity = URI + open-instance epoch + editor identity of the
   live active editor (else `TargetIdentityMismatch`).
5. A current `RestorePlanHandoff` exists, revalidates, and the internal
   execution gate passes (else `NoCurrentPlan`/`StaleHandoff`/
   `GenerationMismatch`/`MalformedSpan`/`InternalGateFailed`) — rejected
   before transport, per M3D-22.
6. Target is one local ordinary file `TextDocument` (`file:` scheme, no
   notebook/custom editor/remote) and exactly one editor instance.
7. Exactly one selection, empty (caret only) (else `SelectionNotEmpty`/
   `CaretMismatch`).
8. The target range is the host-owned UTF-16 range from the host snapshot —
   never derived from service scalar units — and is well-formed, within
   bounds, and does not split a surrogate pair (else `RangeMismatch`).
9. The exact expected rendered text equals the live document text at that
   range (else `TextMismatch`).
10. `TextDocument.version` equals the snapshot revision (else
    `RevisionMismatch`).
11. Secure state is exactly `KnownNonSecure` (else `SecureTarget`/
    `SecureUnknown`).
12. Session state is exactly `SupportedLocal` (else `UnsupportedSession`/
    `SessionUnknown`).
13. Composition state is exactly **proven `Inactive` with the host
    advertising `CAP_COMPOSITION_PROOF`** (else `CompositionActive`/
    `CompositionUnknown`; unknown is never treated as inactive).
14. Request capabilities match, `request_id` is non-empty and idempotent
    (exact duplicate replays the recorded result; conflicting reuse rejects
    `RequestIdReuse`), and replacement/range fields are well-formed.

## Mutation transaction contract

One host-owned edit transaction performs compare-and-replace atomically;
there is no separate read-then-mutate authorization step. Inside the single
`TextEditor.edit` callback (mechanics proven in M3D-17): re-read the
expected range immediately before queuing; if it no longer matches, queue
nothing and reject `TextMismatch`. Queue exactly one replace of the expected
range with the intended replacement. After the transaction resolves:

- `Applied(new_revision)` — only if all four hold: the transaction was
  accepted; the expected range/text still matched at transaction time; the
  exact intended replacement was committed; and post-commit verification is
  consistent (next document version is exactly `revision + 1` and the full
  text equals prefix + replacement + suffix).
- `Rejected(reason)` — any eligibility failure, a refused transaction
  (`EditTransactionRefused`), or a re-read mismatch with an unchanged
  version.
- `Indeterminate(reason)` — the transaction result was lost
  (`EditOutcomeLost`) or the post-commit state is not exactly provable
  (`AmbiguousCommit`, e.g. version advanced by more than the single edit or
  the text is not the exact intended outcome).

## Indeterminate recovery policy

1. Never auto-retry an `Indeterminate` outcome; no retry code path exists.
2. An exact duplicate request id returns the recorded result when known
   (ledger replay, M3D-19/M3D-21).
3. If the outcome cannot be proven after a crash or lost response, the
   result **remains `Indeterminate`** — it is never upgraded to Applied or
   Rejected by guesswork.
4. Before any future request for the same logical target (same document
   identity + rendered token), an explicit **reconciliation readback
   workflow** must run and be owner-acknowledged: a fresh host snapshot is
   read and compared — intended replacement present => resolved
   `applied-acknowledged`; original rendered token still present => resolved
   `not-applied`; anything else => resolved `conflict` for human review.
   Only a resolved target unblocks new requests; conflicting evidence keeps
   the target blocked.
5. No "best guess" retry under any circumstances.

## Composition gate

The real VS Code host today reports `composition: Unknown` and does not
advertise `CAP_COMPOSITION_PROOF` (ADR 0022/0023); therefore **real Applied
remains disabled and every real request fails closed
`Rejected(CompositionUnknown)`**. This design defines the future
precondition — a proven-Inactive composition source bound to the host
transaction, satisfying one of the ADR 0023 reopening conditions — without
inventing evidence that does not exist.

## Verdict

**CONTROLLED_MUTATION_CONTRACT_READY.**

The eligibility, transaction, and recovery contract above is complete and
implementable-on-approval; realization additionally requires (a) a real
proven-Inactive composition source, (b) owner approval of this recovery
policy, and (c) the release-hardening items classified in ADR 0029 before
any production use.
