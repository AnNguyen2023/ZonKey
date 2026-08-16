/**
 * M3D-22 real-VS-Code handoff integration entry, executed inside the real
 * extension host while a real ZonKey validation endpoint (started with
 * `--handoff-token resume`) is listening.
 *
 * Validates the real decision pipeline end to end, query/reject only: the
 * service produces a RestorePlanHandoff through the real Telex/detection/
 * policy path, revalidates it, passes the internal execution gate, and maps
 * it into a handoff request; the extension queries that handoff over the
 * pipe, merges it with its own real VS Code snapshot (the host keeps owning
 * the UTF-16 range), sends the request through the transport, and receives
 * the fail-closed `rejected:CompositionUnknown`. The document never changes.
 */
import * as vscode from "vscode";
import assert from "node:assert/strict";
import { VsCodeHostAdapter, requestFromSnapshot } from "../src/adapter.ts";
import { canonicalJson } from "../src/contract.ts";
import { NamedPipeClient } from "../src/pipe-client.ts";
import { createRealBinding } from "../src/vscode-binding.ts";

interface HandoffAnswer {
  requestId: string;
  renderedToken: string;
  replacementToken: string;
  renderedUnits: number;
  replacementUnits: number;
  generation: number;
}

let currentStep = "start";

export async function run(): Promise<void> {
  try {
    await runInner();
  } catch (error) {
    console.error(
      `m3d22_failure step=${currentStep} ${JSON.stringify(error, [
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

function parseHandoff(payload: string): HandoffAnswer {
  assert.ok(payload.startsWith("handoff:"), `not a handoff payload: ${payload}`);
  const parts = payload.slice("handoff:".length).split("|");
  assert.equal(parts.length, 6, "handoff payload has six fields");
  return {
    requestId: parts[0],
    renderedToken: parts[1],
    replacementToken: parts[2],
    renderedUnits: Number(parts[3]),
    replacementUnits: Number(parts[4]),
    generation: Number(parts[5]),
  };
}

async function runInner(): Promise<void> {
  const pipeName = process.env.ZONKEY_M3D22_PIPE ?? "";
  assert.ok(pipeName.length > 0, "ZONKEY_M3D22_PIPE is required");

  currentStep = "activate";
  const extension = vscode.extensions.getExtension(
    "zonkey-spike.zonkey-vscode-spike",
  );
  assert.ok(extension !== undefined, "extension is not present");
  await extension.activate();
  assert.ok(extension.isActive, "extension did not activate");

  currentStep = "editor";
  const folders = vscode.workspace.workspaceFolders;
  assert.ok(folders !== undefined && folders.length === 1, "no workspace folder");
  const fileUri = vscode.Uri.joinPath(folders[0].uri, "handoff-probe.txt");

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
  const handoff = parseHandoff(handoffPayload);
  assert.ok(handoff.requestId.startsWith("handoff-"), "request id from generation");
  assert.equal(handoff.generation, 1);
  assert.ok(handoff.renderedToken.length > 0, "rendered token present");
  assert.ok(handoff.replacementToken.length > 0, "replacement token present");
  assert.equal(
    handoff.renderedUnits,
    [...handoff.renderedToken].length,
    "rendered units are scalar counts",
  );

  // The editor content embeds the rendered token; the host owns the range.
  currentStep = "editor_with_token";
  const content = `hello ${handoff.renderedToken} end\n`;
  await vscode.workspace.fs.writeFile(fileUri, new TextEncoder().encode(content));
  const document = await vscode.workspace.openTextDocument(fileUri);
  const editor = await vscode.window.showTextDocument(document, { preview: false });
  const tokenEnd = content.indexOf(handoff.renderedToken) + handoff.renderedToken.length;
  const caret = document.positionAt(tokenEnd);
  editor.selection = new vscode.Selection(caret, caret);
  const before = document.getText();
  const versionBefore = document.version;

  currentStep = "snapshot";
  const binding = createRealBinding();
  const adapter = new VsCodeHostAdapter(binding);
  const captured = adapter.captureSnapshot({
    expected_text: handoff.renderedToken,
    replacement: handoff.replacementToken,
  });
  assert.ok(captured.ok, `snapshot refused: ${captured.ok ? "" : captured.reason}`);
  assert.equal(captured.snapshot.composition, "Unknown");
  assert.equal(captured.snapshot.expected_text, handoff.renderedToken);
  // The UTF-16 range comes from the host snapshot, never from service units.
  assert.equal(captured.snapshot.range.end, captured.snapshot.caret);

  currentStep = "request";
  const canonical = canonicalJson(
    requestFromSnapshot(captured.snapshot, handoff.requestId),
  );
  const result = await client.request(
    handoff.requestId,
    "Unknown",
    canonical,
    10_000,
  );
  assert.equal(result, "DEFINITE|rejected:CompositionUnknown");

  currentStep = "replay";
  const replay = await client.request(
    handoff.requestId,
    "Unknown",
    canonical,
    10_000,
  );
  assert.equal(replay, result);

  currentStep = "invariants";
  assert.equal(document.getText(), before);
  assert.equal(document.version, versionBefore);
  client.destroy();
  vscode.window.showInformationMessage("Zonkey M3D-22 handoff validation passed");
}
