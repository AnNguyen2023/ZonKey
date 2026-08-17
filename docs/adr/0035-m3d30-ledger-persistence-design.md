# ADR 0035: M3D-30 production ledger sizing / persistence review

Status: Proposed for M3D-30 owner review.

M3D-30 is a design-only review of production behavior for the request
ledger and the `RecoveryRegistry` across normal bounded operation, process
restart/crash, and ambiguous `Indeterminate` outcomes. Nothing is
implemented in this milestone; real Applied stays disabled and
`CompositionUnknown` stays fail-closed. It closes the "Bounded production
ledger sizing/persistence" BLOCKS_RELEASE item from ADR 0032.

## 1. Capacity model

An in-memory bound already exists structurally: every ledger/registry
string arrives inside one frame bounded by `MAX_FRAME_BYTES` (64 KiB), so
each ledger entry is hard-bounded by roughly two frames (request + result)
and each registry target by one frame plus a result string. Production
choices:

- Request ledger: **256 entries** (the existing `DEFAULT_LEDGER_CAPACITY`).
  Hard memory bound ≈ 256 × 2 × 64 KiB ≈ 32 MiB; realistic entries are
  ~1–4 KiB, so typical use is ≈ 1 MiB. 256 is far beyond operator-paced
  request cadence and keeps the duplicate-replay window wide.
- Blocked/recovery targets: **128 entries**. Unresolved targets are
  per-incident and human-paced; 128 simultaneous unresolved targets is
  already an operational incident worth stopping at. Hard bound ≈ 32 MiB,
  typical ≈ 64 KiB. (Endpoints currently pass 64; 64 is also acceptable —
  the bound, not the exact number, is the safety property.)
- Eviction rules (new design decisions, fail-closed):
  - Ledger FIFO eviction of a **Definite** entry is safe: duplicates
    re-execute, and the compare-and-replace contract (ADR 0030) makes
    re-execution of an already-applied edit reject with `TextMismatch`, so
    no double-apply path exists even after eviction plus restart.
  - Ledger eviction of an **Ambiguous** entry must **escalate, not forget**:
    before the entry is dropped, its logical target is blocked in the
    registry (and, when persistence lands, durably recorded). Idempotency
    protection continues via target blocking.
  - Registry eviction must never drop an unresolved block. Eviction
    considers only acknowledged entries; if the bound is reached with all
    entries unresolved, `block` fails closed with a typed error and the
    operator must reconcile first. This removes the only in-memory path
    that could silently unblock an unresolved `Indeterminate`.

## 2. What must survive restart

| State | Persist? | Rationale |
|---|---|---|
| Definite outcomes (incl. rejections) | No | Deterministic handlers reproduce identical rejections; a lost Definite-Applied replay is made safe by compare-and-replace (re-execution rejects, never duplicates). |
| Ambiguous/Indeterminate outcomes | **Yes, as blocked targets** | ADR 0030 §3: an unproven outcome is never upgraded by guesswork; §4 blocks the logical target until reconcile + owner ack. |
| Blocked logical targets + reconcile/ack state | **Yes** | The hard rule: a restart must never silently convert an unresolved target into clean/unblocked state. |
| Session identity | **Never** | Sessions are per-lifecycle by design (M3D-29 nonce identity); a stale session must authorize nothing. Durable state is keyed by logical target (URI + expected token), not by session. |
| Ledger request→result map as a whole | No | Only its Ambiguous subset matters, and only via the blocked-target representation above. |

Minimal durable record per blocked target: document URI, salted hash of the
expected token, salted hash of the intended replacement, UTF-16 range,
verdict/acknowledgement state, creation metadata (generation, formatted
timestamp), and a per-file 128-bit random salt stored in the header. The
 plaintext of document text is never stored; reconciliation remains an
exact equality check because verdicts compare hash(live readback) against
the stored hashes.

## 3. Restart / crash semantics

- Startup loads the durable state. Any unresolved blocked target puts the
  endpoint in **recovery-required mode**: requests whose logical target is
  blocked are rejected (`TargetBlocked`) until reconciliation plus explicit
  owner acknowledgement completes. Read-only traffic (observe, handoff
  queries) is unaffected.
- Persisted entries carry no session. The first `RECONCILE`/`ACK` after a
  restart rebinds the entry to the current session; commands from other
  sessions are still rejected. This preserves session invalidation without
  losing blocks.
- A corrupt, torn, or unreadable state file fails closed: the endpoint
  surfaces a typed `StateFileUnreadable` error and serves no
  mutation-bound requests; it never interprets partial state and never
  defaults to "no blocks". With real Applied disabled today the operational
  impact is zero (the composition gate already rejects everything), so the
  fail-closed posture costs nothing now and is correct later.
- Write ordering: the block is durably recorded **before** the endpoint
  answers with the Ambiguous outcome. A crash in the loss-to-write window
  is covered by the same compare-and-replace backstop plus operator audit;
  this residual is stated, not hidden.

## 4. Storage model comparison

1. **No persistence + fail-closed startup lock** — safe only while real
   Applied is disabled; violates the §3 rule in the Applied era because
   unresolved blocks are silently forgotten. Rejected as the release model;
   it remains an accurate description of today's behavior (ADR 0033).
2. **Local append-only journal** — natural write-ahead and torn-tail
   tolerance, but needs rotation/compaction and replay logic for a state
   that is tiny and rarely written. Disproportionate surface.
3. **Compact durable state file** — the whole state is small (≤ 128
   records) and changes are operator-paced; atomic replace gives simple,
   verifiable crash atomicity. **Chosen.**
4. Registry keys / embedded DB / event log — heavier or wrong scope.

Chosen mechanics: a single file under a ZonKey subdirectory of
`%LOCALAPPDATA%`, written as temp file → `FlushFileBuffers` →
`MoveFileExW(MOVEFILE_WRITE_THROUGH | REPLACE_EXISTING)`; fixed
little-endian record format with header version, record count, length, and
CRC32; single writer (the endpoint process); explicit current-user-only
DACL built with the same M3D-29 `SetEntriesInAclW` guard.

## 5. Integrity and privacy

- Bounded file size: hard cap ≈ 256 KiB (128 records × ~1.2 KiB worst case
  URI + hashes + state, plus header); anything larger fails closed.
- Atomic and durable per state change (block, verdict, ack); operator-paced
  frequency keeps the flush cost irrelevant.
- Corruption ⇒ fail closed (recovery-required mode + typed error); no
  partial interpretation, no auto-reset.
- Document text never persists in plaintext — only salted SHA-256 of the
  expected and replacement tokens; the URI (a local path) persists because
  the operator must be able to see which document is blocked. Diagnostics
  stay length/enum-sanitized as today.
- Current-user-only access on the file, inheriting the M3D-29 DACL guard;
  no world-readable state.

## Verdict

**DURABLE_RECOVERY_MODEL_READY** (design). Persistence is required (the
restart-unblock rule cannot be honestly satisfied without it in the Applied
era), the durable set is exactly the blocked-target metadata above, and
the compact-file model with fail-closed corruption handling and hashed
tokens is fully specified. Implementation is a separate milestone requiring
owner approval; until then today's no-persistence behavior remains the
documented, fail-closed interim, and no Applied path exists.
