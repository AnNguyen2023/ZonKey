# ADR 0034: M3D-29 named-pipe ACL / peer-truth hardening

Status: Proposed for M3D-29 owner review.

M3D-29 hardens the security boundary of the existing Windows named-pipe
transport (ADR 0025) without touching the protocol contract, the ledger
semantics, or any mutation path. Real Applied stays disabled and
`CompositionUnknown` stays fail-closed.

## Explicit ACL

Every pipe instance created by `zonkey-win::pipe_transport` now carries an
explicit DACL built with `SetEntriesInAclW`: one allow ACE granting the
creating process's user SID `GENERIC_ALL`, and nothing else. The default
process DACL is never used, so other interactive users, `Everyone`,
administrators, and `LOCAL SYSTEM` are denied by omission — including for
reads. Accurate admin statement: a Windows administrator can still take
ownership of the pipe object (or enable `SeSecurityPrivilege`), rewrite the
DACL, and connect; that is inherent administrative power and is a documented
residual threat, not an access-control defect. `FILE_FLAG_FIRST_PIPE_INSTANCE`
makes creation fail closed if anything squatted the pipe name first. If the
DACL (or the session nonce, or the peer-verification inputs) cannot be
built, no listener is created at all — there is no fallback to weaker
security.

## Peer truth

Determination: for the current single-user, single-machine architecture,
current-user SID validation is sufficient, implemented as two independent
layers. First, the OS enforces the explicit DACL at connect time. Second,
the server verifies the peer itself: immediately after `ConnectNamedPipe`
it impersonates the client strictly at identification level (clients open
with `SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION`, so the server can
identify but never act as the client), reads the client token's user SID,
compares it with its own SID via `EqualSid`, and always calls `RevertToSelf`
before serving anything. Any failure — impersonation, token query, SID
mismatch, revert — drops the connection unserved. PID, HWND, and pipe name
are never trusted identity. This is SID identity inspection, not
cryptographic authentication; no such claim is made.

## Pipe identity

`zonkey_win::pipe_transport::generate_pipe_name` produces
`\\.\pipe\zonkey-<prefix>-<128-bit BCryptGenRandom nonce>`, and every server
lifecycle now binds an unpredictable 128-bit nonce into its session id. A
new lifecycle therefore always has a new identity; a stale pipe name stops
resolving (connect fails closed) and a stale session id is rejected by the
existing session check before any execution. Reusing an old identity
authorizes nothing.

## Robustness fix (evidence-based)

A reproducible connect flake under parallel test load (including the two
baseline flakes and a caught `server_restart_invalidates_session_history`
panic) traced to `open_raw_handle`: on `ERROR_PIPE_BUSY` it performed a
single 50 ms `WaitNamedPipeW` and returned `ConnectTimeout` on the first
miss, violating its own deadline contract. The fix keeps honoring the
caller's full connect deadline instead of giving up early; no test timeout
was loosened and no assertion weakened.

## Verification

Unit tests in `pipe_security` (SID validity/stability, nonce uniqueness,
attribute construction) and `pipe_transport` (DACL inspection asserting
current-user-only allow ACEs on a live pipe, generated-name uniqueness and
format, per-lifecycle session nonces, restart changing identity with the
old pipe failing closed and the stale session rejected, plus the existing
protocol/session/frame/ledger/disconnect/timeout suite unchanged).
Unauthorized-SID rejection is not testable in-process with a single user
account; it is enforced by construction (explicit DACL plus the fail-closed
peer check) and remains covered by the ACL inspection test. Repeated
full-workspace runs verify parallel stability. No mutation, no Applied, no
composition bypass, no SendInput, no clipboard.

## Verdict

**PIPE_SECURITY_HARDENED** for the single-user development architecture:
explicit owner-only DACL, fail-closed identification-level peer verification,
unpredictable per-lifecycle identity, and an evidence-based connect fix.
Cryptographic authentication, cross-machine use, and service hardening
remain out of scope and unchanged in the ADR 0032 release-gate matrix.
