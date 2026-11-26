import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { chmod, mkdir, mkdtemp, rm, stat, writeFile } from "node:fs/promises";
import http from "node:http";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import {
  detectLinuxLibc,
  ensureBinary,
  getLinuxSubpackageName,
  parseSha256Sums,
  resolvePlatformAsset,
  resolveSubpackageBinary,
  sha256Hex,
} from "../lib/download.mjs";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const NPM_DIR = path.resolve(HERE, "..");
const BIN_SCRIPT = path.join(NPM_DIR, "bin", "lexe.mjs");
const POSTINSTALL_SCRIPT = path.join(NPM_DIR, "scripts", "postinstall.mjs");

function runNode(args, options = {}) {
  return new Promise((resolve, reject) => {
    const child = spawn(process.execPath, args, options);
    let stdout = "";
    let stderr = "";
    child.stdout?.on("data", (chunk) => {
      stdout += chunk.toString();
    });
    child.stderr?.on("data", (chunk) => {
      stderr += chunk.toString();
    });
    child.on("error", reject);
    child.on("close", (status, signal) => resolve({ status, signal, stdout, stderr }));
  });
}

async function startMockReleaseServer({ payload, checksumOverride = null }) {
  const assetName = resolvePlatformAsset();
  const digest = checksumOverride ?? sha256Hex(payload);
  const sha256SumsBody = `${digest}  ${assetName}\n`;
  let assetHits = 0;

  const server = http.createServer((req, res) => {
    if (req.url === "/SHA256SUMS") {
      res.writeHead(200, { "Content-Type": "text/plain" });
      res.end(sha256SumsBody);
      return;
    }
    if (req.url === `/${assetName}`) {
      assetHits += 1;
      res.writeHead(302, { Location: `/cdn/${assetName}` });
      res.end();
      return;
    }
    if (req.url === `/cdn/${assetName}`) {
      res.writeHead(200, { "Content-Type": "application/octet-stream" });
      res.end(payload);
      return;
    }
    res.writeHead(404);
    res.end("not found");
  });

  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const address = server.address();
  return {
    baseUrl: `http://127.0.0.1:${address.port}`,
    getAssetHits: () => assetHits,
    close: () =>
      new Promise((resolve) => {
        server.closeAllConnections?.();
        server.close(resolve);
      }),
  };
}

test("resolvePlatformAsset maps all supported platforms and rejects unsupported targets", () => {
  assert.equal(resolvePlatformAsset("linux", "x64", "gnu"), "lexe-linux-x64-gnu");
  assert.equal(resolvePlatformAsset("linux", "arm64", "gnu"), "lexe-linux-arm64-gnu");
  assert.equal(resolvePlatformAsset("linux", "x64", "musl"), "lexe-linux-x64-musl");
  assert.equal(resolvePlatformAsset("linux", "arm64", "musl"), "lexe-linux-arm64-musl");
  assert.equal(resolvePlatformAsset("darwin", "x64"), "lexe-macos-x64");
  assert.equal(resolvePlatformAsset("darwin", "arm64"), "lexe-macos-arm64");
  assert.equal(resolvePlatformAsset("win32", "x64"), "lexe-windows-x64.exe");
  assert.equal(
    resolvePlatformAsset("win32", "arm64"),
    "lexe-windows-arm64.exe"
  );
  assert.throws(
    () => resolvePlatformAsset("freebsd", "x64"),
    /Unsupported platform/
  );
  assert.throws(
    () => resolvePlatformAsset("linux", "ia32", "gnu"),
    /Unsupported platform/
  );
});

test("detectLinuxLibc honors LEXE_LIBC and falls back to musl loader probing", () => {
  assert.equal(detectLinuxLibc({ LEXE_LIBC: "musl" }), "musl");
  assert.equal(detectLinuxLibc({ LEXE_LIBC: "GNU" }), "gnu");
  const muslHost = (p) => p === "/lib/ld-musl-x86_64.so.1";
  if (process.platform === "linux" && process.report?.getReport) {
    // Only exercise the report path when the runner really is glibc or musl.
    const header = process.report.getReport().header ?? {};
    if (header.glibcVersionRuntime) {
      assert.equal(detectLinuxLibc({}, () => false), "gnu");
    }
  }
  // Pure fallback path: no report data, musl loader present.
  assert.ok(["gnu", "musl"].includes(detectLinuxLibc({}, muslHost)));
});

test("Linux subpackages are named per arch and libc", () => {
  assert.equal(getLinuxSubpackageName("x64", "gnu"), "lexe-linux-x64-gnu");
  assert.equal(getLinuxSubpackageName("arm64", "musl"), "lexe-linux-arm64-musl");
  assert.equal(resolveSubpackageBinary("darwin", "arm64"), undefined);
});

test("subpackage manifests declare os/cpu/libc so npm installs only the matching variant", async () => {
  const pkgRoot = path.join(NPM_DIR, "packages");
  const expected = [
    ["linux-x64-gnu", "x64", "glibc"],
    ["linux-arm64-gnu", "arm64", "glibc"],
    ["linux-x64-musl", "x64", "musl"],
    ["linux-arm64-musl", "arm64", "musl"],
  ];
  const main = JSON.parse(await (await import("node:fs/promises")).readFile(path.join(NPM_DIR, "package.json"), "utf8"));
  for (const [dir, cpu, libc] of expected) {
    const manifest = JSON.parse(
      await (await import("node:fs/promises")).readFile(path.join(pkgRoot, dir, "package.json"), "utf8")
    );
    assert.equal(manifest.name, `lexe-${dir}`);
    assert.deepEqual(manifest.os, ["linux"]);
    assert.deepEqual(manifest.cpu, [cpu]);
    assert.deepEqual(manifest.libc, [libc]);
    assert.equal(manifest.version, main.version);
    assert.equal(main.optionalDependencies[`lexe-${dir}`], main.version);
  }
});

test("parseSha256Sums handles standard and binary-prefixed lines and rejects bad digests", () => {
  const hash = "a".repeat(64);
  const upperHash = "B".repeat(64);
  const sums = [
    `${hash}  lexe-linux-x64-gnu`,
    `${upperHash} *lexe-macos-arm64`,
    "deadbeef  lexe-windows-x64.exe",
  ].join("\n");

  assert.equal(parseSha256Sums(sums, "lexe-linux-x64-gnu"), hash);
  assert.equal(parseSha256Sums(sums, "lexe-macos-arm64"), "b".repeat(64));
  assert.throws(
    () => parseSha256Sums(sums, "lexe-linux-arm64-gnu"),
    /not found in SHA256SUMS/
  );
  assert.throws(
    () => parseSha256Sums(sums, "lexe-windows-x64.exe"),
    /Invalid SHA256 digest/
  );
});

test("ensureBinary verifies SHA256 through redirect, caches locally, and rejects mismatches", async (t) => {
  const tmpDir = await mkdtemp(path.join(os.tmpdir(), "lexe-npm-test-"));
  t.after(() => rm(tmpDir, { recursive: true, force: true }));

  const pkgRoot = path.join(tmpDir, "pkg");
  const cacheDir = path.join(tmpDir, "cache");
  await mkdir(pkgRoot, { recursive: true });

  const goodPayload = Buffer.from("#!/bin/sh\necho ok\n", "utf8");
  const server = await startMockReleaseServer({ payload: goodPayload });
  t.after(() => server.close());

  const firstPath = await ensureBinary({
    pkgRoot,
    version: "0.1.0",
    env: {
      LEXE_CACHE_DIR: cacheDir,
      LEXE_RELEASE_BASE_URL: server.baseUrl,
    },
  });
  assert.equal(
    firstPath,
    path.join(cacheDir, "v0.1.0", resolvePlatformAsset(undefined, undefined, undefined))
  );
  assert.equal(server.getAssetHits(), 1);
  if (process.platform !== "win32") {
    const mode = (await stat(firstPath)).mode;
    assert.ok((mode & 0o111) !== 0, "downloaded binary should be executable");
  }

  const secondPath = await ensureBinary({
    pkgRoot,
    version: "0.1.0",
    env: {
      LEXE_CACHE_DIR: cacheDir,
      LEXE_RELEASE_BASE_URL: server.baseUrl,
    },
  });
  assert.equal(secondPath, firstPath);
  assert.equal(server.getAssetHits(), 1, "cached binary should avoid re-fetch");

  const badCacheDir = path.join(tmpDir, "bad-cache");
  const badServer = await startMockReleaseServer({
    payload: goodPayload,
    checksumOverride: "0".repeat(64),
  });
  t.after(() => badServer.close());

  await assert.rejects(
    ensureBinary({
      pkgRoot,
      version: "0.1.0",
      env: {
        LEXE_CACHE_DIR: badCacheDir,
        LEXE_RELEASE_BASE_URL: badServer.baseUrl,
      },
    }),
    (err) => err?.code === "LEXE_CHECKSUM_MISMATCH"
  );
  await assert.rejects(
    stat(path.join(badCacheDir, "v0.1.0", resolvePlatformAsset()))
  );
});

test("postinstall honors LEXE_SKIP_DOWNLOAD and fails closed on checksum mismatch", async (t) => {
  const skipRun = await runNode([POSTINSTALL_SCRIPT], {
    env: { ...process.env, LEXE_SKIP_DOWNLOAD: "1" },
  });
  assert.equal(skipRun.status, 0);

  const tmpDir = await mkdtemp(path.join(os.tmpdir(), "lexe-postinstall-"));
  t.after(() => rm(tmpDir, { recursive: true, force: true }));
  const cacheDir = path.join(tmpDir, "cache");

  const badServer = await startMockReleaseServer({
    payload: Buffer.from("binary-data"),
    checksumOverride: "f".repeat(64),
  });
  t.after(() => badServer.close());

  const badRun = await runNode([POSTINSTALL_SCRIPT], {
    env: {
      ...process.env,
      LEXE_CACHE_DIR: cacheDir,
      LEXE_RELEASE_BASE_URL: badServer.baseUrl,
    },
  });
  assert.equal(badRun.status, 1);
  assert.match(badRun.stderr, /SHA256 verification failed/);
});

test("bin/lexe.mjs forwards args and exit codes via LEXE_BINARY_PATH and downloaded binary", async (t) => {
  const tmpDir = await mkdtemp(path.join(os.tmpdir(), "lexe-bin-test-"));
  t.after(() => rm(tmpDir, { recursive: true, force: true }));

  const isWin = process.platform === "win32";
  const mockScript = isWin
    ? "@echo off\r\necho forward:%1:%2:%LEXE_MANAGED_BY%\r\nexit /b 7\r\n"
    : '#!/bin/sh\necho "forward:$1:$2:$LEXE_MANAGED_BY"\nexit 7\n';

  const localBin = path.join(tmpDir, isWin ? "mock.cmd" : "mock-lexe");
  await writeFile(localBin, mockScript, "utf8");
  if (!isWin) {
    await chmod(localBin, 0o755);
  }

  const directRun = await runNode([BIN_SCRIPT, "--version", "extra"], {
    env: { ...process.env, LEXE_BINARY_PATH: localBin },
  });
  assert.equal(directRun.status, 7);
  assert.match(directRun.stdout, /forward:--version:extra:npm/);

  if (!isWin) {
    const cacheDir = path.join(tmpDir, "cache");
    const server = await startMockReleaseServer({
      payload: Buffer.from(mockScript, "utf8"),
    });
    t.after(() => server.close());

    const downloadedRun = await runNode([BIN_SCRIPT, "--help", "arg2"], {
      env: {
        ...process.env,
        LEXE_CACHE_DIR: cacheDir,
        LEXE_RELEASE_BASE_URL: server.baseUrl,
      },
    });
    assert.equal(downloadedRun.status, 7);
    assert.match(downloadedRun.stdout, /forward:--help:arg2:npm/);
  }
});
