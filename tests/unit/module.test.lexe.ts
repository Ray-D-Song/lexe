// Lexe-only: the `.test.lexe.ts` suffix keeps host Vitest from collecting this fixture.
const CWD = process.cwd();
const _require = require;

const Module = _require("node:module");
const legacyModule = _require("module");

it("node:module should be the same as module", () => {
  expect(Module).toStrictEqual(legacyModule);
});

it("default export should be a callable Module constructor", () => {
  expect(typeof Module).toBe("function");
  expect(typeof legacyModule).toBe("function");
  expect(Module.prototype.require).toBeInstanceOf(Function);
});

const { createRequire, _resolveFilename, _nodeModulePaths, _cache, _extensions, builtinModules, isBuiltin } =
  Module;

it("should resolve 'node:module via createRequire()", () => {
  const __require = createRequire(import.meta.url);
  expect(__require("node:module").createRequire).toBeDefined();
});

it("createRequire returns a require function with resolve and cache", () => {
  const localRequire = createRequire(import.meta.url);
  expect(typeof localRequire).toBe("function");
  expect(typeof localRequire.resolve).toBe("function");
  expect(localRequire.cache).toBe(_cache);
  expect(_require.cache).toBe(_cache);
});

it("createRequire accepts URL objects", () => {
  const localRequire = createRequire(new URL(import.meta.url));
  expect(localRequire.resolve("node:path")).toBe("node:path");
});

it("require.resolve resolves builtins, relative fixtures, and package entries", () => {
  expect(_require.resolve("node:path")).toBe("node:path");
  expect(_require.resolve("path")).toBe("path");
  expect(_require.resolve(`${CWD}/fixtures/hello.js`)).toContain("fixtures/hello.js");
  expect(_require.resolve("package.json")).toContain("package.json");
});

it("createRequire.resolve preserves node: prefix for builtins", () => {
  const localRequire = createRequire(import.meta.url);
  expect(localRequire.resolve("node:path")).toBe("node:path");
  expect(localRequire.resolve("path")).toBe("path");
});

function expectModuleNotFound(fn: () => unknown) {
  try {
    fn();
    throw new Error("expected MODULE_NOT_FOUND");
  } catch (err: any) {
    expect(err.code).toBe("MODULE_NOT_FOUND");
  }
}

it("require.resolve rejects builtin specifiers with trailing slash", () => {
  expectModuleNotFound(() => _require.resolve("node:path/"));
  expectModuleNotFound(() => _require.resolve("path/"));
});

it("createRequire.resolve rejects builtin specifiers with trailing slash", () => {
  const localRequire = createRequire(import.meta.url);
  expectModuleNotFound(() => localRequire.resolve("node:path/"));
  expectModuleNotFound(() => localRequire.resolve("path/"));
});

it("Module._resolveFilename resolves builtins and relative fixtures", () => {
  expect(_resolveFilename("fs")).toBe("fs");
  expect(_resolveFilename("node:fs")).toBe("fs");
  expect(_resolveFilename(`${CWD}/fixtures/hello.js`, null, false, {})).toContain(
    "fixtures/hello.js",
  );
});

it("Module._nodeModulePaths returns ordered node_modules directories", () => {
  const paths = _nodeModulePaths(`${CWD}/a/b/c`);
  expect(paths.length).toBeGreaterThan(0);
  expect(paths[0]).toContain("/a/b/c/node_modules");
  expect(paths.at(-1)).toContain("/node_modules");
});

it("Module._nodeModulePaths skips nested node_modules segments", () => {
  const paths = _nodeModulePaths("/a/node_modules");
  expect(paths).not.toContain("/a/node_modules/node_modules");
  expect(paths).toContain("/a/node_modules");
});

it("Module.prototype.require.call resolves relative fixtures", () => {
  const mod = new Module(`${CWD}/fixtures/hello.js`);
  const hello = Module.prototype.require.call(mod, "./hello.js");
  expect(hello.hello).toBe("hello world!");
});

it("handles cyclic requires", () => {
  const a = _require(`${CWD}/fixtures/a.js`);
  const b = _require(`${CWD}/fixtures/b.js`);
  expect(a.done).toEqual(b.done);
});

it("deleting require.cache entry reloads the module", () => {
  const target = _require.resolve(`${CWD}/fixtures/hello.js`);
  const first = _require(`${CWD}/fixtures/hello.js`);
  delete _require.cache[target];
  const second = _require(`${CWD}/fixtures/hello.js`);
  expect(first).not.toBe(second);
  expect(first.hello).toBe(second.hello);
});

it("deleting require.cache entry reloads json modules", () => {
  const target = _require.resolve(`${CWD}/fixtures/package.json`);
  const first = _require(`${CWD}/fixtures/package.json`);
  delete _require.cache[target];
  const second = _require(`${CWD}/fixtures/package.json`);
  expect(first).not.toBe(second);
  expect(first.private).toBe(second.private);
});

it("module records link parent and children", () => {
  const aFile = _require.resolve(`${CWD}/fixtures/a.js`);
  _require(`${CWD}/fixtures/a.js`);
  const aRecord = _require.cache[aFile];
  const bFile = _require.resolve(`${CWD}/fixtures/b.js`);
  const bRecord = _require.cache[bFile];
  expect(aRecord.parent).toBeNull();
  expect(bRecord.parent).toBe(aRecord);
  expect(aRecord.children).toContain(bRecord);
});

it("overwriting _resolveFilename affects subsequent require calls", () => {
  const original = _resolveFilename;
  const aliasTarget = `${CWD}/fixtures/hello.js`;
  try {
    Module._resolveFilename = (request, parent, isMain, options) => {
      if (request === "aliased-fixture") {
        return aliasTarget;
      }
      return original.call(Module, request, parent, isMain, options);
    };
    const mod = { filename: `${CWD}/fixtures/hello.js`, paths: _nodeModulePaths(CWD) };
    expect(Module.prototype.require.call(mod, "aliased-fixture").hello).toBe("hello world!");
  } finally {
    Module._resolveFilename = original;
  }
});

it("isBuiltin recognizes lexe builtins", () => {
  expect(isBuiltin("path")).toBe(true);
  expect(isBuiltin("node:path")).toBe(true);
  expect(isBuiltin("not-a-real-builtin-module")).toBe(false);
});

it("builtinModules is populated", () => {
  expect(Array.isArray(builtinModules)).toBe(true);
  expect(builtinModules).toContain("path");
});

const EXT_FIXTURES = `${CWD}/fixtures/require-extensions`;

it("require.extensions, Module._extensions, and createRequire share one table", () => {
  expect(_require.extensions).toBeDefined();
  expect(typeof _require.extensions).toBe("object");
  expect(_require.extensions[".js"]).toBeInstanceOf(Function);
  expect(_require.extensions[".json"]).toBeInstanceOf(Function);
  expect(_extensions).toBe(_require.extensions);
  expect(createRequire(import.meta.url).extensions).toBe(_require.extensions);
});

it("runtime mutations to require.extensions are visible to all require functions", () => {
  const calls: string[] = [];
  const original = _require.extensions[".js"];
  const custom = function (mod: typeof Module.prototype, filename: string) {
    calls.push(filename);
    mod._compile("module.exports = 'hooked';", filename);
  };
  try {
    _require.extensions[".js"] = custom;
    expect(_extensions[".js"]).toBe(custom);
    expect(createRequire(import.meta.url).extensions[".js"]).toBe(custom);

    const target = `${EXT_FIXTURES}/compile-target.js`;
    delete _require.cache[target];
    expect(_require(target)).toBe("hooked");
    expect(calls).toContain(target);

    delete _require.extensions[".js"];
    expect(_extensions[".js"]).toBeUndefined();
    delete _require.cache[target];
    expect(_require(target)).toBe("original");
  } finally {
    _require.extensions[".js"] = original;
  }
});

it("wrapping the default .js handler via _compile transforms exports", () => {
  const original = _require.extensions[".js"];
  const target = `${EXT_FIXTURES}/d.js`;
  try {
    delete _require.cache[target];
    _require.extensions[".js"] = function (mod, filename) {
      const originalCompile = mod._compile;
      mod._compile = function (code: string, compileFilename: string) {
        originalCompile.call(mod, 'module.exports = "transformed";', compileFilename);
      };
      original(mod, filename);
    };
    expect(_require(target)).toBe("transformed");

    _require.extensions[".js"] = original;
    delete _require.cache[target];
    expect(_require(target)).toBe("d.js");
  } finally {
    _require.extensions[".js"] = original;
  }
});

it("custom extension handlers support explicit and extensionless require", () => {
  const customPath = `${EXT_FIXTURES}/config.custom`;
  const extensionlessPath = `${EXT_FIXTURES}/config`;
  const handler = function (mod: typeof Module.prototype, filename: string) {
    mod.exports = { loaded: filename };
  };
  try {
    _require.extensions[".custom"] = handler;
    const extensionlessResolved = _require.resolve(extensionlessPath);
    delete _require.cache[customPath];
    delete _require.cache[extensionlessResolved];

    expect(_require(customPath)).toEqual({ loaded: customPath });
    expect(extensionlessResolved).toContain("config.custom");
    expect(_require(extensionlessPath)).toEqual({ loaded: extensionlessResolved });

    const localRequire = createRequire(import.meta.url);
    delete _require.cache[customPath];
    expect(localRequire(customPath)).toEqual({ loaded: customPath });
  } finally {
    delete _require.extensions[".custom"];
  }
});

it("nested require keeps parent module context for subsequent relative imports", () => {
  const parentPath = `${EXT_FIXTURES}/nested/parent.js`;
  delete _require.cache[parentPath];
  delete _require.cache[`${EXT_FIXTURES}/nested/child.js`];
  delete _require.cache[`${EXT_FIXTURES}/nested/after-child.js`];
  expect(_require(parentPath)).toEqual({ child: "child", after: "after" });
});

it("handler errors roll back load state so a later require can succeed", () => {
  const target = `${EXT_FIXTURES}/error-target.js`;
  const original = _require.extensions[".js"];
  const failing = function () {
    throw new Error("boom");
  };
  try {
    delete _require.cache[target];
    _require.extensions[".js"] = failing;
    expect(() => _require(target)).toThrow("boom");

    _require.extensions[".js"] = original;
    delete _require.cache[target];
    expect(_require(target)).toBe("ok");
  } finally {
    _require.extensions[".js"] = original;
  }
});

it("composite extension names prefer the first registered match", () => {
  const target = `${EXT_FIXTURES}/file.test.js`;
  const originalJs = _require.extensions[".js"];
  try {
    delete _require.cache[target];
    _require.extensions[".test.js"] = function (mod: typeof Module.prototype) {
      mod.exports = "from-test-handler";
    };
    expect(_require(target)).toBe("from-test-handler");
  } finally {
    delete _require.extensions[".test.js"];
    _require.extensions[".js"] = originalJs;
  }
});

it("default .js loading runs through _compile even when source mentions import", () => {
  const target = `${EXT_FIXTURES}/false-esm.js`;
  let compileCalled = false;
  const original = _require.extensions[".js"];
  try {
    delete _require.cache[target];
    _require.extensions[".js"] = function (mod: typeof Module.prototype, filename: string) {
      const compile = mod._compile;
      mod._compile = function (code: string, compileFilename: string) {
        compileCalled = true;
        compile.call(mod, code, compileFilename);
      };
      original(mod, filename);
    };
    expect(_require(target)).toBe("import something");
    expect(compileCalled).toBe(true);
  } finally {
    _require.extensions[".js"] = original;
  }
});

it("native _compile falls back to import loader for ESM-style .js files", () => {
  const target = `${CWD}/fixtures/hello.js`;
  delete _require.cache[target];
  expect(_require(target).hello).toBe("hello world!");
});

it("native _compile preserves syntax errors for invalid JavaScript", () => {
  const target = `${EXT_FIXTURES}/broken.js`;
  delete _require.cache[target];
  expect(() => _require(target)).toThrow();
});

it("_compile uses inline source instead of re-reading the file on ESM fallback", () => {
  const target = `${CWD}/fixtures/hello.js`;
  const mod = new Module(target);
  mod._compile("export default 123;", target);
  expect(mod.exports.default).toBe(123);
});

it("custom require extensions do not affect ESM import resolution", async () => {
  const extensionlessPath = `${EXT_FIXTURES}/config`;
  const handler = function (mod: typeof Module.prototype, filename: string) {
    mod.exports = { loaded: filename };
  };
  try {
    _require.extensions[".custom"] = handler;
    expect(_require.resolve(extensionlessPath)).toContain("config.custom");

    let importFailed = false;
    try {
      await import(extensionlessPath);
    } catch {
      importFailed = true;
    }
    expect(importFailed).toBe(true);
  } finally {
    delete _require.extensions[".custom"];
  }
});

it("new Module(...)._compile runs in module context", () => {
  const filename = `${EXT_FIXTURES}/d.js`;
  const mod = new Module(filename);
  expect(mod.paths.length).toBeGreaterThan(0);
  expect(mod.parent).toBeNull();
  mod._compile('module.exports = { via: "compile", paths: module.paths.length };', filename);
  expect(mod.exports).toEqual({ via: "compile", paths: mod.paths.length });
  expect(mod.loaded).toBe(false);
});

it("CommonJS relative import() resolves from the compiling filename", async () => {
  const parent = `${EXT_FIXTURES}/dynamic-import-parent.cjs`;
  delete _require.cache[parent];
  delete _require.cache[`${EXT_FIXTURES}/dynamic-import-child.cjs`];

  // require() → Module._load → _compile; relative import() must not use eval_script.
  const loaded = _require(parent);
  const child = await loaded;
  expect(child.marker).toBe("dynamic-import-child");
});
