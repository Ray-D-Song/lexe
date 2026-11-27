#!/usr/bin/env node
import { spawn } from "node:child_process";
import fs from "node:fs/promises";
import fsSync from "node:fs";
import os from "node:os";
import path from "node:path";

const ROOT = process.cwd();
const PARALLEL_DIR = path.join(ROOT, "refs/node/test/parallel");
const HARNESS_PATH = path.join(ROOT, "compat/node-upstream/harness.mjs");
const DEFAULT_MANIFEST_PATH = path.join(ROOT, "compat/node-upstream/manifest.json");
const DEFAULT_LOG_PATH = path.join(ROOT, "compat/node-upstream/compat.log");

const TEARDOWN_GUARD =
  /LEXE_QJS_GC_DIAGNOSTICS: residual|Assertion failed|SIGABRT|SIGSEGV|shutdown incomplete|driver has not finished|retained JS owners/i;

const DEFAULT_TIMEOUT_MS = 15_000;
const DEFAULT_CONCURRENCY = 6;

const CURATED_SUITES = [
  { suite: "assert", prefixes: ["test-assert"] },
  { suite: "diagnostics_channel", prefixes: ["test-diagnostics-channel"] },
  { suite: "events", prefixes: ["test-events-"] },
  { suite: "http2", prefixes: ["test-http2-"] },
  { suite: "path", prefixes: ["test-path-"] },
  { suite: "querystring", prefixes: ["test-querystring"] },
  { suite: "string_decoder", prefixes: ["test-string-decoder"] },
  { suite: "url", prefixes: ["test-url-"] },
];

const UNSUPPORTED_FLAGS = new Set([
  "--expose-internals",
  "--allow-natives-syntax",
  "--experimental-permission",
  "--permission",
  "--experimental-test-snapshots",
  "--experimental-loader",
  "--loader",
  "--experimental-strip-types",
  "--no-experimental-strip-types",
  "--experimental-transform-types",
  "--experimental-wasm-modules",
  "--experimental-vm-modules",
  "--inspect",
  "--inspect-brk",
  "--prof",
  "--cpu-prof",
  "--heap-prof",
]);

function parseArgs(argv) {
  const opts = {
    lexe: null,
    suite: null,
    filter: null,
    manifest: DEFAULT_MANIFEST_PATH,
    discover: false,
    updateManifest: false,
    passingOnly: false,
    withNodeBaseline: false,
    concurrency: DEFAULT_CONCURRENCY,
    timeoutMs: DEFAULT_TIMEOUT_MS,
  };
  const positional = [];
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--suite") opts.suite = argv[++i];
    else if (arg === "--filter") opts.filter = argv[++i];
    else if (arg === "--manifest") opts.manifest = path.resolve(ROOT, argv[++i]);
    else if (arg === "--discover") opts.discover = true;
    else if (arg === "--update-manifest") opts.updateManifest = true;
    else if (arg === "--passing-only") opts.passingOnly = true;
    else if (arg === "--with-node-baseline") opts.withNodeBaseline = true;
    else if (arg === "--concurrency") opts.concurrency = Number.parseInt(argv[++i], 10);
    else if (arg === "--timeout-ms") opts.timeoutMs = Number.parseInt(argv[++i], 10);
    else positional.push(arg);
  }
  opts.lexe = positional[0] ? path.resolve(ROOT, positional[0]) : null;
  return opts;
}

function extractHeaderFlags(filePath) {
  const content = fsSync.readFileSync(filePath, "utf8").slice(0, 2048);
  const match = content.match(/\/\/\s*Flags:\s*(.+)/);
  if (!match) return [];
  return match[1].trim().split(/\s+/).filter(Boolean);
}

function hasUnsupportedFlag(flags) {
  for (const flag of flags) {
    const key = flag.split("=")[0];
    if (UNSUPPORTED_FLAGS.has(key)) return flag;
  }
  return null;
}

function classifyOutcome(result) {
  if (result.timedOut) return "fail";
  if (TEARDOWN_GUARD.test(result.stderr)) return "fail";
  if (result.code !== 0) return "fail";
  if (/^1\.\.0\s+#\s*Skipped:/m.test(result.stdout)) return "skip";
  return "pass";
}

function spawnTest(command, args, env, timeoutMs) {
  return new Promise((resolve) => {
    const child = spawn(command, args, {
      cwd: ROOT,
      env,
      stdio: ["ignore", "pipe", "pipe"],
    });
    const stdoutChunks = [];
    const stderrChunks = [];
    let timedOut = false;
    const timer = setTimeout(() => {
      timedOut = true;
      child.kill("SIGKILL");
    }, timeoutMs);

    child.stdout.on("data", (d) => stdoutChunks.push(d));
    child.stderr.on("data", (d) => stderrChunks.push(d));
    child.on("close", (code, signal) => {
      clearTimeout(timer);
      resolve({
        code,
        signal,
        timedOut,
        stdout: Buffer.concat(stdoutChunks).toString("utf8"),
        stderr: Buffer.concat(stderrChunks).toString("utf8"),
      });
    });
    child.on("error", (err) => {
      clearTimeout(timer);
      resolve({
        code: -1,
        signal: null,
        timedOut: false,
        stdout: "",
        stderr: err.message,
      });
    });
  });
}

async function runPool(items, concurrency, fn) {
  const results = new Array(items.length);
  let cursor = 0;
  async function worker() {
    while (true) {
      const idx = cursor;
      cursor += 1;
      if (idx >= items.length) return;
      results[idx] = await fn(items[idx], idx);
    }
  }
  const workers = Array.from({ length: Math.min(concurrency, items.length) }, () => worker());
  await Promise.all(workers);
  return results;
}

async function discoverEntries() {
  const files = (await fs.readdir(PARALLEL_DIR))
    .filter((f) => f.endsWith(".js") || f.endsWith(".mjs"))
    .sort();
  const entries = {};
  for (const file of files) {
    const matched = CURATED_SUITES.find((s) => s.prefixes.some((p) => file.startsWith(p)));
    if (!matched) continue;
    const fullPath = path.join(PARALLEL_DIR, file);
    const flags = extractHeaderFlags(fullPath);
    const unsupported = hasUnsupportedFlag(flags);
    entries[file] = {
      suite: matched.suite,
      expected: unsupported ? "skip" : "xfail",
      ...(unsupported ? { reason: `unsupported flag ${unsupported}` } : {}),
    };
  }
  return entries;
}

async function main() {
  const opts = parseArgs(process.argv.slice(2));
  if (!opts.lexe) {
    console.error(
      "Usage: node compat/node-upstream/run.mjs <lexe-bin> [--suite <name>] [--filter <str>] [--discover] [--update-manifest] [--with-node-baseline]"
    );
    process.exit(2);
  }
  if (!fsSync.existsSync(PARALLEL_DIR)) {
    throw new Error(
      `Missing ${path.relative(ROOT, PARALLEL_DIR)}. Run: git submodule update --init --depth 1 refs/node`
    );
  }

  let manifest;
  if (opts.discover) {
    manifest = {
      nodeVersion: "v24.3.0",
      tests: await discoverEntries(),
    };
  } else {
    manifest = JSON.parse(await fs.readFile(opts.manifest, "utf8"));
  }

  const selected = Object.entries(manifest.tests)
    .filter(([file, meta]) => {
      if (opts.suite && meta.suite !== opts.suite) return false;
      if (opts.filter && !file.includes(opts.filter)) return false;
      if (opts.passingOnly && meta.expected !== "pass") return false;
      return true;
    })
    .sort(([a], [b]) => a.localeCompare(b));

  if (selected.length === 0) {
    console.log("No matching upstream Node tests selected.");
    return;
  }

  const tmpRoot = await fs.mkdtemp(path.join(os.tmpdir(), "lexe-node-upstream-"));
  const logLines = [];
  const regressions = [];
  const xpasses = [];
  const counts = { pass: 0, xfail: 0, skip: 0, fail: 0, xpass: 0 };

  try {
    const outcomes = await runPool(selected, opts.concurrency, async ([file, meta], idx) => {
      const fullPath = path.join(PARALLEL_DIR, file);
      if (!fsSync.existsSync(fullPath)) {
        return { file, meta, status: "fail", reason: "missing upstream file in refs/node" };
      }

      if (meta.expected === "skip" && !opts.discover) {
        return { file, meta, status: "skip", reason: meta.reason ?? "manifest skip" };
      }

      const flags = extractHeaderFlags(fullPath);
      const unsupported = hasUnsupportedFlag(flags);
      if (unsupported) {
        return { file, meta, status: "skip", reason: `unsupported flag ${unsupported}` };
      }

      if (opts.withNodeBaseline) {
        const nodeRes = await spawnTest(
          process.execPath,
          [fullPath],
          { ...process.env, NODE_TEST_DIR: tmpRoot, TEST_THREAD_ID: `n${idx}` },
          opts.timeoutMs
        );
        if (classifyOutcome(nodeRes) === "fail") {
          return { file, meta, status: "skip", reason: "failed on host Node baseline" };
        }
      }

      const lexeRes = await spawnTest(
        opts.lexe,
        [HARNESS_PATH, fullPath],
        {
          ...process.env,
          NODE_SKIP_FLAG_CHECK: "1",
          NODE_TEST_KNOWN_GLOBALS: "0",
          NODE_TEST_DIR: tmpRoot,
          TEST_THREAD_ID: `l${idx}`,
        },
        opts.timeoutMs
      );
      const actual = classifyOutcome(lexeRes);
      return {
        file,
        meta,
        status: actual,
        timedOut: lexeRes.timedOut,
        code: lexeRes.code,
        stderr: lexeRes.stderr.trim(),
        stdout: lexeRes.stdout.trim(),
      };
    });

    for (const item of outcomes) {
      const { file, meta, status } = item;
      if (opts.discover || opts.updateManifest) {
        manifest.tests[file] = {
          suite: meta.suite,
          expected: status === "fail" ? "xfail" : status,
          ...(status === "skip" && item.reason ? { reason: item.reason } : {}),
        };
      }

      if (status === "skip") {
        counts.skip += 1;
        logLines.push(`SKIP  ${file} (${item.reason ?? meta.reason ?? ""})`);
        continue;
      }

      if (meta.expected === "pass") {
        if (status === "pass") {
          counts.pass += 1;
          logLines.push(`PASS  ${file}`);
        } else {
          counts.fail += 1;
          regressions.push(item);
          logLines.push(
            `FAIL  ${file} (expected pass, got ${status}, exit=${item.code}, timedOut=${item.timedOut})\n${item.stderr || item.stdout}`
          );
        }
      } else if (meta.expected === "xfail") {
        if (status === "pass") {
          counts.xpass += 1;
          xpasses.push(file);
          logLines.push(`XPASS ${file}`);
        } else {
          counts.xfail += 1;
          logLines.push(`XFAIL ${file} (exit=${item.code}${item.timedOut ? " timeout" : ""})`);
        }
      }
    }

    if (opts.discover || opts.updateManifest) {
      await fs.writeFile(opts.manifest, JSON.stringify(manifest, null, 2) + "\n");
      console.log(`Updated ${path.relative(ROOT, opts.manifest)} (${selected.length} tests).`);
    }

    await fs.writeFile(DEFAULT_LOG_PATH, logLines.join("\n") + "\n");

    const bySuite = {};
    for (const [file, meta] of Object.entries(manifest.tests)) {
      if (opts.suite && meta.suite !== opts.suite) continue;
      if (opts.filter && !file.includes(opts.filter)) continue;
      bySuite[meta.suite] ??= { pass: 0, xfail: 0, skip: 0 };
      bySuite[meta.suite][meta.expected] = (bySuite[meta.suite][meta.expected] ?? 0) + 1;
    }

    console.log("Suite summary (manifest expectations):");
    for (const [suite, stats] of Object.entries(bySuite).sort()) {
      console.log(`  ${suite}: pass=${stats.pass}, xfail=${stats.xfail}, skip=${stats.skip}`);
    }
    console.log(
      `Run result: pass=${counts.pass}, xfail=${counts.xfail}, xpass=${counts.xpass}, skip=${counts.skip}, regressions=${counts.fail}`
    );

    if (xpasses.length > 0 && !opts.discover && !opts.updateManifest) {
      console.log(`XPASS (unexpectedly passing; consider promoting to "pass"): ${xpasses.join(", ")}`);
    }
    if (regressions.length > 0 && !opts.discover && !opts.updateManifest) {
      for (const r of regressions) {
        console.error(`REGRESSION: ${r.file} (exit=${r.code}, timedOut=${r.timedOut})`);
      }
      process.exitCode = 1;
    }
  } finally {
    await fs.rm(tmpRoot, { recursive: true, force: true });
  }
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
