// Raster-only: the `.test.raster.ts` suffix keeps host Vitest from collecting this fixture.
//
// These cases pin behaviour that either differs from the Node 24.21 binary the
// probes ran on (documented in the PR) or cannot be observed through Vitest's
// module wrapper, which turns ES namespaces into plain objects.
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { pathToFileURL } from "node:url";
import Module, {
  constants,
  createRequire,
  enableCompileCache,
  findPackageJSON,
  flushCompileCache,
  getCompileCacheDir,
  syncBuiltinESMExports,
} from "node:module";
import * as osNs from "node:os";
import * as pathNs from "node:path";

const requireHere = createRequire(path.join(os.tmpdir(), "module-extras-require.js"));

const codeOf = (fn: () => unknown): string | undefined => {
  try {
    fn();
  } catch (error: any) {
    return error.code;
  }
  return "did not throw";
};

describe("module.findPackageJSON without a package.json", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "module-extras-raster-"));

  afterAll(() => fs.rmSync(root, { recursive: true, force: true }));

  it("returns the resolved path when the search reaches a node_modules directory", () => {
    const file = path.join(root, "pkg/node_modules/nopkg/f.js");
    fs.mkdirSync(path.dirname(file), { recursive: true });
    fs.writeFileSync(file, "x\n");
    fs.writeFileSync(path.join(root, "pkg/package.json"), "{}");
    expect(findPackageJSON(pathToFileURL(file).href)).toBe(file);
  });

  it("returns the resolved path when no ancestor has a package.json", () => {
    const file = path.join(root, "plain/f.js");
    fs.mkdirSync(path.dirname(file), { recursive: true });
    fs.writeFileSync(file, "x\n");
    let ancestor: string | undefined;
    for (let dir = root; ; dir = path.dirname(dir)) {
      if (fs.existsSync(path.join(dir, "package.json"))) {
        ancestor = path.join(dir, "package.json");
        break;
      }
      if (path.dirname(dir) === dir) break;
    }
    expect(findPackageJSON(pathToFileURL(file).href)).toBe(ancestor ?? file);
  });
});

describe("module compile cache", () => {
  it("reports that the cache could not be enabled", () => {
    const result = enableCompileCache(path.join(os.tmpdir(), "raster-compile-cache-unused"));
    expect(result.status).toBe(constants.compileCacheStatus.FAILED);
    expect(typeof result.message).toBe("string");
    expect(result.directory).toBeUndefined();
    expect(enableCompileCache().status).toBe(constants.compileCacheStatus.FAILED);
  });

  it("leaves the cache directory unset and creates nothing on disk", () => {
    const dir = path.join(os.tmpdir(), `raster-compile-cache-${process.pid}`);
    enableCompileCache(dir);
    expect(getCompileCacheDir()).toBeUndefined();
    expect(fs.existsSync(dir)).toBe(false);
    expect(flushCompileCache()).toBeUndefined();
  });

  it("validates the cache directory argument like Node", () => {
    expect(codeOf(() => (enableCompileCache as any)(5))).toBe("ERR_INVALID_ARG_TYPE");
    expect(() => (enableCompileCache as any)(5)).toThrow("cacheDir should be a string");
  });

  it("is on the Module constructor, so tools that probe it for a function keep working", () => {
    expect(Module.enableCompileCache).toBe(enableCompileCache);
    expect(Module.getCompileCacheDir).toBe(getCompileCacheDir);
    expect(Module.flushCompileCache).toBe(flushCompileCache);
    expect(requireHere("node:module").enableCompileCache).toBe(enableCompileCache);
  });
});

describe("module.syncBuiltinESMExports", () => {
  it("takes no arguments and returns undefined", () => {
    expect(syncBuiltinESMExports.length).toBe(0);
    expect(syncBuiltinESMExports()).toBeUndefined();
  });

  it("makes a patched CommonJS export visible to ES imports", () => {
    const osCjs = requireHere("os");
    const original = osCjs.hostname;
    try {
      osCjs.hostname = () => "patched";
      expect(osNs.hostname).toBe(original);
      syncBuiltinESMExports();
      expect(osNs.hostname()).toBe("patched");
    } finally {
      osCjs.hostname = original;
      syncBuiltinESMExports();
    }
    expect(osNs.hostname).toBe(original);
  });

  it("does not add new names and clears deleted ones", () => {
    const osCjs = requireHere("os");
    const original = osCjs.arch;
    try {
      osCjs.addedAfterTheFact = 1;
      delete osCjs.arch;
      syncBuiltinESMExports();
      expect("addedAfterTheFact" in osNs).toBe(false);
      expect(typeof osNs.arch).toBe("undefined");
    } finally {
      delete osCjs.addedAfterTheFact;
      osCjs.arch = original;
      syncBuiltinESMExports();
    }
    expect(osNs.arch).toBe(original);
  });

  it("syncs every loaded builtin, not only the one that was patched", () => {
    const osCjs = requireHere("os");
    const pathCjs = requireHere("path");
    const originalHostname = osCjs.hostname;
    const originalJoin = pathCjs.join;
    try {
      osCjs.hostname = () => "patched-os";
      pathCjs.join = () => "patched-path";
      syncBuiltinESMExports();
      expect(osNs.hostname()).toBe("patched-os");
      expect(pathNs.join("a", "b")).toBe("patched-path");
    } finally {
      osCjs.hostname = originalHostname;
      pathCjs.join = originalJoin;
      syncBuiltinESMExports();
    }
  });

  it("keeps the default export identical to require()", async () => {
    const imported = await import("node:os");
    expect(imported.default).toBe(requireHere("os"));
  });

  it("reaches modules imported with and without the node: prefix", async () => {
    const osCjs = requireHere("os");
    const original = osCjs.type;
    const bare = await import("os");
    try {
      osCjs.type = () => "patched-type";
      syncBuiltinESMExports();
      expect(bare.type()).toBe("patched-type");
      expect((await import("node:os")).type()).toBe("patched-type");
    } finally {
      osCjs.type = original;
      syncBuiltinESMExports();
    }
  });
});
