# ADR 0027: M3D-22 real decision handoff into the transport request

Status: Proposed for M3D-22 owner review.

M3D-22 connects the real ZonKey decision pipeline to the existing host
transport request path, strictly query/reject. `zonkey-service::transport`
gains `build_host_request`: it takes a submitted `RestorePlanHandoff`,
requires a current handoff to exist, runs revalidation plus the internal
execution gate, double-checks scalar span consistency, and maps only
validated fields (rendered token, replacement, scalar unit counts,
generation) into a `HandoffRequest` whose `request_id` is deterministically
`handoff-<generation>`, so ledger idempotency follows naturally. Every
non-passing outcome rejects before any transport involvement: no current
plan, stale handoff, generation mismatch, malformed span, or gate failure.
The mapping deliberately contains no host-native range: UTF-16 range, caret,
and document identity remain owned by the host snapshot/adapter.

The pipe binding stays native I/O only. `zonkey-win::pipe_transport` gains a
read-only `HANDOFF` query (session-checked, never touching the request
ledger) answered by a `HandoffProvider`; `zonkey-cli serve-host-validation
--handoff-token resume` feeds the real `DiagnosticDecisionProcessor` through
the real Telex/detection/policy path (the feed is scripted platform-neutral
events; no keyboard input is observed) and serves the validated handoff. The
VS Code side gains `NamedPipeClient.handoffQuery`; the M3D-22 integration
entry queries the handoff, embeds the rendered token (Telex renders "resume"
as "réume") into a real editor file, captures a real snapshot with the host
owning the UTF-16 range, and sends the request under the handoff's request
id. The real result remains `DEFINITE|rejected:CompositionUnknown`,
duplicates replay without re-execution, and the document never changes.

No `TextEditor.edit`, no Applied path, no SendInput, no generic Win32
mutation, no clipboard, no suppression/replay, no composition bypass. The
`feed_token` helper and `--handoff-token` flag exist to drive the real
decision pipeline deterministically in tests and endpoints; they are not a
runtime input path. Security claims are unchanged (ADR 0025).
