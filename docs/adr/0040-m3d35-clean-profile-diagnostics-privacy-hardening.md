# ADR 0040: M3D-35 clean-profile harness and diagnostics privacy hardening

Status: In progress for M3D-35 owner review.

## Context

The M3D-33 clean-profile regression reported `MODULE_NOT_FOUND` before the
extension test entry point ran. The VS Code test process inherited
`ELECTRON_RUN_AS_NODE=1` from the development environment. Electron therefore
started as plain Node, interpreted the workspace directory passed by
`@vscode/test-electron` as a module path, and failed before activation.

## Decision

The clean-profile harness removes `ELECTRON_RUN_AS_NODE` from the environment
of the Electron test application. It launches an isolated user-data directory,
extensions directory, discovery directory, and workspace. Endpoint identity is
read from the current-user discovery record; CLI output is not used as a
secret-bearing transport channel. The packaged VSIX is checked against an
exact allow-list containing only the manifest, package metadata, README, and
production bundle.

Runtime diagnostics are production-safe by default:

- native key diagnostics emit only event categories, never key values;
- endpoint status and CLI output omit pipe and session identities;
- connect failures expose typed reasons, not raw OS error/path text;
- recovery `LIST` emits counts only, while explicit operator commands retain
  typed verdicts without echoing stored URI or token values;
- decision diagnostics remain length/enum metadata; raw token display remains
  available only through the already explicit development `--show-token`
  option documented by ADR 0008.

No document mutation, Applied result, `TextEditor.edit`, SendInput, clipboard,
suppression, or CompositionUnknown bypass is introduced.

## Verification

The clean-profile E2E must prove activation, endpoint discovery, recovery/query,
CompositionUnknown, and unchanged document text/version. Unit and integration
tests must prove diagnostic strings omit pipe/session identities and recovery
plaintext. Packaging must pass the VSIX exact-content allow-list. Full Rust,
npm, typecheck, transport, and clean-profile gates remain required before
closing M3D-35.
