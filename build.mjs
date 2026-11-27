import * as esbuild from "esbuild";
import fs from "node:fs/promises";
import path from "node:path";

process.env.NODE_PATH = ".";

const SRC_DIR = path.join("lexe_core", "src", "modules", "js");
const TESTS_DIR = "tests";
const TESTS_SUB_DIR = process.env.TEST_SUB_DIR || "unit";
const OUT_DIR = "bundle/js";
const MINIFY_JS = process.env.JS_MINIFY !== "0";
const ENTRYPOINTS = [
  "stream",
  "stream/consumers",
  "stream/promises",
  "readline",
  "readline/promises",
  "worker_threads",
  "http2",
  "test",
  "@lexe/test/index",
  "@lexe/test/worker",
];

async function readFilesRecursive(dir, filePredicate) {
  const dirents = await fs.readdir(dir, { withFileTypes: true });
  const files = await Promise.all(
    dirents.map((dirent) => {
      const filePath = path.join(dir, dirent.name);

      if (dirent.isDirectory()) {
        return readFilesRecursive(filePath, filePredicate);
      }
      return filePredicate(filePath) ? filePath : [];
    })
  );
  return Array.prototype.concat(...files);
}

const TEST_FILES = await readFilesRecursive(
  path.join(TESTS_DIR, TESTS_SUB_DIR),
  (filePath) =>
    filePath.endsWith(".test.ts") ||
    filePath.endsWith(".test.lexe.ts") ||
    filePath.endsWith(".spec.ts") ||
    filePath.endsWith(".any.js")
);

const ES_BUILD_OPTIONS = {
  splitting: MINIFY_JS,
  minify: MINIFY_JS,
  sourcemap: false,
  target: "es2023",
  outdir: OUT_DIR,
  bundle: true,
  logLevel: "info",
  platform: "browser",
  format: "esm",
  external: [
    "assert",
    "node:assert",
    "async_hooks",
    "node:async_hooks",
    "buffer",
    "node:buffer",
    "child_process",
    "node:child_process",
    "console",
    "node:console",
    "crypto",
    "node:crypto",
    "dgram",
    "node:dgram",
    "dns",
    "node:dns",
    "events",
    "node:events",
    "fs",
    "node:fs",
    "module",
    "node:module",
    "net",
    "node:net",
    "os",
    "node:os",
    "path",
    "node:path",
    "path/posix",
    "node:path/posix",
    "path/win32",
    "node:path/win32",
    "perf_hooks",
    "node:perf_hooks",
    "process",
    "node:process",
    "stream",
    "node:stream",
    "stream/consumers",
    "node:stream/consumers",
    "stream/promises",
    "node:stream/promises",
    "stream/web",
    "node:stream/web",
    "readline",
    "node:readline",
    "readline/promises",
    "node:readline/promises",
    "sqlite",
    "node:sqlite",
    "node:sea",
    "node:ffi",
    "worker_threads",
    "node:worker_threads",
    "http2",
    "node:http2",
    "test",
    "node:test",
    "querystring",
    "node:querystring",
    "diagnostics_channel",
    "node:diagnostics_channel",
    "string_decoder",
    "node:string_decoder",
    "timers",
    "node:timers",
    "timers/promises",
    "node:timers/promises",
    "inspector",
    "node:inspector",
    "tls",
    "node:tls",
    "http",
    "node:http",
    "https",
    "node:https",
    "tty",
    "node:tty",
    "url",
    "node:url",
    "util",
    "node:util",
    "v8",
    "node:v8",
    "wasi",
    "node:wasi",
    "constants",
    "node:constants",
    "vm",
    "node:vm",
    "zlib",
    "node:zlib",
    "lexe:hex",
    "lexe:timezone",
    "lexe:util",
    "lexe:qjs",
    "lexe:xml",
  ],
};

const requireProcessPlugin = {
  name: "require-process",
  setup(build) {
    build.onResolve({ filter: /^process\/$/ }, () => {
      return { path: "process", external: true };
    });
  },
};

async function createOutputDirectories() {
  await fs.rm(OUT_DIR, { recursive: true, force: true });
  await fs.mkdir(OUT_DIR, { recursive: true });
}

async function buildLibrary() {
  const defaultLibEsBuildOption = {
    chunkNames: "lexe-[name]-runtime-[hash]",
    ...ES_BUILD_OPTIONS,
    splitting: false,
    keepNames: true,
    nodePaths: ["."],
  };

  const entryPoints = {};
  ENTRYPOINTS.forEach((entry) => {
    entryPoints[entry] = path.join(SRC_DIR, entry);
  });
  await esbuild.build({
    ...defaultLibEsBuildOption,
    entryPoints,
    plugins: [requireProcessPlugin],
  });

  const testEntryPoints = TEST_FILES.reduce((acc, entry) => {
    const { name, dir } = path.parse(entry);
    const parentDir = path.basename(dir);
    acc[path.join("__tests__", parentDir, name)] = entry;
    return acc;
  }, {});

  await esbuild.build({
    ...defaultLibEsBuildOption,
    entryPoints: testEntryPoints,
  });
}

console.log("Building...");

await createOutputDirectories();
await buildLibrary();
