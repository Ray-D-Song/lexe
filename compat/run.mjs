import { spawn } from "node:child_process";
import { constants as fsConstants } from "node:fs";
import fs from "node:fs/promises";
import fsSync from "node:fs";
import net from "node:net";
import os from "node:os";
import path from "node:path";

const HTTP_CHECK_TIMEOUT_MS = 5_000;
const BUILD_TIMEOUT_MS = 120_000;
const SCRIPT_TIMEOUT_MS = 60_000;
const NODE_BASELINE_TIMEOUT_MS = 30_000;
const READINESS_TIMEOUT_MS = 30_000;
const READINESS_REQUEST_TIMEOUT_MS = 2_000;
const SERVER_STOP_TIMEOUT_MS = 5_000;
const MYSQL_DOCKER_IMAGE = "mysql:8.4";
const MYSQL_HEALTH_TIMEOUT_MS = 120_000;
const MYSQL_DOCKER_RUN_TIMEOUT_MS = 180_000;
const MYSQL_DOCKER_HOST = "127.0.0.1";
const POSTGRES_DOCKER_IMAGE = "postgres:16.14-bookworm";
const POSTGRES_DOCKER_HOST = "127.0.0.1";
const POSTGRES_HEALTH_TIMEOUT_MS = 120_000;
const POSTGRES_DOCKER_RUN_TIMEOUT_MS = 180_000;

const TEARDOWN_GUARD =
  /LEXE_QJS_GC_DIAGNOSTICS: residual|Assertion failed|SIGABRT|SIGSEGV|shutdown incomplete|driver has not finished|retained JS owners/i;

const BETTER_SQLITE3_SCENARIOS = [
  "addon-only",
  "create-db",
  "create-db-loop",
  "exec",
  "explicit-close",
  "prepare",
];

function positiveLoopCount(name, fallback) {
  const value = Number.parseInt(process.env[name] ?? String(fallback), 10);
  if (!Number.isSafeInteger(value) || value < 1) {
    throw new Error(`${name} must be a positive integer`);
  }
  return value;
}

const [name, lexePath] = process.argv.slice(2);
const root = process.cwd();
const cases = {
  next: {
    directory: "compat/next",
  },
  "vite-plus": {
    directory: "compat/vite-plus",
    command: "node_modules/vite-plus/bin/vp",
    args: ["build"],
    output: "dist",
    checks: [
      ["index.js"],
      ["index.cjs"],
      ["style.css"],
      [".vite", "manifest.json"],
    ],
  },
  "better-sqlite3": {
    directory: "compat/better-sqlite3",
    scripts: [
      {
        script: "test.cjs",
        successMarker: "better-sqlite3 compat OK",
      },
      ...BETTER_SQLITE3_SCENARIOS.map((scenario) => ({
        script: "isolation.mjs",
        env: {
          COMPAT_SCENARIO: scenario,
        },
        successMarker: `better-sqlite3 isolation OK: ${scenario}`,
        skipNodeBaseline: true,
      })),
    ],
  },
  mysql2: {
    directory: "compat/mysql2",
    script: "test.cjs",
    successMarker: "mysql2 compat OK",
  },
  "node-postgres": {
    directory: "compat/node-postgres",
    script: "test.cjs",
    successMarker: "node-postgres compat OK",
    allowLexeFailure: true,
  },
  "v8-hello": {
    directory: "compat/v8-hello",
    buildCommand: "npm run build",
    script: "test.cjs",
    successMarker: "v8-hello compat OK",
  },
  "node-sqlite": {
    directory: "compat/node-sqlite",
    parity: "parity.mjs",
    buildCommand: "node build-extension.mjs",
  },
  "napi-hello": {
    directory: "compat/napi-hello",
    buildCommand: "yarn build",
    scripts: [
      {
        script: "test.cjs",
        successMarker: "napi-hello compat OK",
        maxDurationMs: 2_000,
        expectCode: 0,
      },
      {
        script: "test-async-work-notimer.cjs",
        successMarker: "async-work-notimer-ok",
        maxDurationMs: 1_000,
        expectCode: 0,
      },
      {
        script: "test-tsfn-unref-fast.cjs",
        maxDurationMs: 2_000,
        expectCode: 0,
      },
      {
        script: "test-tsfn-unref-timer.cjs",
        successMarker: "timer-tsfn-ok",
        maxDurationMs: 2_000,
        expectCode: 0,
      },
      {
        script: "test-tsfn-refd-hold.cjs",
        expectStillRunning: { checkMs: 400, killAfterMs: 1_500 },
      },
      {
        script: "test-module-exports.cjs",
        successMarker: "module-exports-replace-ok",
        maxDurationMs: 1_000,
        expectCode: 0,
      },
    ],
  },
};

const testCase = cases[name];
if (!testCase || !lexePath) {
  throw new Error(
    "Usage: node compat/run.mjs <next|vite-plus|better-sqlite3|mysql2|node-postgres|v8-hello|napi-hello|node-sqlite> <lexe>"
  );
}

const directory = path.join(root, testCase.directory);
const lexe = path.resolve(root, lexePath);
const logPath = path.join(directory, "compat.log");

if (name === "next") {
  await runNextStandalone(directory, lexe, logPath, root);
} else if (name === "node-sqlite") {
  await runNodeSqliteCompat(testCase, directory, lexe, logPath, root);
} else if (
  name === "better-sqlite3" ||
  name === "mysql2" ||
  name === "node-postgres" ||
  name === "napi-hello" ||
  name === "v8-hello"
) {
  await runScriptCompat(testCase, directory, lexe, logPath, root);
} else {
  await runVitePlusBuild(testCase, directory, lexe, logPath, root);
}

async function runVitePlusBuild(testCase, directory, lexe, logPath, root) {
  const output = path.join(directory, testCase.output);
  const command = path.join(directory, testCase.command);
  const logParts = [];

  // Phase 1: vp --version must exit cleanly (no abort/assert/JS_CONTEXT hang).
  const versionResult = await spawnCollect(
    lexe,
    [command, "--version"],
    { cwd: directory, env: { ...process.env } },
    SCRIPT_TIMEOUT_MS
  );
  logParts.push(
    `$ ${lexe} ${command} --version\n\n` +
      `exit: ${versionResult.code ?? versionResult.signal}${versionResult.timedOut ? " (timed out)" : ""}\n\n` +
      `stdout:\n${versionResult.stdout}\n\nstderr:\n${versionResult.stderr}\n`
  );
  process.stdout.write(versionResult.stdout);
  process.stderr.write(versionResult.stderr);

  if (versionResult.timedOut || versionResult.code !== 0) {
    await fs.writeFile(logPath, logParts.join("\n"));
    throw new Error(
      `${name} vp --version exited with ${versionResult.code ?? versionResult.signal}` +
        (versionResult.timedOut ? " (timed out)" : "") +
        `. See ${path.relative(root, logPath)}.`
    );
  }
  const versionOut = `${versionResult.stdout}\n${versionResult.stderr}`;
  const INVALID_TEARDOWN =
    /SIGABRT|Assertion failed|JS_CONTEXT|gc_obj_list|shutdown incomplete|driver has not finished|leaking env/i;
  if (INVALID_TEARDOWN.test(versionOut)) {
    await fs.writeFile(logPath, logParts.join("\n"));
    throw new Error(
      `${name} vp --version output contains abort/assert residual. See ${path.relative(root, logPath)}.`
    );
  }
  if (!/\d+\.\d+\.\d+/.test(versionResult.stdout)) {
    await fs.writeFile(logPath, logParts.join("\n"));
    throw new Error(
      `${name} vp --version stdout missing version string. See ${path.relative(root, logPath)}.`
    );
  }

  // Phase 2: single verified build (artifacts + manifest parse).
  await fs.rm(output, { recursive: true, force: true });

  const result = await spawnCollect(
    lexe,
    [command, ...testCase.args],
    {
      cwd: directory,
      env: { ...process.env },
    },
    BUILD_TIMEOUT_MS
  );

  const timedOut = result.timedOut ? " (timed out)" : "";
  logParts.push(
    `$ ${lexe} ${command} ${testCase.args.join(" ")}\n\n` +
      `exit: ${result.code ?? result.signal}${timedOut}\n\n` +
      `stdout:\n${result.stdout}\n\nstderr:\n${result.stderr}\n`
  );
  process.stdout.write(result.stdout);
  process.stderr.write(result.stderr);

  const outputExists = await pathExists(output);

  if (result.timedOut) {
    await fs.writeFile(logPath, logParts.join("\n"));
    throw new Error(
      `${name} build timed out after ${BUILD_TIMEOUT_MS}ms. ` +
        `See ${path.relative(root, logPath)} for captured output.`
    );
  }

  if (result.code !== 0) {
    await fs.writeFile(logPath, logParts.join("\n"));
    throw new Error(
      `${name} build exited with ${result.code ?? result.signal}`
    );
  }

  if (!outputExists) {
    await fs.writeFile(logPath, logParts.join("\n"));
    throw new Error(
      `${name} exited 0 but produced no ${testCase.output}/ directory. ` +
        `stdout empty=${result.stdout.length === 0}, stderr empty=${result.stderr.length === 0}. ` +
        `See ${path.relative(root, logPath)} for the captured Lexe child output.`
    );
  }

  for (const segments of testCase.checks) {
    await fs.access(path.join(output, ...segments));
  }

  const [esm, cjs, css, manifestText] = await Promise.all([
    fs.readFile(path.join(output, "index.js"), "utf8"),
    fs.readFile(path.join(output, "index.cjs"), "utf8"),
    fs.readFile(path.join(output, "style.css"), "utf8"),
    fs.readFile(path.join(output, ".vite", "manifest.json"), "utf8"),
  ]);
  let manifest;
  try {
    manifest = JSON.parse(manifestText);
  } catch (err) {
    await fs.writeFile(logPath, logParts.join("\n"));
    throw new Error(
      `${name} dist/.vite/manifest.json is not valid JSON: ${err instanceof Error ? err.message : String(err)}`
    );
  }
  if (
    !esm.includes("Button") ||
    !cjs.includes("Button") ||
    !css.includes(".lexe-button") ||
    !manifestText.includes("src/index.tsx") ||
    typeof manifest !== "object" ||
    manifest === null
  ) {
    await fs.writeFile(logPath, logParts.join("\n"));
    throw new Error(
      "Vite+ build output is missing an expected library artifact"
    );
  }

  // Phase 3: stability gate (vp --version ×N, vp build ×M into a temp dir).
  // Defaults match the plan; override with COMPAT_VP_VERSION_LOOPS / COMPAT_VP_BUILD_LOOPS.
  const versionLoops = Math.max(
    0,
    Number.parseInt(process.env.COMPAT_VP_VERSION_LOOPS ?? "100", 10) || 0
  );
  const buildLoops = Math.max(
    0,
    Number.parseInt(process.env.COMPAT_VP_BUILD_LOOPS ?? "20", 10) || 0
  );

  for (let i = 1; i <= versionLoops; i++) {
    const r = await spawnCollect(
      lexe,
      [command, "--version"],
      { cwd: directory, env: { ...process.env } },
      SCRIPT_TIMEOUT_MS
    );
    if (r.timedOut || r.code !== 0) {
      logParts.push(
        `\n# stability vp --version #${i}\nexit: ${r.code ?? r.signal}\nstdout:\n${r.stdout}\nstderr:\n${r.stderr}`
      );
      await fs.writeFile(logPath, logParts.join("\n"));
      throw new Error(
        `${name} stability: vp --version failed on iteration ${i}/${versionLoops} ` +
          `(exit ${r.code ?? r.signal})`
      );
    }
    if (INVALID_TEARDOWN.test(`${r.stdout}\n${r.stderr}`)) {
      logParts.push(
        `\n# stability vp --version #${i}\nstdout:\n${r.stdout}\nstderr:\n${r.stderr}`
      );
      await fs.writeFile(logPath, logParts.join("\n"));
      throw new Error(
        `${name} stability: vp --version abort/assert on iteration ${i}/${versionLoops}`
      );
    }
  }

  if (buildLoops > 0) {
    const stabilityDist = await fs.mkdtemp(
      path.join(os.tmpdir(), "lexe-vp-build-")
    );
    try {
      for (let i = 1; i <= buildLoops; i++) {
        // Wipe fixture dist each iteration; keep repo user artifacts elsewhere.
        await fs.rm(output, { recursive: true, force: true });
        const r = await spawnCollect(
          lexe,
          [command, ...testCase.args],
          { cwd: directory, env: { ...process.env } },
          BUILD_TIMEOUT_MS
        );
        if (r.timedOut || r.code !== 0) {
          logParts.push(
            `\n# stability vp build #${i}\nexit: ${r.code ?? r.signal}\nstdout:\n${r.stdout}\nstderr:\n${r.stderr}`
          );
          await fs.writeFile(logPath, logParts.join("\n"));
          throw new Error(
            `${name} stability: vp build failed on iteration ${i}/${buildLoops} ` +
              `(exit ${r.code ?? r.signal})`
          );
        }
        for (const segments of testCase.checks) {
          await fs.access(path.join(output, ...segments));
        }
        // Preserve last good dist copy for diagnostics (temp, not repo).
        if (i === buildLoops) {
          await fs.cp(output, path.join(stabilityDist, "dist"), {
            recursive: true,
          });
        }
      }
    } finally {
      await fs.rm(stabilityDist, { recursive: true, force: true }).catch(() => {});
    }
  }

  logParts.push(
    `\n# stability\nvp --version ×${versionLoops} ok\nvp build ×${buildLoops} ok`
  );
  await fs.writeFile(logPath, logParts.join("\n"));
  console.log(
    `${name} compatibility build passed (version×${versionLoops}, build×${buildLoops})`
  );
}

async function runNodeSqliteCompat(testCase, directory, lexe, logPath, root) {
  const logParts = [];
  const INVALID_TEARDOWN =
    /LEXE_QJS_GC_DIAGNOSTICS: residual|LEXE_QJS_CONTEXT_REFS|Assertion failed|SIGABRT|SIGSEGV|shutdown incomplete|driver has not finished|leaking env|double free|pending backup|retained JS owners/i;

  try {
    await assertLexeExecutable(lexe, logParts);

    if (testCase.buildCommand && process.env.COMPAT_SKIP_BUILD !== "1") {
      logParts.push(`# Build extension\n$ ${testCase.buildCommand}`);
      console.log(`[compat-node-sqlite] building extension: ${testCase.buildCommand}`);
      const buildResult = await spawnCollect(
        "sh",
        ["-c", testCase.buildCommand],
        { cwd: directory, env: { ...process.env } },
        BUILD_TIMEOUT_MS
      );
      logParts.push(
        `exit: ${buildResult.code ?? buildResult.signal}\n\nstdout:\n${buildResult.stdout}\n\nstderr:\n${buildResult.stderr}`
      );
      if (buildResult.code !== 0) {
        throw new Error(
          `node-sqlite extension build failed (exit ${buildResult.code ?? buildResult.signal}). ` +
            `See ${path.relative(root, logPath)}.`
        );
      }
    }

    const extensionPath = resolveSqliteExtensionPath(directory);
    logParts.push(`\n# Extension\npath: ${extensionPath ?? "(not built)"}`);

    const sharedTmpRoot = await fs.mkdtemp(
      path.join(os.tmpdir(), "lexe-node-sqlite-parity-")
    );
    const nodeTmpRoot = path.join(sharedTmpRoot, "node");
    const lexeTmpRoot = path.join(sharedTmpRoot, "lexe");
    await fs.mkdir(nodeTmpRoot, { recursive: true });
    await fs.mkdir(lexeTmpRoot, { recursive: true });
    const lifecycleLoops = process.env.COMPAT_SQLITE_LIFECYCLE_LOOPS ?? "100";
    const backupLoops = process.env.COMPAT_SQLITE_BACKUP_LOOPS ?? "20";
    const baseParityEnv = {
      ...process.env,
      LIFECYCLE_LOOPS: lifecycleLoops,
      BACKUP_LOOPS: backupLoops,
      ...(extensionPath ? { SQLITE_EXTENSION_PATH: extensionPath } : {}),
    };

    const skipNodeBaseline = process.env.COMPAT_SKIP_NODE_BASELINE === "1";
    let nodeParity = null;

    if (!skipNodeBaseline) {
      const nodeCmd = `${process.execPath} ${testCase.parity}`;
      logParts.push(`\n# Node parity\n$ ${nodeCmd}`);
      console.log(`[compat-node-sqlite] Node parity: ${nodeCmd}`);
      const nodeResult = await spawnCompatChild(
        process.execPath,
        [testCase.parity],
        {
          cwd: directory,
          env: {
            ...baseParityEnv,
            PARITY_TMP_ROOT: nodeTmpRoot,
          },
        },
        Math.min(NODE_BASELINE_TIMEOUT_MS * 4, SCRIPT_TIMEOUT_MS * 4),
        logParts
      );
      validateCompatRun("node-sqlite/parity.mjs", "Node parity", nodeResult, {
        maxDurationMs: SCRIPT_TIMEOUT_MS * 4,
        expectCode: 0,
        logPath,
        root,
        logParts,
        lexeNotStarted: true,
        mustNotContainStdout: INVALID_TEARDOWN,
      });
      nodeParity = parseParityJson(nodeResult.stdout, "Node parity");
    }

    const lexeCmd = `${lexe} ${testCase.parity}`;
    logParts.push(`\n# Lexe parity\n$ ${lexeCmd}`);
    console.log(`[compat-node-sqlite] Lexe parity: ${lexeCmd}`);
    const lexeResult = await spawnCompatChild(
      lexe,
      [testCase.parity],
      {
        cwd: directory,
        env: {
          ...baseParityEnv,
          PARITY_TMP_ROOT: lexeTmpRoot,
        },
      },
      SCRIPT_TIMEOUT_MS * 4,
      logParts
    );
    validateCompatRun("node-sqlite/parity.mjs", "Lexe parity", lexeResult, {
      maxDurationMs: SCRIPT_TIMEOUT_MS * 4,
      expectCode: 0,
      logPath,
      root,
      logParts,
      lexeNotStarted: false,
      mustNotContainStdout: INVALID_TEARDOWN,
    });
    const lexeParity = parseParityJson(lexeResult.stdout, "Lexe parity");

    if (nodeParity) {
      const nodeNorm = normalizeParityOutput(nodeParity);
      const lexeNorm = normalizeParityOutput(lexeParity);
      const diff = diffParityOutputs(nodeNorm, lexeNorm);
      logParts.push(`\n# Parity diff\n${diff || "(no differences)"}`);
      if (diff) {
        throw new Error(
          `node-sqlite parity mismatch between Node and Lexe:\n${diff}\n` +
            `See ${path.relative(root, logPath)}.`
        );
      }
    }

    const compatMode = skipNodeBaseline ? "Lexe only" : "Node baseline + Lexe";
    console.log(`node-sqlite compatibility passed (${compatMode})`);
  } finally {
    await writeLog(logPath, logParts);
  }
}

function resolveSqliteExtensionPath(directory) {
  const buildDir = path.join(directory, "build");
  if (!fsSync.existsSync(buildDir)) {
    return null;
  }
  const candidates = fsSync
    .readdirSync(buildDir)
    .filter((file) => file.startsWith("sqlite_extension"));
  if (candidates.length === 0) {
    return null;
  }
  return path.join(buildDir, candidates[0]);
}

function parseParityJson(stdout, label) {
  const lines = stdout.trim().split("\n");
  const jsonLine = [...lines].reverse().find((line) => line.startsWith("{"));
  if (!jsonLine) {
    throw new Error(`${label} stdout missing JSON payload`);
  }
  try {
    return JSON.parse(jsonLine);
  } catch (err) {
    throw new Error(
      `${label} stdout JSON parse failed: ${err instanceof Error ? err.message : String(err)}`
    );
  }
}

function normalizeParityOutput(parityOutput) {
  const clone = structuredClone(parityOutput);
  delete clone.runtime;
  for (const entry of clone.results ?? []) {
    if (entry?.value && typeof entry.value === "object") {
      entry.value = normalizeParityValue(entry.value);
    }
    if (entry?.error?.message) {
      entry.error.message = normalizeParityText(entry.error.message);
    }
  }
  return clone;
}

function normalizeParityValue(value) {
  if (value === null || typeof value !== "object") {
    return typeof value === "string" ? normalizeParityText(value) : value;
  }
  if (Array.isArray(value)) {
    return value.map((item) => normalizeParityValue(item));
  }
  if (value instanceof Uint8Array || Buffer.isBuffer(value)) {
    return { byteLength: value.byteLength ?? value.length };
  }
  const out = {};
  for (const [key, item] of Object.entries(value)) {
    if (key === "__proto__") {
      continue;
    }
    out[key] = normalizeParityValue(item);
  }
  return out;
}

function normalizeParityText(text) {
  return String(text)
    .replaceAll(String(process.pid), "<pid>")
    .replace(/\/(?:var|tmp|private)\/folders\/[^\s"']+/g, "<tmp>")
    .replace(/sqlite_extension\.(?:dylib|so|dll)/g, "sqlite_extension<ext>")
    .replace(/ExperimentalWarning:[^\n]*/g, "<warning>")
    .replace(/Require stack:[\s\S]*/g, "")
    .replace(/from '[^']+'/g, "from '<module>'")
    .trim();
}

function diffParityOutputs(nodeParity, lexeParity) {
  const nodeJson = JSON.stringify(nodeParity, null, 2);
  const lexeJson = JSON.stringify(lexeParity, null, 2);
  if (nodeJson === lexeJson) {
    return "";
  }
  const nodeResults = new Map(
    (nodeParity.results ?? []).map((entry) => [entry.name, entry])
  );
  const lines = [];
  for (const lexeEntry of lexeParity.results ?? []) {
    const nodeEntry = nodeResults.get(lexeEntry.name);
    if (!nodeEntry) {
      lines.push(`- missing Node result: ${lexeEntry.name}`);
      continue;
    }
    const nodeValue = JSON.stringify(nodeEntry);
    const lexeValue = JSON.stringify(lexeEntry);
    if (nodeValue !== lexeValue) {
      lines.push(`- ${lexeEntry.name}`);
      lines.push(`  node:   ${nodeValue}`);
      lines.push(`  lexe: ${lexeValue}`);
    }
  }
  return lines.join("\n");
}

async function runScriptCompat(testCase, directory, lexe, logPath, root) {
  if (name === "mysql2") {
    await runMysql2ScriptCompat(testCase, directory, lexe, logPath, root);
    return;
  }

  if (name === "node-postgres") {
    await runPostgresScriptCompat(testCase, directory, lexe, logPath, root);
    return;
  }

  const logParts = [];
  const childEnv = { ...process.env };

  try {
    if (testCase.buildCommand && process.env.COMPAT_SKIP_BUILD !== "1") {
      logParts.push(`# Build addon\n$ ${testCase.buildCommand}`);
      console.log(`[compat-${name}] building addon: ${testCase.buildCommand}`);
      const buildResult = await spawnCollect(
        "sh",
        ["-c", testCase.buildCommand],
        { cwd: directory, env: { ...process.env } },
        BUILD_TIMEOUT_MS
      );
      logParts.push(
        `exit: ${buildResult.code ?? buildResult.signal}\n\nstdout:\n${buildResult.stdout}\n\nstderr:\n${buildResult.stderr}`
      );
      if (buildResult.code !== 0) {
        throw new Error(
          `${name} addon build failed (exit ${buildResult.code ?? buildResult.signal}). ` +
            `See ${path.relative(root, logPath)}.`
        );
      }
    }

    const scripts = testCase.scripts ?? [
      {
        script: testCase.script,
        successMarker: testCase.successMarker,
        maxDurationMs: SCRIPT_TIMEOUT_MS,
        expectCode: 0,
      },
    ];

    const loopCount =
      name === "better-sqlite3"
        ? positiveLoopCount("COMPAT_BETTER_SQLITE3_LOOPS", 1)
        : 1;

    for (let loop = 0; loop < loopCount; loop++) {
      if (loopCount > 1) {
        logParts.push(`\n# better-sqlite3 stability loop ${loop + 1}/${loopCount}`);
        console.log(`[compat-${name}] stability loop ${loop + 1}/${loopCount}`);
      }
      for (const spec of scripts) {
        const specWithGuard =
          name === "better-sqlite3"
            ? {
                ...spec,
                mustNotContainStdout:
                  spec.mustNotContainStdout ?? TEARDOWN_GUARD,
              }
            : spec;
        await runCompatScript(
          specWithGuard,
          directory,
          lexe,
          logParts,
          root,
          logPath,
          childEnv
        );
      }
    }

    const compatMode =
      process.env.COMPAT_SKIP_NODE_BASELINE === "1"
        ? "Lexe only"
        : "Node baseline + Lexe";
    console.log(
      `${name} compatibility passed (${scripts.length} script(s), ${compatMode})`
    );
  } finally {
    await writeLog(logPath, logParts);
  }
}

async function runCompatScript(
  spec,
  directory,
  lexe,
  logParts,
  root,
  logPath,
  env,
  betweenPhases = null
) {
  const {
    script,
    args = [],
    env: specEnv = {},
    successMarker,
    maxDurationMs = SCRIPT_TIMEOUT_MS,
    expectCode = 0,
    mustNotContainStdout,
    expectStillRunning,
    allowLexeFailure = false,
    skipNodeBaseline: specSkipNodeBaseline = false,
  } = spec;
  const label = `${name}/${script}`;
  const skipNodeBaseline =
    process.env.COMPAT_SKIP_NODE_BASELINE === "1" || specSkipNodeBaseline;
  const childEnv = {
    ...env,
    ...specEnv,
  };
  const stdioModes =
    name === "better-sqlite3" ? ["pipe", "redirect"] : ["pipe"];

  if (expectStillRunning) {
    const { checkMs = 400, killAfterMs = 1_500 } = expectStillRunning;
    const phases = skipNodeBaseline
      ? [["Lexe run", lexe, [script]]]
      : [
          ["Node baseline", process.execPath, [script]],
          ["Lexe run", lexe, [script]],
        ];
    for (const [phase, command, args] of phases) {
      const cmdLine = `${command} ${args.join(" ")}`;
      logParts.push(`\n# ${phase}: ${script} (still-running)\n$ ${cmdLine}`);
      console.log(`[compat-${label}] ${phase} (still-running): ${cmdLine}`);
      const result = await spawnStillRunning(
        command,
        args,
        { cwd: directory, env },
        checkMs,
        killAfterMs
      );
      logParts.push(
        `still alive at ${checkMs}ms, killed after ${killAfterMs}ms\n\nstdout:\n${result.stdout}\n\nstderr:\n${result.stderr}`
      );
      if (!result.stillAliveAtCheck) {
        throw new Error(
          `${label} ${phase} exited before ${checkMs}ms. ` +
            `See ${path.relative(root, logPath)}.`
        );
      }
    }
    return { lexePassed: true };
  }

  if (!skipNodeBaseline) {
    const nodeCmd = `${process.execPath} ${script}`;
    logParts.push(`\n# Node baseline: ${script}\n$ ${nodeCmd}`);
    console.log(`[compat-${label}] Node baseline: ${nodeCmd}`);

    const nodeResult = await spawnCompatChild(
      process.execPath,
      [script, ...args],
      { cwd: directory, env: childEnv },
      Math.min(NODE_BASELINE_TIMEOUT_MS, maxDurationMs),
      logParts
    );

    validateCompatRun(label, "Node baseline", nodeResult, {
      maxDurationMs,
      expectCode,
      successMarker,
      mustNotContainStdout,
      logPath,
      root,
      logParts,
      lexeNotStarted: true,
    });

    if (betweenPhases) {
      await betweenPhases();
    }
  }

  for (const stdioMode of stdioModes) {
    const lexeCmd = `${lexe} ${script}${args.length ? ` ${args.join(" ")}` : ""}`;
    logParts.push(`\n# Lexe run (${stdioMode}): ${script}\n$ ${lexeCmd}`);
    console.log(`[compat-${label}] Lexe run (${stdioMode}): ${lexeCmd}`);

    const lexeResult =
      stdioMode === "redirect"
        ? await spawnRedirected(
            lexe,
            [script, ...args],
            { cwd: directory, env: childEnv },
            maxDurationMs
          )
        : await spawnCompatChild(
            lexe,
            [script, ...args],
            { cwd: directory, env: childEnv },
            maxDurationMs,
            logParts
          );

    let lexePassed = true;

    try {
      validateCompatRun(
        `${label}/${stdioMode}`,
        "Lexe run",
        lexeResult,
        {
          maxDurationMs,
          expectCode,
          successMarker,
          mustNotContainStdout,
          logPath,
          root,
          logParts,
          lexeNotStarted: false,
        }
      );
    } catch (error) {
      if (!allowLexeFailure) {
        throw error;
      }

      lexePassed = false;
      const message = formatSpawnError(error);
      logParts.push(`\n# Non-blocking Lexe probe failure\n${message}`);
      emitGitHubWarning(`${label}/${stdioMode}: ${message}`);
    }

    if (!lexePassed) {
      return { lexePassed };
    }
  }

  return { lexePassed: true };
}

function validateCompatRun(
  label,
  phase,
  result,
  {
    maxDurationMs,
    expectCode,
    successMarker,
    mustNotContainStdout,
    logPath,
    root,
    logParts,
    lexeNotStarted,
  }
) {
  process.stdout.write(result.stdout);
  process.stderr.write(result.stderr);

  const exitLabel = result.timedOut
    ? `timeout after ${maxDurationMs}ms`
    : String(result.code ?? result.signal);
  logParts.push(
    `exit: ${exitLabel}\n\nstdout:\n${result.stdout}\n\nstderr:\n${result.stderr}`
  );

  if (result.timedOut) {
    throw new Error(
      `${label} ${phase} timed out after ${maxDurationMs}ms. ` +
        `See ${path.relative(root, logPath)}.` +
        (lexeNotStarted ? " Lexe was not started." : "")
    );
  }

  if (result.code !== expectCode) {
    throw new Error(
      `${label} ${phase} expected exit ${expectCode}, got ${result.code ?? result.signal}. ` +
        `See ${path.relative(root, logPath)}.` +
        (lexeNotStarted ? " Lexe was not started." : "")
    );
  }

  if (successMarker && !result.stdout.includes(successMarker)) {
    throw new Error(
      `${label} ${phase} exited ${expectCode} but stdout missing "${successMarker}". ` +
        `See ${path.relative(root, logPath)}.` +
        (lexeNotStarted ? " Lexe was not started." : "")
    );
  }

  if (mustNotContainStdout) {
    const haystack = `${result.stdout}\n${result.stderr}`;
    const blocked =
      typeof mustNotContainStdout === "string"
        ? haystack.includes(mustNotContainStdout)
        : mustNotContainStdout.test(haystack);
    if (blocked) {
      throw new Error(
        `${label} ${phase} output must not match teardown guard ${mustNotContainStdout}. ` +
          `See ${path.relative(root, logPath)}.`
      );
    }
  }
}

async function runMysql2ScriptCompat(
  testCase,
  directory,
  lexe,
  logPath,
  root
) {
  const logParts = [];
  let containerId = null;
  let cleanupDone = false;
  let signalExitCode = null;

  const appendContainerDiagnostics = async () => {
    if (!containerId) {
      return;
    }
    const [stateStatus, exitCode, health, ports] = await Promise.all([
      execDocker(["inspect", "--format={{.State.Status}}", containerId]),
      execDocker(["inspect", "--format={{.State.ExitCode}}", containerId]),
      execDocker(["inspect", "--format={{.State.Health.Status}}", containerId]),
      execDocker(["port", containerId, "3306/tcp"]),
    ]);
    logParts.push(
      "\n# container diagnostics\n" +
        `state: ${redactSecrets(stateStatus.stdout.trim())}\n` +
        `exit-code: ${redactSecrets(exitCode.stdout.trim())}\n` +
        `health: ${redactSecrets(health.stdout.trim())}\n` +
        `port: ${redactSecrets(ports.stdout.trim())}`
    );
    const logs = await execDocker(["logs", containerId]);
    logParts.push(
      `\n# docker logs\nstdout:\n${redactSecrets(logs.stdout)}\nstderr:\n${redactSecrets(logs.stderr)}`
    );
  };

  const stopContainer = async (reason) => {
    if (!containerId) {
      return { ok: true, message: "no container to stop" };
    }
    const stopResult = await execDocker(["stop", "--time", "5", containerId]);
    if (stopResult.code !== 0) {
      return {
        ok: false,
        message:
          `docker stop failed (exit ${stopResult.code}, ${reason}): ` +
          `${redactSecrets(stopResult.stderr.trim())}`,
      };
    }
    return { ok: true, message: `stopped (${reason})` };
  };

  const cleanup = async (reason) => {
    if (cleanupDone) {
      return { ok: true, message: "already cleaned up" };
    }
    cleanupDone = true;
    let stopResult;
    try {
      stopResult = await stopContainer(reason);
    } catch (err) {
      stopResult = {
        ok: false,
        message: `docker stop error: ${err instanceof Error ? err.message : String(err)}`,
      };
    }
    logParts.push(`\n# Container cleanup\n${stopResult.message}`);
    try {
      await writeLog(logPath, logParts);
    } catch {
      // do not override the original test error
    }
    return stopResult;
  };

  const onSignal = (signal) => {
    if (signalExitCode !== null) {
      return;
    }
    signalExitCode = signal === "SIGINT" ? 130 : 143;
    cleanup(signal).finally(() => process.exit(signalExitCode));
  };

  process.on("SIGINT", onSignal);
  process.on("SIGTERM", onSignal);

  let testFailed = false;
  try {
    await assertLexeExecutable(lexe, logParts);
    logParts.push(`# Docker\nimage: ${MYSQL_DOCKER_IMAGE}`);

    const dockerVersion = await execDocker([
      "version",
      "--format",
      "{{.Server.Version}}",
    ]);
    if (dockerVersion.code !== 0 || !dockerVersion.stdout.trim()) {
      throw new Error(
        "Docker daemon is not available. Install Docker and ensure the daemon is running." +
          (dockerVersion.stderr.trim()
            ? `\n${dockerVersion.stderr.trim()}`
            : "")
      );
    }
    logParts.push(`docker-server: ${dockerVersion.stdout.trim()}`);

    const runArgs = [
      "run",
      "--detach",
      "--rm",
      "--env",
      "MYSQL_ROOT_PASSWORD=compat-root",
      "--env",
      "MYSQL_DATABASE=lexe_compat",
      "--env",
      "MYSQL_USER=lexe",
      "--env",
      "MYSQL_PASSWORD=lexe",
      "--publish",
      `${MYSQL_DOCKER_HOST}::3306`,
      "--health-cmd=mysqladmin ping -h 127.0.0.1 -uroot -pcompat-root --silent",
      "--health-interval=2s",
      "--health-timeout=5s",
      "--health-retries=30",
      MYSQL_DOCKER_IMAGE,
    ];
    logParts.push(
      `\n# docker run\n$ docker ${redactSecrets(runArgs.join(" "))}`
    );

    const runResult = await execDocker(runArgs, MYSQL_DOCKER_RUN_TIMEOUT_MS);
    if (runResult.timedOut) {
      logParts.push(
        `exit: timeout after ${MYSQL_DOCKER_RUN_TIMEOUT_MS / 1000}s`
      );
      throw new Error(
        `Timed out starting MySQL container after ${MYSQL_DOCKER_RUN_TIMEOUT_MS / 1000}s ` +
          "(image pull or container start may be slow)"
      );
    }
    if (runResult.code !== 0) {
      logParts.push(
        `exit: ${runResult.code}\nstderr:\n${redactSecrets(runResult.stderr)}`
      );
      throw new Error(
        `Failed to start MySQL container: ${redactSecrets(runResult.stderr.trim()) || "unknown error"}`
      );
    }

    containerId = runResult.stdout.trim();
    if (!containerId) {
      throw new Error("docker run produced no container ID");
    }
    logParts.push(`container-id: ${containerId}`);

    const portResult = await execDocker(["port", containerId, "3306/tcp"]);
    const port = parseDockerPort(portResult.stdout);
    if (!port) {
      await appendContainerDiagnostics();
      throw new Error(
        `Failed to parse mapped port from: ${redactSecrets(portResult.stdout.trim() || portResult.stderr.trim())}`
      );
    }
    logParts.push(`host: ${MYSQL_DOCKER_HOST}\nport: ${port}`);

    await waitForMysqlHealthy(
      containerId,
      logParts,
      appendContainerDiagnostics
    );

    const testEnv = {
      ...process.env,
      MYSQL_HOST: MYSQL_DOCKER_HOST,
      MYSQL_PORT: port,
      MYSQL_DATABASE: "lexe_compat",
      MYSQL_USER: "lexe",
      MYSQL_PASSWORD: "lexe",
    };
    logParts.push(
      `\n# Database config\nhost: ${MYSQL_DOCKER_HOST}\nport: ${port}\ndatabase: lexe_compat\nuser: lexe`
    );

    const scripts = testCase.scripts ?? [
      {
        script: testCase.script,
        successMarker: testCase.successMarker,
        maxDurationMs: SCRIPT_TIMEOUT_MS,
        expectCode: 0,
      },
    ];

    for (const spec of scripts) {
      await runCompatScript(
        spec,
        directory,
        lexe,
        logParts,
        root,
        logPath,
        testEnv,
        () => flushMysqlAuthCache(containerId, logParts)
      );
    }

    const compatMode =
      process.env.COMPAT_SKIP_NODE_BASELINE === "1"
        ? "Lexe only"
        : "Node baseline + Lexe";
    console.log(
      `${name} compatibility passed (${scripts.length} script(s), ${compatMode})`
    );
  } catch (err) {
    testFailed = true;
    logParts.push(`\n# Error\n${formatSpawnError(err)}`);
    try {
      await appendContainerDiagnostics();
    } catch {
      // ignore diagnostic failures
    }
    throw err;
  } finally {
    process.off("SIGINT", onSignal);
    process.off("SIGTERM", onSignal);
    if (signalExitCode === null) {
      const cleanupResult = await cleanup(testFailed ? "error" : "complete");
      if (!testFailed && !cleanupResult.ok) {
        throw new Error(
          `MySQL container cleanup failed: ${cleanupResult.message}`
        );
      }
    }
  }
}

async function runPostgresScriptCompat(
  testCase,
  directory,
  lexe,
  logPath,
  root
) {
  const logParts = [];
  let containerId = null;
  let certDir = null;
  let cleanupDone = false;
  let signalExitCode = null;

  const appendContainerDiagnostics = async () => {
    await appendPostgresDiagnostics(containerId, logParts);
  };

  const stopContainer = async (reason) => {
    if (!containerId) {
      return { ok: true, message: "no container to stop" };
    }
    const stopResult = await execDocker(["stop", "--time", "5", containerId]);
    if (stopResult.code !== 0) {
      return {
        ok: false,
        message:
          `docker stop failed (exit ${stopResult.code}, ${reason}): ` +
          `${redactSecrets(stopResult.stderr.trim())}`,
      };
    }
    return { ok: true, message: `stopped (${reason})` };
  };

  const cleanupCerts = async () => {
    if (!certDir) {
      return { ok: true, message: "no certificate directory to remove" };
    }
    const dir = certDir;
    certDir = null;
    try {
      await fs.rm(dir, { recursive: true, force: true });
      logParts.push(`\n# Certificate cleanup\nremoved: ${dir}`);
      return { ok: true, message: `removed cert dir ${dir}` };
    } catch (err) {
      const message =
        `certificate cleanup failed: ${err instanceof Error ? err.message : String(err)}`;
      logParts.push(`\n# Certificate cleanup\n${message}`);
      return { ok: false, message };
    }
  };

  const cleanup = async (reason) => {
    if (cleanupDone) {
      return { ok: true, message: "already cleaned up" };
    }
    cleanupDone = true;
    let stopResult;
    try {
      stopResult = await stopContainer(reason);
    } catch (err) {
      stopResult = {
        ok: false,
        message: `docker stop error: ${err instanceof Error ? err.message : String(err)}`,
      };
    }
    logParts.push(`\n# Container cleanup\n${stopResult.message}`);
    const certResult = await cleanupCerts();
    try {
      await writeLog(logPath, logParts);
    } catch {
      // do not override the original test error
    }
    return {
      ok: stopResult.ok && certResult.ok,
      message: [stopResult.message, certResult.message]
        .filter(Boolean)
        .join("; "),
    };
  };

  const onSignal = (signal) => {
    if (signalExitCode !== null) {
      return;
    }
    signalExitCode = signal === "SIGINT" ? 130 : 143;
    cleanup(signal).finally(() => process.exit(signalExitCode));
  };

  process.on("SIGINT", onSignal);
  process.on("SIGTERM", onSignal);

  let testFailed = false;
  try {
    await assertLexeExecutable(lexe, logParts);
    logParts.push(`# Docker\nimage: ${POSTGRES_DOCKER_IMAGE}`);

    const dockerVersion = await execDocker([
      "version",
      "--format",
      "{{.Server.Version}}",
    ]);
    if (dockerVersion.code !== 0 || !dockerVersion.stdout.trim()) {
      throw new Error(
        "Docker daemon is not available. Install Docker and ensure the daemon is running." +
          (dockerVersion.stderr.trim()
            ? `\n${dockerVersion.stderr.trim()}`
            : "")
      );
    }
    logParts.push(`docker-server: ${dockerVersion.stdout.trim()}`);

    // Separate short-lived CA + leaf server cert so:
    // - rustls can trust the CA (BasicConstraints CA:TRUE) via ssl.ca
    // - the leaf is a valid end-entity cert (not CaUsedAsEndEntity)
    // - Node and Lexe both verify with the exported CA file
    const postgresBootstrap = `
set -eu
umask 077

# Root CA (trust anchor exported to the host as PG_CA_FILE)
openssl req \\
  -x509 \\
  -newkey rsa:2048 \\
  -sha256 \\
  -nodes \\
  -days 1 \\
  -subj /CN=LexeCompatCA \\
  -addext basicConstraints=critical,CA:TRUE,pathlen:0 \\
  -addext keyUsage=critical,keyCertSign,cRLSign \\
  -keyout /tmp/lexe-ca.key \\
  -out /tmp/lexe-ca.crt

# Server leaf key + CSR
openssl req \\
  -newkey rsa:2048 \\
  -sha256 \\
  -nodes \\
  -subj /CN=localhost \\
  -keyout /tmp/lexe-postgres.key \\
  -out /tmp/lexe-postgres.csr

printf '%s\\n' \\
  'subjectAltName=DNS:localhost,IP:127.0.0.1' \\
  'basicConstraints=CA:FALSE' \\
  'keyUsage=digitalSignature,keyEncipherment' \\
  'extendedKeyUsage=serverAuth' \\
  > /tmp/lexe-server-ext.cnf

# Sign leaf with CA
openssl x509 \\
  -req \\
  -in /tmp/lexe-postgres.csr \\
  -CA /tmp/lexe-ca.crt \\
  -CAkey /tmp/lexe-ca.key \\
  -CAcreateserial \\
  -days 1 \\
  -sha256 \\
  -extfile /tmp/lexe-server-ext.cnf \\
  -out /tmp/lexe-postgres.crt

# Publish CA at the path the host copies for client verification
cp /tmp/lexe-ca.crt /tmp/lexe-postgres-ca.crt

chown postgres:postgres \\
  /tmp/lexe-postgres.key \\
  /tmp/lexe-postgres.crt \\
  /tmp/lexe-ca.crt \\
  /tmp/lexe-postgres-ca.crt
chmod 600 /tmp/lexe-postgres.key /tmp/lexe-ca.key

exec docker-entrypoint.sh postgres \\
  -c ssl=on \\
  -c ssl_cert_file=/tmp/lexe-postgres.crt \\
  -c ssl_key_file=/tmp/lexe-postgres.key
`;

    const runArgs = [
      "run",
      "--detach",
      "--rm",
      "--env",
      "POSTGRES_DB=lexe_compat",
      "--env",
      "POSTGRES_USER=lexe",
      "--env",
      "POSTGRES_PASSWORD=lexe-compat-secret",
      "--env",
      "POSTGRES_INITDB_ARGS=--auth-host=scram-sha-256",
      "--publish",
      `${POSTGRES_DOCKER_HOST}::5432`,
      "--health-cmd=pg_isready -U lexe -d lexe_compat",
      "--health-interval=2s",
      "--health-timeout=5s",
      "--health-retries=30",
      POSTGRES_DOCKER_IMAGE,
      "sh",
      "-ceu",
      postgresBootstrap,
    ];
    logParts.push(
      `\n# docker run\n$ docker ${redactSecrets(runArgs.join(" "))}`
    );

    const runResult = await execDocker(runArgs, POSTGRES_DOCKER_RUN_TIMEOUT_MS);
    if (runResult.timedOut) {
      logParts.push(
        `exit: timeout after ${POSTGRES_DOCKER_RUN_TIMEOUT_MS / 1000}s`
      );
      throw new Error(
        `Timed out starting PostgreSQL container after ${POSTGRES_DOCKER_RUN_TIMEOUT_MS / 1000}s ` +
          "(image pull or container start may be slow)"
      );
    }
    if (runResult.code !== 0) {
      logParts.push(
        `exit: ${runResult.code}\nstderr:\n${redactSecrets(runResult.stderr)}`
      );
      throw new Error(
        `Failed to start PostgreSQL container: ${redactSecrets(runResult.stderr.trim()) || "unknown error"}`
      );
    }

    containerId = runResult.stdout.trim();
    if (!containerId) {
      throw new Error("docker run produced no container ID");
    }
    logParts.push(`container-id: ${containerId}`);

    const portResult = await execDocker(["port", containerId, "5432/tcp"]);
    const port = parseDockerPort(portResult.stdout);
    if (!port) {
      await appendContainerDiagnostics();
      throw new Error(
        `Failed to parse mapped port from: ${redactSecrets(portResult.stdout.trim() || portResult.stderr.trim())}`
      );
    }
    logParts.push(`host: ${POSTGRES_DOCKER_HOST}\nport: ${port}`);

    await waitForPostgresHealthy(
      containerId,
      logParts,
      appendContainerDiagnostics
    );

    certDir = await fs.mkdtemp(
      path.join(os.tmpdir(), "lexe-node-postgres-")
    );
    const caPath = path.join(certDir, "postgres-ca.crt");
    await copyPostgresCa(containerId, caPath, logParts);

    // libpq-style password file for pgpass / PGPASSFILE tests.
    // Format: hostname:port:database:username:password
    const pgpassPath = path.join(certDir, ".pgpass");
    const pgpassLine = [
      POSTGRES_DOCKER_HOST,
      port,
      "lexe_compat",
      "lexe",
      "lexe-compat-secret",
    ].join(":");
    await fs.writeFile(pgpassPath, `${pgpassLine}\n`, { mode: 0o600 });
    // Ensure mode even when umask interfered with writeFile mode on some hosts.
    await fs.chmod(pgpassPath, 0o600);

    const testEnv = {
      ...process.env,
      PGHOST: POSTGRES_DOCKER_HOST,
      PGPORT: port,
      PGDATABASE: "lexe_compat",
      PGUSER: "lexe",
      PGPASSWORD: "lexe-compat-secret",
      PG_CA_FILE: caPath,
      PGPASSFILE: pgpassPath,
    };
    logParts.push(
      `\n# Database config\nhost: ${POSTGRES_DOCKER_HOST}\nport: ${port}\ndatabase: lexe_compat\nuser: lexe\nca: ${caPath}\npgpass: ${pgpassPath}`
    );

    const scripts = testCase.scripts ?? [
      {
        script: testCase.script,
        successMarker: testCase.successMarker,
        maxDurationMs: SCRIPT_TIMEOUT_MS,
        expectCode: 0,
        allowLexeFailure: testCase.allowLexeFailure === true,
      },
    ];

    let lexePassed = true;
    for (const spec of scripts) {
      const result = await runCompatScript(
        spec,
        directory,
        lexe,
        logParts,
        root,
        logPath,
        testEnv,
        () => resetPostgresSchema(containerId, logParts)
      );
      if (result && result.lexePassed === false) {
        lexePassed = false;
      }
    }

    const compatMode =
      process.env.COMPAT_SKIP_NODE_BASELINE === "1"
        ? "Lexe only"
        : "Node baseline + Lexe";
    if (lexePassed) {
      console.log(
        `${name} compatibility passed (${scripts.length} script(s), ${compatMode})`
      );
    } else {
      console.log(
        "node-postgres compatibility probe completed with Lexe failures"
      );
    }
  } catch (err) {
    testFailed = true;
    logParts.push(`\n# Error\n${formatSpawnError(err)}`);
    try {
      await appendContainerDiagnostics();
    } catch {
      // ignore diagnostic failures
    }
    throw err;
  } finally {
    process.off("SIGINT", onSignal);
    process.off("SIGTERM", onSignal);
    if (signalExitCode === null) {
      const cleanupResult = await cleanup(testFailed ? "error" : "complete");
      if (!testFailed && !cleanupResult.ok) {
        throw new Error(
          `PostgreSQL cleanup failed: ${cleanupResult.message}`
        );
      }
    }
  }
}

async function assertLexeExecutable(lexePath, logParts) {
  logParts.push(`\n# Lexe preflight\nlexe: ${lexePath}`);
  try {
    const stat = await fs.stat(lexePath);
    if (!stat.isFile()) {
      throw new Error(`Lexe runtime is not a file: ${lexePath}`);
    }
    await fs.access(lexePath, fsConstants.X_OK);
  } catch (err) {
    const code =
      err && typeof err === "object" && "code" in err ? err.code : null;
    if (code === "ENOENT" || code === "ENOTDIR") {
      throw new Error(
        `Lexe runtime not found: ${lexePath}\n` +
          "Build it first with: cargo build"
      );
    }
    if (code === "EACCES") {
      throw new Error(
        `Lexe runtime is not executable: ${lexePath}\n` +
          "Build it first with: cargo build"
      );
    }
    throw err;
  }
}

async function spawnCompatChild(command, args, options, timeoutMs, logParts) {
  try {
    return await spawnCollect(command, args, options, timeoutMs);
  } catch (err) {
    const detail = formatSpawnError(err);
    logParts.push(`exit: spawn error\n\nstdout:\n\nstderr:\n${detail}`);
    throw err;
  }
}

function formatSpawnError(err) {
  if (err instanceof Error && err.stack) {
    return err.stack;
  }
  return String(err);
}

function redactSecrets(text) {
  if (!text) {
    return "";
  }
  return String(text)
    .replace(/([A-Za-z_]*PASSWORD=)[^\s"']*/gi, "$1***")
    .replace(/(^|\s)-p\S+/gm, "$1-p***");
}

async function flushMysqlAuthCache(containerId, logParts) {
  const flushArgs = [
    "exec",
    containerId,
    "mysql",
    "-uroot",
    "-pcompat-root",
    "-e",
    "FLUSH PRIVILEGES;",
  ];
  const loggedCmd = redactSecrets(`docker ${flushArgs.join(" ")}`);
  logParts.push(
    `\n# Flush authentication cache before Lexe run\n$ ${loggedCmd}`
  );
  console.log(
    "[compat-mysql2] flushing authentication cache before Lexe run"
  );

  const result = await execDocker(flushArgs);
  logParts.push(
    `exit: ${result.code ?? result.signal}\n\nstdout:\n${redactSecrets(result.stdout)}\n\nstderr:\n${redactSecrets(result.stderr)}`
  );
  if (result.code !== 0) {
    throw new Error(
      `Failed to flush MySQL authentication cache: ${redactSecrets(result.stderr.trim()) || "unknown error"}`
    );
  }
}

async function resetPostgresSchema(containerId, logParts) {
  const sql =
    "DROP SCHEMA public CASCADE; CREATE SCHEMA public; GRANT ALL ON SCHEMA public TO lexe;";
  const resetArgs = [
    "exec",
    containerId,
    "psql",
    "-v",
    "ON_ERROR_STOP=1",
    "-U",
    "lexe",
    "-d",
    "lexe_compat",
    "-c",
    sql,
  ];
  const loggedCmd = redactSecrets(`docker ${resetArgs.join(" ")}`);
  logParts.push(
    `\n# Reset public schema before Lexe run\n$ ${loggedCmd}`
  );
  console.log("[compat-node-postgres] resetting public schema before Lexe run");

  const result = await execDocker(resetArgs);
  logParts.push(
    `exit: ${result.code ?? result.signal}\n\nstdout:\n${redactSecrets(result.stdout)}\n\nstderr:\n${redactSecrets(result.stderr)}`
  );
  if (result.code !== 0) {
    throw new Error(
      `Failed to reset PostgreSQL schema: ${redactSecrets(result.stderr.trim()) || "unknown error"}`
    );
  }
}

async function copyPostgresCa(containerId, caPath, logParts) {
  // Prefer the dedicated CA path; fall back to legacy single-cert layout.
  const copyArgs = [
    "cp",
    `${containerId}:/tmp/lexe-postgres-ca.crt`,
    caPath,
  ];
  logParts.push(
    `\n# Export PostgreSQL CA\n$ docker ${copyArgs.join(" ")}`
  );
  const result = await execDocker(copyArgs);
  logParts.push(
    `exit: ${result.code ?? result.signal}\n\nstdout:\n${redactSecrets(result.stdout)}\n\nstderr:\n${redactSecrets(result.stderr)}`
  );
  if (result.code !== 0) {
    throw new Error(
      `Failed to copy PostgreSQL CA: ${redactSecrets(result.stderr.trim()) || "unknown error"}`
    );
  }
}

async function appendPostgresDiagnostics(containerId, logParts) {
  if (!containerId) {
    return;
  }
  const [stateStatus, exitCode, health, ports] = await Promise.all([
    execDocker(["inspect", "--format={{.State.Status}}", containerId]),
    execDocker(["inspect", "--format={{.State.ExitCode}}", containerId]),
    execDocker(["inspect", "--format={{.State.Health.Status}}", containerId]),
    execDocker(["port", containerId, "5432/tcp"]),
  ]);
  logParts.push(
    "\n# container diagnostics\n" +
      `state: ${redactSecrets(stateStatus.stdout.trim())}\n` +
      `exit-code: ${redactSecrets(exitCode.stdout.trim())}\n` +
      `health: ${redactSecrets(health.stdout.trim())}\n` +
      `port: ${redactSecrets(ports.stdout.trim())}`
  );
  const logs = await execDocker(["logs", containerId]);
  logParts.push(
    `\n# docker logs\nstdout:\n${redactSecrets(logs.stdout)}\nstderr:\n${redactSecrets(logs.stderr)}`
  );
}

function emitGitHubWarning(message) {
  const escaped = String(message)
    .replace(/%/g, "%25")
    .replace(/\r/g, "%0D")
    .replace(/\n/g, "%0A");
  console.warn(
    `::warning title=node-postgres Lexe compatibility::${escaped}`
  );
}

function execDocker(args, timeoutMs = 60_000) {
  return spawnCollect("docker", args, { env: { ...process.env } }, timeoutMs);
}

function parseDockerPort(output) {
  const line = output.trim().split("\n")[0]?.trim();
  if (!line) {
    return null;
  }
  const match = /^127\.0\.0\.1:(\d+)$/.exec(line);
  return match ? match[1] : null;
}

async function waitForMysqlHealthy(containerId, logParts, appendDiagnostics) {
  const deadline = Date.now() + MYSQL_HEALTH_TIMEOUT_MS;
  let lastStatus = null;

  while (Date.now() < deadline) {
    const stateResult = await execDocker([
      "inspect",
      "--format={{.State.Status}}",
      containerId,
    ]);
    if (stateResult.code !== 0) {
      await appendDiagnostics();
      throw new Error("MySQL container no longer exists");
    }

    const stateStatus = stateResult.stdout.trim();
    if (stateStatus === "exited") {
      await appendDiagnostics();
      throw new Error("MySQL container exited before becoming healthy");
    }

    const healthResult = await execDocker([
      "inspect",
      "--format={{.State.Health.Status}}",
      containerId,
    ]);
    const status = healthResult.stdout.trim();
    if (status && status !== lastStatus) {
      logParts.push(`health: ${status}`);
      lastStatus = status;
    }

    if (status === "healthy") {
      return;
    }
    if (status === "unhealthy") {
      await appendDiagnostics();
      throw new Error("MySQL container became unhealthy");
    }

    await sleep(1_000);
  }

  await appendDiagnostics();
  throw new Error(
    `MySQL container health check timed out after ${MYSQL_HEALTH_TIMEOUT_MS / 1000}s`
  );
}

async function waitForPostgresHealthy(containerId, logParts, appendDiagnostics) {
  const deadline = Date.now() + POSTGRES_HEALTH_TIMEOUT_MS;
  let lastStatus = null;

  while (Date.now() < deadline) {
    const stateResult = await execDocker([
      "inspect",
      "--format={{.State.Status}}",
      containerId,
    ]);
    if (stateResult.code !== 0) {
      await appendDiagnostics();
      throw new Error("PostgreSQL container no longer exists");
    }

    const stateStatus = stateResult.stdout.trim();
    if (stateStatus === "exited") {
      await appendDiagnostics();
      throw new Error("PostgreSQL container exited before becoming healthy");
    }

    const healthResult = await execDocker([
      "inspect",
      "--format={{.State.Health.Status}}",
      containerId,
    ]);
    const status = healthResult.stdout.trim();
    if (status && status !== lastStatus) {
      logParts.push(`health: ${status}`);
      lastStatus = status;
    }

    if (status === "healthy") {
      return;
    }
    if (status === "unhealthy") {
      await appendDiagnostics();
      throw new Error("PostgreSQL container became unhealthy");
    }

    await sleep(1_000);
  }

  await appendDiagnostics();
  throw new Error(
    `PostgreSQL container health check timed out after ${POSTGRES_HEALTH_TIMEOUT_MS / 1000}s`
  );
}

async function runNextStandalone(directory, lexe, logPath, root) {
  const logParts = [];
  const outputDir = path.join(directory, ".next");
  const standaloneDir = path.join(outputDir, "standalone");
  const serverEntry = path.join(standaloneDir, "server.js");
  const nextCli = path.join(directory, "node_modules/next/dist/bin/next");

  await fs.rm(outputDir, { recursive: true, force: true });

  // --- Phase 1: Node builds standalone (not Lexe) ---
  const buildCmd = `${process.execPath} ${nextCli} build`;
  logParts.push(`# Node build\n$ ${buildCmd}`);
  console.log(`[compat-next] building with system Node: ${buildCmd}`);

  const buildResult = await spawnCollect(
    process.execPath,
    [nextCli, "build"],
    {
      cwd: directory,
      env: {
        ...process.env,
        NEXT_TELEMETRY_DISABLED: "1",
        NODE_ENV: "production",
      },
    },
    BUILD_TIMEOUT_MS
  );

  const buildExitLabel = buildResult.timedOut
    ? `timeout after ${BUILD_TIMEOUT_MS}ms`
    : String(buildResult.code ?? buildResult.signal);
  logParts.push(
    `exit: ${buildExitLabel}\n\nstdout:\n${buildResult.stdout}\n\nstderr:\n${buildResult.stderr}`
  );
  process.stdout.write(buildResult.stdout);
  process.stderr.write(buildResult.stderr);

  if (buildResult.timedOut) {
    await writeLog(logPath, logParts);
    throw new Error(
      `Next Node build timed out after ${BUILD_TIMEOUT_MS}ms. ` +
        `See ${path.relative(root, logPath)} for Node build stdout/stderr. ` +
        `Lexe was not started.`
    );
  }

  if (buildResult.code !== 0) {
    await writeLog(logPath, logParts);
    throw new Error(
      `Next Node build failed (exit ${buildResult.code ?? buildResult.signal}). ` +
        `See ${path.relative(root, logPath)} for Node build stdout/stderr. ` +
        `Lexe was not started.`
    );
  }

  if (!(await pathExists(serverEntry))) {
    await writeLog(logPath, logParts);
    throw new Error(
      `Next Node build succeeded but missing standalone entry: ${path.relative(root, serverEntry)}. ` +
        `Ensure next.config has output: "standalone". See ${path.relative(root, logPath)}.`
    );
  }

  logParts.push(
    `\n# Standalone entry present\n${path.relative(root, serverEntry)}`
  );

  // --- Phase 2: Lexe runs standalone server ---
  const port = await getFreePort();
  const host = "127.0.0.1";
  const baseUrl = `http://${host}:${port}`;
  const serverEnv = {
    ...process.env,
    HOSTNAME: host,
    PORT: String(port),
    NODE_ENV: "production",
    NEXT_TELEMETRY_DISABLED: "1",
  };

  const startCmd = `${lexe} ${serverEntry}`;
  logParts.push(
    `\n# Lexe start\n$ ${startCmd}\n` +
      `cwd: ${standaloneDir}\n` +
      `HOSTNAME=${host} PORT=${port} NODE_ENV=production NEXT_TELEMETRY_DISABLED=1`
  );
  console.log(`[compat-next] starting with Lexe on ${baseUrl}`);

  let server = null;
  let serverExit = null;
  let stdout = "";
  let stderr = "";
  let healthResults = [];
  let teardownFailure = null;

  try {
    server = spawn(lexe, [serverEntry], {
      cwd: standaloneDir,
      env: serverEnv,
      stdio: ["ignore", "pipe", "pipe"],
    });

    server.stdout.on("data", (chunk) => {
      stdout += chunk;
      process.stdout.write(chunk);
    });
    server.stderr.on("data", (chunk) => {
      stderr += chunk;
      process.stderr.write(chunk);
    });
    server.on("error", (err) => {
      serverExit = { code: null, signal: null, error: err };
    });
    server.on("close", (code, signal) => {
      if (!serverExit) {
        serverExit = { code, signal, error: null };
      }
    });

    // Wait for readiness via health endpoint (not console text)
    const ready = await waitForReady({
      url: `${baseUrl}/api/health`,
      timeoutMs: READINESS_TIMEOUT_MS,
      isExited: () => serverExit !== null,
      getExit: () => serverExit,
    });

    if (!ready.ok) {
      const lastErrorLine = ready.lastError
        ? `last readiness error: ${ready.lastError}\n`
        : "last readiness error: (none recorded)\n";
      logParts.push(
        `\n# Lexe early exit / readiness\n` +
          `reason: ${ready.reason}\n` +
          lastErrorLine +
          `exit: ${formatExit(serverExit)}\n\n` +
          `stdout:\n${stdout}\n\nstderr:\n${stderr}`
      );
      await writeLog(logPath, logParts);
      if (ready.reason === "exited") {
        throw new Error(
          `Lexe exited before listening (${formatExit(serverExit)}). ` +
            (ready.lastError
              ? `Last readiness error: ${ready.lastError}. `
              : "") +
            `See ${path.relative(root, logPath)} for Lexe stdout/stderr.`
        );
      }
      throw new Error(
        `Lexe standalone server readiness timed out after ${READINESS_TIMEOUT_MS / 1000}s ` +
          `(health ${baseUrl}/api/health). ` +
          (ready.lastError
            ? `Last readiness error: ${ready.lastError}. `
            : "") +
          `See ${path.relative(root, logPath)} for Lexe stdout/stderr.`
      );
    }

    logParts.push(
      `\n# Lexe ready\nhealth: ${baseUrl}/api/health OK\n\nstdout so far:\n${stdout}\n\nstderr so far:\n${stderr}`
    );

    // --- Phase 3: HTTP assertions ---
    const checks = [
      {
        name: "GET /",
        path: "/",
        expectStatus: 200,
        assert: (body) => {
          if (!body.includes("Lexe Next compatibility fixture")) {
            return 'body missing "Lexe Next compatibility fixture"';
          }
          return null;
        },
      },
      {
        name: "GET /api/health",
        path: "/api/health",
        expectStatus: 200,
        assert: (body) => {
          let json;
          try {
            json = JSON.parse(body);
          } catch {
            return `invalid JSON: ${truncate(body, 200)}`;
          }
          if (json?.status !== "ok") {
            return `expected { "status": "ok" }, got ${JSON.stringify(json)}`;
          }
          return null;
        },
      },
      {
        name: "GET /posts/42",
        path: "/posts/42",
        expectStatus: 200,
        assert: (body) => {
          // Next SSR may insert HTML comments between text nodes ("Post <!-- -->42").
          const normalized = body.replace(/<!--[\s\S]*?-->/g, "");
          if (!normalized.includes("Post 42")) {
            return 'body missing "Post 42" (after stripping HTML comments)';
          }
          return null;
        },
      },
    ];

    const failures = [];
    for (const check of checks) {
      if (serverExit) {
        failures.push(
          `${check.name}: skipped (Lexe already exited: ${formatExit(serverExit)})`
        );
        logParts.push(
          `\n# HTTP ${check.name}\nskipped: Lexe exited ${formatExit(serverExit)}`
        );
        continue;
      }

      try {
        // One AbortSignal covers both fetch headers and body consumption.
        const signal = AbortSignal.timeout(HTTP_CHECK_TIMEOUT_MS);
        const res = await fetch(`${baseUrl}${check.path}`, { signal });
        const body = await res.text();
        const statusOk = res.status === check.expectStatus;
        const assertMsg = statusOk ? check.assert(body) : null;
        const ok = statusOk && !assertMsg;

        logParts.push(
          `\n# HTTP ${check.name}\n` +
            `url: ${baseUrl}${check.path}\n` +
            `status: ${res.status} (expected ${check.expectStatus})\n` +
            `ok: ${ok}\n` +
            (assertMsg ? `assert: ${assertMsg}\n` : "") +
            `body:\n${truncate(body, 2000)}`
        );

        if (!statusOk) {
          failures.push(
            `${check.name}: expected status ${check.expectStatus}, got ${res.status}`
          );
        } else if (assertMsg) {
          failures.push(`${check.name}: ${assertMsg}`);
        }
      } catch (err) {
        const msg = err instanceof Error ? err.message : String(err);
        logParts.push(`\n# HTTP ${check.name}\nerror: ${msg}`);
        failures.push(`${check.name}: request failed: ${msg}`);
      }
    }

    // Concurrent AsyncLocalStorage isolation across overlapping requests.
    if (!serverExit) {
      const alsIds = ["a1", "b2", "c3", "d4", "e5", "f6"];
      try {
        const results = await Promise.all(
          alsIds.map(async (id) => {
            const signal = AbortSignal.timeout(HTTP_CHECK_TIMEOUT_MS);
            const res = await fetch(`${baseUrl}/api/als/${id}`, { signal });
            const body = await res.text();
            let json;
            try {
              json = JSON.parse(body);
            } catch {
              return {
                id,
                ok: false,
                detail: `invalid JSON status=${res.status} body=${truncate(body, 200)}`,
              };
            }
            const ok = res.status === 200 && json?.id === id;
            return {
              id,
              ok,
              detail: ok
                ? null
                : `status=${res.status} body=${JSON.stringify(json)}`,
            };
          })
        );

        const alsLog = results
          .map((r) => `  ${r.id}: ${r.ok ? "ok" : r.detail}`)
          .join("\n");
        logParts.push(`\n# HTTP concurrent ALS\n${alsLog}`);

        for (const r of results) {
          if (!r.ok) {
            failures.push(`GET /api/als/${r.id}: ${r.detail}`);
          }
        }
      } catch (err) {
        const msg = err instanceof Error ? err.message : String(err);
        logParts.push(`\n# HTTP concurrent ALS\nerror: ${msg}`);
        failures.push(`concurrent ALS: request failed: ${msg}`);
      }
    } else {
      failures.push(
        `concurrent ALS: skipped (Lexe already exited: ${formatExit(serverExit)})`
      );
      logParts.push(
        `\n# HTTP concurrent ALS\nskipped: Lexe exited ${formatExit(serverExit)}`
      );
    }

    // Capture any additional server output after requests
    logParts.push(
      `\n# Lexe output after HTTP checks\nstdout:\n${stdout}\n\nstderr:\n${stderr}\n` +
        `exit so far: ${formatExit(serverExit)}`
    );

    await writeLog(logPath, logParts);

    if (failures.length > 0) {
      throw new Error(
        `Next standalone runtime checks failed:\n  - ${failures.join("\n  - ")}\n` +
          `See ${path.relative(root, logPath)} for full diagnostics.`
      );
    }

    const healthLoops = positiveLoopCount("COMPAT_NEXT_HEALTH_LOOPS", 20);
    for (let i = 0; i < healthLoops; i++) {
      if (serverExit) {
        failures.push(`health loop ${i + 1}: skipped (Lexe already exited)`);
        healthResults.push({
          loop: i + 1,
          skipped: true,
          reason: `Lexe exited ${formatExit(serverExit)}`,
        });
        break;
      }
      try {
        const signal = AbortSignal.timeout(HTTP_CHECK_TIMEOUT_MS);
        const res = await fetch(`${baseUrl}/api/health`, { signal });
        const body = await res.text();
        healthResults.push({ loop: i + 1, status: res.status, body });
        if (res.status !== 200) {
          failures.push(
            `health loop ${i + 1}: expected status 200, got ${res.status}`
          );
          break;
        }
        let json;
        try {
          json = JSON.parse(body);
        } catch {
          failures.push(`health loop ${i + 1}: invalid JSON`);
          break;
        }
        if (json?.status !== "ok") {
          failures.push(
            `health loop ${i + 1}: expected { "status": "ok" }, got ${JSON.stringify(json)}`
          );
          break;
        }
      } catch (err) {
        const msg = err instanceof Error ? err.message : String(err);
        healthResults.push({ loop: i + 1, error: msg });
        failures.push(`health loop ${i + 1}: request failed: ${msg}`);
        break;
      }
    }

    logParts.push(
      `\n# Health probe results (${healthResults.length}/${healthLoops})\n` +
        healthResults
          .map((entry) => {
            if (entry.skipped) {
              return `loop ${entry.loop}: skipped (${entry.reason})`;
            }
            if (entry.error) {
              return `loop ${entry.loop}: error ${entry.error}`;
            }
            return `loop ${entry.loop}: status ${entry.status} body ${entry.body}`;
          })
          .join("\n")
    );
    await writeLog(logPath, logParts);

    if (failures.length > 0) {
      throw new Error(
        `Next standalone runtime checks failed:\n  - ${failures.join("\n  - ")}\n` +
          `See ${path.relative(root, logPath)} for full diagnostics.`
      );
    }

    console.log(
      "next compatibility standalone runtime passed " +
        `(Node build + Lexe run; HTTP / /api/health /posts/42 + concurrent ALS + ${healthLoops}x health OK)`
    );
  } finally {
    if (server) {
      await stopProcess(server, SERVER_STOP_TIMEOUT_MS);
      logParts.push(
        `\n# Server cleanup\nsent SIGTERM then SIGKILL if needed; final exit: ${formatExit(serverExit)}`
      );
      const output = `${stdout}\n${stderr}`;
      if (TEARDOWN_GUARD.test(output)) {
        teardownFailure = new Error(
          `Next standalone teardown output contains abort/assert residual. ` +
            `See ${path.relative(root, logPath)}.`
        );
      }
      try {
        await writeLog(logPath, logParts);
      } catch {
        // ignore secondary log write failures during cleanup
      }
    }
  }

  if (teardownFailure) {
    throw teardownFailure;
  }
}

/**
 * Spawn a process and verify it is still running at `checkMs`, then kill it.
 */
function spawnStillRunning(command, args, options, checkMs, killAfterMs) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      ...options,
      stdio: ["ignore", "pipe", "pipe"],
    });
    let stdout = "";
    let stderr = "";
    let exited = false;
    let exitCode = null;

    child.stdout.on("data", (chunk) => (stdout += chunk));
    child.stderr.on("data", (chunk) => (stderr += chunk));
    child.on("error", reject);
    child.on("close", (code) => {
      exited = true;
      exitCode = code;
    });

    setTimeout(() => {
      if (exited) {
        reject(
          new Error(
            `process exited before ${checkMs}ms (exit ${exitCode ?? "signal"})`
          )
        );
        return;
      }
      const killDelay = Math.max(0, killAfterMs - checkMs);
      setTimeout(() => {
        try {
          child.kill("SIGTERM");
        } catch {
          // ignore
        }
        setTimeout(() => {
          resolve({ stdout, stderr, stillAliveAtCheck: true });
        }, 100);
      }, killDelay);
    }, checkMs);
  });
}

/**
 * Spawn with stdout/stderr redirected to temp files (not shell pipes).
 */
async function spawnRedirected(command, args, options, timeoutMs) {
  const stdoutPath = path.join(
    os.tmpdir(),
    `lexe-stdout-${process.pid}-${Date.now()}.log`
  );
  const stderrPath = path.join(
    os.tmpdir(),
    `lexe-stderr-${process.pid}-${Date.now()}.log`
  );

  const stdout = fsSync.openSync(stdoutPath, "w");
  const stderr = fsSync.openSync(stderrPath, "w");

  try {
    const result = await spawnCollect(
      command,
      args,
      {
        ...options,
        stdio: ["ignore", stdout, stderr],
      },
      timeoutMs
    );

    return {
      ...result,
      stdout: await fs.readFile(stdoutPath, "utf8"),
      stderr: await fs.readFile(stderrPath, "utf8"),
    };
  } finally {
    fsSync.closeSync(stdout);
    fsSync.closeSync(stderr);
    await fs.rm(stdoutPath, { force: true });
    await fs.rm(stderrPath, { force: true });
  }
}

/**
 * Spawn a process, collect stdout/stderr, optionally enforce a wall-clock timeout.
 * On timeout: SIGTERM, then SIGKILL after a short grace period.
 * Returns { code, signal, stdout, stderr, timedOut }.
 */
function spawnCollect(command, args, options, timeoutMs = 0) {
  return new Promise((resolve, reject) => {
    const stdio = options.stdio ?? ["ignore", "pipe", "pipe"];

    const child = spawn(command, args, {
      ...options,
      stdio,
    });
    let stdout = "";
    let stderr = "";
    let timedOut = false;
    let settled = false;
    let killTimer = null;

    const finish = (code, signal) => {
      if (settled) return;
      settled = true;
      if (timer) clearTimeout(timer);
      if (killTimer) clearTimeout(killTimer);
      resolve({ code, signal, stdout, stderr, timedOut });
    };

    child.stdout?.on("data", (chunk) => {
      stdout += chunk;
    });
    child.stderr?.on("data", (chunk) => {
      stderr += chunk;
    });
    child.on("error", (err) => {
      if (settled) return;
      settled = true;
      if (timer) clearTimeout(timer);
      if (killTimer) clearTimeout(killTimer);
      reject(err);
    });
    child.on("close", (code, signal) => finish(code, signal));

    let timer = null;
    if (timeoutMs > 0) {
      timer = setTimeout(() => {
        timedOut = true;
        try {
          child.kill("SIGTERM");
        } catch {
          // ignore
        }
        killTimer = setTimeout(() => {
          try {
            if (child.exitCode === null && child.signalCode === null) {
              child.kill("SIGKILL");
            }
          } catch {
            // ignore
          }
        }, 2_000);
      }, timeoutMs);
    }
  });
}

function getFreePort() {
  return new Promise((resolve, reject) => {
    const server = net.createServer();
    server.listen(0, "127.0.0.1", () => {
      const address = server.address();
      const port = typeof address === "object" && address ? address.port : 0;
      server.close((err) => {
        if (err) reject(err);
        else resolve(port);
      });
    });
    server.on("error", reject);
  });
}

async function waitForReady({ url, timeoutMs, isExited, getExit }) {
  const deadline = Date.now() + timeoutMs;
  let lastError = null;

  while (Date.now() < deadline) {
    if (isExited()) {
      return { ok: false, reason: "exited", exit: getExit(), lastError };
    }
    try {
      const res = await fetch(url, {
        signal: AbortSignal.timeout(READINESS_REQUEST_TIMEOUT_MS),
      });
      if (res.ok) {
        return { ok: true };
      }
      lastError = `status ${res.status}`;
    } catch (err) {
      lastError = err instanceof Error ? err.message : String(err);
    }
    await sleep(250);
  }

  if (isExited()) {
    return { ok: false, reason: "exited", exit: getExit(), lastError };
  }
  return { ok: false, reason: "timeout", lastError };
}

function stopProcess(child, timeoutMs) {
  return new Promise((resolve) => {
    if (child.exitCode !== null || child.signalCode !== null) {
      resolve();
      return;
    }

    let settled = false;
    const done = () => {
      if (settled) return;
      settled = true;
      resolve();
    };

    child.once("close", done);

    try {
      child.kill("SIGTERM");
    } catch {
      done();
      return;
    }

    const timer = setTimeout(() => {
      try {
        if (child.exitCode === null && child.signalCode === null) {
          child.kill("SIGKILL");
        }
      } catch {
        // ignore
      }
      // Give SIGKILL a moment, then resolve either way
      setTimeout(done, 500);
    }, timeoutMs);

    child.once("close", () => clearTimeout(timer));
  });
}

async function pathExists(p) {
  try {
    await fs.access(p);
    return true;
  } catch {
    return false;
  }
}

async function writeLog(logPath, parts) {
  await fs.writeFile(logPath, parts.join("\n") + "\n");
}

function formatExit(exit) {
  if (!exit) return "still running";
  if (exit.error) return `spawn error: ${exit.error.message}`;
  if (exit.signal) return `signal ${exit.signal}`;
  return `code ${exit.code}`;
}

function truncate(text, max) {
  if (text.length <= max) return text;
  return text.slice(0, max) + `\n... (${text.length - max} more bytes)`;
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}
