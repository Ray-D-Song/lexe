// Lexe-only: the `.test.lexe.ts` suffix keeps host Vitest from collecting this fixture.
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
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "module-extras-lexe-"));

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
  it("enables on-disk QuickJS bytecode compile cache and reports ALREADY_ENABLED on reuse", () => {
    const cacheDir = fs.mkdtempSync(path.join(os.tmpdir(), "lexe-compile-cache-"));
    try {
      const result = enableCompileCache(cacheDir);
      expect(result.status).toBe(constants.compileCacheStatus.ENABLED);
      expect(typeof result.directory).toBe("string");
      const resolvedDir = getCompileCacheDir();
      expect(typeof resolvedDir).toBe("string");
      expect(fs.existsSync(resolvedDir!)).toBe(true);

      const second = enableCompileCache(cacheDir);
      expect(second.status).toBe(constants.compileCacheStatus.ALREADY_ENABLED);
      expect(second.directory).toBe(resolvedDir);

      // Compiling and flushing a CJS module writes a bytecode cache entry to disk.
      const modDir = fs.mkdtempSync(path.join(os.tmpdir(), "lexe-cc-mod-"));
      try {
        const modFile = path.join(modDir, "cached.cjs");
        fs.writeFileSync(modFile, "module.exports = { answer: 42 };\n");
        expect(requireHere(modFile)).toEqual({ answer: 42 });
        expect(flushCompileCache()).toBeUndefined();
        expect(fs.readdirSync(resolvedDir!).length).toBeGreaterThan(0);

        // Removing from require.cache still executes cleanly via disk bytecode cache.
        delete requireHere.cache[modFile];
        expect(requireHere(modFile)).toEqual({ answer: 42 });
      } finally {
        fs.rmSync(modDir, { recursive: true, force: true });
      }
    } finally {
      fs.rmSync(cacheDir, { recursive: true, force: true });
    }
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

describe("module.stripTypeScriptTypes and TS execution", () => {
  it("strips type annotations in strip mode while preserving line count", () => {
    const src = [
      "import type { Foo } from './types';",
      "import { type Bar, baz } from './mod';",
      "export interface Item<T> { value: T }",
      "export type Id = string | number;",
      "export const count: number = (1 as number) satisfies number;",
      "export function id<T>(x: T): T { return x!; }",
    ].join("\n");
    const stripped = Module.stripTypeScriptTypes(src);
    expect(stripped.split("\n").length).toBe(src.split("\n").length);
    expect(stripped).not.toContain("import type");
    expect(stripped).not.toContain("type Bar");
    expect(stripped).not.toContain("export interface");
    expect(stripped).not.toContain("export type");
    expect(stripped).not.toContain(" as ");
    expect(stripped).not.toContain("satisfies");
    expect(stripped).toMatch(/import \{\s+baz \} from '\.\/mod';/);
    expect(stripped).toContain("export const count");
    expect(stripped).toContain("export function id");
  });

  it("rejects non-erasable TS syntax in default strip mode and supports transform mode", () => {
    expect(codeOf(() => Module.stripTypeScriptTypes("enum E { A = 1 }"))).toBe(
      "ERR_UNSUPPORTED_TYPESCRIPT_SYNTAX"
    );
    expect(codeOf(() => Module.stripTypeScriptTypes("namespace N { export const x = 1; }"))).toBe(
      "ERR_UNSUPPORTED_TYPESCRIPT_SYNTAX"
    );
    expect(
      codeOf(() =>
        Module.stripTypeScriptTypes("class C { constructor(public x: number) {} }")
      )
    ).toBe("ERR_UNSUPPORTED_TYPESCRIPT_SYNTAX");
    expect(
      codeOf(() => Module.stripTypeScriptTypes("const x = 1", { mode: "strip", sourceMap: true } as any))
    ).toBe("ERR_INVALID_ARG_VALUE");

    const transformed = Module.stripTypeScriptTypes(
      "enum E { A, B = 5, C }\nnamespace N { export const v: number = 7; }\nclass Box { constructor(public readonly v: number) {} }",
      { mode: "transform", sourceMap: true, sourceUrl: "box.ts" }
    );
    expect(transformed).toContain('E[E["C"] = 6] = "C"');
    expect(transformed).toContain("N.v");
    expect(transformed).toContain("this.v = v;");
    expect(transformed).toContain("//# sourceMappingURL=data:application/json;base64,");
    expect(transformed).toContain("//# sourceURL=box.ts");
  });

  it("loads .ts and .cts modules via require() and import()", async () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "lexe-ts-exec-"));
    try {
      const ctsPath = path.join(dir, "helper.cts");
      const mtsPath = path.join(dir, "entry.mts");
      fs.writeFileSync(ctsPath, "const n: number = 21;\nexports.double = (x: number): number => x * 2;\nexports.n = n;\n");
      fs.writeFileSync(mtsPath, "export interface Res { ok: boolean }\nexport const value: number = 123;\n");

      const cjsMod = requireHere(ctsPath);
      expect(cjsMod.n).toBe(21);
      expect(cjsMod.double(21)).toBe(42);
      expect(Module._load(ctsPath, null, false)).toBe(cjsMod);

      const esmMod = await import(pathToFileURL(mtsPath).href);
      expect(esmMod.value).toBe(123);
    } finally {
      fs.rmSync(dir, { recursive: true, force: true });
    }
  });
});

describe("module.Module._load and CJS static helpers", () => {
  it("exposes wrap, wrapper, _stat, _resolveLookupPaths, _findPath, and _load", () => {
    expect(Array.isArray(Module.wrapper)).toBe(true);
    expect(Module.wrapper.length).toBe(2);
    expect(Module.wrap("return 1;")).toContain("function (exports, require, module, __filename, __dirname)");
    expect(Module._load("node:path", null, false).join("a", "b")).toBe(path.join("a", "b"));
    expect(Module._stat(os.tmpdir())).toBe(1);
    expect(Module._stat(path.join(os.tmpdir(), "nonexistent-lexe-path-xyz"))).toBe(-1);
    expect(Array.isArray(Module._resolveLookupPaths("dep", null))).toBe(true);
  });
});

describe("module.register worker-thread async hooks", () => {
  it("supports async initialize with MessagePort transfer, chained resolve/load, and nextLoad", async () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "lexe-reg-"));
    try {
      const diskFile = path.join(dir, "disk.mjs");
      fs.writeFileSync(diskFile, "export const base = 10;\n");

      const { port1, port2 } = new MessageChannel();
      const events: string[] = [];
      port1.on("message", (m: string) => events.push(m));

      const hookA = [
        "export async function initialize({ port }) {",
        "  await new Promise((r) => setTimeout(r, 10));",
        "  port.postMessage('init-a');",
        "}",
        "export async function resolve(spec, ctx, nextResolve) {",
        "  if (spec === 'virtual:chain') return { url: 'virtual:chain', shortCircuit: true };",
        "  return await nextResolve(spec, ctx);",
        "}",
        "export async function load(url, ctx, nextLoad) {",
        "  if (url === 'virtual:chain') {",
        "    await new Promise((r) => setTimeout(r, 5));",
        "    return { format: 'module', shortCircuit: true, source: 'export const x = 5;' };",
        "  }",
        "  const res = await nextLoad(url, ctx);",
        "  if (url.endsWith('disk.mjs')) {",
        "    return { format: 'module', shortCircuit: true, source: res.source + '\\nexport const plus = base + 5;' };",
        "  }",
        "  return res;",
        "}",
      ].join("\n");

      const hookB = [
        "export async function resolve(spec, ctx, nextResolve) {",
        "  return await nextResolve(spec, ctx);",
        "}",
        "export async function load(url, ctx, nextLoad) {",
        "  const res = await nextLoad(url, ctx);",
        "  if (url === 'virtual:chain') {",
        "    return { format: 'module', shortCircuit: true, source: res.source + '\\nexport const y = x * 3;' };",
        "  }",
        "  return res;",
        "}",
      ].join("\n");

      Module.register("data:text/javascript," + encodeURIComponent(hookA), {
        parentURL: "data:",
        data: { port: port2 },
        transferList: [port2],
      });
      Module.register("data:text/javascript," + encodeURIComponent(hookB), {
        parentURL: "data:",
      });

      const virt = await import("virtual:chain");
      expect(virt.x).toBe(5);
      expect(virt.y).toBe(15);

      const disk = await import(pathToFileURL(diskFile).href);
      expect(disk.base).toBe(10);
      expect(disk.plus).toBe(15);

      await new Promise((r) => setTimeout(r, 10));
      expect(events).toEqual(["init-a"]);
      port1.close();
    } finally {
      fs.rmSync(dir, { recursive: true, force: true });
    }
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
