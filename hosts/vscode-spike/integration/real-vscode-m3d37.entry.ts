/**
 * M3D-37 one-window physical smoke entry.
 *
 * This entry is loaded by a dummy development extension only so VS Code's
 * test runner can start. The ZonKey extension under test is the packaged VSIX
 * installed into the isolated profile. The endpoint client is the packaged
 * extension's own discovered client; this entry never opens a second pipe
 * client and never feeds scripted keyboard input.
 */
import assert from "node:assert/strict";
import * as vscode from "vscode";
import { parseHandoffPayload } from "../src/handoff.ts";

const EXTENSION_ID = "zonkey-spike.zonkey-vscode-spike";

interface ExtensionApi {
  endpointState: {
    client?: {
      handoffQuery(timeoutMs: number): Promise<string>;
    };
    last: { status: string };
  };
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function waitFor<T>(
  probe: () => T | undefined | Promise<T | undefined>,
  timeoutMs: number,
  label: string,
): Promise<T> {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    const value = await probe();
    if (value !== undefined) return value;
    if (Date.now() >= deadline) throw new Error(`timeout waiting for ${label}`);
    await sleep(200);
  }
}

function handoffFixture(payload: string): { renderedToken: string } {
  const parsed = parseHandoffPayload(payload);
  assert.equal(parsed.kind, "current", "live handoff must be current");
  return { renderedToken: parsed.handoff.renderedToken };
}

function failureKind(error: unknown): string {
  const message = error instanceof Error ? error.message : String(error);
  if (message.includes("live RestorePlanHandoff") || message.includes("timeout waiting")) {
    return "HANDOFF_TIMEOUT";
  }
  if (message.includes("packaged command") || message.includes("not registered")) {
    return "PACKAGED_COMMAND_UNAVAILABLE";
  }
  if (message.includes("document text") || message.includes("document version")) {
    return "DOCUMENT_CHANGED";
  }
  if (message.includes("unexpected packaged command result")) return "UNEXPECTED_RESULT";
  return "RUNNER_FAILURE";
}

async function runInner(): Promise<void> {
  const extension = vscode.extensions.getExtension(EXTENSION_ID);
  assert.ok(extension !== undefined, "packaged VSIX is not installed");
  await waitFor(() => (extension.isActive ? true : undefined), 60_000, "VSIX activation");
  const api = extension.exports as ExtensionApi;
  const commands = await vscode.commands.getCommands(true);
  assert.ok(
    commands.includes("zonkeySpike.checkCurrentHandoff"),
    "packaged command is not registered",
  );

  const folders = vscode.workspace.workspaceFolders;
  assert.ok(folders !== undefined && folders.length === 1, "one smoke workspace is required");
  const inputUri = vscode.Uri.joinPath(folders[0].uri, "m3d37-physical-input.txt");
  await vscode.workspace.fs.writeFile(inputUri, new TextEncoder().encode(""));
  const inputDocument = await vscode.workspace.openTextDocument(inputUri);
  await vscode.window.showTextDocument(inputDocument, { preview: false });

  // Keep the ordinary local file open, but put physical typing in the
  // integrated terminal so the owner input cannot alter the host fixture.
  // The global WH_KEYBOARD_LL observer still sees exactly the physical keys.
  const terminal = vscode.window.createTerminal({ name: "ZonKey physical smoke" });
  terminal.show(false);
  vscode.window.showInformationMessage(
    "ZonKey ready: type dungf + Space, then resume + Space, then stop typing.",
  );

  let noCurrentHandoffObserved = false;
  const handoff = await waitFor(
    async () => {
      if (api.endpointState.last.status !== "connected" || api.endpointState.client === undefined) {
        return undefined;
      }
      const payload = await api.endpointState.client.handoffQuery(10_000);
      const parsed = parseHandoffPayload(payload);
      if (parsed.kind === "none") {
        if (!noCurrentHandoffObserved) {
          noCurrentHandoffObserved = true;
          console.log("M3D37 NO_CURRENT_HANDOFF_OBSERVED");
        }
        return undefined;
      }
      if (parsed.kind === "current") return payload;
      return undefined;
    },
    Number(process.env.ZONKEY_M3D37_WAIT_SECONDS ?? "300") * 1000,
    "live RestorePlanHandoff",
  );
  assert.ok(noCurrentHandoffObserved, "negative/no-current handoff state was not observed");
  terminal.dispose();
  console.log("M3D37 LIVE_HANDOFF_OBSERVED");

  // Build a real local-file host fixture from the currently observed live
  // handoff. The packaged command immediately queries the endpoint again;
  // if the live handoff is stale or invalidated it fails closed.
  const { renderedToken } = handoffFixture(handoff);
  const targetUri = vscode.Uri.joinPath(folders[0].uri, "m3d37-host-target.txt");
  const targetContent = `candidate ${renderedToken} end\n`;
  await vscode.workspace.fs.writeFile(targetUri, new TextEncoder().encode(targetContent));
  const targetDocument = await vscode.workspace.openTextDocument(targetUri);
  const targetEditor = await vscode.window.showTextDocument(targetDocument, { preview: false });
  const targetEnd = targetContent.indexOf(renderedToken) + renderedToken.length;
  const targetCaret = targetDocument.positionAt(targetEnd);
  targetEditor.selection = new vscode.Selection(targetCaret, targetCaret);
  const beforeText = targetDocument.getText();
  const beforeVersion = targetDocument.version;

  const result = await vscode.commands.executeCommand<string>(
    "zonkeySpike.checkCurrentHandoff",
  );
  assert.equal(result, "Rejected(CompositionUnknown)", "unexpected packaged command result");
  assert.equal(targetDocument.getText(), beforeText, "document text changed");
  assert.equal(targetDocument.version, beforeVersion, "document version changed");
  console.log("M3D37_PACKAGED_COMMAND_OK");
  console.log("M3D37_DOCUMENT_UNCHANGED_OK");
  vscode.window.showInformationMessage("ZonKey M3D-37 physical smoke passed");
}

export async function run(): Promise<void> {
  try {
    await runInner();
  } catch (error) {
    console.error(`M3D37_FAILURE kind=${failureKind(error)}`);
    throw error;
  }
}
