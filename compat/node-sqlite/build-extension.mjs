import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const vendor = path.resolve(here, "../../modules/lexe_sqlite/vendor");
const buildDir = path.join(here, "build");
fs.mkdirSync(buildDir, { recursive: true });

/** Compiler/link flags for optional ASan (env is not auto-applied by cc). */
function sanitizeFlags() {
  // Explicit LEXE_SQLITE_SANITIZE wins: none|0 disables even if CFLAGS is set.
  if (Object.prototype.hasOwnProperty.call(process.env, "LEXE_SQLITE_SANITIZE")) {
    const sanitize = process.env.LEXE_SQLITE_SANITIZE || "";
    if (sanitize === "address") {
      return ["-fsanitize=address", "-fno-omit-frame-pointer", "-g"];
    }
    if (sanitize === "" || sanitize === "0" || sanitize === "none") {
      return [];
    }
    console.error(
      `unsupported LEXE_SQLITE_SANITIZE=${sanitize} (expected address|none)`,
    );
    process.exit(1);
  }
  // Unset only: optional ad-hoc fallback for callers that still set CFLAGS.
  const cflags = process.env.CFLAGS;
  if (cflags && cflags.trim()) {
    return cflags.trim().split(/\s+/);
  }
  return [];
}

let output;
let args;
if (process.platform === "win32") {
  output = path.join(buildDir, "sqlite_extension.dll");
  args = ["-shared", `-I${vendor}`, "-o", output, path.join(here, "extension.c")];
} else if (process.platform === "darwin") {
  output = path.join(buildDir, "sqlite_extension.dylib");
  args = [
    "-dynamiclib",
    "-fPIC",
    `-I${vendor}`,
    "-o",
    output,
    path.join(here, "extension.c"),
  ];
} else {
  output = path.join(buildDir, "sqlite_extension.so");
  args = [
    "-shared",
    "-fPIC",
    `-I${vendor}`,
    "-o",
    output,
    path.join(here, "extension.c"),
  ];
}

// Inject before sources so both compile and link see -fsanitize=*.
const extra = sanitizeFlags();
if (extra.length > 0) {
  args = [...extra, ...args];
}

const cc = process.env.CC || "cc";
const result = spawnSync(cc, args, { stdio: "inherit" });
if (result.status !== 0) {
  process.exit(result.status ?? 1);
}

console.log(`built ${output}`);
if (extra.length > 0) {
  console.log(`with flags: ${extra.join(" ")}`);
}
