import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import {
  cpSync,
  existsSync,
  mkdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const scriptDir = dirname(fileURLToPath(import.meta.url));
const repoRoot = dirname(scriptDir);
const extensionRoot = join(repoRoot, "hosts", "vscode-spike");
const cli = join(
  repoRoot,
  "target",
  "x86_64-pc-windows-msvc",
  "release",
  "zonkey-cli.exe",
);
const vsix = join(extensionRoot, "zonkey-vscode-spike-0.0.1.vsix");
const guide = join(repoRoot, "docs", "beta-query-reject-operator.md");
const packageJsonPath = join(extensionRoot, "package.json");
const output = join(repoRoot, "release", "zonkey-query-reject-beta-0.0.1");
const allowDirty = process.env.ZONKEY_ALLOW_DIRTY_RELEASE === "1";

for (const path of [cli, vsix, guide]) {
  if (!existsSync(path)) {
    throw new Error(`required beta input is missing: ${path}`);
  }
}

const gitStatus = execFileSync("git", ["status", "--porcelain"], {
  cwd: repoRoot,
  encoding: "utf8",
});
if (gitStatus.length > 0 && !allowDirty) {
  throw new Error("refusing release packaging from a dirty worktree; set ZONKEY_ALLOW_DIRTY_RELEASE=1 only for local validation");
}

function sha256(path) {
  return createHash("sha256").update(readFileSync(path)).digest("hex");
}

const packageJson = JSON.parse(readFileSync(packageJsonPath, "utf8"));
const cargoToml = readFileSync(join(repoRoot, "Cargo.toml"), "utf8");
const cliVersion = /^version\s*=\s*"([^"]+)"/m.exec(cargoToml)?.[1];
if (cliVersion === undefined) {
  throw new Error("workspace CLI version is missing");
}
const gitCommit = execFileSync("git", ["rev-parse", "HEAD"], {
  cwd: repoRoot,
  encoding: "utf8",
}).trim();

rmSync(output, { recursive: true, force: true });
mkdirSync(output, { recursive: true });
cpSync(cli, join(output, "zonkey-cli.exe"));
cpSync(vsix, join(output, "zonkey-vscode-spike-0.0.1.vsix"));
cpSync(guide, join(output, "OPERATOR.md"));

const manifest = {
  zonkey_version: packageJson.version,
  git_commit: gitCommit,
  git_worktree_clean: gitStatus.length === 0,
  platform: "Windows 11 x64 / x86_64-pc-windows-msvc",
  cli_version: cliVersion,
  vsix_version: packageJson.version,
  protocol_ids: ["zonkey.host-transport/1", "zonkey.vscode-host/1"],
  recovery_state_format: "recovery-state v2",
  unit_schema_utf16: 2,
  artifacts: {
    cli: {
      file: "zonkey-cli.exe",
      sha256: sha256(cli),
      bytes: readFileSync(cli).length,
    },
    vsix: {
      file: "zonkey-vscode-spike-0.0.1.vsix",
      sha256: sha256(vsix),
      bytes: readFileSync(vsix).length,
    },
  },
};
const manifestPath = join(output, "release-manifest.json");
writeFileSync(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`, "utf8");

const checksummedFiles = [
  "zonkey-cli.exe",
  "zonkey-vscode-spike-0.0.1.vsix",
  "OPERATOR.md",
  "release-manifest.json",
];
const checksums = checksummedFiles
  .map((file) => `${sha256(join(output, file))}  ${file}`)
  .join("\n");
const checksumPath = join(output, "SHA256SUMS.txt");
writeFileSync(checksumPath, `${checksums}\n`, "utf8");

for (const line of checksums.split("\n")) {
  const [expected, file] = line.split("  ");
  if (sha256(join(output, file)) !== expected) {
    throw new Error(`checksum verification failed for ${file}`);
  }
}
console.log(`BETA_PACKAGE_OK ${output}`);
