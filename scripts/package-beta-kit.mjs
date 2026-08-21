/**
 * TB-002/TB-003 standalone beta tester kit packaging (dev-side only).
 *
 * TRANSACTIONAL: everything is built/downloaded into a unique staging
 * directory and verified there (pinned VS Code 1.133.0 zip, all required
 * artifacts, manifest, SHA256SUMS, checksum re-verification). Only after
 * every check passes is the staging directory published as the final kit
 * (previous kit is renamed aside and deleted only after the new one is in
 * place; a mid-publish failure rolls back). A failed build/download/
 * verification leaves the previous known-good kit byte-for-byte untouched
 * and cleans the staging directory, exiting with a typed failure:
 *   PACKAGING_DOWNLOAD | PACKAGING_VERIFY | PACKAGING_PUBLISH
 * Network/DNS errors are surfaced verbatim, never hidden.
 *
 * Usage (from the repo, machine with Node): node scripts/package-beta-kit.mjs
 * Test hook: ZONKEY_KIT_VSCODE_URL overrides the pinned archive URL
 * (failure-injection only; never set for a real publish).
 */
import { execFileSync } from "node:child_process";
import { createHash, randomBytes } from "node:crypto";
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  renameSync,
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
const releaseRoot = join(repoRoot, "release");
const finalDir = join(releaseRoot, `zonkey-beta-kit-${kitVersion}`);
const stagingDir = join(
  releaseRoot,
  `.kit-staging-${kitVersion}-${process.pid}-${randomBytes(4).toString("hex")}`,
);
const retiredDir = join(
  releaseRoot,
  `.kit-retired-${kitVersion}-${process.pid}-${randomBytes(4).toString("hex")}`,
);

const zipName = "vscode-1.133.0-win32-x64-archive.zip";
const artifactNames = [
  "zonkey-cli.exe",
  "zonkey-vscode-spike-0.0.1.vsix",
  "m3d37-physical-smoke.cjs",
  zipName,
  "OPERATOR.md",
  "run-beta-smoke.ps1",
];

function typedFail(type, detail) {
  console.error(`PACKAGING_FAIL:${type}`);
  console.error(String(detail));
  process.exitCode = 1;
}

function cleanupStaging() {
  rmSync(stagingDir, { recursive: true, force: true });
}

process.on("exit", () => {
  cleanupStaging();
  rmSync(retiredDir, { recursive: true, force: true });
});
for (const signal of ["SIGINT", "SIGTERM"]) {
  process.on(signal, () => {
    cleanupStaging();
    rmSync(retiredDir, { recursive: true, force: true });
    process.exit(130);
  });
}

function git(args) {
  return execFileSync("git", args, { cwd: repoRoot, encoding: "utf8" }).trim();
}

function sha256(path) {
  return createHash("sha256").update(readFileSync(path)).digest("hex").toUpperCase();
}

// ---- 1. Build/download into the unique staging directory ----------------
let stagingBuilt = false;
try {
  mkdirSync(releaseRoot, { recursive: true });
  mkdirSync(stagingDir);
  stagingBuilt = true;
  console.log(`KIT_STAGING=${stagingDir}`);

  // 1a. Compile the smoke entry from source (same pipeline as compile:m3d37).
  const esbuildBin = join(extensionRoot, "node_modules", "esbuild", "bin", "esbuild");
  execFileSync(
    process.execPath,
    [esbuildBin, "integration/real-vscode-m3d37.entry.ts", "--bundle", "--platform=node", "--format=cjs", "--external:vscode", "--outfile=out/m3d37-physical-smoke.cjs"],
    { cwd: extensionRoot, stdio: "inherit" },
  );

  // 1b. Fetch the pinned VS Code archive zip (failure typed, error verbatim).
  const zipUrl =
    process.env.ZONKEY_KIT_VSCODE_URL ??
    `https://update.code.visualstudio.com/${vscodeVersion}/${vscodePlatform}/stable`;
  console.log(`Downloading pinned VS Code ${vscodeVersion} (${vscodePlatform})...`);
  let zipBytes;
  try {
    const response = await fetch(zipUrl, { redirect: "follow" });
    if (!response.ok) {
      throw new Error(`HTTP ${response.status} for ${zipUrl}`);
    }
    zipBytes = Buffer.from(await response.arrayBuffer());
  } catch (error) {
    typedFail("PACKAGING_DOWNLOAD", error);
    throw error;
  }
  writeFileSync(join(stagingDir, zipName), zipBytes);
  console.log(`Downloaded ${(zipBytes.length / 1e6).toFixed(1)} MB`);

  // 1c. Copy artifacts + runner into staging.
  const sources = new Map([
    ["zonkey-cli.exe", join(releaseDir, "zonkey-cli.exe")],
    ["zonkey-vscode-spike-0.0.1.vsix", join(releaseDir, "zonkey-vscode-spike-0.0.1.vsix")],
    ["m3d37-physical-smoke.cjs", join(extensionRoot, "out", "m3d37-physical-smoke.cjs")],
    ["OPERATOR.md", join(releaseDir, "OPERATOR.md")],
    ["run-beta-smoke.ps1", join(repoRoot, "scripts", "beta-kit", "run-beta-smoke.ps1")],
  ]);
  for (const [name, source] of sources) {
    if (!existsSync(source)) {
      typedFail("PACKAGING_VERIFY", `kit artifact missing: ${source}`);
      throw new Error("verify");
    }
    copyFileSync(source, join(stagingDir, name));
  }

  // 1d. Manifest + checksums inside staging. `git_tracked_changes_clean`
  //     is intentionally a tracked-changes-only semantic (untracked local
  //     files never affect kit provenance); the source commit is recorded
  //     separately.
  const manifest = {
    kit_kind: "standalone-beta-tester",
    kit_version: kitVersion,
    platform: "Windows 11 x64 / x86_64-pc-windows-msvc",
    vscode_version: vscodeVersion,
    vscode_platform: vscodePlatform,
    generated_from_git_commit: git(["rev-parse", "HEAD"]),
    git_tracked_changes_clean: git(["status", "--porcelain", "-uno"]) === "",
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
  writeFileSync(
    join(stagingDir, "release-manifest.json"),
    JSON.stringify(manifest, null, 2) + "\n",
    "utf8",
  );
  const sums = [];
  for (const name of [...artifactNames, "release-manifest.json"]) {
    sums.push(`${sha256(join(stagingDir, name))}  ${name}`);
  }
  writeFileSync(join(stagingDir, "SHA256SUMS.txt"), sums.join("\n") + "\n", "utf8");

  // ---- 2. Verify EVERYTHING inside staging before any publish ----------
  try {
    for (const name of [...artifactNames, "release-manifest.json", "SHA256SUMS.txt"]) {
      if (!existsSync(join(stagingDir, name))) {
        throw new Error(`staging file missing after build: ${name}`);
      }
    }
    if (manifest.vscode_version !== vscodeVersion) {
      throw new Error("manifest pinned VS Code version mismatch");
    }
    for (const name of [...artifactNames, "release-manifest.json"]) {
      const actual = sha256(join(stagingDir, name)).toLowerCase();
      const expected = sums
        .find((line) => line.endsWith(`  ${name}`))
        ?.slice(0, 64)
        .toLowerCase();
      if (actual !== expected) {
        throw new Error(`staging checksum mismatch for ${name}`);
      }
    }
    console.log("KIT_STAGING_VERIFY_OK");
  } catch (error) {
    typedFail("PACKAGING_VERIFY", error);
    throw error;
  }

  // ---- 3. Publish: previous kit survives any failure here ---------------
  try {
    if (existsSync(finalDir)) {
      renameSync(finalDir, retiredDir);
    }
    try {
      renameSync(stagingDir, finalDir);
      stagingBuilt = false; // published; no longer staging
    } catch (error) {
      if (existsSync(retiredDir) && !existsSync(finalDir)) {
        renameSync(retiredDir, finalDir); // roll back to the previous kit
      }
      throw error;
    }
    rmSync(retiredDir, { recursive: true, force: true });
  } catch (error) {
    typedFail("PACKAGING_PUBLISH", error);
    throw error;
  }

  let total = 0;
  for (const name of readdirSync(finalDir)) {
    total += statSync(join(finalDir, name)).size;
  }
  console.log(`KIT_PUBLISHED=${finalDir}`);
  console.log(`KIT_SIZE_MB=${(total / 1e6).toFixed(1)}`);
  console.log(`KIT_COMMIT=${manifest.generated_from_git_commit} tracked_clean=${manifest.git_tracked_changes_clean}`);
  console.log("KIT_OK");
} catch {
  // Typed failure already printed; force a typed fallback when a
  // non-typed path threw, then clean the staging directory.
  if (process.exitCode === 0) {
    typedFail("PACKAGING_VERIFY", "unexpected packaging failure");
  }
  if (stagingBuilt) {
    cleanupStaging();
    stagingBuilt = false;
  }
}
