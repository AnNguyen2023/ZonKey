# ADR 0039: M3D-34 transport lifecycle hardening (single-active-client MVP)

Status: Proposed for M3D-34 owner review.

M3D-34 hardens the named-pipe transport lifecycle for the production MVP
with an explicit **single-active-client** model and a proven, fixed,
deterministic shutdown. Real Applied stays disabled,
`CompositionUnknown` stays fail-closed, no mutation exists.

## Shutdown bug: proven and fixed

The concern "server shutdown may block indefinitely while actively serving
a client" was **real and reproduced deterministically**. Cause: the
listener thread served each connection with a blocking `ReadFile`, so with
an idle-but-connected client (or a request in flight) the thread sat
inside `serve_connection`; `shutdown()`'s stop-flag plus bounded wake
connection cannot interrupt a blocked read, and `join()` waited forever.
Two watchdog-bounded regression tests demonstrated the hang before any
fix (3s/5s budgets both exceeded).

Fix: the server connection loop now uses a **stop-aware read**
(`read_frame_stop_aware`): `PeekNamedPipe` gates every `ReadFile` so an
idle client never blocks the listener; the stop flag is polled between
empty peeks (10 ms interval); peek/read failures and malformed frames
close the connection fail-closed exactly as before. Shutdown now
completes deterministically — bounded by the poll interval plus at most
the current handler run — with the duplicate cancellation handle owned and
closed, and the client reader thread reaped after cancellation. A failed
thread-handle registration fails closed before entering a blocking read, and
connect/handshake failures close their pipe handle. No timeout was inflated
anywhere. As with any synchronous handler contract, shutdown remains bounded
by the handler's own completion; the transport does not detach or leak a
handler thread.

## Single-active-client model (explicit)

One pipe instance (`nMaxInstances = 1`) serves exactly one connected
client at a time — the production MVP deliberately avoids multi-client
support. A second concurrent connect receives `ERROR_PIPE_BUSY`, waits
only within the caller's bounded deadline, then fails closed
(`ConnectTimeout`); there is no connection queue and no unbounded thread
or wait growth (one listener thread per endpoint, one reader thread per
in-flight client read, all reaped deterministically). A noisy client
(malformed frames) is dropped fail-closed and a clean client connects
immediately afterwards — no starvation. Because only one client can hold
the connection, two clients can never race one logical host session; the
ledger and durable recovery state are only ever touched by the single
active client.

## Reconnect and restart semantics (validated)

Disconnect + reconnect within the same server lifecycle reuses the same
session id and replays duplicates from the ledger without re-execution —
reconnect never creates a fresh authorization context. Ten consecutive
reconnects keep the session stable and preserve duplicate-replay
idempotency. A server restart is a new lifecycle with a new pipe name and
session identity; stale sessions reject before execution (existing tests,
re-validated). Client crash mid-request maps to `Indeterminate`, and the
same request id replays the recorded outcome after reconnect.

## Multi-window isolation

Isolation is enforced at the two layers that own identity: separate VS
Code windows/extension hosts are distinct host-contract identities
(ADR 0037 adapter tests — session mismatch rejects cross-window
requests), and the endpoint serves one active client at a time, so one
window can never consume or replay another window's in-flight
requests/results. Endpoint discovery is a single unambiguous
current-user record per lifecycle; unknown protocols fail closed
(ADR 0038 tests).

## Verification

New Rust tests: shutdown-with-idle-connected-client (bounded 3s),
shutdown-during-active-request (bounded 5s, overlapping a slow handler),
second-concurrent-client bounded fail-closed with the active client
unaffected, noisy-client drop + immediate recovery, ten-round reconnect
loop with session stability and duplicate-replay idempotency. Existing
lifecycle tests (reconnect/restart/stale-session/disconnect-Indeterminate)
all re-validated. Gates: fmt, clippy `-D warnings`, **244 passed workspace
tests plus 6 ignored interactive tests**, msvc check, npm 43/43 + typecheck,
and a 10× `pipe_transport` stability loop. The M3D-33 clean-profile command
was also attempted but stopped in its VS Code harness before assertions:
`@vscode/test-electron` tried to load the temporary workspace directory as
a test module (`MODULE_NOT_FOUND`); it did not produce either M3D33 marker.

## Verdict

**TRANSPORT_LIFECYCLE_HARDENED** with the single-active-client model as
the validated production MVP: the shutdown defect is fixed with
evidence-backed deterministic semantics, concurrency fail-closes
boundedly, reconnect/restart semantics are pinned by tests, and resource
behavior is bounded by construction. The separate M3D-33 clean-profile
harness remains an external regression blocker, not an M3D-34 transport
finding. No mutation, no Applied, no composition change.
