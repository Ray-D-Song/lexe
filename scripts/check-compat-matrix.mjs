#!/usr/bin/env node
import fs from "node:fs";
import path from "node:path";

const ROOT = process.cwd();
const COMPAT_DOC = path.join(ROOT, "docs/compatibility.md");
const ALLOWLIST_PATH = path.join(ROOT, "compat/api-surface/allowlist.json");
const PROBE_PATH = path.join(ROOT, "compat/api-surface/probe.mjs");

const compatText = fs.readFileSync(COMPAT_DOC, "utf8");
const allowlist = JSON.parse(fs.readFileSync(ALLOWLIST_PATH, "utf8"));
const probeText = fs.readFileSync(PROBE_PATH, "utf8");

// Parse TRACKED_MODULES from probe.mjs so matrix and probe stay synchronized
const trackedMatch = probeText.match(/const TRACKED_MODULES = \[([\s\S]*?)\];/);
if (!trackedMatch) {
  throw new Error("Could not locate TRACKED_MODULES in compat/api-surface/probe.mjs");
}
const trackedModules = new Set(
  [...trackedMatch[1].matchAll(/"([^"]+)"/g)].map((m) => m[1])
);

const lines = compatText.split(/\r?\n/);
const tableRows = [];
let inNodeTable = false;

for (const line of lines) {
  if (line.startsWith("| [Node.js API]")) {
    inNodeTable = true;
    continue;
  }
  if (inNodeTable) {
    if (!line.startsWith("|")) break;
    if (line.startsWith("| ---")) continue;
    const cells = line
      .split("|")
      .slice(1, -1)
      .map((c) => c.trim());
    if (cells.length !== 4) {
      throw new Error(`Malformed compatibility table row (expected 4 columns): ${line}`);
    }
    const [api, nodeCol, lexeCol, notes] = cells;
    if (!api.startsWith("node:")) {
      throw new Error(`Unexpected row in Node.js API table: ${api}`);
    }
    tableRows.push({
      mod: api.slice("node:".length),
      api,
      nodeCol,
      lexeCol,
      notes,
    });
  }
}

if (tableRows.length === 0) {
  throw new Error("Failed to parse Node.js API table in docs/compatibility.md");
}

const failures = [];
const moduleAllowlist = allowlist.modules ?? {};

for (const row of tableRows) {
  if (!trackedModules.has(row.mod)) {
    failures.push(`[${row.api}] listed in docs/compatibility.md but missing from compat/api-surface/probe.mjs TRACKED_MODULES`);
  }

  const entry = moduleAllowlist[row.mod] ?? {};
  const isUnimplemented = entry.unimplemented === true;
  const hasSurfaceGaps =
    (entry.missingExports?.length ?? 0) > 0 ||
    (entry.missingConstants?.length ?? 0) > 0 ||
    entry.missingConstantsObj === true ||
    (entry.typeMismatches?.length ?? 0) > 0 ||
    (entry.esmCjsMissingInEsm?.length ?? 0) > 0 ||
    entry.isBuiltinBareMismatch === true ||
    entry.bareCjsMismatch === true ||
    entry.getBuiltinMismatch === true;

  if (row.lexeCol === "✘") {
    if (!isUnimplemented) {
      failures.push(`[${row.api}] marked ✘ in docs/compatibility.md, but loads in Lexe according to allowlist.json`);
    }
    continue;
  }

  if (isUnimplemented) {
    failures.push(`[${row.api}] is unimplemented in allowlist.json, so docs/compatibility.md must mark it ✘ (got "${row.lexeCol}")`);
    continue;
  }

  const isPlainPass = row.lexeCol === "✔︎";
  if (isPlainPass && hasSurfaceGaps) {
    failures.push(
      `[${row.api}] marked ✔︎ in docs/compatibility.md without ⚠️, but compat/api-surface/allowlist.json records surface gaps`
    );
  }

  if (row.lexeCol.includes("⚠️")) {
    const hasHeading =
      compatText.includes(`#### \`${row.api}\``) ||
      compatText.includes(`### \`${row.api}\``) ||
      row.notes.length > 0;
    if (!hasHeading) {
      failures.push(`[${row.api}] marked ${row.lexeCol} without partial-support details`);
    }
  }
}

for (const mod of Object.keys(moduleAllowlist)) {
  if (!trackedModules.has(mod)) {
    failures.push(`[${mod}] present in allowlist.json but not in TRACKED_MODULES`);
  }
}

if (failures.length > 0) {
  for (const f of failures) {
    console.error(`MATRIX FAIL: ${f}`);
  }
  process.exit(1);
}

console.log(
  `Compatibility matrix audit passed: ${tableRows.length} Node.js API rows verified against compat/api-surface/allowlist.json.`
);
