/**
 * M3D-23 single-command smoke harness.
 *
 * Fixes the manual 3-terminal race where the owner's own keystrokes (needed
 * to launch the E2E step) were observed by the global hook and invalidated
 * the live handoff. This runner pre-starts everything, polls the read-only
 * HANDOFF query until REAL keyboard input produces a live RestoreCandidate,
 * then — with zero further keystrokes — closes the poll connection and
 * automatically runs the real-VS-Code validation. The handoff is never
 * copied or frozen; the E2E query reads the current live handoff.
 *
 * Owner steps: start this script, type `dungf`+Space then `resume`+Space on
 * the physical keyboard, then stop typing. Expected final marker:
 * M3D23_LIVE_SMOKE_OK (live) or M3D23_SMOKE_TOOLING_OK
 * (--scripted-tooling-check, a tooling self-check only, never live evidence).
 *
 * Requires `cargo build -p zonkey-cli` and `npm run compile:handoff-live`.
 */
import { runTests } from "@vscode/test-electron";
import { spawn } from "node:child_process";
import { existsSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import net from "node:net";

const here = dirname(fileURLToPath(import.meta.url));
const extensionRoot = join(here, "..");
const repoRoot = join(extensionRoot, "..", "..");
const cliPath = join(repoRoot, "target", "debug", "zonkey-cli.exe");
const entryPath = join(extensionRoot, "out", "handoff-live-validation.cjs");
const scriptedTooling = process.argv.includes("--scripted-tooling-check");
const label = scriptedTooling ? "SCRIPTED_TOOLING_CHECK" : "LIVE_SMOKE";
const pipe = `\\\\.\\pipe\\zonkey-m3d23-smoke-${Date.now()}`;
const waitSeconds = Number(process.env.ZONKEY_M3D23_WAIT_SECONDS ?? "300");

if (!existsSync(cliPath)) {
  console.error(`${label} missing ${cliPath}; run: cargo build -p zonkey-cli`);
  process.exit(2);
}
if (!existsSync(entryPath)) {
  console.error(`${label} missing ${entryPath}; run: npm run compile:handoff-live`);
  process.exit(2);
}

const serverArgs = scriptedTooling
  ? [
      "serve-host-validation",
      "--pipe",
      pipe,
      "--max-seconds",
      "600",
      "--handoff-token",
      "resume",
    ]
  : ["handoff-live", "--pipe", pipe];

const server = spawn(cliPath, serverArgs, { stdio: ["ignore", "pipe", "pipe"] });
let serverOutput = "";
server.stdout.on("data", (chunk) => {
  serverOutput += chunk.toString("utf8");
});
server.stderr.on("data", (chunk) => {
  serverOutput += chunk.toString("utf8");
});

function frame(payload) {
  const body = Buffer.from(payload, "utf8");
  const head = Buffer.alloc(4);
  head.writeUInt32LE(body.length, 0);
  return Buffer.concat([head, body]);
}

async function connectPipe() {
  const variants = [pipe, pipe.replace(/^\\\\\.\\/, "\\\\?\\")];
  for (const variant of variants) {
    try {
      return await new Promise((resolve, reject) => {
        const socket = net.connect(variant);
        const timer = setTimeout(() => {
          socket.destroy();
          reject(new Error("connect timeout"));
        }, 8000);
        socket.once("connect", () => {
          clearTimeout(timer);
          resolve(socket);
        });
        socket.once("error", (error) => {
          clearTimeout(timer);
          reject(error);
        });
      });
    } catch {
      // Try the next pipe-path form.
    }
  }
  return null;
}

function makeReader(socket) {
  const pending = [];
  socket.on("data", (chunk) => {
    reader.buffer = Buffer.concat([reader.buffer, chunk]);
    while (reader.buffer.length >= 4) {
      const length = reader.buffer.readUInt32LE(0);
      if (reader.buffer.length < 4 + length) break;
      const payload = reader.buffer.subarray(4, 4 + length).toString("utf8");
      reader.buffer = reader.buffer.subarray(4 + length);
      pending.shift()?.(payload);
    }
  });
  socket.on("error", () => {});
  socket.on("close", () => {
    for (const waiter of pending.splice(0)) waiter("__closed__");
  });
  const reader = {
    buffer: Buffer.alloc(0),
    next(timeoutMs) {
      return new Promise((resolve, reject) => {
        const timer = setTimeout(() => {
          const index = pending.indexOf(waiter);
          if (index !== -1) pending.splice(index, 1);
          reject(new Error("read timeout"));
        }, timeoutMs);
        const waiter = (payload) => {
          clearTimeout(timer);
          if (payload === "__closed__") reject(new Error("closed"));
          else resolve(payload);
        };
        pending.push(waiter);
      });
    },
  };
  return reader;
}

try {
  const readyDeadline = Date.now() + 20_000;
  const readyMarker = scriptedTooling ? "endpoint ready" : "observer_thread_started";
  while (!serverOutput.includes(readyMarker) && Date.now() < readyDeadline) {
    await new Promise((resolve) => setTimeout(resolve, 250));
  }
  if (!serverOutput.includes(readyMarker)) {
    throw new Error(`endpoint never became ready: ${serverOutput}`);
  }
  console.log(`${label} endpoint_ready pipe=${pipe} (${scriptedTooling ? "scripted tooling endpoint" : "real WH_KEYBOARD_LL observer"})`);

  const socket = await connectPipe();
  if (socket === null) throw new Error("probe could not connect to the pipe");
  const reader = makeReader(socket);
  socket.write(frame("HELLO|zonkey.host-transport/1"));
  const welcome = await reader.next(8000);
  const session = welcome.slice("WELCOME|".length);
  console.log(`${label} session=${session}`);
  if (!scriptedTooling) {
    console.log(`${label} NOW TYPE ON THE PHYSICAL KEYBOARD (any app):`);
    console.log(`${label}   1) dungf then Space   (negative Keep case)`);
    console.log(`${label}   2) resume then Space  (RestoreCandidate)`);
    console.log(`${label}   3) STOP TYPING — the harness finishes by itself`);
  }

  const deadline = Date.now() + waitSeconds * 1000;
  let polls = 0;
  let handoffPayload = null;
  let sawNegative = false;
  while (handoffPayload === null && Date.now() < deadline) {
    socket.write(frame(`HANDOFF|${session}`));
    let text;
    try {
      const payload = await reader.next(4000);
      text = payload.replace(/^RESULT\|DEFINITE\|/, "");
    } catch (error) {
      throw new Error(`poll failed: ${error.message}`);
    }
    polls += 1;
    if (text.startsWith("handoff:")) {
      handoffPayload = text;
      console.log(`${label} poll=${polls} HANDOFF_OBSERVED ${text}`);
    } else {
      if (text.includes("NoCurrentPlan")) sawNegative = true;
      console.log(`${label} poll=${polls} no-handoff ${text}`);
      await new Promise((resolve) => setTimeout(resolve, 2000));
    }
  }
  // Free the single pipe instance before VS Code connects; no keystrokes
  // are needed for this transition.
  socket.destroy();
  if (handoffPayload === null) {
    throw new Error(
      `no live handoff within ${waitSeconds}s${scriptedTooling ? "" : " (did you type resume then Space?)"}; last server output: ${serverOutput.slice(-300)}`,
    );
  }
  if (!scriptedTooling) {
    console.log(
      `${label} negative_seen=${sawNegative} (type dungf+Space first if false and rerun)`,
    );
  }

  console.log(`${label} starting real VS Code validation automatically`);
  const workspaceDir = mkdtempSync(join(tmpdir(), "zonkey-m3d23-smoke-"));
  try {
    await runTests({
      extensionDevelopmentPath: extensionRoot,
      extensionTestsPath: entryPath,
      launchArgs: [workspaceDir, "--disable-workspace-trust"],
      extensionTestsEnv: { ZONKEY_M3D23_PIPE: pipe },
    });
    console.log(
      scriptedTooling ? "M3D23_SMOKE_TOOLING_OK" : "M3D23_LIVE_SMOKE_OK",
    );
  } finally {
    rmSync(workspaceDir, { recursive: true, force: true });
  }
} finally {
  server.kill();
}
