#!/usr/bin/env node
import fs from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";

/**
 * Compare Node.js and Lexe API surface probe outputs against allowlist.json.
 * Returns { ok, failures, warnings, staleAllowlist, summary }.
 */
export function compareApiSurface(nodePayload, lexePayload, allowlist) {
  const failures = [];
  const warnings = [];
  const staleAllowlist = [];
  const moduleAllowlist = allowlist.modules ?? {};
  const globalAllowlist = allowlist.globals ?? {};

  const nodeModules = nodePayload.modules ?? {};
  const lexeModules = lexePayload.modules ?? {};

  for (const modName of Object.keys(nodeModules).sort()) {
    const nm = nodeModules[modName];
    const lm = lexeModules[modName];
    const entry = moduleAllowlist[modName] ?? {};

    if (!lm) {
      failures.push(`[${modName}] missing from Lexe probe output`);
      continue;
    }

    if (entry.unimplemented === true) {
      if (lm.cjsLoaded || lm.esmLoaded) {
        staleAllowlist.push(`[${modName}] marked unimplemented in allowlist.json, but now loads in Lexe`);
      }
      continue;
    }

    if (!nm.cjsLoaded) continue;

    if (!lm.cjsLoaded) {
      failures.push(`[${modName}] CJS require("node:${modName}") failed in Lexe: ${lm.cjsError ?? "unknown error"}`);
      continue;
    }
    if (!lm.esmLoaded) {
      failures.push(`[${modName}] ESM import("node:${modName}") failed in Lexe: ${lm.esmError ?? "unknown error"}`);
    }

    // ESM <-> CJS parity inside Lexe itself (catches missing ModuleDef::declare entries)
    const allowedEsmMissing = new Set(entry.esmCjsMissingInEsm ?? []);
    for (const key of lm.esmCjsMissingInEsm ?? []) {
      if (!allowedEsmMissing.has(key)) {
        failures.push(`[${modName}] CJS export "${key}" missing from ESM named exports in Lexe (check ModuleDef::declare)`);
      }
    }
    for (const key of allowedEsmMissing) {
      if (!(lm.esmCjsMissingInEsm ?? []).includes(key)) {
        staleAllowlist.push(`[${modName}] esmCjsMissingInEsm "${key}" is now exported in ESM`);
      }
    }

    const allowedEsmExtra = new Set(entry.esmExtraOverCjs ?? []);
    for (const key of lm.esmExtraOverCjs ?? []) {
      if (!allowedEsmExtra.has(key)) {
        failures.push(`[${modName}] ESM named export "${key}" missing from CJS require() in Lexe`);
      }
    }
    for (const key of allowedEsmExtra) {
      if (!(lm.esmExtraOverCjs ?? []).includes(key)) {
        staleAllowlist.push(`[${modName}] esmExtraOverCjs "${key}" no longer diverges`);
      }
    }

    // Builtin resolution consistency
    if (nm.isBuiltinBare !== lm.isBuiltinBare && !entry.isBuiltinBareMismatch) {
      failures.push(`[${modName}] isBuiltin("${modName}") mismatch: Node=${nm.isBuiltinBare}, Lexe=${lm.isBuiltinBare}`);
    }
    if (entry.isBuiltinBareMismatch && nm.isBuiltinBare === lm.isBuiltinBare) {
      staleAllowlist.push(`[${modName}] isBuiltinBareMismatch is now resolved`);
    }
    if (nm.bareCjsLoaded !== lm.bareCjsLoaded && !entry.bareCjsMismatch) {
      failures.push(`[${modName}] require("${modName}") mismatch: Node=${nm.bareCjsLoaded}, Lexe=${lm.bareCjsLoaded}`);
    }
    if (nm.getBuiltinLoaded !== lm.getBuiltinLoaded && !entry.getBuiltinMismatch) {
      failures.push(`[${modName}] process.getBuiltinModule("node:${modName}") mismatch: Node=${nm.getBuiltinLoaded}, Lexe=${lm.getBuiltinLoaded}`);
    }

    // Top-level exports comparison
    const nKeys = nm.cjs?.keys ?? {};
    const lKeys = lm.cjs?.keys ?? {};
    const allowedMissing = new Set(entry.missingExports ?? []);
    const allowedTypeMismatches = new Set(entry.typeMismatches ?? []);

    for (const key of Object.keys(nKeys).sort()) {
      if (!(key in lKeys)) {
        if (!allowedMissing.has(key)) {
          failures.push(`[${modName}] missing export "${key}" (Node type: ${nKeys[key]})`);
        }
        continue;
      }
      if (lKeys[key] !== nKeys[key]) {
        const sig = `${key}:${nKeys[key]}!=${lKeys[key]}`;
        if (!allowedTypeMismatches.has(sig)) {
          failures.push(`[${modName}] export "${key}" type mismatch: Node=${nKeys[key]}, Lexe=${lKeys[key]}`);
        }
      }
    }

    for (const key of allowedMissing) {
      if (key in lKeys) {
        staleAllowlist.push(`[${modName}] missingExports "${key}" is now present in Lexe`);
      }
    }
    for (const sig of allowedTypeMismatches) {
      const [key] = sig.split(":");
      if (key in lKeys && lKeys[key] === nKeys[key]) {
        staleAllowlist.push(`[${modName}] typeMismatches "${sig}" now matches Node`);
      }
    }

    // Constants comparison
    if (nm.constants?.kind === "object") {
      if (lm.constants?.kind !== "object") {
        if (!entry.missingConstantsObj) {
          failures.push(`[${modName}] missing constants object`);
        }
      } else {
        const allowedMissingConstants = new Set(entry.missingConstants ?? []);
        const lConst = lm.constants.keys;
        for (const cKey of Object.keys(nm.constants.keys).sort()) {
          if (!(cKey in lConst) && !allowedMissingConstants.has(cKey)) {
            failures.push(`[${modName}] missing constant "constants.${cKey}"`);
          }
        }
        for (const cKey of allowedMissingConstants) {
          if (cKey in lConst) {
            staleAllowlist.push(`[${modName}] missingConstants "${cKey}" is now present in Lexe`);
          }
        }
      }
    }
  }

  // Globals comparison
  const allowedMissingGlobals = new Set(globalAllowlist.missing ?? []);
  for (const gName of Object.keys(nodePayload.globals ?? {}).sort()) {
    const ng = nodePayload.globals[gName];
    const lg = lexePayload.globals?.[gName];
    if (ng.kind !== "undefined" && (!lg || lg.kind === "undefined")) {
      if (!allowedMissingGlobals.has(gName)) {
        failures.push(`[globalThis] missing global "${gName}" (Node type: ${ng.kind})`);
      }
    }
  }
  for (const gName of allowedMissingGlobals) {
    const lg = lexePayload.globals?.[gName];
    if (lg && lg.kind !== "undefined") {
      staleAllowlist.push(`[globalThis] missing "${gName}" is now defined`);
    }
  }

  if (staleAllowlist.length > 0) {
    for (const msg of staleAllowlist) {
      warnings.push(`STALE ALLOWLIST: ${msg}`);
    }
  }

  return {
    ok: failures.length === 0 && staleAllowlist.length === 0,
    failures,
    warnings,
    staleAllowlist,
    summary: {
      modulesChecked: Object.keys(nodeModules).length,
      globalsChecked: Object.keys(nodePayload.globals ?? {}).length,
      failureCount: failures.length,
      staleCount: staleAllowlist.length,
    },
  };
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  const [nodeFile, lexeFile, allowlistFile] = process.argv.slice(2);
  if (!nodeFile || !lexeFile) {
    console.error("Usage: node compat/api-surface/check.mjs <node.json> <lexe.json> [allowlist.json]");
    process.exit(2);
  }
  const nodePayload = JSON.parse(fs.readFileSync(nodeFile, "utf8"));
  const lexePayload = JSON.parse(fs.readFileSync(lexeFile, "utf8"));
  const allowlistPath =
    allowlistFile ?? path.join(path.dirname(new URL(import.meta.url).pathname), "allowlist.json");
  const allowlist = JSON.parse(fs.readFileSync(allowlistPath, "utf8"));
  const result = compareApiSurface(nodePayload, lexePayload, allowlist);
  for (const w of result.warnings) console.warn(w);
  for (const f of result.failures) console.error(`FAIL: ${f}`);
  console.log(
    `API surface check: ${result.summary.modulesChecked} modules, ${result.summary.globalsChecked} globals, ` +
      `${result.summary.failureCount} failures, ${result.summary.staleCount} stale allowlist entries.`
  );
  if (!result.ok) process.exit(1);
}
