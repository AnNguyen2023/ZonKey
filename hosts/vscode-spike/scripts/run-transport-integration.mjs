/**
 * Self-contained M3D-21 orchestration: spawns the zonkey-cli host-validation
 * pipe endpoint directly (no shell, so the pipe path is never mangled),
 * runs the real-VS-Code transport integration via @vscode/test-electron with
 * the pipe name delivered through `extensionTestsEnv`, then stops the
 * endpoint. Requires `cargo build -p zonkey-cli` beforehand.
 */
import { runTests } from "@vscode/test-electron";
import { spawn } from "node:child_process";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const extensionRoot = join(here, "..");
const repoRoot = join(extensionRoot, "..", "..");
const cliPath = join(repoRoot, "target", "debug", "zonkey-cli.exe");
const pipeName = `\\\\.\\pipe\\zonkey-m3d21-${Date.now()}`;
const workspaceDir = mkdtempSync(join(tmpdir(), "zonkey-m3d21-"));
writeFileSync(join(workspaceDir, "transport-probe.txt"), "placeholder\n");

const server = spawn(
  cliPath,
  ["serve-host-validation", "--pipe", pipeName, "--max-seconds", "240"],
  { stdio: ["ignore", "pipe", "pipe"] },
);
let serverOutput = "";
server.stdout.on("data", (chunk) => {
  serverOutput += chunk.toString("utf8");
});
server.stderr.on("data", (chunk) => {
  serverOutput += chunk.toString("utf8");
});

try {
  const deadline = Date.now() + 20_000;
  while (!serverOutput.includes("endpoint ready") && Date.now() < deadline) {
    await new Promise((resolve) => setTimeout(resolve, 250));
  }
  if (!serverOutput.includes("endpoint ready")) {
    throw new Error(`validation endpoint never became ready: ${serverOutput}`);
  }
  console.log(`m3d21_endpoint_ready pipe=${pipeName}`);
  await runTests({
    extensionDevelopmentPath: extensionRoot,
    extensionTestsPath: join(extensionRoot, "out", "transport-validation.cjs"),
    launchArgs: [workspaceDir, "--disable-workspace-trust"],
    extensionTestsEnv: { ZONKEY_M3D21_PIPE: pipeName },
  });
  console.log("M3D21_TRANSPORT_VALIDATION_OK");
} finally {
  server.kill();
  rmSync(workspaceDir, { recursive: true, force: true });
}
