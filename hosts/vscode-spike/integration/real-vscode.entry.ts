/**
 * Real-VS-Code validation entry for M3D-17, executed inside the real
 * extension host by @vscode/test-electron. Validates: extension activation,
 * the `zonkeySpike.probe` command, evidence-snapshot mapping on a real local
 * file editor, the fail-closed `CompositionUnknown` result, unchanged text,
 * and a simple non-empty-selection rejection. Read-only expectations: no
 * mutation may occur in any step.
 */
import * as vscode from "vscode";
import assert from "node:assert/strict";
import { VsCodeHostAdapter, requestFromSnapshot } from "../src/adapter.ts";
import { createRealBinding } from "../src/vscode-binding.ts";

const SENTINEL = "zonkey-spike-target";
const REPLACEMENT = "zonkey-spike-applied";

export async function run(): Promise<void> {
  // 1. Extension activates.
  const extension = vscode.extensions.getExtension(
    "zonkey-spike.zonkey-vscode-spike",
  );
  assert.ok(extension !== undefined, "extension is not present");
  await extension.activate();
  assert.ok(extension.isActive, "extension did not activate");

  // One ordinary local file, one editor, one caret, no selection.
  const folders = vscode.workspace.workspaceFolders;
  assert.ok(folders !== undefined && folders.length === 1, "no workspace folder");
  const fileUri = vscode.Uri.joinPath(folders[0].uri, "probe-target.txt");
  const content = `hello ${SENTINEL} world\n`;
  await vscode.workspace.fs.writeFile(fileUri, new TextEncoder().encode(content));
  const document = await vscode.workspace.openTextDocument(fileUri);
  const editor = await vscode.window.showTextDocument(document, {
    preview: false,
  });
  const sentinelEnd = content.indexOf(SENTINEL) + SENTINEL.length;
  const caret = document.positionAt(sentinelEnd);
  editor.selection = new vscode.Selection(caret, caret);

  const before = document.getText();
  const versionBefore = document.version;

  // 2-4. Real binding evidence snapshot mapping.
  const binding = createRealBinding();
  const adapter = new VsCodeHostAdapter(binding);
  const captured = adapter.captureSnapshot({
    expected_text: SENTINEL,
    replacement: REPLACEMENT,
  });
  assert.ok(captured.ok, `snapshot refused: ${captured.ok ? "" : captured.reason}`);
  const snapshot = captured.snapshot;
  assert.equal(snapshot.protocol_id, "zonkey.vscode-host/1");
  assert.ok(snapshot.identity.document_uri.startsWith("file:///"));
  assert.equal(snapshot.identity.document_epoch, 1);
  assert.ok(snapshot.identity.editor_id >= 1);
  assert.equal(snapshot.revision, versionBefore);
  assert.deepEqual(snapshot.range, {
    unit_schema: 2,
    start: sentinelEnd - SENTINEL.length,
    end: sentinelEnd,
  });
  assert.equal(snapshot.expected_text, SENTINEL);
  assert.equal(snapshot.caret, sentinelEnd);
  assert.deepEqual(snapshot.selection, { kind: "Empty" });
  assert.equal(snapshot.secure, "KnownNonSecure");
  assert.equal(snapshot.session, "SupportedLocal");
  assert.equal(snapshot.composition, "Unknown");

  // 5. Real host result must be fail-closed CompositionUnknown.
  const result = await adapter.apply(
    requestFromSnapshot(snapshot, "real-vscode-validation-1"),
  );
  assert.deepEqual(result, { kind: "Rejected", reason: "CompositionUnknown" });

  // 6. Document text must remain unchanged.
  assert.equal(document.getText(), before);
  assert.equal(document.version, versionBefore);

  // 2. The probe command runs against the same editor without mutation.
  await vscode.commands.executeCommand("zonkeySpike.probe");
  assert.equal(document.getText(), before);
  assert.equal(document.version, versionBefore);

  // Simple rejection: non-empty selection refuses before any edit.
  const anchor = document.positionAt(content.indexOf(SENTINEL));
  editor.selection = new vscode.Selection(anchor, caret);
  const refused = adapter.captureSnapshot({
    expected_text: SENTINEL,
    replacement: REPLACEMENT,
  });
  assert.deepEqual(refused, { ok: false, reason: "SelectionNotEmpty" });
  await vscode.commands.executeCommand("zonkeySpike.probe");
  assert.equal(document.getText(), before);
  assert.equal(document.version, versionBefore);

  vscode.window.showInformationMessage("Zonkey spike: real VS Code validation passed");
}
