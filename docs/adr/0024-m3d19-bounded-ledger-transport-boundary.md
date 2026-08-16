# ADR 0024: M3D-19 bounded ledger and transport boundary

Status: Proposed for M3D-19 owner review.

M3D-19 hardens cooperating-host protocol plumbing without enabling any real
mutation. It adds a platform-neutral transport boundary in
`zonkey-service::transport` and a bounded ledger to the VS Code host adapter.
No named pipe is opened, no TSF is implemented, no composition policy
changes, and real VS Code applies remain fail-closed at `CompositionUnknown`.

## Bounded ledger

Both ledgers (Rust `BoundedRequestLedger`, TypeScript adapter ledger) use the
same semantics: fixed non-zero capacity with deterministic FIFO eviction —
the oldest *inserted* request id is evicted when full, and lookups never
refresh order. An exact duplicate request id with an identical canonical
request replays the recorded result without re-execution; a known id with a
different canonical request rejects as reuse. Every terminal outcome kind,
including `Indeterminate`, is retained and replayed verbatim; nothing in
either implementation retries. A session restart (a hello with a new session
id) invalidates all history. Eviction is safe, not lossy-by-accident: an
evicted duplicate is re-validated against live host state before anything
executes, so re-execution can only happen after a full contract re-check.

## Transport boundary

The core is payload-opaque framing plus session state, deliberately free of
sockets, clocks, JSON parsing, and OS dependencies:

- Frames are little-endian `u32` length prefixes plus UTF-8 payloads, capped
  at 64 KiB; empty, oversized, or non-UTF-8 frames fail closed and the
  connection is dropped rather than resynchronized.
- A hello establishes `zonkey.host-transport/1` and binds exactly one session
  id. The session id is the only trusted identity: process ids, window
  handles, and port numbers are never accepted as evidence. A wrong protocol
  id rejects without state change; a new session id on a later hello is a
  restart and invalidates the ledger.
- Timeouts and connection loss after a request was sent but before any result
  arrived map to one ambiguous outcome (`connection_lost_before_result`) that
  is recorded and replayed; there is no retry path in the core.

The destination transport for Windows 11 x64 is a localhost named pipe with
per-session security; that OS binding is intentionally not implemented in
M3D-19. The contract is exercised by deterministic in-memory tests, including
a dummy-host request pipeline that executes exactly once per request id.

## Host adapter impact

The VS Code spike adapter now constructs with a bounded ledger (default 256)
whose capacity is validated up front; all existing duplicate, reuse, and
session-restart tests pass unchanged. No production `Applied` path opens: the
composition gate is untouched and composition evidence remains unknown.

## Non-goals

No SendInput, Win32 mutation, clipboard, suppression/replay, composition
bypass, or weakening of any fail-closed policy. No production auto-restore
wiring, no VS Code transport connection, and no multi-host support.
