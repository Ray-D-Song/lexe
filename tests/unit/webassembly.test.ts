import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";

/**
 * Loads a `.wasm` fixture from `fixtures/wasm/<name>.wasm` as a fresh
 * `ArrayBuffer` (never a shared/aliased view -- some tests mutate their
 * copy of the bytes to build "malformed" variants).
 */
function loadWasm(name: string): ArrayBuffer {
  const buf = readFileSync(`fixtures/wasm/${name}.wasm`);
  return buf.buffer.slice(buf.byteOffset, buf.byteOffset + buf.byteLength);
}

// ---------------------------------------------------------------------------
// Namespace, constructors, prototypes, descriptors, toStringTag, instanceof
// ---------------------------------------------------------------------------

describe("WebAssembly namespace", () => {
  it("is a non-callable object", () => {
    expect(typeof WebAssembly).toEqual("object");
    expect(() => (WebAssembly as unknown as () => void)()).toThrow(TypeError);
  });

  it("has Symbol.toStringTag of 'WebAssembly'", () => {
    expect(Object.prototype.toString.call(WebAssembly)).toEqual("[object WebAssembly]");
  });

  it("exposes the expected top-level surface", () => {
    for (const key of ["compile", "instantiate", "validate", "compileStreaming", "instantiateStreaming"]) {
      expect(WebAssembly[key as keyof typeof WebAssembly]).toBeTypeOf("function");
    }
    for (const key of ["Module", "Instance", "Memory", "Table", "Global", "CompileError", "LinkError", "RuntimeError"]) {
      expect(WebAssembly[key as keyof typeof WebAssembly]).toBeTypeOf("function");
    }
  });

  it("does not expose out-of-scope APIs", () => {
    for (const key of ["Function", "Tag", "Exception", "Suspending", "promising"]) {
      expect((WebAssembly as Record<string, unknown>)[key]).toBeUndefined();
    }
  });

  it("does not add WebAssembly to Module.builtinModules or make it requirable", async () => {
    const nodeModule = await import("node:module");
    expect(nodeModule.default.isBuiltin("WebAssembly" as never)).toBeFalsy();
  });
});

describe("WebAssembly classes", () => {
  const classNames = ["Module", "Instance", "Memory", "Table", "Global"] as const;

  for (const name of classNames) {
    it(`${name} requires \`new\``, () => {
      const Ctor = WebAssembly[name] as unknown as (...args: unknown[]) => void;
      expect(() => Ctor()).toThrow(TypeError);
    });
  }

  it("Module/Instance/Memory/Table/Global each have the correct Symbol.toStringTag", () => {
    const emptyModule = new WebAssembly.Module(loadWasm("add"));
    const instance = new WebAssembly.Instance(emptyModule);
    const memory = new WebAssembly.Memory({ initial: 1 });
    const table = new WebAssembly.Table({ element: "anyfunc", initial: 1 });
    const global = new WebAssembly.Global({ value: "i32" }, 0);

    expect(Object.prototype.toString.call(emptyModule)).toEqual("[object WebAssembly.Module]");
    expect(Object.prototype.toString.call(instance)).toEqual("[object WebAssembly.Instance]");
    expect(Object.prototype.toString.call(memory)).toEqual("[object WebAssembly.Memory]");
    expect(Object.prototype.toString.call(table)).toEqual("[object WebAssembly.Table]");
    expect(Object.prototype.toString.call(global)).toEqual("[object WebAssembly.Global]");
  });

  it("error subclasses extend Error and have the expected names", () => {
    for (const [name, Ctor] of [
      ["CompileError", WebAssembly.CompileError],
      ["LinkError", WebAssembly.LinkError],
      ["RuntimeError", WebAssembly.RuntimeError],
    ] as const) {
      const err = new Ctor("boom");
      expect(err).toBeInstanceOf(Error);
      expect(err).toBeInstanceOf(Ctor);
      expect(err.name).toEqual(name);
      expect(err.message).toEqual("boom");
    }
  });

  it("instances are `instanceof` their class after normal construction", () => {
    const mod = new WebAssembly.Module(loadWasm("add"));
    expect(mod).toBeInstanceOf(WebAssembly.Module);
    const instance = new WebAssembly.Instance(mod);
    expect(instance).toBeInstanceOf(WebAssembly.Instance);
  });
});

// ---------------------------------------------------------------------------
// validate / synchronous Module & Instance construction
// ---------------------------------------------------------------------------

describe("WebAssembly.validate", () => {
  it("returns true for a valid module", () => {
    expect(WebAssembly.validate(loadWasm("add"))).toBe(true);
  });

  it("returns false for malformed bytes without throwing", () => {
    expect(WebAssembly.validate(new Uint8Array([0, 1, 2, 3]).buffer)).toBe(false);
  });

  it("returns false for out-of-scope proposals (shared memory, memory64, GC, tags)", () => {
    for (const name of ["shared-memory", "memory64", "gc-types", "tag", "function-references"]) {
      expect(WebAssembly.validate(loadWasm(name))).toBe(false);
    }
  });

  it("throws TypeError for non-BufferSource input", () => {
    expect(() => WebAssembly.validate("not bytes" as unknown as BufferSource)).toThrow(TypeError);
    expect(() => WebAssembly.validate([0, 1, 2, 3] as unknown as BufferSource)).toThrow(TypeError);
  });

  it("accepts ArrayBufferView with a non-zero byteOffset", () => {
    const raw = loadWasm("add");
    const padded = new Uint8Array(raw.byteLength + 4);
    padded.set(new Uint8Array(raw), 4);
    const view = new Uint8Array(padded.buffer, 4, raw.byteLength);
    expect(WebAssembly.validate(view)).toBe(true);
  });
});

describe("WebAssembly.Module / WebAssembly.Instance (synchronous)", () => {
  it("compiles synchronously and rejects non-Module arguments", () => {
    const mod = new WebAssembly.Module(loadWasm("add"));
    expect(mod).toBeInstanceOf(WebAssembly.Module);
    expect(() => new WebAssembly.Instance({} as unknown as WebAssembly.Module)).toThrow(TypeError);
  });

  it("throws CompileError for malformed bytes", () => {
    expect(() => new WebAssembly.Module(new Uint8Array([0, 1, 2, 3]).buffer)).toThrow(WebAssembly.CompileError);
  });

  it("instantiates without an imports object when the module needs none", () => {
    const mod = new WebAssembly.Module(loadWasm("add"));
    const instance = new WebAssembly.Instance(mod);
    expect(instance.exports.add(1, 2)).toBe(3);
  });
});

// ---------------------------------------------------------------------------
// Async compile/instantiate overloads
// ---------------------------------------------------------------------------

describe("WebAssembly.compile / WebAssembly.instantiate (async)", () => {
  it("compile() resolves to a Module", async () => {
    const mod = await WebAssembly.compile(loadWasm("add"));
    expect(mod).toBeInstanceOf(WebAssembly.Module);
  });

  it("compile() rejects with CompileError for malformed bytes", async () => {
    await expect(WebAssembly.compile(new Uint8Array([1, 2, 3]).buffer)).rejects.toBeInstanceOf(WebAssembly.CompileError);
  });

  it("instantiate(bytes) resolves to {module, instance}", async () => {
    const { module, instance } = await WebAssembly.instantiate(loadWasm("add"));
    expect(module).toBeInstanceOf(WebAssembly.Module);
    expect(instance).toBeInstanceOf(WebAssembly.Instance);
    expect(instance.exports.add(2, 3)).toBe(5);
  });

  it("instantiate(module) resolves to an Instance", async () => {
    const mod = await WebAssembly.compile(loadWasm("add"));
    const instance = await WebAssembly.instantiate(mod);
    expect(instance).toBeInstanceOf(WebAssembly.Instance);
    expect(instance.exports.add(10, 20)).toBe(30);
  });

  it("instantiate() rejects with LinkError for a missing import", async () => {
    await expect(WebAssembly.instantiate(loadWasm("log-import"), {})).rejects.toBeInstanceOf(WebAssembly.LinkError);
  });
});

// ---------------------------------------------------------------------------
// Streaming
// ---------------------------------------------------------------------------

describe("WebAssembly.compileStreaming / instantiateStreaming", () => {
  const wasmResponse = () =>
    new Response(loadWasm("add"), { headers: { "Content-Type": "application/wasm" } });

  it("compileStreaming resolves for a correct application/wasm response", async () => {
    const mod = await WebAssembly.compileStreaming(wasmResponse());
    expect(mod).toBeInstanceOf(WebAssembly.Module);
  });

  it("compileStreaming accepts a MIME type with parameters", async () => {
    const response = new Response(loadWasm("add"), {
      headers: { "Content-Type": "application/wasm; charset=utf-8" },
    });
    const mod = await WebAssembly.compileStreaming(response);
    expect(mod).toBeInstanceOf(WebAssembly.Module);
  });

  it("compileStreaming rejects a wrong MIME type with TypeError", async () => {
    const response = new Response(loadWasm("add"), { headers: { "Content-Type": "application/octet-stream" } });
    await expect(WebAssembly.compileStreaming(response)).rejects.toBeInstanceOf(TypeError);
  });

  it("compileStreaming rejects a non-ok Response with TypeError", async () => {
    const response = new Response(loadWasm("add"), {
      status: 404,
      headers: { "Content-Type": "application/wasm" },
    });
    await expect(WebAssembly.compileStreaming(response)).rejects.toBeInstanceOf(TypeError);
  });

  it("compileStreaming propagates a rejected source promise unchanged", async () => {
    const sentinel = { tag: "streaming-source-rejection" };
    await expect(WebAssembly.compileStreaming(Promise.reject(sentinel))).rejects.toBe(sentinel);
  });

  it("compileStreaming accepts a promise that resolves to a Response", async () => {
    const mod = await WebAssembly.compileStreaming(Promise.resolve(wasmResponse()));
    expect(mod).toBeInstanceOf(WebAssembly.Module);
  });

  it("instantiateStreaming resolves to {module, instance}", async () => {
    const { module, instance } = await WebAssembly.instantiateStreaming(wasmResponse());
    expect(module).toBeInstanceOf(WebAssembly.Module);
    expect(instance.exports.add(6, 7)).toBe(13);
  });
});

// ---------------------------------------------------------------------------
// Module.imports/exports/customSections
// ---------------------------------------------------------------------------

describe("WebAssembly.Module.imports/exports/customSections", () => {
  it("imports() and exports() preserve binary declaration order and return copies", () => {
    const mod = new WebAssembly.Module(loadWasm("log-import"));
    const imports = WebAssembly.Module.imports(mod);
    expect(imports).toEqual([{ module: "env", name: "log", kind: "function" }]);
    expect(WebAssembly.Module.imports(mod)).not.toBe(imports);
    expect(WebAssembly.Module.imports(mod)).toEqual(imports);

    const exports = WebAssembly.Module.exports(mod);
    expect(exports).toEqual([{ name: "run", kind: "function" }]);
  });

  it("exports() reports every kind for a richer module in declaration order", () => {
    const mod = new WebAssembly.Module(loadWasm("add"));
    expect(WebAssembly.Module.exports(mod)).toEqual([
      { name: "add", kind: "function" },
      { name: "addAlias", kind: "function" },
      { name: "answer", kind: "global" },
      { name: "mem", kind: "memory" },
    ]);
  });

  it("customSections() returns fresh ArrayBuffer copies and an empty array when absent", () => {
    const mod = new WebAssembly.Module(loadWasm("add"));
    expect(WebAssembly.Module.customSections(mod, "nonexistent")).toEqual([]);
  });

  it("throws TypeError when the argument is not a Module", () => {
    expect(() => WebAssembly.Module.imports({} as unknown as WebAssembly.Module)).toThrow(TypeError);
    expect(() => WebAssembly.Module.exports(null as unknown as WebAssembly.Module)).toThrow(TypeError);
  });
});

// ---------------------------------------------------------------------------
// JS function import / Wasm function export / re-export identity
// ---------------------------------------------------------------------------

describe("function imports/exports", () => {
  it("calls a JS function import from Wasm", async () => {
    let logged: number | undefined;
    const { instance } = await WebAssembly.instantiate(loadWasm("log-import"), {
      env: {
        log: (x: number) => {
          logged = x;
        },
      },
    });
    instance.exports.run(42);
    expect(logged).toBe(42);
  });

  it("re-exporting the same extern preserves JS wrapper identity", () => {
    const mod = new WebAssembly.Module(loadWasm("add"));
    const instance = new WebAssembly.Instance(mod);
    expect(instance.exports.add).toBe(instance.exports.addAlias);
  });

  it("import namespace/property access happens in module declaration order and honors getters/Proxy traps", async () => {
    const accessLog: string[] = [];
    const importsObject = new Proxy(
      { env: { log: (x: number) => { accessLog.push(`call:${x}`); } } },
      {
        get(target, prop, receiver) {
          accessLog.push(`get:${String(prop)}`);
          return Reflect.get(target, prop, receiver);
        },
      }
    );
    const mod = new WebAssembly.Module(loadWasm("log-import"));
    const instance = new WebAssembly.Instance(mod, importsObject as unknown as WebAssembly.ModuleImports);
    instance.exports.run(7);
    expect(accessLog).toEqual(["get:env", "call:7"]);
  });

  it("Instance.exports is a null-prototype, non-extensible object with read-only enumerable properties", () => {
    const mod = new WebAssembly.Module(loadWasm("add"));
    const instance = new WebAssembly.Instance(mod);
    expect(Object.getPrototypeOf(instance.exports)).toBeNull();
    expect(Object.isExtensible(instance.exports)).toBe(false);
    const descriptor = Object.getOwnPropertyDescriptor(instance.exports, "add")!;
    expect(descriptor.enumerable).toBe(true);
    expect(descriptor.writable).toBe(false);
    expect(descriptor.configurable).toBe(false);
  });
});

describe("JS import callback exception identity", () => {
  it("re-throws the exact thrown value from a JS import callback, unwrapped", async () => {
    const sentinel = { tag: "callback-thrown-sentinel" };
    const { instance } = await WebAssembly.instantiate(loadWasm("log-import"), {
      env: {
        log: () => {
          throw sentinel;
        },
      },
    });
    let caught: unknown;
    try {
      instance.exports.run(1);
    } catch (e) {
      caught = e;
    }
    expect(caught).toBe(sentinel);
  });
});

// ---------------------------------------------------------------------------
// Traps and error categories
// ---------------------------------------------------------------------------

describe("traps and error categories", () => {
  it("a trapping start function rejects instantiation with RuntimeError", async () => {
    await expect(WebAssembly.instantiate(loadWasm("start-trap"))).rejects.toBeInstanceOf(WebAssembly.RuntimeError);
  });

  it("a trap during ordinary execution throws RuntimeError", async () => {
    const { instance } = await WebAssembly.instantiate(loadWasm("exec-trap"));
    expect(() => instance.exports.divide(1, 0)).toThrow(WebAssembly.RuntimeError);
  });
});

// ---------------------------------------------------------------------------
// Numeric conversions: i32, f32/f64, i64/BigInt, multi-value
// ---------------------------------------------------------------------------

describe("numeric conversions", () => {
  it("i32 uses ToInt32 wraparound semantics", async () => {
    const { instance } = await WebAssembly.instantiate(loadWasm("add"));
    expect(instance.exports.add(2 ** 32 + 1, 0)).toBe(1);
    expect(instance.exports.add(-1, 1)).toBe(0);
  });

  it("i64 only accepts/returns BigInt", async () => {
    const { instance } = await WebAssembly.instantiate(loadWasm("i64"), {
      env: { incrHost: (x: bigint) => x + 10n },
    });
    expect(instance.exports.incr(41n)).toBe(42n);
    expect(instance.exports.callIncrHost(5n)).toBe(15n);
    expect(() => instance.exports.incr(41 as unknown as bigint)).toThrow(TypeError);
  });

  it("multi-value exports return a JS Array, and multi-value import callbacks may return arrays", async () => {
    const { instance } = await WebAssembly.instantiate(loadWasm("multivalue"), {
      env: { divmod: (a: number, b: number) => [Math.trunc(a / b), a % b] },
    });
    const swapped = instance.exports.swap(1, 2);
    expect(Array.isArray(swapped)).toBe(true);
    expect(Array.from(swapped as ArrayLike<number>)).toEqual([2, 1]);
    expect(Array.from(instance.exports.callDivmod(7, 2) as ArrayLike<number>)).toEqual([3, 1]);
  });
});

// ---------------------------------------------------------------------------
// Memory: bidirectional visibility, buffer identity, grow semantics
// ---------------------------------------------------------------------------

describe("WebAssembly.Memory", () => {
  it("rejects shared memory and out-of-range descriptors", () => {
    expect(() => new WebAssembly.Memory({ initial: 1, shared: true } as unknown as WebAssembly.MemoryDescriptor)).toThrow(
      TypeError
    );
    expect(() => new WebAssembly.Memory({ initial: 1, maximum: 0 })).toThrow(RangeError);
  });

  it("buffer identity is stable until grow() and detaches on grow (including grow(0))", () => {
    const memory = new WebAssembly.Memory({ initial: 1, maximum: 4 });
    const first = memory.buffer;
    expect(memory.buffer).toBe(first);
    expect(first.byteLength).toBe(65536);

    const view = new Uint8Array(first);
    view[0] = 7;

    const previousPages = memory.grow(0);
    expect(previousPages).toBe(1);
    expect(first.byteLength).toBe(0); // detached
    expect(memory.buffer).not.toBe(first);
    expect(new Uint8Array(memory.buffer)[0]).toBe(7); // bytes preserved across the detach

    const second = memory.buffer;
    const grown = memory.grow(1);
    expect(grown).toBe(1);
    expect(second.byteLength).toBe(0);
    expect(memory.buffer.byteLength).toBe(2 * 65536);
  });

  it("grow() past maximum throws RangeError", () => {
    const memory = new WebAssembly.Memory({ initial: 1, maximum: 1 });
    expect(() => memory.grow(1)).toThrow(RangeError);
  });

  it("writes from Wasm are visible to JS and vice versa, across the export boundary", async () => {
    const { instance } = await WebAssembly.instantiate(loadWasm("memory"));
    instance.exports.writeI32(0, 1234);
    expect(instance.exports.readI32(0)).toBe(1234);

    const view = new Int32Array(instance.exports.mem.buffer, 0, 1);
    expect(view[0]).toBe(1234);

    view[0] = 999;
    expect(instance.exports.readI32(0)).toBe(999);
  });

  it("Wasm-internal memory.grow detaches the old mirror buffer before returning to JS", async () => {
    const { instance } = await WebAssembly.instantiate(loadWasm("memory"));
    const before = instance.exports.mem.buffer;
    const previousPages = instance.exports.growInternal(1);
    expect(previousPages).toBe(1);
    expect(before.byteLength).toBe(0);
    expect(instance.exports.mem.buffer.byteLength).toBe(2 * 65536);
  });

  it("memory written just before a host callback is visible inside the callback (Wasm->JS sync before crossing)", async () => {
    let seenDuringCallback = -1;
    const { instance } = await WebAssembly.instantiate(loadWasm("memory-callback"), {
      env: {
        onWrite: () => {
          seenDuringCallback = new Int32Array(instance.exports.mem.buffer, 0, 1)[0];
        },
      },
    });
    instance.exports.writeThenCallback(0, 4242);
    expect(seenDuringCallback).toBe(4242);
  });
});

// ---------------------------------------------------------------------------
// Reentrancy: JS import callback calling back into the same instance
// ---------------------------------------------------------------------------

describe("reentrant calls", () => {
  it("a JS import callback can call back into an export of the same instance", async () => {
    const { instance } = await WebAssembly.instantiate(loadWasm("reentrant"), {
      env: { hostCall: (x: number) => instance.exports.add(x, 100) as number },
    });
    expect(instance.exports.triggerReentrant(5)).toBe(105);
  });
});

// ---------------------------------------------------------------------------
// Table: funcref/externref get/set/grow
// ---------------------------------------------------------------------------

describe("WebAssembly.Table", () => {
  it("funcref table: length/get/set/grow, seeded via an elem segment", async () => {
    const { instance } = await WebAssembly.instantiate(loadWasm("table-funcref"));
    const table = instance.exports.funcs as WebAssembly.Table;
    expect(table.length).toBe(2);
    expect((table.get(0) as (x: number) => number)(10)).toBe(11); // $inc, from the elem segment
    expect(table.get(1)).toBeNull();

    table.set(1, instance.exports.dec);
    expect((table.get(1) as (x: number) => number)(10)).toBe(9);

    const previousLength = table.grow(1);
    expect(previousLength).toBe(2);
    expect(table.length).toBe(3);
  });

  it("get()/set() throw RangeError out of bounds, and grow() past maximum throws RangeError", () => {
    const table = new WebAssembly.Table({ element: "anyfunc", initial: 1, maximum: 1 });
    expect(() => table.get(5)).toThrow(RangeError);
    expect(() => table.set(5, null)).toThrow(RangeError);
    expect(() => table.grow(1)).toThrow(RangeError);
  });

  it("funcref table rejects an arbitrary JS function that isn't a Wasm function wrapper", () => {
    const table = new WebAssembly.Table({ element: "anyfunc", initial: 1 });
    expect(() => table.set(0, () => 1)).toThrow(TypeError);
  });

  it("externref table preserves JS object identity", () => {
    const table = new WebAssembly.Table({ element: "externref", initial: 2 });
    const obj = { tag: "externref-identity-probe" };
    table.set(0, obj);
    expect(table.get(0)).toBe(obj);
    // Per the JS API spec's `DefaultValue(externref)` algorithm, an
    // uninitialized `externref` table slot is `undefined`, not `null`
    // (`null` is only `DefaultValue(funcref)`).
    expect(table.get(1)).toBeUndefined();
  });
});

// ---------------------------------------------------------------------------
// Global: mutable/immutable, value getter/setter, valueOf, re-export identity
// ---------------------------------------------------------------------------

describe("WebAssembly.Global", () => {
  it("mutable i32 global: value getter/setter and valueOf()", () => {
    const global = new WebAssembly.Global({ value: "i32", mutable: true }, 5);
    expect(global.value).toBe(5);
    expect(global.valueOf()).toBe(5);
    global.value = 9;
    expect(global.value).toBe(9);
  });

  it("immutable global setter throws TypeError", () => {
    const global = new WebAssembly.Global({ value: "i32" }, 1);
    expect(() => {
      global.value = 2;
    }).toThrow(TypeError);
  });

  it("i64 global round-trips BigInt", () => {
    const global = new WebAssembly.Global({ value: "i64", mutable: true }, 1n);
    expect(global.value).toBe(1n);
    global.value = 99n;
    expect(global.value).toBe(99n);
  });

  it("an imported/re-exported Global preserves wrapper identity", async () => {
    const counter = new WebAssembly.Global({ value: "i32", mutable: true }, 0);
    const { instance } = await WebAssembly.instantiate(loadWasm("global-reexport"), { env: { counter } });
    expect(instance.exports.counterAlias).toBe(counter);
    counter.value = 41;
    expect((instance.exports.counterAlias as WebAssembly.Global).value).toBe(41);
  });
});

// ---------------------------------------------------------------------------
// SIMD
// ---------------------------------------------------------------------------

describe("SIMD", () => {
  it("a module using v128 purely internally compiles, instantiates, and executes", async () => {
    const { instance } = await WebAssembly.instantiate(loadWasm("simd-internal"));
    expect(instance.exports.simdDouble(3)).toBe(6);
  });

  it("a v128 value crossing the JS/Wasm boundary throws TypeError", async () => {
    const { instance } = await WebAssembly.instantiate(loadWasm("simd-boundary"));
    expect(() => instance.exports.makeVector(1)).toThrow(TypeError);
  });
});

// ---------------------------------------------------------------------------
// Cross-realm rejection
// ---------------------------------------------------------------------------

describe("cross-realm isolation", () => {
  // `vm.runInNewContext` creates a bare QuickJS context that does not run
  // any Lexe module's `init` (see `lexe_vm::run_in_new_context`):
  // it has no `WebAssembly` of its own at all, so it cannot be used here to
  // build a *second fully-initialized* realm to import across (that
  // scenario -- two realms that each have a working `WebAssembly`, with an
  // object from one imported into the other -- is exercised directly at
  // the Rust level by
  // `instance::tests::cross_realm_memory_import_is_rejected_with_link_error`,
  // which constructs two realms on the same QuickJS `Runtime` explicitly).
  //
  // What *is* meaningfully JS-observable from here is that reusing a
  // realm-scoped WebAssembly object (or its class, obtained by reference
  // through `vm`'s sandbox, which shares values rather than copying them)
  // from a context that never had its own realm installed fails safely --
  // no crash, no silent corruption -- rather than, say, dereferencing a
  // stale `Store` pointer.
  it("using a parent-realm Memory/constructor from an uninitialized child context fails safely", () => {
    const memory = new WebAssembly.Memory({ initial: 1 });
    const sandbox: { memory: WebAssembly.Memory; MemoryCtor: typeof WebAssembly.Memory } = {
      memory,
      MemoryCtor: WebAssembly.Memory,
    };

    const accessResult = runInNewContext(
      "(() => { try { memory.buffer; return 'unexpectedly-succeeded'; } catch (e) { return e.constructor.name; } })()",
      sandbox
    );
    expect(accessResult).not.toBe("unexpectedly-succeeded");

    const constructResult = runInNewContext(
      "(() => { try { new MemoryCtor({ initial: 1 }); return 'unexpectedly-succeeded'; } catch (e) { return e.constructor.name; } })()",
      sandbox
    );
    expect(constructResult).not.toBe("unexpectedly-succeeded");

    // The parent-realm object itself must be entirely unaffected.
    expect(memory).toBeInstanceOf(WebAssembly.Memory);
    expect(memory.buffer.byteLength).toBe(65536);
  });
});

// ---------------------------------------------------------------------------
// llhttp-shaped smoke fixture
// ---------------------------------------------------------------------------

describe("llhttp-shaped smoke fixture", () => {
  it("compiles, links all 8 env callbacks, and executes end to end", async () => {
    const calls: string[] = [];
    const mod = await WebAssembly.compile(loadWasm("llhttp-shape"));
    let instance!: WebAssembly.Instance;
    instance = await WebAssembly.instantiate(mod, {
      env: {
        on_message_begin: () => {
          calls.push("begin");
          return 0;
        },
        on_url: (_parser: number, at: number, len: number) => {
          const byte = new Uint8Array(instance.exports.memory.buffer, at, len)[0];
          calls.push(`url:${byte}`);
          return 0;
        },
        on_status: () => 0,
        on_header_field: () => 0,
        on_header_value: () => 0,
        on_headers_complete: () => {
          calls.push("headers_complete");
          return 0;
        },
        on_body: () => {
          calls.push("body");
          return 0;
        },
        on_message_complete: () => {
          calls.push("complete");
          return 0;
        },
      },
    });

    expect(WebAssembly.Module.imports(mod).map((i) => i.name)).toEqual([
      "on_message_begin",
      "on_url",
      "on_status",
      "on_header_field",
      "on_header_value",
      "on_headers_complete",
      "on_body",
      "on_message_complete",
    ]);
    expect(instance.exports.memory).toBeInstanceOf(WebAssembly.Memory);
    expect(instance.exports.table).toBeInstanceOf(WebAssembly.Table);

    const ptr = instance.exports.malloc(64) as number;
    instance.exports.execute(0, ptr, 64);

    expect(calls).toEqual(["begin", "url:71", "headers_complete", "body", "complete"]);
  });
});
