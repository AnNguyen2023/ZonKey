/**
 * M3D-28 real-VS-Code recovery E2E (query/readback only). Connects to a
 * running validation endpoint, exercises the operator recovery lifecycle
 * over the real pipe against a real editor snapshot: block a logical target
 * derived from a real document, reject ack-before-reconcile, reconcile with
 * a contradictory readback (ConflictHumanReview) and with the exact host
 * readback (NotApplied), acknowledge to unblock, and verify the document's
 * text and version never change. No mutation, no Applied path.
 */
import * as vscode from "vscode";
import assert from "node:assert/strict";
import { VsCodeHostAdapter } from "../src/adapter.ts";
import { NamedPipeClient } from "../src/pipe-client.ts";
import { createRealBinding } from "../src/vscode-binding.ts";

const SENTINEL = "zonkey-spike-target";

let currentStep = "start";

export async function run(): Promise<void> {
  try {
    await runInner();
  } catch (error) {
    console.error(
      `m3d28_failure step=${currentStep} ${JSON.stringify(error, [
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
  const pipeName = process.env.ZONKEY_M3D28_PIPE ?? "";
  assert.ok(pipeName.length > 0, "ZONKEY_M3D28_PIPE is required");

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
  const fileUri = vscode.Uri.joinPath(folders[0].uri, "recovery-probe.txt");
  const content = `hello ${SENTINEL} end\n`;
  await vscode.workspace.fs.writeFile(fileUri, new TextEncoder().encode(content));
  const document = await vscode.workspace.openTextDocument(fileUri);
  const editor = await vscode.window.showTextDocument(document, { preview: false });
  const sentinelEnd = content.indexOf(SENTINEL) + SENTINEL.length;
  const caret = document.positionAt(sentinelEnd);
  editor.selection = new vscode.Selection(caret, caret);
  const before = document.getText();
  const versionBefore = document.version;

  // A real host snapshot provides the logical-target fields and the live
  // readback text; the host keeps owning the UTF-16 range.
  const binding = createRealBinding();
  const adapter = new VsCodeHostAdapter(binding);
  const captured = adapter.captureSnapshot({
    expected_text: SENTINEL,
    replacement: "zonkey-spike-applied",
  });
  assert.ok(captured.ok, `snapshot refused: ${captured.ok ? "" : captured.reason}`);
  assert.equal(captured.snapshot.composition, "Unknown");
  const uri = captured.snapshot.identity.document_uri;
  const liveReadback = captured.snapshot.expected_text;
  const rangeStart = captured.snapshot.range.start;
  const rangeEnd = captured.snapshot.range.end;

  currentStep = "connect";
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

  currentStep = "recovery_lifecycle";
  // Empty list.
  assert.equal(await client.recoveryCommand("LIST", 10_000), "recovery-list|0");
  // Block the real logical target.
  assert.equal(
    await client.recoveryCommand(
      `BLOCK|${uri}|${SENTINEL}|zonkey-spike-applied|${rangeStart}|${rangeEnd}`,
      10_000,
    ),
    "recovery-blocked",
  );
  const listed = await client.recoveryCommand("LIST", 10_000);
  assert.ok(listed.startsWith("recovery-list|1"), `list=${listed}`);
  assert.ok(listed.includes(SENTINEL), "listed target carries the real token");
  // Ack before reconciliation is rejected.
  assert.equal(
    await client.recoveryCommand(`ACK|${uri}|${SENTINEL}`, 10_000),
    "recovery-error:AckBeforeReconcile",
  );
  // Contradictory readback: conflict for human review, then ack to unblock.
  assert.equal(
    await client.recoveryCommand(`RECONCILE|${uri}|${SENTINEL}|mangled!`, 10_000),
    "recovery-verdict:ConflictHumanReview",
  );
  assert.equal(await client.recoveryCommand(`ACK|${uri}|${SENTINEL}`, 10_000), "recovery-acked");
  assert.equal(await client.recoveryCommand("LIST", 10_000), "recovery-list|0");

  // Second cycle: the exact host readback equals the original rendered text,
  // so reconciliation proves nothing was applied.
  assert.equal(
    await client.recoveryCommand(
      `BLOCK|${uri}|${SENTINEL}|zonkey-spike-applied|${rangeStart}|${rangeEnd}`,
      10_000,
    ),
    "recovery-blocked",
  );
  assert.equal(
    await client.recoveryCommand(`RECONCILE|${uri}|${SENTINEL}|${liveReadback}`, 10_000),
    "recovery-verdict:NotApplied",
  );
  assert.equal(await client.recoveryCommand(`ACK|${uri}|${SENTINEL}`, 10_000), "recovery-acked");
  assert.equal(await client.recoveryCommand("LIST", 10_000), "recovery-list|0");

  currentStep = "invariants";
  assert.equal(document.getText(), before);
  assert.equal(document.version, versionBefore);
  client.destroy();
  vscode.window.showInformationMessage("Zonkey M3D-28 recovery validation passed");
}
