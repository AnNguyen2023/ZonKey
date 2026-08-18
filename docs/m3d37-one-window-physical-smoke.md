# M3D-37 one-window packaged physical smoke

This is the owner smoke for the packaged query/reject surface. It uses one
isolated VS Code window and the real `WH_KEYBOARD_LL` observer. It does not
use `SendInput`, scripted keyboard input, `TextEditor.edit`, or mutation.

Build the release CLI once:

```powershell
cargo build --release -p zonkey-cli --target x86_64-pc-windows-msvc
```

Run the single harness command:

```powershell
cd D:\Zonkey\hosts\vscode-spike
npm run smoke:m3d37
```

When the single VS Code window says it is ready, type only with the physical
keyboard:

1. `dungf` then Space.
2. `resume` then Space.
3. Stop typing immediately.

The harness owns endpoint startup, packaged VSIX installation, discovery,
live-HANDOFF polling, command invocation, and cleanup. It must print:

```text
M3D37 LIVE_HANDOFF_OBSERVED
M3D37_PACKAGED_COMMAND_OK
M3D37_DOCUMENT_UNCHANGED_OK
M3D37_ONE_WINDOW_SMOKE_OK
```

The expected sanitized operator result is
`Rejected(CompositionUnknown)`. This remains query/reject-only; real Applied
and all document mutation remain disabled.
