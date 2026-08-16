# ADR 0026: M3D-21 real VS Code transport integration (query/reject only)

Status: Proposed for M3D-21 owner review.

M3D-21 connects the real VS Code extension host to the real Windows
named-pipe transport end to end, strictly query/reject. The extension side
gains a TypeScript pipe client (`hosts/vscode-spike/src/pipe-client.ts`)
speaking the exact M3D-20 wire protocol (length-prefixed UTF-8 frames,
`HELLO`/`WELCOME` session handshake, `REQ` envelope carrying the request id,
composition state, and canonical request JSON). The service side gains a
query-only composition gate in `zonkey-service::transport`
(`composition_gate_outcome`: `Unknown` → `rejected:CompositionUnknown`,
`Active` → `rejected:CompositionActive`, even `Inactive` →
`rejected:ExecutionNotImplemented` because no host execution is approved)
exposed by `zonkey-cli serve-host-validation` over the existing pipe server.
No `TextEditor.edit` is enabled, no composition policy is weakened, and no
mutation path exists anywhere in the chain.

The real-VS-Code integration test
(`integration/real-vscode-transport.entry.ts`, run via `npm run
test:transport`) orchestrates everything against real processes: a real
`zonkey-cli` pipe server, real VS Code desktop via `@vscode/test-electron`,
one ordinary local file editor with a single empty caret. It asserts
activation, a real snapshot with `composition: Unknown`, the session-bound
handshake, the request crossing the pipe, the recorded duplicate replay, a
same-tick disconnect yielding the caller-side loss (`ConnectionLost` →
`Indeterminate` semantics) resolved by reconnecting and replaying the same
request id, and a document whose text and version never change.

Measured integration facts worth recording: the Electron extension host
connects when the pipe is addressed through the `\\?\pipe\` device-path form
(the `\\.\pipe\` form connects from plain Node but hung in the ext host), and
the spike pipe server serves one connection at a time, so a client must close
its connection before another can connect. Both facts are handled in the
integration entry and are boundaries of this spike, not general transport
guarantees.

Security claims are unchanged from ADR 0025: default process DACL, no
impersonation, no trust in PID/HWND/pipe name, and a server-issued session id
that is a correlation token, not authentication.
