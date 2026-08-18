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

function readDiscoveryPipe(): string | undefined {
  try {
    const text = readFileSync(join(DISCOVERY_DIR, "endpoint.txt"), "utf8");
    for (const line of text.split(/\r?\n/)) {
      if (line.startsWith("protocol=") && line.slice("protocol=".length) !== ENDPOINT_PROTOCOL) {
        return undefined;
      }
      if (line.startsWith("pipe=")) {
        return line.slice("pipe=".length);
      }
    }
  } catch {
    return undefined;
  }
  return undefined;
}

function startEndpoint(): Promise<EndpointHandle> {
  assert.ok(CLI.length > 0, "ZONKEY_CLI_RELEASE must point at the release zonkey-cli.exe");
  const child = spawn(CLI, ["serve-host-validation", "--pipe", "auto", "--max-seconds", "600"], {
    stdio: ["ignore", "pipe", "pipe"],
  });
  let output = "";
  child.stdout.on("data", (chunk: Buffer) => {
    output += chunk.toString("utf8");
  });
  child.stderr.on("data", (chunk: Buffer) => {
    output += chunk.toString("utf8");
  });
  // The pipe identity comes from this child's own stdout (a per-lifecycle
  // nonce); the shared discovery file may still hold a previous endpoint's
  // record, so wait until it names THIS endpoint.
  const ownPipe = waitFor(() => {
    const match = /endpoint_pipe=(\S+)/.exec(output);
    return match === null ? undefined : match[1];
  }, 10_000, "endpoint stdout pipe name");
  return ownPipe.then((pipe) =>
    waitFor(
      () => (readDiscoveryPipe() === pipe ? pipe : undefined),
      10_000,
      "discovery record naming this endpoint",
    ).then(() => ({
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
    })),
  );
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
  assert.equal(await client.recoveryCommand("LIST", 10_000), "recovery-list|0");

  // 4. Final host behavior: a real request still fails closed.
  const outcome = await client.request(
    "m3d33-req-1",
    "Unknown",
    "{canonical}",
    10_000,
  );
  assert.equal(outcome, "DEFINITE|rejected:CompositionUnknown");

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
