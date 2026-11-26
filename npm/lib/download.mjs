import { createHash } from "node:crypto";
import { existsSync, readFileSync, statSync } from "node:fs";
import { chmod, mkdir, rename, rm, writeFile } from "node:fs/promises";
import { createRequire } from "node:module";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

export const DEFAULT_RELEASE_BASE_URL =
  "https://github.com/Ray-D-Song/lexe/releases/download";

const PKG_ROOT = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  ".."
);

const PLATFORM_ASSETS = new Map([
  ["darwin:x64", "lexe-macos-x64"],
  ["darwin:arm64", "lexe-macos-arm64"],
  ["win32:x64", "lexe-windows-x64.exe"],
  ["win32:arm64", "lexe-windows-arm64.exe"],
]);

/** Linux release assets, keyed by `arch` then `libc` (`gnu` | `musl`). */
const LINUX_ASSETS = new Map([
  ["x64:gnu", "lexe-linux-x64-gnu"],
  ["arm64:gnu", "lexe-linux-arm64-gnu"],
  ["x64:musl", "lexe-linux-x64-musl"],
  ["arm64:musl", "lexe-linux-arm64-musl"],
]);

/**
 * Detect the libc flavour of a Linux host.
 * Returns `"gnu"` for glibc and `"musl"` for musl. Defaults to `"gnu"` when unknown,
 * because glibc is the common case and the gnu build is the only one that supports N-API addons.
 */
export function detectLinuxLibc(env = process.env, fsExists = existsSync) {
  const forced = env.LEXE_LIBC?.trim().toLowerCase();
  if (forced === "gnu" || forced === "musl") {
    return forced;
  }
  try {
    const report = process.report?.getReport?.();
    const header = typeof report === "string" ? JSON.parse(report).header : report?.header;
    if (header && header.glibcVersionRuntime) {
      return "gnu";
    }
    if (header && !header.glibcVersionRuntime) {
      return "musl";
    }
  } catch {
    // Fall through to filesystem probing.
  }
  const muslLoaders = ["/lib/ld-musl-x86_64.so.1", "/lib/ld-musl-aarch64.so.1"];
  return muslLoaders.some((loader) => fsExists(loader)) ? "musl" : "gnu";
}

/** Return the npm subpackage name that carries the Linux binary for `arch` and `libc`. */
export function getLinuxSubpackageName(arch, libc) {
  return `lexe-linux-${arch}-${libc}`;
}

/** Resolve the GitHub Release asset name for a given Node `platform`, `arch` and (Linux only) `libc`. */
export function resolvePlatformAsset(
  platform = process.platform,
  arch = process.arch,
  libc = platform === "linux" ? detectLinuxLibc() : undefined
) {
  if (platform === "linux") {
    const asset = LINUX_ASSETS.get(`${arch}:${libc}`);
    if (!asset) {
      throw new Error(
        `Unsupported platform/architecture: ${platform}-${arch}-${libc}. Supported Linux targets: linux-x64-gnu, linux-arm64-gnu, linux-x64-musl, linux-arm64-musl.`
      );
    }
    return asset;
  }
  const asset = PLATFORM_ASSETS.get(`${platform}:${arch}`);
  if (!asset) {
    throw new Error(
      `Unsupported platform/architecture: ${platform}-${arch}. Supported targets: linux-x64-gnu, linux-arm64-gnu, linux-x64-musl, linux-arm64-musl, darwin-x64, darwin-arm64, win32-x64, win32-arm64.`
    );
  }
  return asset;
}

/**
 * Locate the binary shipped in the matching `lexe-linux-<arch>-<libc>` subpackage.
 * Returns `undefined` when the subpackage is not installed (e.g. `--omit=optional`).
 */
export function resolveSubpackageBinary(
  platform = process.platform,
  arch = process.arch,
  libc = platform === "linux" ? detectLinuxLibc() : undefined
) {
  if (platform !== "linux") {
    return undefined;
  }
  const requireFromHere = createRequire(import.meta.url);
  try {
    const pkgJson = requireFromHere.resolve(
      `${getLinuxSubpackageName(arch, libc)}/package.json`
    );
    const bin = path.join(path.dirname(pkgJson), "bin", "lexe");
    return isRegularFile(bin) ? bin : undefined;
  } catch {
    return undefined;
  }
}

/** Read the package version from `npm/package.json`. */
export function getPackageVersion(pkgRoot = PKG_ROOT) {
  const pkgPath = path.join(pkgRoot, "package.json");
  const pkg = JSON.parse(readFileSync(pkgPath, "utf8"));
  if (!pkg.version || typeof pkg.version !== "string") {
    throw new Error(`Invalid package version in ${pkgPath}`);
  }
  return pkg.version;
}

/** Compute the release download base URL (supports `LEXE_RELEASE_BASE_URL` override). */
export function getReleaseBaseUrl(
  version = getPackageVersion(),
  env = process.env
) {
  const custom = env.LEXE_RELEASE_BASE_URL?.trim();
  if (custom) {
    return custom.replace(/\/+$/, "");
  }
  return `${DEFAULT_RELEASE_BASE_URL}/v${version}`;
}

/** Compute the versioned cache directory for downloaded runtime binaries. */
export function getCacheDir(
  version = getPackageVersion(),
  env = process.env,
  platform = process.platform
) {
  let root = env.LEXE_CACHE_DIR?.trim();
  if (!root) {
    if (platform === "win32") {
      const localAppData = env.LOCALAPPDATA?.trim();
      root = localAppData
        ? path.join(localAppData, "lexe", "cache")
        : path.join(os.homedir(), ".cache", "lexe");
    } else if (env.XDG_CACHE_HOME?.trim()) {
      root = path.join(env.XDG_CACHE_HOME.trim(), "lexe");
    } else {
      const home = env.HOME?.trim() || os.homedir();
      root = path.join(home, ".cache", "lexe");
    }
  }
  return path.join(root, `v${version}`);
}

/** Return the in-package vendor binary path populated during writable postinstall runs. */
export function getVendorBinaryPath(assetName, pkgRoot = PKG_ROOT) {
  return path.join(pkgRoot, "vendor", assetName);
}

/** Parse a `SHA256SUMS` file and extract the expected digest for `assetName`. */
export function parseSha256Sums(contents, assetName) {
  for (const rawLine of contents.split(/\r?\n/)) {
    const line = rawLine.trim();
    if (!line) {
      continue;
    }
    const parts = line.split(/\s+/);
    if (parts.length < 2) {
      continue;
    }
    const [digest, rawFile] = parts;
    const file = rawFile.replace(/^\*/, "");
    if (file === assetName) {
      if (!/^[0-9a-fA-F]{64}$/.test(digest)) {
        throw new Error(
          `Invalid SHA256 digest '${digest}' for asset '${assetName}' in SHA256SUMS`
        );
      }
      return digest.toLowerCase();
    }
  }
  throw new Error(`Asset '${assetName}' not found in SHA256SUMS`);
}

/** Compute lowercase hex SHA256 digest of a Buffer or Uint8Array. */
export function sha256Hex(buffer) {
  return createHash("sha256").update(buffer).digest("hex");
}

function isRegularFile(filePath) {
  try {
    return statSync(filePath).isFile();
  } catch {
    return false;
  }
}

async function fetchBytes(url, version) {
  const response = await fetch(url, {
    redirect: "follow",
    headers: {
      "User-Agent": `lexe-npm/${version}`,
    },
  });
  if (!response.ok) {
    throw new Error(
      `Failed to fetch ${url}: HTTP ${response.status} ${response.statusText}`
    );
  }
  return Buffer.from(await response.arrayBuffer());
}

async function atomicWriteBinary(targetPath, bytes, platform) {
  const dir = path.dirname(targetPath);
  const fileName = path.basename(targetPath);
  await mkdir(dir, { recursive: true });
  const tmpPath = path.join(
    dir,
    `.${fileName}.${process.pid}.${Math.random().toString(16).slice(2)}.tmp`
  );
  try {
    await writeFile(tmpPath, bytes);
    if (platform !== "win32") {
      await chmod(tmpPath, 0o755);
    }
    await rename(tmpPath, targetPath);
    return targetPath;
  } finally {
    await rm(tmpPath, { force: true }).catch(() => {});
  }
}

/**
 * Locate or download and SHA256-verify the Lexe native binary for the host platform.
 * Precedence:
 * 1. `LEXE_BINARY_PATH` (explicit file path, must exist)
 * 2. Linux only: the `lexe-linux-<arch>-<libc>` subpackage installed by npm
 * 3. Package-local `<pkgRoot>/vendor/<asset>`
 * 4. User cache `<cacheDir>/v<version>/<asset>`
 * 5. Download `<asset>` + `SHA256SUMS` from GitHub Releases and verify SHA256
 */
export async function ensureBinary(options = {}) {
  const pkgRoot = options.pkgRoot ?? PKG_ROOT;
  const env = options.env ?? process.env;
  const platform = options.platform ?? process.platform;
  const arch = options.arch ?? process.arch;
  const version = options.version ?? getPackageVersion(pkgRoot);
  const preferPackageDir = options.preferPackageDir ?? false;

  const explicitPath = env.LEXE_BINARY_PATH?.trim();
  if (explicitPath) {
    const resolved = path.resolve(explicitPath);
    if (!isRegularFile(resolved)) {
      throw new Error(
        `LEXE_BINARY_PATH does not point to an existing file: ${resolved}`
      );
    }
    return resolved;
  }

  const libc = platform === "linux" ? (options.libc ?? detectLinuxLibc(env)) : undefined;
  const assetName = resolvePlatformAsset(platform, arch, libc);

  const subpackageBinary = options.resolveSubpackage === false
    ? undefined
    : resolveSubpackageBinary(platform, arch, libc);
  if (subpackageBinary) {
    return subpackageBinary;
  }

  const vendorPath = getVendorBinaryPath(assetName, pkgRoot);
  if (isRegularFile(vendorPath)) {
    return vendorPath;
  }

  const cacheDir = getCacheDir(version, env, platform);
  const cachedPath = path.join(cacheDir, assetName);
  if (isRegularFile(cachedPath)) {
    return cachedPath;
  }

  const baseUrl = getReleaseBaseUrl(version, env);
  const sumsUrl = `${baseUrl}/SHA256SUMS`;
  const assetUrl = `${baseUrl}/${assetName}`;

  const sumsBytes = await fetchBytes(sumsUrl, version);
  const expectedSha = parseSha256Sums(sumsBytes.toString("utf8"), assetName);

  const assetBytes = await fetchBytes(assetUrl, version);
  const actualSha = sha256Hex(assetBytes);
  if (actualSha !== expectedSha) {
    const err = new Error(
      `SHA256 verification failed for ${assetName}: expected ${expectedSha}, got ${actualSha}`
    );
    err.code = "LEXE_CHECKSUM_MISMATCH";
    throw err;
  }

  if (preferPackageDir) {
    try {
      return await atomicWriteBinary(vendorPath, assetBytes, platform);
    } catch (err) {
      if (err && ["EACCES", "EROFS", "EPERM"].includes(err.code)) {
        return await atomicWriteBinary(cachedPath, assetBytes, platform);
      }
      throw err;
    }
  }

  return await atomicWriteBinary(cachedPath, assetBytes, platform);
}
