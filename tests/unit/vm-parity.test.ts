import vm, {
  Module,
  Script,
  SourceTextModule,
  SyntheticModule,
  compileFunction,
  constants,
  createContext,
  createScript,
  isContext,
  measureMemory,
  runInContext,
  runInNewContext,
  runInThisContext,
} from "node:vm";

it("exports the complete node:vm API surface on both ESM and CJS", () => {
  const cjsVm = require("node:vm");
  const expectedNames = [
    "Module",
    "Script",
    "SourceTextModule",
    "SyntheticModule",
    "compileFunction",
    "constants",
    "createContext",
    "createScript",
    "isContext",
    "measureMemory",
    "runInContext",
    "runInNewContext",
    "runInThisContext",
  ];
  for (const name of expectedNames) {
    expect((vm as Record<string, unknown>)[name]).toBeDefined();
    expect(cjsVm[name]).toBe((vm as Record<string, unknown>)[name]);
  }
  expect(Script).toBeTypeOf("function");
  expect(Module).toBeTypeOf("function");
  expect(SourceTextModule).toBeTypeOf("function");
  expect(SyntheticModule).toBeTypeOf("function");
  expect(createScript).toBeTypeOf("function");
  expect(createContext).toBeTypeOf("function");
  expect(isContext).toBeTypeOf("function");
  expect(runInContext).toBeTypeOf("function");
  expect(runInNewContext).toBeTypeOf("function");
  expect(runInThisContext).toBeTypeOf("function");
  expect(compileFunction).toBeTypeOf("function");
  expect(measureMemory).toBeTypeOf("function");

  expect(Object.getPrototypeOf(constants)).toBeNull();
  expect(Object.isFrozen(constants)).toBe(true);
  expect(typeof constants.DONT_CONTEXTIFY).toBe("symbol");
  expect(typeof constants.USE_MAIN_CONTEXT_DEFAULT_LOADER).toBe("symbol");
});

it("validates isContext arguments and tracks contextification", () => {
  for (const bad of [null, undefined, 1, "str", true, Symbol("s"), () => {}]) {
    expect(() => isContext(bad as unknown as object)).toThrow(TypeError);
  }
  const plain = {};
  const arr = [1, 2];
  expect(isContext(plain)).toBe(false);
  expect(isContext(arr)).toBe(false);

  const created = createContext(plain);
  expect(created).toBe(plain);
  expect(isContext(plain)).toBe(true);
  // Idempotent
  expect(createContext(plain)).toBe(plain);

  // runInNewContext contextifies its sandbox so runInContext works afterwards
  const sb: Record<string, unknown> = { a: 1 };
  runInNewContext("let b = 41; var c = a + b;", sb);
  expect(isContext(sb)).toBe(true);
  expect(sb.c).toBe(42);
  expect(runInContext("b + c", sb)).toBe(83);
});

it("implements live contextification with isolated realm intrinsics, symbols, and descriptors", () => {
  const sym = Symbol("hiddenSym");
  const proto = { inheritedVal: 77 };
  const sandbox: Record<string | symbol, unknown> = Object.create(proto);
  sandbox.x = 10;
  sandbox[sym] = "symVal";
  Object.defineProperty(sandbox, "hidden", {
    value: "secret",
    enumerable: false,
    configurable: true,
    writable: true,
  });
  Object.defineProperty(sandbox, "selfThis", {
    enumerable: true,
    configurable: true,
    get() {
      return this;
    },
  });

  const ctx = createContext(sandbox);
  expect(runInContext("globalThis === this", ctx)).toBe(true);
  expect(runInContext("globalThis !== sandboxRef", Object.assign(sandbox, { sandboxRef: sandbox }))).toBe(true);
  expect(runInContext("Object !== hostObj", Object.assign(sandbox, { hostObj: Object }))).toBe(true);
  expect(runInContext("selfThis === sandboxRef", ctx)).toBe(true);
  expect(runInContext("hidden", ctx)).toBe("secret");
  expect(runInContext("globalThis[s]", Object.assign(sandbox, { s: sym }))).toBe("symVal");
  expect(runInContext("inheritedVal", ctx)).toBe(77);
  // Builtin Object.prototype properties still come from the child realm, not the host realm
  expect(runInContext("toString === Object.prototype.toString", ctx)).toBe(true);

  const visibleKeys = runInContext("Object.keys(globalThis)", ctx) as string[];
  expect(visibleKeys).toContain("x");
  expect(visibleKeys).not.toContain("hidden");
  expect(visibleKeys).not.toContain("Array");

  const ownNames = runInContext("Object.getOwnPropertyNames(globalThis)", ctx) as string[];
  expect(ownNames).toContain("hidden");
  expect(ownNames).toContain("Array");

  // Sloppy write to non-writable builtin `undefined` must not mutate sandbox
  runInContext("undefined = 123;", ctx);
  expect(Object.prototype.hasOwnProperty.call(sandbox, "undefined")).toBe(false);

  // Persistent lexical & var bindings across runs
  runInContext("let lex = 5; const cst = 6; var vr = 7;", ctx);
  expect(runInContext("lex + cst + vr", ctx)).toBe(18);
  expect(sandbox.vr).toBe(7);
  expect(Object.prototype.hasOwnProperty.call(sandbox, "lex")).toBe(false);

  // delete global property propagates to sandbox
  expect(runInContext("delete x", ctx)).toBe(true);
  expect("x" in sandbox).toBe(false);
});

it("supports vm.constants.DONT_CONTEXTIFY", () => {
  const c = createContext(constants.DONT_CONTEXTIFY) as Record<string, unknown>;
  expect(isContext(c)).toBe(true);
  expect(runInContext("globalThis === this", c)).toBe(true);
  expect(runInContext("globalThis", c)).toBe(c);
  runInContext("var added = 123;", c);
  expect(c.added).toBe(123);

  const res = runInNewContext("globalThis.v = 9; v", constants.DONT_CONTEXTIFY);
  expect(res).toBe(9);
});

it("enforces codeGeneration.strings and codeGeneration.wasm", async () => {
  const wasmBytes = new Uint8Array([0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00]);
  const ctx = createContext(
    { wasmBytes },
    { codeGeneration: { strings: false, wasm: false } }
  );

  const ChildEvalError = runInContext("EvalError", ctx) as ErrorConstructor;
  for (const snippet of [
    "eval('1 + 1')",
    "eval(123)",
    "new Function('return 1')()",
  ]) {
    let cgErr: unknown;
    try {
      runInContext(snippet, ctx);
    } catch (e) {
      cgErr = e;
    }
    expect(cgErr).toBeDefined();
    expect((cgErr as { name?: string }).name).toBe("EvalError");
    expect(cgErr).toBeInstanceOf(ChildEvalError);
  }

  expect(runInContext("WebAssembly.validate(wasmBytes)", ctx)).toBe(true);
  let syncErr: unknown;
  try {
    runInContext("new WebAssembly.Module(wasmBytes)", ctx);
  } catch (e) {
    syncErr = e;
  }
  expect(syncErr).toBeDefined();
  expect((syncErr as Error).name).toBe("CompileError");
  expect((syncErr as Error).message).toContain("disallowed by embedder");

  let asyncCompileErr: unknown;
  try {
    await (runInContext("WebAssembly.compile(wasmBytes)", ctx) as Promise<unknown>);
  } catch (e) {
    asyncCompileErr = e;
  }
  expect((asyncCompileErr as Error).name).toBe("CompileError");

  let asyncInstErr: unknown;
  try {
    await (runInContext("WebAssembly.instantiate(wasmBytes)", ctx) as Promise<unknown>);
  } catch (e) {
    asyncInstErr = e;
  }
  expect((asyncInstErr as Error).name).toBe("CompileError");
});

it("supports microtaskMode: 'afterEvaluate' and catches infinite microtask loops with timeout", () => {
  const sb: Record<string, unknown> = { done: false };
  const ctx = createContext(sb, { microtaskMode: "afterEvaluate" });
  runInContext("Promise.resolve(42).then((v) => { done = v; })", ctx);
  expect(sb.done).toBe(42);

  let err: unknown;
  try {
    runInContext(
      "Promise.resolve().then(function loop() { return Promise.resolve().then(loop); })",
      ctx,
      { timeout: 30 }
    );
  } catch (e) {
    err = e;
  }
  expect((err as { code?: string }).code).toBe("ERR_SCRIPT_EXECUTION_TIMEOUT");
});

it("implements Script, createScript, directives, cachedData, and timeouts", () => {
  const code = "base + 2\n//# sourceURL=unit-vm.js\n//# sourceMappingURL=unit-vm.js.map";
  const s1 = new Script(code, {
    filename: "unit-vm.js",
    lineOffset: 5,
    produceCachedData: true,
  });
  expect(s1.sourceURL).toBe("unit-vm.js");
  expect(s1.sourceMapURL).toBe("unit-vm.js.map");
  expect(s1.cachedDataProduced).toBe(true);
  expect(Buffer.isBuffer(s1.cachedData)).toBe(true);

  const exportedBuf = s1.createCachedData();
  expect(Buffer.isBuffer(exportedBuf)).toBe(true);

  const s2 = createScript(code, {
    filename: "unit-vm.js",
    lineOffset: 5,
    cachedData: exportedBuf,
  });
  expect(s2.cachedDataRejected).toBe(false);
  expect(s2.runInNewContext({ base: 40 })).toBe(42);

  const sBad = new Script(code, {
    filename: "unit-vm.js",
    lineOffset: 5,
    cachedData: Buffer.from("invalid-cache-bytes"),
  });
  expect(sBad.cachedDataRejected).toBe(true);
  expect(sBad.runInNewContext({ base: 10 })).toBe(12);

  expect(() => new Script("1", { cachedData: "bad" as unknown as Buffer })).toThrow(TypeError);

  const rThis = runInThisContext("100 + 23", { filename: "this-ctx.js" });
  expect(rThis).toBe(123);

  // Timeout validation
  for (const bad of ["10", null, true]) {
    expect(() => runInNewContext("1", {}, { timeout: bad as unknown as number })).toThrow(TypeError);
  }
  for (const badRange of [0, -1, 1.5, NaN, Infinity, 4294967296]) {
    let err: unknown;
    try {
      runInNewContext("1", {}, { timeout: badRange });
    } catch (e) {
      err = e;
    }
    expect((err as { code?: string }).code).toBe("ERR_OUT_OF_RANGE");
  }

  // Timeout triggers catchably in outer try/catch
  let timeoutErr: unknown;
  try {
    runInNewContext("while (true) {}", {}, { timeout: 25 });
  } catch (e) {
    timeoutErr = e;
  }
  expect(timeoutErr).toBeInstanceOf(Error);
  expect((timeoutErr as { code?: string }).code).toBe("ERR_SCRIPT_EXECUTION_TIMEOUT");
});

it("implements compileFunction with params, parsingContext, contextExtensions, and cachedData", () => {
  const ctx = createContext({ multiplier: 3 });
  const ext1 = { offset: 1, shadow: 100 };
  const ext2 = { shadow: 2 };
  const fn = compileFunction("offset += 4; return (a + b) * multiplier + offset + shadow;", ["a", "b"], {
    parsingContext: ctx,
    contextExtensions: [ext1, ext2],
    produceCachedData: true,
    filename: "compiled-fn.js",
  }) as ((a: number, b: number) => number) & {
    cachedData?: Buffer;
    cachedDataProduced?: boolean;
  };

  expect(fn.cachedDataProduced).toBe(true);
  expect(Buffer.isBuffer(fn.cachedData)).toBe(true);
  expect(fn(4, 6)).toBe(37); // (4+6)*3 + 5 + 2
  expect(ext1.offset).toBe(5);

  const fnFromCache = compileFunction(
    "return (a + b) * 2;",
    ["a", "b"],
    { cachedData: Buffer.from("corrupt") }
  ) as ((a: number, b: number) => number) & { cachedDataRejected?: boolean };
  expect(fnFromCache.cachedDataRejected).toBe(true);
  expect(fnFromCache(3, 4)).toBe(14);
});

it("implements importModuleDynamically on Script and SourceTextModule", async () => {
  const sMissing = new Script("import('nope')");
  let missingErr: unknown;
  try {
    await (sMissing.runInNewContext({}) as Promise<unknown>);
  } catch (e) {
    missingErr = e;
  }
  expect((missingErr as { code?: string }).code).toBe("ERR_VM_DYNAMIC_IMPORT_CALLBACK_MISSING");

  const sDefault = new Script("import('node:path')", {
    importModuleDynamically: constants.USE_MAIN_CONTEXT_DEFAULT_LOADER,
  });
  const pathNs = await (sDefault.runInNewContext({}) as Promise<{
    join: (...args: string[]) => string;
  }>);
  expect(typeof pathNs.join).toBe("function");
  expect(pathNs.join("foo", "bar")).toContain("bar");

  const calls: Array<{ spec: string; sameRef: boolean; attrsProto: unknown; typeAttr: unknown }> = [];
  const ctx = createContext({ factor: 6 });
  const sCustom = new Script("import('dyn-dep', { with: { type: 'custom' } })", {
    async importModuleDynamically(specifier: string, referencingScriptOrModule: unknown, importAttributes: Record<string, string>) {
      calls.push({
        spec: specifier,
        sameRef: referencingScriptOrModule === sCustom,
        attrsProto: Object.getPrototypeOf(importAttributes),
        typeAttr: importAttributes.type,
      });
      const mod = new SourceTextModule("export const answer = factor * 7;", { context: ctx });
      await mod.link(() => {
        throw new Error("unexpected link");
      });
      await mod.evaluate();
      return mod;
    },
  });
  const ns = await (sCustom.runInContext(ctx) as Promise<{ answer: number }>);
  expect(ns.answer).toBe(42);
  expect(calls.length).toBe(1);
  expect(calls[0]).toEqual({
    spec: "dyn-dep",
    sameRef: true,
    attrsProto: null,
    typeAttr: "custom",
  });
});

it("implements Module, SourceTextModule, SyntheticModule, cyclic links, TLA, import.meta, and cache", async () => {
  expect(() => new (Module as unknown as new () => unknown)()).toThrow(TypeError);

  const ctx = createContext({ base: 10 });
  const syn = new SyntheticModule(
    ["val", "extra"],
    function () {
      this.setExport("val", 30);
      this.setExport("extra", 2);
    },
    { identifier: "vm:syn", context: ctx }
  );
  expect(syn instanceof Module).toBe(true);
  expect(syn.status).toBe("unlinked");
  expect(syn.dependencySpecifiers).toBeUndefined();
  expect(() => syn.setExport("val", 1)).toThrow(/linked/);

  const depA = new SourceTextModule(
    `import { getB } from "mod-b";
     import { val } from "vm:syn";
     export const aVal = 1 + val;
     export function sumCycle() { return aVal + getB(); }`,
    { identifier: "mod-a", context: ctx }
  );
  const depB = new SourceTextModule(
    `import { aVal } from "mod-a";
     export function getB() { return aVal + base; }`,
    {
      identifier: "mod-b",
      context: ctx,
      initializeImportMeta(meta: { url?: string }) {
        meta.url = "custom://mod-b";
      },
    }
  );
  const entry = new SourceTextModule(
    `import { sumCycle } from "mod-a";
     import { getB } from "mod-b";
     const awaited = await Promise.resolve(getB());
     export const total = sumCycle() + awaited;`,
    {
      identifier: "entry.mjs",
      context: ctx,
      initializeImportMeta(meta: { url?: string; tag?: string }, mod: unknown) {
        meta.url = "custom://entry.mjs";
        meta.tag = (mod as { identifier: string }).identifier;
      },
    }
  );
  expect(entry instanceof Module).toBe(true);
  expect(entry.status).toBe("unlinked");
  expect(Array.from(entry.dependencySpecifiers ?? [])).toEqual(["mod-a", "mod-b"]);

  let nsBeforeErr: unknown;
  try {
    void entry.namespace;
  } catch (e) {
    nsBeforeErr = e;
  }
  expect((nsBeforeErr as { code?: string }).code).toBe("ERR_VM_MODULE_STATUS");

  const modCacheBuf = entry.createCachedData();
  expect(Buffer.isBuffer(modCacheBuf)).toBe(true);
  const cachedEntry = new SourceTextModule(
    `import { sumCycle } from "mod-a";
     import { getB } from "mod-b";
     const awaited = await Promise.resolve(getB());
     export const total = sumCycle() + awaited;`,
    { identifier: "entry.mjs", context: ctx, cachedData: modCacheBuf }
  );
  expect((cachedEntry as { cachedDataRejected?: boolean }).cachedDataRejected).toBe(false);

  const bySpec: Record<string, Module> = {
    "mod-a": depA,
    "mod-b": depB,
    "vm:syn": syn,
  };
  await entry.link((spec) => bySpec[spec]);
  expect(entry.status).toBe("linked");
  expect(syn.status).toBe("linked");

  let relinkErr: unknown;
  try {
    await entry.link(() => syn);
  } catch (e) {
    relinkErr = e;
  }
  expect((relinkErr as { code?: string }).code).toBe("ERR_VM_MODULE_ALREADY_LINKED");

  // Linking across different contexts must reject
  const otherCtx = createContext({});
  const foreign = new SourceTextModule("export const x = 1;", { context: otherCtx });
  const hostMod = new SourceTextModule("import { x } from 'x';", { context: ctx });
  let diffCtxErr: unknown;
  try {
    await hostMod.link(() => foreign);
  } catch (e) {
    diffCtxErr = e;
  }
  expect((diffCtxErr as { code?: string }).code).toBe("ERR_VM_MODULE_DIFFERENT_CONTEXT");

  await entry.evaluate();
  expect(entry.status).toBe("evaluated");
  // aVal = 1 + 30 = 31; bVal = 31 + 10 = 41; sumCycle = 72; total = 72 + 41 = 113
  expect((entry.namespace as { total: number }).total).toBe(113);

  // createCachedData after evaluate throws ERR_VM_MODULE_CANNOT_CREATE_CACHED_DATA
  let cacheAfterErr: unknown;
  try {
    entry.createCachedData();
  } catch (e) {
    cacheAfterErr = e;
  }
  expect((cacheAfterErr as { code?: string }).code).toBe(
    "ERR_VM_MODULE_CANNOT_CREATE_CACHED_DATA"
  );

  // SyntheticModule live binding update & unknown export error
  syn.setExport("val", 100);
  expect((syn.namespace as { val: number }).val).toBe(100);
  expect(() => syn.setExport("missingExport", 1)).toThrow(ReferenceError);

  // Errored module preserves status & error identity across re-evaluate
  const badMod = new SourceTextModule("throw new Error('module-boom');", { context: ctx });
  await badMod.link(() => {
    throw new Error("no deps");
  });
  let firstEvalErr: unknown;
  try {
    await badMod.evaluate();
  } catch (e) {
    firstEvalErr = e;
  }
  expect(badMod.status).toBe("errored");
  expect(badMod.error).toBe(firstEvalErr);
  let secondEvalErr: unknown;
  try {
    await badMod.evaluate();
  } catch (e) {
    secondEvalErr = e;
  }
  expect(secondEvalErr).toBe(firstEvalErr);
});

it("implements measureMemory", async () => {
  expect(() => measureMemory({ mode: "bad" as "summary" })).toThrow();
  expect(() => measureMemory({ execution: "bad" as "default" })).toThrow();

  const summary = await measureMemory();
  expect(typeof summary.total.jsMemoryEstimate).toBe("number");
  expect(summary.total.jsMemoryRange.length).toBe(2);

  const detailed = (await measureMemory({ mode: "detailed", execution: "eager" })) as {
    total: { jsMemoryEstimate: number };
    current: { jsMemoryEstimate: number };
    other: unknown[];
    WebAssembly: { code: number; metadata: number };
  };
  expect(typeof detailed.current.jsMemoryEstimate).toBe("number");
  expect(Array.isArray(detailed.other)).toBe(true);
  expect(typeof detailed.WebAssembly.code).toBe("number");
});
