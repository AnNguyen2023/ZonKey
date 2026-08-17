# ADR 0036: M3D-31 durable recovery persistence implementation

Status: Proposed for M3D-31 owner review.

M3D-31 implements the approved ADR 0035 durable recovery model and closes
both of its residuals with WAL-like durable preflight intent semantics.
Real Applied stays disabled, `CompositionUnknown` stays fail-closed, and no
mutation path exists; this milestone only makes recovery state survive
restarts safely.

## What is implemented

- `zonkey_service::recovery_codec`: a dependency-free, bounded, versioned
  file format (magic `ZNKYREC1`, version 1, 128-bit per-file salt, entry
  count ≤ 128, total size ≤ 256 KiB, CRC32 integrity) plus clean-room
  SHA-256 (NIST-vector tested) and CRC-32. Records are either **blocked**
  targets or **pending** preflight intents and persist only URI, UTF-16
  range, salted SHA-256 hashes of the expected/replacement tokens, verdict
  state, a generation marker, and the creating request id. Plaintext
  document text is never encoded; every malformation, truncation, oversize,
  unknown version, or checksum mismatch decodes to a typed error.
- `zonkey_service::transport`: `RecoveryDescriptor` carries request id,
  logical target URI, host-owned UTF-16 range (carried verbatim, never
  derived from service scalar lengths), expected/replacement values, and
  generation with any future mutation-capable host request. `RecoveryText`
  splits fresh plaintext blocks from hash-restored targets (reconciliation
  compares hashes and fails closed without the salt); restored targets load
  with an empty session id and rebind to the current session on the first
  valid operator action; the registry never evicts unresolved blocks and
  rejects new blocks with `RegistryFull` when full.
- `zonkey-win::recovery_store`: `DurableRecoveryStore` over
  `%LOCALAPPDATA%\ZonKey\recovery-state.bin`. Every mutation is
  write-through-before-commit (clone → mutate → encode → temp file with the
  M3D-29 current-user-only DACL → `FlushFileBuffers` →
  `MoveFileExW(MOVEFILE_WRITE_THROUGH | MOVEFILE_REPLACE_EXISTING)` →
  commit in memory); a failed durable write leaves the live state
  untouched. A corrupt, truncated, oversized, or unknown-version file
  poisons the store: every recovery command answers a typed
  `recovery-error:StateUnreadable:…`, `target_blocked` fails closed, and
  the file is never treated as empty; writes never fall back to a weaker
  ACL. Production endpoints use ledger capacity 256 and the durable
  registry capacity 128.

## Durable preflight (closure of residual 1)

A descriptor-carrying request family (`REQX`, same session-bound framing
and the same fail-closed handler as plain requests — no execution path)
participates in a compact intent protocol:

1. Before a fresh request may execute, a `PendingRecovery` record is
   persisted durably (`begin_pending`); preflight failure answers
   `rejected:RecoveryPreflightFailed` and the handler never runs.
2. A blocked logical target answers `rejected:TargetBlocked` before any
   execution, across restarts, until reconcile + owner ack.
3. The intent resolves only after the delivery attempt: a delivered
   definitive no-mutation rejection clears it durably; a delivered Applied
   or ambiguous outcome promotes it to a block; a failed delivery
   (disconnect, lost response) promotes it unconditionally.
4. A crash at any point reloads either the pending record or the promoted
   unresolved block — a restart can never look silently clean.

## Recovery-backed ledger eviction (closure of residual 2)

`BoundedRequestLedger` records descriptor requests via
`record_with_recovery`: an `Ambiguous` entry whose durable recovery record
already exists is evictable, so the ledger stays bounded without ever
forgetting an unbacked ambiguous result (unbacked entries remain pinned,
and an all-unbacked ledger refuses to record). The durable recovery state
is the source of truth for anything evicted.

## Honest residuals

Both ADR 0035 residuals are closed; what remains stated exactly: (1) a
crash between a client's transport-level loss of a plain (descriptor-free)
request and any durable record still relies on the compare-and-replace
backstop — with real Applied disabled every handler outcome is a
deterministic rejection, so nothing can double-apply today; (2) plain
`REQ` traffic remains outside the preflight protocol by design (it carries
no logical-target metadata and cannot mutate).

## Verification

Codec tests (NIST/CRC vectors, blocked and pending roundtrips for every
verdict state, determinism within a salt, salt sensitivity,
plaintext-absence, all fail-closed corruption modes, cap enforcement);
transport tests (recovery-backed vs unbacked ambiguous eviction and
saturation, registry-full rejection, restore/rebind/ack semantics,
hash-mode reconciliation); store tests (clean create/reload, restart
preservation with rebind and durable ack removal, per-verdict persistence,
corrupt/truncated/oversized/bad-magic/bad-version poisoning with the file
left intact, no plaintext on disk, deterministic encoding, fresh salts,
full-unresolved rejection durability, leftover-temp crash simulation,
state-file DACL inspection, pending-restart-as-blocked, clear after
definitive rejection, promote lifecycle, pending capacity bound, poisoned
fail-closed); pipe tests (definitive rejection clears durably, ambiguous
outcome promotes and blocks across a restart until reconcile + ack, client
disconnect after send promotes, preflight failure never executes). Gates:
fmt, clippy `-D warnings`, 230 workspace tests, msvc check, repeated
stability loops, host npm 31/31 + typecheck.

## Verdict

**DURABLE_RECOVERY_PERSISTENCE_IMPLEMENTED** with both residuals closed
per the durable preflight and recovery-backed eviction design above. No
mutation, no Applied, no composition change.
