import { spawnSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const npmDir = path.resolve(here, "..");

const result = spawnSync(
  "npm",
  ["pack", "--dry-run", "--json"],
  { cwd: npmDir, encoding: "utf8" }
);

if (result.status !== 0) {
  console.error(result.stderr || result.stdout);
  process.exit(result.status ?? 1);
}

const stdout = result.stdout.trim();
const jsonStart = stdout.indexOf("[");
if (jsonStart < 0) {
  console.error("npm pack --json did not emit a JSON array");
  console.error(stdout);
  process.exit(1);
}

const payload = JSON.parse(stdout.slice(jsonStart));
const files = new Set(
  (Array.isArray(payload) ? payload[0]?.files : payload?.files)?.map(
    (entry) => entry.path ?? entry
  ) ?? []
);

const required = [
  "package.json",
  "README.md",
  "bin/lexe.mjs",
  "lib/download.mjs",
  "scripts/postinstall.mjs",
];

const missing = required.filter((name) => !files.has(name));
if (missing.length > 0) {
  console.error("npm pack is missing required package files:");
  for (const name of missing) {
    console.error(`  - ${name}`);
  }
  process.exit(1);
}

const forbiddenPrefixes = ["vendor/", "test/"];
const unexpected = [...files].filter((filePath) =>
  forbiddenPrefixes.some((prefix) => filePath.startsWith(prefix))
);
if (unexpected.length > 0) {
  console.error("npm pack unexpectedly includes non-distributable files:");
  for (const filePath of unexpected) {
    console.error(`  - ${filePath}`);
  }
  process.exit(1);
}

console.log("npm pack for lexe includes required CLI wrapper files cleanly");
