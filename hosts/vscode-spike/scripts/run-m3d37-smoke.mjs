/**
 * M3D-37 one-window packaged physical smoke runner.
 *
 * Starts the real handoff-live endpoint, installs the production VSIX into a
 * clean profile, and launches exactly one VS Code window. The installed
 * extension owns the only pipe client and polls the live handoff. The owner
 * types only physical keyboard input; no SendInput or scripted feed exists in
 * this path. The test entry invokes the packaged command automatically and
 * cleans all temporary state on exit.
 */
import { runTests } from "@vscode/test-electron";
import { execFileSync, spawn } from "node:child_process";
import {
  existsSync,
  mkdtempSync,
  appendFileSync,
  readdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const extensionRoot = join(here, "..");
const repoRoot = join(extensionRoot, "..", "..");
const cliPath =
  process.env.ZONKEY_M3D37_CLI_PATH ??
  join(repoRoot, "target", "x86_64-pc-windows-msvc", "release", "zonkey-cli.exe");
const vsixPath =
  process.env.ZONKEY_M3D37_VSIX_PATH ??
  join(extensionRoot, "zonkey-vscode-spike-0.0.1.vsix");
const entryPath = join(extensionRoot, "out", "m3d37-physical-smoke.cjs");
const waitSeconds = Number(process.env.ZONKEY_M3D37_WAIT_SECONDS ?? "300");
const transcriptPath = process.env.ZONKEY_PILOT_TRANSCRIPT;

function pilotMarker(stage, details = "") {
  if (transcriptPath === undefined) return;
  const suffix = details.length === 0 ? "" : ` ${details}`;
  appendFileSync(
    transcriptPath,
    `${new Date().toISOString()} stage=${stage}${suffix}\n`,
    "utf8",
  );
}

function pilotFailureRecorded() {
  if (transcriptPath === undefined || !existsSync(transcriptPath)) return false;
  return readFileSync(transcriptPath, "utf8").includes("stage=PILOT_SMOKE_FAIL:");
}

function recordedFailureStage() {
  if (transcriptPath === undefined || !existsSync(transcriptPath)) return undefined;
  const match = readFileSync(transcriptPath, "utf8").match(/stage=PILOT_SMOKE_FAIL:([A-Z_]+)/);
  return match?.[1];
}

if (!existsSync(cliPath)) {
  throw new Error(
    `release CLI missing: ${cliPath} (run cargo build --release -p zonkey-cli --target x86_64-pc-windows-msvc)`,
  );
}
if (!existsSync(vsixPath) || !existsSync(entryPath)) {
  throw new Error("packaged VSIX or smoke entry missing; run npm run smoke:m3d37");
}
function locateCodeCli() {
  const base = join(extensionRoot, ".vscode-test");
  const candidates = existsSync(base) ? readdirSync(base) : [];
  for (const candidate of candidates) {
    if (!candidate.startsWith("vscode-win32-x64-archive-")) continue;
    const cli = join(base, candidate, "bin", "code.cmd");
    if (existsSync(cli)) return cli;
  }
  throw new Error("VS Code CLI not found; run an existing test:* command once first");
}

function readDiscovery(dir) {
  try {
    const fields = new Map();
    for (const line of readFileSync(join(dir, "endpoint.txt"), "utf8").split(/\r?\n/)) {
      if (line.length === 0) continue;
      const separator = line.indexOf("=");
      if (separator > 0) fields.set(line.slice(0, separator), line.slice(separator + 1));
    }
    const pid = Number(fields.get("pid"));
    const pipe = fields.get("pipe");
    if (
      fields.get("protocol") !== "zonkey.host-transport/1" ||
      typeof pipe !== "string" ||
      !Number.isInteger(pid) ||
      pid <= 0
    ) return undefined;
    return { pid, pipe };
  } catch {
    return undefined;
  }
}

async function waitFor(probe, timeoutMs, label) {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    const value = probe();
    if (value !== undefined) return value;
    if (Date.now() >= deadline) throw new Error(`timeout waiting for ${label}`);
    await new Promise((resolve) => setTimeout(resolve, 200));
  }
}

const profileDir = mkdtempSync(join(tmpdir(), "zonkey-m3d37-userdata-"));
const extensionsDir = mkdtempSync(join(tmpdir(), "zonkey-m3d37-exts-"));
const discoveryDir = mkdtempSync(join(tmpdir(), "zonkey-m3d37-discovery-"));
const workspaceDir = mkdtempSync(join(tmpdir(), "zonkey-m3d37-workspace-"));
const dummyDevDir = mkdtempSync(join(tmpdir(), "zonkey-m3d37-devdummy-"));
writeFileSync(
  join(dummyDevDir, "package.json"),
  JSON.stringify({
    name: "zonkey-m3d37-test-runner",
    publisher: "zonkey-spike",
    version: "0.0.1",
    engines: { vscode: "^1.90.0" },
  }),
  "utf8",
);

let endpoint;
let failed = false;

function failureKind(error) {
  const message = error instanceof Error ? error.message : String(error);
  if (message.includes("live endpoint discovery")) return "ENDPOINT_STARTUP";
  if (message.includes("live RestorePlanHandoff")) return "LIVE_HANDOFF_TIMEOUT";
  if (message.includes("packaged command") || message.includes("not registered")) {
    return "PACKAGED_COMMAND";
  }
  if (message.includes("document text") || message.includes("document version")) {
    return "DOCUMENT_CHANGED";
  }
  if (message.includes("unexpected packaged command result")) return "PACKAGED_COMMAND";
  return "OTHER_TYPED_FAILURE";
}
async function stopEndpoint() {
  if (endpoint === undefined) return;
  if (endpoint.pid !== undefined) {
    spawn("taskkill", ["/PID", String(endpoint.pid), "/T", "/F"]);
    await waitFor(() => (endpoint.child.exitCode !== null ? true : undefined), 10_000, "endpoint exit");
  }
  endpoint = undefined;
}

try {
  const codeCli = locateCodeCli();
  execFileSync(
    "cmd",
    ["/c", codeCli, `--user-data-dir=${profileDir}`, `--extensions-dir=${extensionsDir}`, "--install-extension", vsixPath, "--force"],
    { stdio: "inherit" },
  );

  const child = spawn(cliPath, ["handoff-live", "--pipe", "auto"], {
    env: { ...process.env, ZONKEY_ENDPOINT_DIR: discoveryDir },
    stdio: ["ignore", "ignore", "ignore"],
  });
  endpoint = { child, pid: child.pid };
  await waitFor(
    () => {
      const record = readDiscovery(discoveryDir);
      return record !== undefined && record.pid === child.pid ? record : undefined;
    },
    20_000,
    "live endpoint discovery",
  );
  pilotMarker("PILOT_ENDPOINT_STARTED");
  console.log("M3D37 ONE_WINDOW_READY");
  console.log("M3D37 OWNER: type dungf + Space, then resume + Space, then stop typing");

  await runTests({
    extensionDevelopmentPath: dummyDevDir,
    extensionTestsPath: entryPath,
    extensionTestsEnv: {
      ZONKEY_ENDPOINT_DIR: discoveryDir,
      ZONKEY_M3D37_WAIT_SECONDS: String(waitSeconds),
      ZONKEY_PILOT_TRANSCRIPT: transcriptPath,
      ELECTRON_RUN_AS_NODE: undefined,
    },
    launchArgs: [
      workspaceDir,
      "--disable-workspace-trust",
      `--user-data-dir=${profileDir}`,
      `--extensions-dir=${extensionsDir}`,
    ],
  });
  pilotMarker("PILOT_SMOKE_OK");
  pilotMarker("exit_code=0 document_unchanged=true");
  console.log("M3D37_ONE_WINDOW_SMOKE_OK");
} catch (error) {
  failed = true;
  const stage = recordedFailureStage() ?? failureKind(error);
  if (!pilotFailureRecorded()) {
    pilotMarker(`PILOT_SMOKE_FAIL:${stage}`);
    pilotMarker("exit_code=1 document_unchanged=unknown");
  }
  console.error(`PILOT_SMOKE_FAIL:${stage}`);
  console.error(`M3D37_FAILURE kind=${failureKind(error)}`);
} finally {
  pilotMarker("PILOT_TRANSCRIPT_FLUSHED_BEFORE_CLEANUP");
  await stopEndpoint();
  if (process.env.ZONKEY_KEEP_PROFILE !== "1") {
    rmSync(profileDir, { recursive: true, force: true });
    rmSync(extensionsDir, { recursive: true, force: true });
    rmSync(discoveryDir, { recursive: true, force: true });
    rmSync(workspaceDir, { recursive: true, force: true });
    rmSync(dummyDevDir, { recursive: true, force: true });
  } else {
    console.log(`KEEP profile=${profileDir} exts=${extensionsDir} discovery=${discoveryDir}`);
  }
}
if (failed) process.exitCode = 1;
