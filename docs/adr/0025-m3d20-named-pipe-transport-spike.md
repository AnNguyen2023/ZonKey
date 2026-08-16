# ADR 0025: M3D-20 Windows named-pipe transport spike

Status: Proposed for M3D-20 owner review.

M3D-20 binds the M3D-19 transport contract to a localhost Windows named
pipe, isolated in `zonkey-win::pipe_transport`. One listener hosts one
dummy/test host; the client is the ZonKey service side. Frames, the
`zonkey.host-transport/1` protocol id, the session-bound bounded ledger, and
ambiguous-loss semantics come from `zonkey-service::transport`; this spike
adds only pipe I/O, a bounded read timeout, and clean teardown. It performs
no VS Code `Applied`, no composition change, no editor mutation, and no
auto-restore wiring.

## Binding shape

The server creates one duplex byte-mode pipe instance
(`\\.\pipe\zonkey-m3d20-...`, one instance, in/out buffers sized to the
64 KiB frame bound) and accepts connections in a loop until shut down. The
handshake is `HELLO|<protocol>` → `WELCOME|<session>`; the server-issued
session id is generated per listener (counter plus nanosecond timestamp) and
is the only request binding. Requests are `REQ|<session>|<request_id>|
<canonical>`; replies are `RESULT|DEFINITE|...` / `RESULT|AMBIGUOUS|...`,
with `ERROR|protocol_mismatch` refusing handshakes and
`ERROR|session_mismatch` rejecting requests before any execution. Framing,
size limits, malformed-frame rejection, duplicate replay, conflict rejection,
and restart invalidation are exactly the shared contract, re-tested end to
end over a real pipe. The client bounds every read: a blocking reader thread
registers a duplicated thread handle and is cancelled with
`CancelSynchronousIo` at the deadline, after which the connection is marked
broken. Disconnecting after sending a request loses the result
(`Indeterminate` at the caller); re-sending the same request id on a new
connection replays the recorded outcome without re-execution.

## Security, stated exactly

The pipe is created with default security attributes, so its DACL is the
creating process's default DACL; on a standard interactive token that
restricts access to the creating user, administrators, and local system. No
explicit per-user DACL is constructed and no client impersonation is
performed. No client identity — PID, window handle, or pipe name — is
trusted for anything. The session id is a correlation token, not
cryptographic authentication: a same-user local process that guesses the
pipe name can complete its own handshake and would receive its own session.
This spike does not claim authentication stronger than that. Production
hardening (explicit owner-only DACL, named-object lifetime policy, and
per-boot pipe naming) remains future work.

## Non-goals

No real VS Code connection, no `Applied` on any real host, no composition
evidence, no SendInput, no Win32 text mutation, no clipboard, no
suppression/replay, no generic editor support, no auto-restore wiring.
