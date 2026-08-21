/**
 * TB-002 standalone beta tester kit packaging (dev-side only).
 *
 * Assembles a self-contained kit from the repo: the release CLI, the
 * packaged VSIX, the OPERATOR doc, a freshly compiled
 * `m3d37-physical-smoke.cjs` (same esbuild pipeline as
 * `npm run compile:m3d37` — reproducible from source), the pinned VS Code
 * 1.133.0 win32-x64 archive zip (downloaded once here; the tester machine
 * never needs internet), the kit runner, plus release-manifest.json and
 * SHA256SUMS.txt covering every artifact.
 *
 * Usage (from the repo, machine with Node): node scripts/package-beta-kit.mjs
 */
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = join(here, "..");
const extensionRoot = join(repoRoot, "hosts", "vscode-spike");
const releaseDir = join(repoRoot, "release", "zonkey-query-reject-beta-0.0.1");
const kitVersion = "0.1.0";
const vscodeVersion = "1.133.0";
const vscodePlatform = "win32-x64-archive";
const outDir = join(repoRoot, "release", `zonkey-beta-kit-${kitVersion}`);

const files = {
  "zonkey-cli.exe": join(releaseDir, "zonkey-cli.exe"),
  "zonkey-vscode-spike-0.0.1.vsix": join(releaseDir, "zonkey-vscode-spike-0.0.1.vsix"),
  "m3d37-physical-smoke.cjs": join(extensionRoot, "out", "m3d37-physical-smoke.cjs"),
  "vscode-1.133.0-win32-x64-archive.zip": null,
  "OPERATOR.md": join(releaseDir, "OPERATOR.md"),
  "run-beta-smoke.ps1": join(repoRoot, "scripts", "beta-kit", "run-beta-smoke.ps1"),
};

function git(args) {
  return execFileSync("git", args, { cwd: repoRoot, encoding: "utf8" }).trim();
}

function sha256(path) {
  return createHash("sha256").update(readFileSync(path)).digest("hex").toUpperCase();
}

const gitCommit = git(["rev-parse", "HEAD"]);
const gitClean = git(["status", "--porcelain", "-uno"]) === "";

rmSync(outDir, { recursive: true, force: true });
mkdirSync(outDir, { recursive: true });

// 1. Compile the smoke entry from source (same pipeline as compile:m3d37).
const esbuildBin = join(extensionRoot, "node_modules", "esbuild", "bin", "esbuild");
execFileSync(
  process.execPath,
  [esbuildBin, "integration/real-vscode-m3d37.entry.ts", "--bundle", "--platform=node", "--format=cjs", "--external:vscode", "--outfile=out/m3d37-physical-smoke.cjs"],
  { cwd: extensionRoot, stdio: "inherit" },
);

// 2. Fetch the pinned VS Code archive zip (once, on the packaging machine).
const zipName = "vscode-1.133.0-win32-x64-archive.zip";
const zipPath = join(outDir, zipName);
const zipUrl = `https://update.code.visualstudio.com/${vscodeVersion}/${vscodePlatform}/stable`;
console.log(`Downloading pinned VS Code ${vscodeVersion} (${vscodePlatform})...`);
const response = await fetch(zipUrl, { redirect: "follow" });
if (!response.ok) {
  throw new Error(`VS Code archive download failed: HTTP ${response.status}`);
}
const zipBytes = Buffer.from(await response.arrayBuffer());
writeFileSync(zipPath, zipBytes);
console.log(`Downloaded ${(zipBytes.length / 1e6).toFixed(1)} MB`);

// 3. Copy artifacts + runner.
for (const [name, source] of Object.entries(files)) {
  if (source === null) continue;
  if (!existsSync(source)) {
    throw new Error(`kit artifact missing: ${source}`);
  }
  copyFileSync(source, join(outDir, name));
}

// 4. Manifest + checksums (SHA256SUMS covers every file except itself).
const manifest = {
  kit_kind: "standalone-beta-tester",
  kit_version: kitVersion,
  platform: "Windows 11 x64 / x86_64-pc-windows-msvc",
  vscode_version: vscodeVersion,
  vscode_platform: vscodePlatform,
  generated_from_git_commit: gitCommit,
  git_worktree_clean: gitClean,
  entry: "m3d37-physical-smoke.cjs",
  expected_result: "Rejected(CompositionUnknown)",
  safety: {
    applied: "disabled",
    mutation: "none",
    sendinput: "none",
    clipboard: "none",
    composition_gate: "CompositionUnknown fail-closed",
  },
};
writeFileSync(join(outDir, "release-manifest.json"), JSON.stringify(manifest, null, 2) + "\n", "utf8");

const sums = [];
for (const name of readdirSync(outDir)) {
  if (name === "SHA256SUMS.txt") continue;
  sums.push(`${sha256(join(outDir, name))}  ${name}`);
}
writeFileSync(join(outDir, "SHA256SUMS.txt"), sums.join("\n") + "\n", "utf8");

let total = 0;
for (const name of readdirSync(outDir)) {
  total += statSync(join(outDir, name)).size;
}
console.log(`KIT_OK dir=${outDir}`);
console.log(`KIT_SIZE_MB=${(total / 1e6).toFixed(1)}`);
console.log(`KIT_COMMIT=${gitCommit} clean=${gitClean}`);
