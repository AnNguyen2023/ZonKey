# ADR 0037: M3D-32 host / session identity hardening

Status: Proposed for M3D-32 owner review.

M3D-32 hardens VS Code host, session, document, and editor identity
semantics for release correctness, and tightens the recovery rebind rules
that depend on them. Real Applied stays disabled, `CompositionUnknown`
stays fail-closed, no `TextEditor.edit` is added, and nothing mutates.

## Host and session identity

- Host identity remains `vscode:<appName>:<appHost>:<version>` from the
  extension environment; it is instance *metadata*, never a trust root.
  Each VS Code window runs its own extension host with its own
  `env.sessionId`, so separate windows and desktop instances are distinct
  identities by construction; PID/HWND are never used as identity
  (consistent with ADR 0034's named-pipe posture).
- The transport session is bound to one server lifecycle (ADR 0034 nonce
  identity). Reconnecting within a live lifecycle observes the same
  session id and replays idempotently from the ledger; an extension-host
  restart is a new lifecycle with a new session, and requests carrying the
  old session reject `session_mismatch` before execution. The TS adapter
  clears its ledger only on a genuine session change.

## Document and editor identity

- Document identity stays URI + open-instance epoch + (implicitly) the
  owning session. Close/reopen of the same URI yields a new document
  object and therefore a new epoch; stale snapshots reject
  `TargetIdentityMismatch`. In-place reload (revert-from-disk) is now an
  explicit rule: the document object and epoch are retained and the
  revision advances, so stale snapshots fail `RevisionMismatch` — never a
  silent acceptance.
- Editor identity stays per-instance. New: when more than one visible
  editor shows the same file URI (split view / multiple tabs), snapshot
  capture and apply both reject `MultipleEditors` — the scope remains
  exactly one active ordinary local file TextEditor with a single empty
  caret. The real binding counts `vscode.window.visibleTextEditors` by
  URI; the dummy harness exposes the same knob.

## Recovery rebind under identity rules

Persisted recovery targets now carry the host document open-instance epoch
they were bound to (`RecoveryDescriptor.document_epoch`, carried verbatim
from the host snapshot — never derived from service token lengths). A
restored, unbound target rebinds to the calling session only when the
operator's `RECONCILE`/`ACK` command supplies exactly that epoch; a
missing or different epoch answers `recovery-error:EpochMismatch`, leaves
the target blocked and unbound — URI + token match alone never authorizes
attaching old recovery state to a reopened document. Operator-created
manual blocks carry no epoch binding and keep the M3D-31 rebind semantics.
The `RECOVERY` command wire gained the epoch field for RECONCILE/ACK
(`RECONCILE|uri|expected|epoch|live`, `ACK|uri|expected|epoch`) and the
CLI passes it through. The durable state format version moves to 2 to
carry the epoch; an unknown (including previous-version) state file still
decodes to `UnknownVersion` and poisons the store fail-closed.

## Protocol/versioning

Identity schema versions stay explicit: the host snapshot protocol id
(`zonkey.vscode-host/1`) rejects unknown protocol strings
(`ProtocolMismatch`), and the durable recovery state file carries its own
magic/version with unknown versions failing closed.

## Regression found and fixed (real E2E)

The M3D-32 real-VSCode regression gates exposed a genuine M3D-29 defect:
`ImpersonateNamedPipeClient` fails outright for SQOS-incapable clients —
Node's `net.connect` cannot pass `SECURITY_SQOS_PRESENT` — and the peer
check dropped every extension-host connection (handshake `EPIPE`). The
peer trust now falls back, still fail-closed, to comparing the client
process's primary-token user SID via `GetNamedPipeClientProcessId` →
`OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)` → `OpenProcessToken`;
the compared identity remains the user SID and the process id is only a
lookup handle, never trusted identity. Both paths failing still drops the
connection. All four runnable real-VSCode E2Es pass again.

## Verification

TS adapter tests (8 new: second-window distinct identity, stable identity
across reconnect with ledger replay, extension-host restart invalidation,
in-place reload epoch/revision semantics, two-visible-editors rejection,
close/reopen new epoch, unknown protocol schema, per-URI epoch
non-collision). Rust tests (registry epoch gate: exact epoch rebinds,
wrong/omitted epoch refuses and stays blocked; REQX epoch binding
persisted across restart with epoch-gated reconcile/ack over the pipe;
reconnect-keeps-session vs restart-rejects-old-session over the pipe).
Gates: fmt, clippy `-D warnings`, 234 workspace tests, msvc check, npm
39/39 + typecheck, real-VSCode E2Es `REAL_VSCODE_VALIDATION_OK`,
`M3D21_TRANSPORT_VALIDATION_OK`, `M3D22_HANDOFF_VALIDATION_OK`,
`M3D28_RECOVERY_VALIDATION_OK` (handoff-live remains owner-manual: it
needs live physical typing).

## Verdict

**HOST_IDENTITY_HARDENED** for the single-user, local-desktop scope: host
and session lifecycles are explicit and restart-distinguished, document
epochs survive with explicit reload semantics, editor ambiguity rejects,
recovery rebinding requires exact document-epoch compatibility, schema
versions fail closed, and the discovered SQOS regression is fixed with the
trust model documented. No mutation, no Applied, no composition change.
