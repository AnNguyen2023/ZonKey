/**
 * M3D-21 real-VS-Code transport integration entry, executed inside the real
 * extension host by @vscode/test-electron while a real ZonKey named-pipe
 * validation endpoint (zonkey-cli serve-host-validation) is listening.
 *
 * Validates end to end, query/reject only: extension activation, a real local
 * editor snapshot, the HELLO/WELCOME session handshake over the named pipe,
 * the request crossing the pipe, the service-side composition gate returning
 * `rejected:CompositionUnknown`, duplicate replay without re-execution,
 * disconnect-after-request resolving to `Indeterminate` semantics via replay,
 * and an untouched document throughout.
 */
import * as vscode from "vscode";
import assert from "node:assert/strict";
import { VsCodeHostAdapter, requestFromSnapshot } from "../src/adapter.ts";
import { canonicalJson } from "../src/contract.ts";
import { NamedPipeClient } from "../src/pipe-client.ts";
import { createRealBinding } from "../src/vscode-binding.ts";

const SENTINEL = "zonkey-spike-target";
const REPLACEMENT = "zonkey-spike-applied";
let currentStep = "start";

export async function run(): Promise<void> {
  try {
    await runInner();
  } catch (error) {
    console.error(
      `m3d21_failure step=${currentStep} ${JSON.stringify(error, [
        "kind",
        "detail",
        "message",
        "name",
        "stack",
        "actual",
        "expected",
        "operator",
      ])}`,
    );
    throw error;
  }
}

async function runInner(): Promise<void> {
  const pipeName = process.env.ZONKEY_M3D21_PIPE ?? "";
  assert.ok(pipeName.length > 0, "ZONKEY_M3D21_PIPE is required");

  currentStep = "activate";
  // 1. Extension activates.
  const extension = vscode.extensions.getExtension(
    "zonkey-spike.zonkey-vscode-spike",
  );
  assert.ok(extension !== undefined, "extension is not present");
  await extension.activate();
  assert.ok(extension.isActive, "extension did not activate");

  // One ordinary local file, one editor, single caret, no selection.
  const folders = vscode.workspace.workspaceFolders;
  assert.ok(folders !== undefined && folders.length === 1, "no workspace folder");
  const fileUri = vscode.Uri.joinPath(folders[0].uri, "transport-probe.txt");
  const content = `hello ${SENTINEL} end\n`;
  await vscode.workspace.fs.writeFile(fileUri, new TextEncoder().encode(content));
  const document = await vscode.workspace.openTextDocument(fileUri);
  const editor = await vscode.window.showTextDocument(document, { preview: false });
  const sentinelEnd = content.indexOf(SENTINEL) + SENTINEL.length;
  const caret = document.positionAt(sentinelEnd);
  editor.selection = new vscode.Selection(caret, caret);

  const before = document.getText();
  const versionBefore = document.version;

  // 2-3. Real binding snapshot; the honest composition state is Unknown.
  const binding = createRealBinding();
  const adapter = new VsCodeHostAdapter(binding);
  const captured = adapter.captureSnapshot({
    expected_text: SENTINEL,
    replacement: REPLACEMENT,
  });
  assert.ok(captured.ok, `snapshot refused: ${captured.ok ? "" : captured.reason}`);
  assert.equal(captured.snapshot.composition, "Unknown");
  const canonicalOne = canonicalJson(
    requestFromSnapshot(captured.snapshot, "m3d21-req-1"),
  );
  const canonicalTwo = canonicalJson(
    requestFromSnapshot(captured.snapshot, "m3d21-req-2"),
  );

  currentStep = "connect_variants";
  // 4. Extension connects and completes the session-bound handshake. The
  // extension host accepts the `\\?\pipe\` device-path form; both forms are
  // tried and the working one is reused for every later connection.
  const pipeVariants = [
    pipeName,
    pipeName.replace(/^\\\\\.\\/, "\\\\?\\"),
  ];
  let client: NamedPipeClient | undefined;
  let workingPipe = "";
  for (const variant of pipeVariants) {
    try {
      client = await NamedPipeClient.connect(variant, 12_000);
      workingPipe = variant;
      break;
    } catch {
      // Try the next pipe-path form.
    }
  }
  assert.ok(
    client !== undefined,
    `pipe connect failed for all variants of ${pipeName}`,
  );
  assert.ok(client.sessionId().startsWith("sess-"), "server-issued session bound");

  // 5-7. Snapshot/request crosses the pipe; the real host path rejects.
  currentStep = "request_one";
  const result = await client.request("m3d21-req-1", "Unknown", canonicalOne, 10_000);
  assert.equal(result, "DEFINITE|rejected:CompositionUnknown");

  // 8. Duplicate request id replays the recorded rejection, no re-execution.
  currentStep = "request_replay";
  const replay = await client.request("m3d21-req-1", "Unknown", canonicalOne, 10_000);
  assert.equal(replay, result);

  // Document untouched so far.
  assert.equal(document.getText(), before);
  assert.equal(document.version, versionBefore);

  // The spike pipe serves one connection at a time: close the first client
  // before the next connection can be accepted.
  client.destroy();

  // 9. Disconnect after the request, before the result: caller-side loss.
  currentStep = "lost_connect";
  const lost = await NamedPipeClient.connect(workingPipe, 10_000);
  const pending = lost
    .request("m3d21-req-2", "Unknown", canonicalTwo, 10_000)
    .then((value) => ({ resolved: value }))
    .catch((error) => ({ error }));
  // Drop the connection in the same tick as the write: the caller never
  // observes a result, which is exactly the ambiguous-loss scenario.
  lost.destroy();
  const outcome = await pending;
  assert.deepEqual(outcome, { error: { kind: "ConnectionLost" } });

  // Reconnecting and replaying the same request id resolves the ambiguity
  // from the recorded outcome; nothing re-executes.
  currentStep = "resolver_connect";
  const resolver = await NamedPipeClient.connect(workingPipe, 10_000);
  const resolved = await resolver.request(
    "m3d21-req-2",
    "Unknown",
    canonicalTwo,
    10_000,
  );
  assert.equal(resolved, "DEFINITE|rejected:CompositionUnknown");
  resolver.destroy();

  // Final invariants: no mutation ever happened.
  assert.equal(document.getText(), before);
  assert.equal(document.version, versionBefore);
  vscode.window.showInformationMessage("Zonkey M3D-21 transport validation passed");
}
