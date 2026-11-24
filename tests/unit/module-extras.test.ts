import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { pathToFileURL } from "node:url";
import Module, {
  SourceMap,
  constants,
  createRequire,
  findPackageJSON,
  findSourceMap,
  flushCompileCache,
  getSourceMapsSupport,
  setSourceMapsSupport,
} from "node:module";

// Expectations were captured by running the same probes on Node 24 and
// checking them against refs/node/lib. Everything here is portable: the file
// also passes under host Vitest.

const requireHere = createRequire(path.join(os.tmpdir(), "module-extras-require.js"));
const root = fs.mkdtempSync(path.join(os.tmpdir(), "module-extras-"));

const write = (rel: string, contents: string) => {
  const file = path.join(root, rel);
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, contents);
  return file;
};

const codeOf = (fn: () => unknown): string | undefined => {
  try {
    fn();
  } catch (error: any) {
    return error.code;
  }
  return "did not throw";
};

const appJson = write("pkg/package.json", '{"name":"app","version":"1.0.0"}');
const appEntry = write("pkg/src/a.js", "module.exports = 2;\n");
const depJson = write("pkg/node_modules/dep/package.json", '{"name":"dep","main":"lib/index.js"}');
write("pkg/node_modules/dep/lib/index.js", "module.exports = 1;\n");
const selfJson = write("pkg/self/package.json", '{"name":"self","exports":"./i.js"}');
write("pkg/bad/package.json", "{bad");
write("pkg/bad/f.js", "x\n");

const payload = {
  version: 3,
  file: "out.js",
  sources: ["a.ts"],
  sourcesContent: ["let a = 1;"],
  names: ["a"],
  mappings: "AAAAA,IAAIA,CAAC",
};

describe("module.constants", () => {
  it("exposes the compile cache statuses as a frozen null-prototype object", () => {
    expect(constants.compileCacheStatus).toEqual({
      FAILED: 0,
      ENABLED: 1,
      ALREADY_ENABLED: 2,
      DISABLED: 3,
    });
    expect(Object.isFrozen(constants)).toBe(true);
    expect(Object.isFrozen(constants.compileCacheStatus)).toBe(true);
    expect(Object.getPrototypeOf(constants)).toBe(null);
  });

  it("is reachable from the Module constructor and from node:module", () => {
    expect(Module.constants).toBe(constants);
  });
});

describe("module.SourceMap", () => {
  it("is exposed on both the constructor and the named exports", () => {
    expect(Module.SourceMap).toBe(SourceMap);
    expect(SourceMap.length).toBe(1);
  });

  it("returns a copy of the payload", () => {
    const map = new SourceMap(payload);
    expect(map.payload).toEqual(payload);
    map.payload.sources.push("x");
    expect(map.payload.sources).toEqual(["a.ts"]);
  });

  it("looks entries up by generated position", () => {
    const map = new SourceMap(payload);
    expect(map.findEntry(0, 0)).toEqual({
      generatedLine: 0,
      generatedColumn: 0,
      originalSource: "a.ts",
      originalLine: 0,
      originalColumn: 0,
      name: "a",
    });
    expect(map.findEntry(0, 5)).toEqual({
      generatedLine: 0,
      generatedColumn: 5,
      originalSource: "a.ts",
      originalLine: 0,
      originalColumn: 5,
      name: "a",
    });
    expect(map.findEntry(9, 9).originalSource).toBe("a.ts");
  });

  it("maps 1-indexed call sites back to the original source", () => {
    const map = new SourceMap(payload);
    expect(map.findOrigin(1, 1)).toEqual({
      name: "a",
      fileName: "a.ts",
      lineNumber: 1,
      columnNumber: 1,
    });
    expect(map.findOrigin(1, 7)).toEqual({
      name: "a",
      fileName: "a.ts",
      lineNumber: 1,
      columnNumber: 7,
    });
  });

  it("returns an empty object when nothing maps", () => {
    const empty = new SourceMap({ version: 3, sources: [], names: [], mappings: "" });
    expect(empty.findEntry(0, 0)).toEqual({});
    expect(empty.findOrigin(1, 1)).toEqual({});
  });

  it("reads index maps with sections", () => {
    const map = new SourceMap({
      version: 3,
      sections: [{ offset: { line: 1, column: 0 }, map: payload }],
    } as any);
    expect(map.findEntry(1, 0)).toEqual({
      generatedLine: 1,
      generatedColumn: 0,
      originalSource: "a.ts",
      originalLine: 0,
      originalColumn: 0,
      name: "a",
    });
    expect(map.findEntry(0, 0)).toEqual({});
  });

  it("decodes negative VLQ deltas", () => {
    // "CAEA" then a negative source column step ("CAAD" => -1 column).
    const map = new SourceMap({
      version: 3,
      sources: ["a.ts"],
      names: [],
      mappings: "CAAK,EAAD",
    });
    expect(map.findEntry(0, 1).originalColumn).toBe(5);
    expect(map.findEntry(0, 3).originalColumn).toBe(4);
  });

  it("keeps lineLengths only when given a non-empty array", () => {
    expect(new SourceMap(payload).lineLengths).toBeUndefined();
    expect(new SourceMap(payload, { lineLengths: [] }).lineLengths).toBeUndefined();
    const lengths = [10, 5];
    const map = new SourceMap(payload, { lineLengths: lengths });
    expect(map.lineLengths).toEqual([10, 5]);
    expect(map.lineLengths).not.toBe(lengths);
  });

  it("rejects a payload that is not an object", () => {
    expect(codeOf(() => new (SourceMap as any)())).toBe("ERR_INVALID_ARG_TYPE");
    expect(codeOf(() => new (SourceMap as any)(1))).toBe("ERR_INVALID_ARG_TYPE");
    expect(codeOf(() => new (SourceMap as any)([]))).toBe("ERR_INVALID_ARG_TYPE");
    expect(() => new (SourceMap as any)(1)).toThrow(
      'The "payload" argument must be of type object. Received type number (1)'
    );
  });
});

describe("module.findSourceMap", () => {
  const generated = write(
    "maps/gen.js",
    `module.exports = 1;\n//# sourceMappingURL=data:application/json;base64,${Buffer.from(
      JSON.stringify(payload)
    ).toString("base64")}\n`
  );

  afterEach(() => setSourceMapsSupport(false));

  it("starts with source map support disabled", () => {
    expect(getSourceMapsSupport()).toEqual({
      enabled: false,
      nodeModules: false,
      generatedCode: false,
    });
  });

  it("returns undefined for anything that is not a known source", () => {
    expect(findSourceMap(1 as any)).toBeUndefined();
    expect(findSourceMap(undefined as any)).toBeUndefined();
    expect(findSourceMap("node:fs")).toBeUndefined();
    expect(findSourceMap(path.join(root, "missing.js"))).toBeUndefined();
    expect(findSourceMap(appEntry)).toBeUndefined();
  });

  it("does not look at a file while support is disabled", () => {
    requireHere(generated);
    expect(findSourceMap(generated)).toBeUndefined();
  });

  it("finds the map of a loaded file once support is enabled", () => {
    setSourceMapsSupport(true);
    const loaded = write(
      "maps/loaded.js",
      `module.exports = 1;\n//# sourceMappingURL=data:application/json;base64,${Buffer.from(
        JSON.stringify(payload)
      ).toString("base64")}\n`
    );
    requireHere(loaded);
    const map = findSourceMap(loaded);
    expect(map).toBeInstanceOf(SourceMap);
    expect(map!.findOrigin(1, 1).fileName).toBe(pathToFileURL(path.join(root, "maps/a.ts")).href);
    expect(findSourceMap(pathToFileURL(loaded).href)).toBe(map);
  });

  it("ignores files under node_modules unless asked", () => {
    setSourceMapsSupport(true);
    const inside = write(
      "pkg/node_modules/dep/lib/mapped.js",
      `module.exports = 1;\n//# sourceMappingURL=data:application/json;base64,${Buffer.from(
        JSON.stringify(payload)
      ).toString("base64")}\n`
    );
    requireHere(inside);
    expect(findSourceMap(inside)).toBeUndefined();
    setSourceMapsSupport(true, { nodeModules: true });
    const insideLater = write(
      "pkg/node_modules/dep/lib/mapped-later.js",
      `module.exports = 1;\n//# sourceMappingURL=data:application/json;base64,${Buffer.from(
        JSON.stringify(payload)
      ).toString("base64")}\n`
    );
    requireHere(insideLater);
    expect(findSourceMap(insideLater)).toBeInstanceOf(SourceMap);
  });

  it("validates setSourceMapsSupport arguments and freezes the result", () => {
    expect(codeOf(() => setSourceMapsSupport("yes" as any))).toBe("ERR_INVALID_ARG_TYPE");
    expect(codeOf(() => setSourceMapsSupport(true, null as any))).toBe("ERR_INVALID_ARG_TYPE");
    expect(codeOf(() => setSourceMapsSupport(true, { nodeModules: 1 as any }))).toBe(
      "ERR_INVALID_ARG_TYPE"
    );
    setSourceMapsSupport(true, { generatedCode: true });
    const support = getSourceMapsSupport();
    expect(support).toEqual({ enabled: true, nodeModules: false, generatedCode: true });
    expect(Object.isFrozen(support)).toBe(true);
  });

  it("can be assigned without a guard the way Next.js does", () => {
    const m = requireHere("module");
    m.findSourceMap = m.findSourceMap;
    expect(typeof m.findSourceMap).toBe("function");
  });
});

describe("module.findPackageJSON", () => {
  const base = appEntry;

  it("needs a specifier", () => {
    expect(codeOf(() => (findPackageJSON as any)())).toBe("ERR_MISSING_ARGS");
  });

  it("finds the package.json nearest to a relative specifier", () => {
    expect(findPackageJSON("./a.js", base)).toBe(appJson);
    expect(findPackageJSON("../src/a.js", base)).toBe(appJson);
    expect(findPackageJSON("../src/a.js?x=1#h", base)).toBe(appJson);
  });

  it("accepts file URLs as specifier and as base", () => {
    expect(findPackageJSON(pathToFileURL(appEntry).href)).toBe(appJson);
    expect(findPackageJSON(pathToFileURL(appEntry))).toBe(appJson);
    expect(findPackageJSON("../src/a.js", pathToFileURL(base))).toBe(appJson);
    expect(findPackageJSON(pathToFileURL(appEntry).href, "data:")).toBe(appJson);
  });

  it("resolves a bare specifier to the package it names", () => {
    expect(findPackageJSON("dep", base)).toBe(depJson);
    expect(findPackageJSON("dep/lib/index.js", base)).toBe(depJson);
    expect(findPackageJSON("dep", pathToFileURL(base))).toBe(depJson);
  });

  it("resolves the enclosing package by name when it has exports", () => {
    expect(findPackageJSON("self", path.join(root, "pkg/self/x.js"))).toBe(selfJson);
    expect(codeOf(() => findPackageJSON("app", base))).toBe("ERR_MODULE_NOT_FOUND");
  });

  it("treats a trailing slash as a directory without checking it exists", () => {
    expect(findPackageJSON("../self/", base)).toBe(selfJson);
    expect(findPackageJSON("./nowhere/", base)).toBe(appJson);
    expect(findPackageJSON("../self", base)).toBe(appJson);
  });

  it("starts at the package.json of a file inside node_modules", () => {
    expect(
      findPackageJSON(pathToFileURL(path.join(root, "pkg/node_modules/dep/lib/index.js")).href)
    ).toBe(depJson);
  });

  it("reports what could not be found", () => {
    expect(codeOf(() => findPackageJSON("./nope.js", base))).toBe("ERR_MODULE_NOT_FOUND");
    expect(() => findPackageJSON("./nope.js", base)).toThrow(
      `Cannot find module '${path.join(root, "pkg/src/nope.js")}' imported from ${base}`
    );
    expect(codeOf(() => findPackageJSON("missing-pkg", base))).toBe("ERR_MODULE_NOT_FOUND");
    expect(() => findPackageJSON("missing-pkg", base)).toThrow(
      `Cannot find package 'missing-pkg' imported from ${base}`
    );
    expect(codeOf(() => findPackageJSON("@scope/missing", base))).toBe("ERR_MODULE_NOT_FOUND");
    expect(codeOf(() => findPackageJSON("fs", base))).toBe("ERR_MODULE_NOT_FOUND");
  });

  it("rejects specifiers it cannot map to a file", () => {
    expect(codeOf(() => findPackageJSON("node:fs", base))).toBe("ERR_INVALID_URL_SCHEME");
    expect(codeOf(() => findPackageJSON("https://example.com/a.js", base))).toBe(
      "ERR_INVALID_URL_SCHEME"
    );
    expect(codeOf(() => findPackageJSON("@scope", base))).toBe("ERR_INVALID_MODULE_SPECIFIER");
    expect(codeOf(() => findPackageJSON(appEntry))).toBe("ERR_UNSUPPORTED_RESOLVE_REQUEST");
    expect(codeOf(() => findPackageJSON("./x.js"))).toBe("ERR_UNSUPPORTED_RESOLVE_REQUEST");
    expect(codeOf(() => findPackageJSON("./x.js", "./y.js"))).toBe("ERR_INVALID_URL");
    expect(codeOf(() => findPackageJSON("./a", 5 as any))).toBe("ERR_INVALID_ARG_TYPE");
  });

  it("reports a package.json that cannot be parsed", () => {
    const code = codeOf(() => findPackageJSON("./f.js", path.join(root, "pkg/bad/x.js")));
    expect(code).toBe("ERR_INVALID_PACKAGE_CONFIG");
  });
});

describe("module.flushCompileCache", () => {
  it("is a no-op that returns undefined", () => {
    expect(flushCompileCache()).toBeUndefined();
  });

  it("only accepts a boolean", () => {
    expect(codeOf(() => (flushCompileCache as any)("x"))).toBe("ERR_INVALID_ARG_TYPE");
  });
});

afterAll(() => {
  fs.rmSync(root, { recursive: true, force: true });
});
