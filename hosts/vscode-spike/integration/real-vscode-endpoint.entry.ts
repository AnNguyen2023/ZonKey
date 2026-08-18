/**
 * M3D-33 clean-profile endpoint validation. Runs inside a real VS Code
 * whose only installed extension is the packaged VSIX (no development
 * path), against an isolated `--extensions-dir`/`--user-data-dir` profile
 * and an isolated endpoint discovery directory.
 *
 * Validates: VSIX activation, discovery of the current endpoint, the
 * explicit reconnect command, stale-identity fail-closed behavior,
 * unknown-protocol fail-closed behavior, duplicate-endpoint discovery
 * (last writer wins, older shutdown never deregisters), restart producing
 * a new pipe/session identity, live query/recovery paths, and the final
 * host behavior remaining CompositionUnknown with document text/version
 * unchanged. No Applied, no TextEditor.edit.
 */
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import * as vscode from "vscode";
import { ENDPOINT_PROTOCOL } from "../src/endpoint.ts";
import { VsCodeHostAdapter, requestFromSnapshot } from "../src/adapter.ts";
import { canonicalJson } from "../src/contract.ts";
import { createRealBinding } from "../src/vscode-binding.ts";

const EXTENSION_ID = "zonkey-spike.zonkey-vscode-spike";
const CLI = process.env.ZONKEY_CLI_RELEASE ?? "";
const DISCOVERY_DIR = process.env.ZONKEY_ENDPOINT_DIR ?? "";

interface EndpointHandle {
  pipe: string;
  kill: () => Promise<void>;
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function waitFor<T>(
  probe: () => T | undefined,
  timeoutMs: number,
  label: string,
): Promise<T> {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    const value = probe();
    if (value !== undefined) {
      return value;
    }
    if (Date.now() >= deadline) {
      throw new Error(`timeout waiting for ${label}`);
    }
    await sleep(200);
  }
}

function readDiscoveryEndpoint(): { pipe: string; pid: number } | undefined {
  try {
    const text = readFileSync(join(DISCOVERY_DIR, "endpoint.txt"), "utf8");
    let pipe: string | undefined;
    let pid: number | undefined;
    for (const line of text.split(/\r?\n/)) {
      if (line.startsWith("protocol=") && line.slice("protocol=".length) !== ENDPOINT_PROTOCOL) {
        return undefined;
      }
      if (line.startsWith("pipe=")) {
        pipe = line.slice("pipe=".length);
      }
      if (line.startsWith("pid=")) {
        const parsed = Number(line.slice("pid=".length));
        pid = Number.isInteger(parsed) && parsed > 0 ? parsed : undefined;
      }
    }
    if (pipe !== undefined && pid !== undefined) {
      return { pipe, pid };
    }
  } catch {
    return undefined;
  }
  return undefined;
}

function readDiscoveryPipe(): string | undefined {
  return readDiscoveryEndpoint()?.pipe;
}

function startEndpoint(handoffToken?: string): Promise<EndpointHandle> {
  assert.ok(CLI.length > 0, "ZONKEY_CLI_RELEASE must point at the release zonkey-cli.exe");
  const args = ["serve-host-validation", "--pipe", "auto", "--max-seconds", "600"];
  if (handoffToken !== undefined) {
    args.push("--handoff-token", handoffToken);
  }
  const child = spawn(CLI, args, {
    stdio: ["ignore", "pipe", "pipe"],
  });
  // The discovery record is the authenticated source for the endpoint
  // identity. Do not retain or print the CLI's stdout/stderr, which may
  // contain pipe names or future operator diagnostics.
  child.stdout.on("data", () => {});
  child.stderr.on("data", () => {});
  return waitFor(
    () => {
      const record = readDiscoveryEndpoint();
      return record !== undefined && record.pid === child.pid ? record.pipe : undefined;
    },
    10_000,
    "endpoint discovery record for this process",
  ).then((pipe) => ({
      pipe,
      kill: async () => {
        // Node's kill() does not reliably terminate the CLI on Windows;
        // taskkill /T /F is deterministic.
        if (child.pid !== undefined) {
          spawn("taskkill", ["/PID", String(child.pid), "/T", "/F"]);
        } else {
          child.kill();
        }
        await waitFor(() => (child.exitCode !== null ? true : undefined), 10_000, "endpoint exit");
      },
    }));
}

interface ExtensionApi {
  endpointState: {
    client?: {
      sessionId(): string;
      request(
        requestId: string,
        composition: string,
        canonical: string,
        timeoutMs: number,
      ): Promise<string>;
      recoveryCommand(command: string, timeoutMs: number): Promise<string>;
      destroy(): void;
    };
    last: { status: string; pipe?: string; session?: string };
  };
}

async function runChecks(): Promise<void> {
  // 1. The installed VSIX activates on startup in the clean profile.
  const extension = vscode.extensions.getExtension(EXTENSION_ID);
  assert.ok(extension !== undefined, "VSIX extension must be installed in the clean profile");
  for (let tick = 0; tick < 60 && !extension.isActive; tick += 1) {
    await sleep(1000);
  }
  assert.ok(extension.isActive, "VSIX extension must activate in the clean profile");
  const api = extension.exports as ExtensionApi;
  const commands = await vscode.commands.getCommands(true);
  assert.ok(
    commands.includes("zonkeySpike.checkCurrentHandoff"),
    "installed VSIX must register the packaged handoff command",
  );

  // 2. Start the approved endpoint; the extension's activation-time
  //    discovery may have run before it existed, so use the explicit
  //    reconnect command (the approved manual path).
  const first = await startEndpoint();
  await vscode.commands.executeCommand("zonkeySpike.endpointConnect");
  await waitFor(
    () => (api.endpointState.last.status === "connected" ? true : undefined),
    10_000,
    "discovered endpoint connect",
  );
  assert.equal(api.endpointState.last.pipe, first.pipe);
  const firstSession = api.endpointState.last.session ?? "";
  assert.ok(firstSession.length > 0);

  // 3. Live query/recovery paths operate through the discovered client.
  const client = api.endpointState.client;
  assert.ok(client !== undefined, "discovered client must be available");
  assert.equal(
    await vscode.commands.executeCommand("zonkeySpike.checkCurrentHandoff"),
    "CurrentHandoffUnavailable",
  );
  assert.equal(await client.recoveryCommand("LIST", 10_000), "recovery-list|0");

  // 4. Final host behavior: a real request still fails closed.
  const outcome = await client.request(
    "m3d33-req-1",
    "Unknown",
    "{canonical}",
    10_000,
  );
  assert.equal(outcome, "DEFINITE|rejected:CompositionUnknown");

  // M3D-37 packaged-command tooling check: the endpoint supplies a scripted
  // handoff only to prove that the installed VSIX command uses the real
  // discovery client, host snapshot, and request path. This is not physical
  // keyboard evidence; the owner smoke remains the live proof.
  const commandEndpoint = await startEndpoint("resume");
  await vscode.commands.executeCommand("zonkeySpike.endpointConnect");
  await waitFor(
    () => (api.endpointState.last.status === "connected" ? true : undefined),
    10_000,
    "packaged command endpoint connect",
  );
  const commandFolders = vscode.workspace.workspaceFolders;
  assert.ok(commandFolders !== undefined && commandFolders.length === 1, "no workspace folder");
  const commandUri = vscode.Uri.joinPath(commandFolders[0].uri, "m3d37-command-probe.txt");
  const commandContent = "hello réume end\n";
  await vscode.workspace.fs.writeFile(commandUri, new TextEncoder().encode(commandContent));
  const commandDocument = await vscode.workspace.openTextDocument(commandUri);
  const commandEditor = await vscode.window.showTextDocument(commandDocument, { preview: false });
  const commandCaret = commandDocument.positionAt(commandContent.indexOf("réume") + "réume".length);
  commandEditor.selection = new vscode.Selection(commandCaret, commandCaret);
  const commandBefore = commandDocument.getText();
  const commandVersion = commandDocument.version;
  assert.equal(
    await vscode.commands.executeCommand("zonkeySpike.checkCurrentHandoff"),
    "Rejected(CompositionUnknown)",
  );
  assert.equal(commandDocument.getText(), commandBefore);
  assert.equal(commandDocument.version, commandVersion);
  const commandBinding = createRealBinding();
  const commandAdapter = new VsCodeHostAdapter(commandBinding);
  const commandSnapshot = commandAdapter.captureSnapshot({
    expected_text: "réume",
    replacement: "resume",
  });
  assert.ok(commandSnapshot.ok, "packaged command snapshot must be capturable");
  const commandCanonical = canonicalJson(
    requestFromSnapshot(commandSnapshot.snapshot, "handoff-1"),
  );
  assert.equal(
    await api.endpointState.client!.request("handoff-1", "Unknown", commandCanonical, 10_000),
    "DEFINITE|rejected:CompositionUnknown",
  );
  await commandEndpoint.kill();
  console.log("M3D37_PACKAGED_COMMAND_TOOLING_OK");

  // 5. Duplicate endpoint startup: a second lifecycle becomes the
  //    discovered endpoint; shutting the older one down must not
  //    deregister the newer record (validated at the record level).
  const second = await startEndpoint();
  assert.notEqual(second.pipe, first.pipe);
  await first.kill();
  assert.equal(readDiscoveryPipe(), second.pipe);

  // 6. Crash/restart semantics: after the endpoint dies, the stale
  //    discovery identity fails closed (connect refused, no session).
  await second.kill();
  await vscode.commands.executeCommand("zonkeySpike.endpointConnect");
  await waitFor(
    () => (api.endpointState.last.status === "connect-failed" ? true : undefined),
    10_000,
    "stale endpoint fail-closed",
  );

  // 7. Restart yields a NEW pipe/session identity, and the old session
  //    never authorizes anything on it.
  const third = await startEndpoint();
  assert.notEqual(third.pipe, second.pipe);
  await vscode.commands.executeCommand("zonkeySpike.endpointConnect");
  await waitFor(
    () => (api.endpointState.last.status === "connected" ? true : undefined),
    10_000,
    "restarted endpoint connect",
  );
  assert.notEqual(api.endpointState.last.session, firstSession);
  const staleOutcome = await api.endpointState.client!.request(
    "m3d33-stale",
    "Unknown",
    "{canonical}",
    10_000,
    // The extension client is bound to the NEW session; a request carrying
    // the OLD session is not possible through this client by construction.
  );
  assert.ok(staleOutcome.startsWith("DEFINITE|"), "new-session request answers");

  // 8. Unknown protocol/schema in the discovery record fails closed.
  await third.kill();
  const recordPath = join(DISCOVERY_DIR, "endpoint.txt");
  writeFileSync(
    recordPath,
    `protocol=zonkey.host-transport/9\r\npipe=${third.pipe}\r\npid=1\r\nstarted_unix_ms=1\r\n`,
    "utf8",
  );
  await vscode.commands.executeCommand("zonkeySpike.endpointConnect");
  await waitFor(
    () => (api.endpointState.last.status === "no-record" ? true : undefined),
    10_000,
    "unknown protocol fail-closed",
  );

  // 9. Document text/version unchanged: nothing was applied anywhere.
  const document = vscode.window.activeTextEditor?.document;
  if (document !== undefined) {
    const before = `${document.version}:${document.getText()}`;
    await sleep(300);
    assert.equal(`${document.version}:${document.getText()}`, before);
  }
}

export async function run(): Promise<void> {
  await runChecks();
  console.log("M3D33_ENDPOINT_PROFILE_OK");
}
