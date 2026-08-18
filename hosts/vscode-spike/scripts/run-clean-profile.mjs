/**
 * M3D-33 clean-profile orchestration: builds the production VSIX, installs
 * it into an isolated VS Code profile (fresh `--user-data-dir` +
 * `--extensions-dir`), and runs the endpoint validation entry against the
 * *installed* extension (no extensionDevelopmentPath). Requires
 * `cargo build --release -p zonkey-cli` beforehand.
 */
import { runTests } from "@vscode/test-electron";
import { execFileSync } from "node:child_process";
import { existsSync, mkdtempSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const extensionRoot = join(here, "..");
const repoRoot = join(extensionRoot, "..", "..");
const cliRelease = join(
  repoRoot,
  "target",
  "x86_64-pc-windows-msvc",
  "release",
  "zonkey-cli.exe",
);
if (!existsSync(cliRelease)) {
  throw new Error(
    `release CLI missing: ${cliRelease} (cargo build --release --target x86_64-pc-windows-msvc -p zonkey-cli)`,
  );
}

// Locate the VS Code CLI from the test-electron download.
function locateCodeCli() {
  const base = join(extensionRoot, ".vscode-test");
  const candidates = existsSync(base) ? readdirSync(base) : [];
  for (const candidate of candidates) {
    if (!candidate.startsWith("vscode-win32-x64-archive-")) {
      continue;
    }
    const cli = join(base, candidate, "bin", "code.cmd");
    if (existsSync(cli)) {
      return cli;
    }
  }
  throw new Error("VS Code CLI not found under .vscode-test (run any test:* once first)");
}

const profileDir = mkdtempSync(join(tmpdir(), "zonkey-m3d33-userdata-"));
const extensionsDir = mkdtempSync(join(tmpdir(), "zonkey-m3d33-exts-"));
const discoveryDir = mkdtempSync(join(tmpdir(), "zonkey-m3d33-discovery-"));
const workspaceDir = mkdtempSync(join(tmpdir(), "zonkey-m3d33-ws-"));
// VS Code 1.133 only dispatches extension tests when a development
// extension is present; use an inert, manifest-only dummy so the only real
// extension remains the installed VSIX.
const dummyDevDir = mkdtempSync(join(tmpdir(), "zonkey-m3d33-devdummy-"));
writeFileSync(
  join(dummyDevDir, "package.json"),
  JSON.stringify(
    {
      name: "zonkey-test-harness-dummy",
      publisher: "zonkey-spike",
      version: "0.0.1",
      engines: { vscode: "^1.90.0" },
    },
    null,
    2,
  ),
  "utf8",
);
const codeCli = locateCodeCli();
const endpointTestsPath = join(extensionRoot, "out", "endpoint-profile-validation.cjs");

try {
  // Package the production VSIX (idempotent; no dependency changes).
  const vsix = join(extensionRoot, "zonkey-vscode-spike-0.0.1.vsix");
  if (!existsSync(vsix)) {
    execFileSync(
      process.platform === "win32" ? "npx.cmd" : "npx",
      ["--yes", "@vscode/vsce", "package", "--no-dependencies"],
      { cwd: extensionRoot, stdio: "inherit" },
    );
  }
  if (!existsSync(vsix)) {
    throw new Error("VSIX was not produced");
  }

  // Install into the isolated profile (cmd /c: Node no longer spawns
  // .cmd batch files directly).
  execFileSync(
    "cmd",
    [
      "/c",
      codeCli,
      `--user-data-dir=${profileDir}`,
      `--extensions-dir=${extensionsDir}`,
      "--install-extension",
      vsix,
    ],
    { stdio: "inherit" },
  );

  // Run the validation entry against the installed extension only (empty
  // development path array: test-electron would otherwise pass a literal
  // "undefined" dev path).
  await runTests({
    extensionDevelopmentPath: dummyDevDir,
    extensionTestsPath: endpointTestsPath,
    extensionTestsEnv: {
      ZONKEY_ENDPOINT_DIR: discoveryDir,
      ZONKEY_CLI_RELEASE: cliRelease,
      // The development environment may set this for VS Code's own Node
      // helper processes. It must not reach the Electron test application,
      // otherwise Code.exe runs as plain Node and treats the workspace path
      // as the test module (`MODULE_NOT_FOUND`).
      ELECTRON_RUN_AS_NODE: undefined,
    },
    launchArgs: [
      workspaceDir,
      "--disable-workspace-trust",
      `--user-data-dir=${profileDir}`,
      `--extensions-dir=${extensionsDir}`,
    ],
  });
  console.log("M3D33_CLEAN_PROFILE_OK");
} finally {
  if (process.env.ZONKEY_KEEP_PROFILE !== "1") {
    rmSync(profileDir, { recursive: true, force: true });
    rmSync(extensionsDir, { recursive: true, force: true });
    rmSync(discoveryDir, { recursive: true, force: true });
    rmSync(workspaceDir, { recursive: true, force: true });
    rmSync(dummyDevDir, { recursive: true, force: true });
  } else {
    console.log(`KEEP profile=${profileDir} exts=${extensionsDir}`);
  }
}
