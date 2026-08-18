import { existsSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("..", import.meta.url));
const vsix = join(root, "zonkey-vscode-spike-0.0.1.vsix");
if (!existsSync(vsix)) {
  throw new Error(`packaged VSIX not found: ${vsix}`);
}

const allowed = new Set([
  "[Content_Types].xml",
  "extension.vsixmanifest",
  "extension/readme.md",
  "extension/package.json",
  "extension/out/extension.cjs",
]);
const entries = execFileSync("tar", ["-tf", vsix], { encoding: "utf8" })
  .split(/\r?\n/)
  .map((entry) => entry.trim())
  .filter(Boolean);
const unexpected = entries.filter((entry) => !allowed.has(entry));
const missing = [...allowed].filter((entry) => !entries.includes(entry));
if (unexpected.length > 0 || missing.length > 0) {
  throw new Error(
    `VSIX allow-list mismatch; unexpected=${JSON.stringify(unexpected)} missing=${JSON.stringify(missing)}`,
  );
}
console.log("VSIX_ALLOWLIST_OK");
