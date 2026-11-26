// Copy the release Linux binaries into the matching @lexe/linux-* subpackages.
//
// Usage: node scripts/stage-linux-packages.mjs <dir-with-release-assets> <version>
//
// Expects the four assets produced by the release workflow:
//   lexe-linux-x64-gnu, lexe-linux-arm64-gnu, lexe-linux-x64-musl, lexe-linux-arm64-musl
// Each is copied to packages/<name>/bin/lexe, made executable, and the subpackage
// version is set to <version>. The main package's optionalDependencies are pinned to the same version.

import { chmodSync, copyFileSync, existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const npmDir = path.resolve(here, "..");

const [assetDir, version] = process.argv.slice(2);
if (!assetDir || !version) {
  console.error("usage: stage-linux-packages.mjs <asset-dir> <version>");
  process.exit(2);
}

const SUBPACKAGES = [
  ["lexe-linux-x64-gnu", "linux-x64-gnu"],
  ["lexe-linux-arm64-gnu", "linux-arm64-gnu"],
  ["lexe-linux-x64-musl", "linux-x64-musl"],
  ["lexe-linux-arm64-musl", "linux-arm64-musl"],
];

for (const [asset, dirName] of SUBPACKAGES) {
  const src = path.join(assetDir, asset);
  if (!existsSync(src)) {
    console.error(`missing release asset: ${src}`);
    process.exit(1);
  }
  const pkgDir = path.join(npmDir, "packages", dirName);
  const pkgJsonPath = path.join(pkgDir, "package.json");
  const pkg = JSON.parse(readFileSync(pkgJsonPath, "utf8"));
  pkg.version = version;
  writeFileSync(pkgJsonPath, `${JSON.stringify(pkg, null, 2)}\n`);

  const binDir = path.join(pkgDir, "bin");
  mkdirSync(binDir, { recursive: true });
  const dest = path.join(binDir, "lexe");
  copyFileSync(src, dest);
  chmodSync(dest, 0o755);
  console.log(`staged ${asset} -> ${path.relative(npmDir, dest)} (${pkg.name}@${version})`);
}

const mainPath = path.join(npmDir, "package.json");
const main = JSON.parse(readFileSync(mainPath, "utf8"));
for (const [, dirName] of SUBPACKAGES) {
  const name = `@lexe/${dirName}`;
  main.optionalDependencies[name] = version;
}
writeFileSync(mainPath, `${JSON.stringify(main, null, 2)}\n`);
console.log(`pinned optionalDependencies to ${version}`);
