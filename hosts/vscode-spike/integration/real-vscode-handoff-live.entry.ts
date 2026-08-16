/**
 * M3D-23 LIVE real-VS-Code handoff entry (manual smoke, clearly labeled).
 *
 * Requires a running `zonkey-cli handoff-live --pipe <name>` endpoint that
 * the owner has fed with REAL keyboard input (this entry performs no
 * scripted feeding and rejects any such substitution). It queries the live
 * handoff over the pipe, merges it with a real VS Code snapshot (the host
 * owns the UTF-16 range), sends the request through the transport, and must
 * receive the fail-closed `rejected:CompositionUnknown`. The document never
 * changes. Assertions are intentionally relaxed versus the M3D-22 scripted
 * entry: generation and token values come from whatever the owner typed.
 */
import * as vscode from "vscode";
import assert from "node:assert/strict";
import { VsCodeHostAdapter, requestFromSnapshot } from "../src/adapter.ts";
import { canonicalJson } from "../src/contract.ts";
import { NamedPipeClient } from "../src/pipe-client.ts";
import { createRealBinding } from "../src/vscode-binding.ts";

let currentStep = "start";

export async function run(): Promise<void> {
  try {
    await runInner();
  } catch (error) {
    console.error(
      `m3d23_live_failure step=${currentStep} ${JSON.stringify(error, [
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
  const pipeName = process.env.ZONKEY_M3D23_PIPE ?? "";
  assert.ok(pipeName.length > 0, "ZONKEY_M3D23_PIPE is required");

  currentStep = "activate";
  const extension = vscode.extensions.getExtension(
    "zonkey-spike.zonkey-vscode-spike",
  );
  assert.ok(extension !== undefined, "extension is not present");
  await extension.activate();
  assert.ok(extension.isActive, "extension did not activate");

  currentStep = "handoff_query";
  const pipeVariants = [pipeName, pipeName.replace(/^\\\\\.\\/, "\\\\?\\")];
  let client: NamedPipeClient | undefined;
  for (const variant of pipeVariants) {
    try {
      client = await NamedPipeClient.connect(variant, 12_000);
      break;
    } catch {
      // Try the next pipe-path form.
    }
  }
  assert.ok(client !== undefined, `pipe connect failed for ${pipeName}`);
  const handoffPayload = await client.handoffQuery(10_000);
  assert.ok(
    handoffPayload.startsWith("handoff:"),
    `live handoff missing (owner must type a RestoreCandidate first): ${handoffPayload}`,
  );
  const parts = handoffPayload.slice("handoff:".length).split("|");
  assert.equal(parts.length, 6, "handoff payload has six fields");
  const requestId = parts[0];
  const renderedToken = parts[1];
  const replacementToken = parts[2];
  const generation = Number(parts[5]);
  assert.ok(requestId.startsWith("handoff-"), "request id from generation");
  assert.ok(generation >= 1, "generation is positive");
  assert.ok(renderedToken.length > 0, "rendered token present");
  assert.ok(replacementToken.length > 0, "replacement token present");
  assert.equal(requestId, `handoff-${generation}`, "deterministic identity");
  console.error(`m3d23_live_handoff ${handoffPayload}`);

  currentStep = "editor_with_token";
  const folders = vscode.workspace.workspaceFolders;
  assert.ok(folders !== undefined && folders.length === 1, "no workspace folder");
  const fileUri = vscode.Uri.joinPath(folders[0].uri, "handoff-live-probe.txt");
  const content = `live ${renderedToken} end\n`;
  await vscode.workspace.fs.writeFile(fileUri, new TextEncoder().encode(content));
  const document = await vscode.workspace.openTextDocument(fileUri);
  const editor = await vscode.window.showTextDocument(document, { preview: false });
  const tokenEnd = content.indexOf(renderedToken) + renderedToken.length;
  const caret = document.positionAt(tokenEnd);
  editor.selection = new vscode.Selection(caret, caret);
  const before = document.getText();
  const versionBefore = document.version;

  currentStep = "snapshot";
  const binding = createRealBinding();
  const adapter = new VsCodeHostAdapter(binding);
  const captured = adapter.captureSnapshot({
    expected_text: renderedToken,
    replacement: replacementToken,
  });
  assert.ok(captured.ok, `snapshot refused: ${captured.ok ? "" : captured.reason}`);
  assert.equal(captured.snapshot.composition, "Unknown");
  // The UTF-16 range belongs to the host snapshot, never to service units.
  assert.equal(captured.snapshot.range.end, captured.snapshot.caret);

  currentStep = "request";
  const canonical = canonicalJson(requestFromSnapshot(captured.snapshot, requestId));
  const result = await client.request(requestId, "Unknown", canonical, 10_000);
  assert.equal(result, "DEFINITE|rejected:CompositionUnknown");

  currentStep = "replay";
  const replay = await client.request(requestId, "Unknown", canonical, 10_000);
  assert.equal(replay, result);

  currentStep = "invariants";
  assert.equal(document.getText(), before);
  assert.equal(document.version, versionBefore);
  client.destroy();
  vscode.window.showInformationMessage("Zonkey M3D-23 LIVE handoff validation passed");
}
