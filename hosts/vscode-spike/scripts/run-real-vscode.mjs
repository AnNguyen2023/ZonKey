/**
 * Launches real VS Code desktop via @vscode/test-electron, loads the spike
 * extension from this folder, and runs the in-host validation entry. Fails
 * with a non-zero exit code if any validation step rejects.
 */
import { runTests } from "@vscode/test-electron";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const extensionRoot = join(here, "..");
const workspaceDir = mkdtempSync(join(tmpdir(), "zonkey-spike-"));
writeFileSync(join(workspaceDir, "probe-target.txt"), "placeholder\n");

try {
  await runTests({
    extensionDevelopmentPath: extensionRoot,
    extensionTestsPath: join(extensionRoot, "out", "real-vscode-validation.cjs"),
    launchArgs: [workspaceDir, "--disable-workspace-trust"],
  });
  console.log("REAL_VSCODE_VALIDATION_OK");
} finally {
  rmSync(workspaceDir, { recursive: true, force: true });
}
