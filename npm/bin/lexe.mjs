#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import { ensureBinary } from "../lib/download.mjs";

let binaryPath;
try {
  binaryPath = await ensureBinary();
} catch (err) {
  const message = err instanceof Error ? err.message : String(err);
  console.error(`lexe: failed to prepare runtime binary: ${message}`);
  process.exit(1);
}

const result = spawnSync(binaryPath, process.argv.slice(2), {
  stdio: "inherit",
  env: {
    ...process.env,
    LEXE_MANAGED_BY: "npm",
  },
});

if (result.error) {
  console.error(
    `lexe: failed to execute ${binaryPath}: ${result.error.message}`
  );
  process.exit(1);
}

if (result.signal) {
  process.kill(process.pid, result.signal);
}

process.exit(result.status ?? 1);
