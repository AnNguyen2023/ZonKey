/**
 * M3D-23 LIVE runner: expects a running `zonkey-cli handoff-live` endpoint
 * owned by the owner (real keyboard input already typed). Spawns nothing.
 * Requires ZONKEY_M3D23_PIPE in the environment.
 */
import { runTests } from "@vscode/test-electron";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const extensionRoot = join(here, "..");
if (!process.env.ZONKEY_M3D23_PIPE) {
  console.error("ZONKEY_M3D23_PIPE is required (run zonkey-cli handoff-live first)");
  process.exit(2);
}
const workspaceDir = mkdtempSync(join(tmpdir(), "zonkey-m3d23-"));

try {
  await runTests({
    extensionDevelopmentPath: extensionRoot,
    extensionTestsPath: join(extensionRoot, "out", "handoff-live-validation.cjs"),
    launchArgs: [workspaceDir, "--disable-workspace-trust"],
    extensionTestsEnv: { ZONKEY_M3D23_PIPE: process.env.ZONKEY_M3D23_PIPE },
  });
  console.log("M3D23_LIVE_VALIDATION_OK");
} finally {
  rmSync(workspaceDir, { recursive: true, force: true });
}
