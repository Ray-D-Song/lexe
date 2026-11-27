import { WASI } from "node:wasi";
import wasiDefault from "wasi";
import { builtinModules, isBuiltin, createRequire } from "node:module";
import { mkdtempSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";

// Hand-assembled WASI Preview 1 command module:
// Imports: wasi_snapshot_preview1.args_sizes_get (i32, i32)->i32
//          wasi_snapshot_preview1.proc_exit (i32)->()
// Exports: memory, _start
// Calls args_sizes_get(0, 4) then proc_exit(42).
const CMD_WASM = new Uint8Array([
  0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, 0x01, 0x0e, 0x03, 0x60, 0x02, 0x7f, 0x7f, 0x01,
  0x7f, 0x60, 0x01, 0x7f, 0x00, 0x60, 0x00, 0x00, 0x02, 0x4c, 0x02, 0x16, 0x77, 0x61, 0x73, 0x69,
  0x5f, 0x73, 0x6e, 0x61, 0x70, 0x73, 0x68, 0x6f, 0x74, 0x5f, 0x70, 0x72, 0x65, 0x76, 0x69, 0x65,
  0x77, 0x31, 0x0e, 0x61, 0x72, 0x67, 0x73, 0x5f, 0x73, 0x69, 0x7a, 0x65, 0x73, 0x5f, 0x67, 0x65,
  0x74, 0x00, 0x00, 0x16, 0x77, 0x61, 0x73, 0x69, 0x5f, 0x73, 0x6e, 0x61, 0x70, 0x73, 0x68, 0x6f,
  0x74, 0x5f, 0x70, 0x72, 0x65, 0x76, 0x69, 0x65, 0x77, 0x31, 0x09, 0x70, 0x72, 0x6f, 0x63, 0x5f,
  0x65, 0x78, 0x69, 0x74, 0x00, 0x01, 0x03, 0x02, 0x01, 0x02, 0x05, 0x03, 0x01, 0x00, 0x01, 0x07,
  0x13, 0x02, 0x06, 0x6d, 0x65, 0x6d, 0x6f, 0x72, 0x79, 0x02, 0x00, 0x06, 0x5f, 0x73, 0x74, 0x61,
  0x72, 0x74, 0x00, 0x02, 0x0a, 0x0f, 0x01, 0x0d, 0x00, 0x41, 0x00, 0x41, 0x04, 0x10, 0x00, 0x1a,
  0x41, 0x2a, 0x10, 0x01, 0x0b,
]);

// Reactor module importing wasi_snapshot_preview1.environ_sizes_get (i32, i32)->i32,
// exporting memory, _initialize ()->(), get_envc ()->i32.
const REACTOR_WASM = new Uint8Array([
  0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, 0x01, 0x0e, 0x03, 0x60, 0x02, 0x7f, 0x7f, 0x01,
  0x7f, 0x60, 0x00, 0x00, 0x60, 0x00, 0x01, 0x7f, 0x02, 0x2c, 0x01, 0x16, 0x77, 0x61, 0x73, 0x69,
  0x5f, 0x73, 0x6e, 0x61, 0x70, 0x73, 0x68, 0x6f, 0x74, 0x5f, 0x70, 0x72, 0x65, 0x76, 0x69, 0x65,
  0x77, 0x31, 0x11, 0x65, 0x6e, 0x76, 0x69, 0x72, 0x6f, 0x6e, 0x5f, 0x73, 0x69, 0x7a, 0x65, 0x73,
  0x5f, 0x67, 0x65, 0x74, 0x00, 0x00, 0x03, 0x03, 0x02, 0x01, 0x02, 0x05, 0x03, 0x01, 0x00, 0x01,
  0x07, 0x23, 0x03, 0x06, 0x6d, 0x65, 0x6d, 0x6f, 0x72, 0x79, 0x02, 0x00, 0x0b, 0x5f, 0x69, 0x6e,
  0x69, 0x74, 0x69, 0x61, 0x6c, 0x69, 0x7a, 0x65, 0x00, 0x01, 0x08, 0x67, 0x65, 0x74, 0x5f, 0x65,
  0x6e, 0x76, 0x63, 0x00, 0x02, 0x0a, 0x13, 0x02, 0x09, 0x00, 0x41, 0x00, 0x41, 0x04, 0x10, 0x00,
  0x1a, 0x0b, 0x07, 0x00, 0x41, 0x00, 0x28, 0x02, 0x00, 0x0b,
]);

describe("node:wasi", () => {
  it("registers as a built-in module for ESM, CJS require, and getBuiltinModule", () => {
    expect(builtinModules.includes("wasi")).toBe(true);
    expect(isBuiltin("node:wasi")).toBe(true);
    expect(wasiDefault.WASI).toBe(WASI);
    const req = createRequire(import.meta.url);
    expect(req("node:wasi").WASI).toBe(WASI);
    const builtin = (process as unknown as { getBuiltinModule?: (id: string) => { WASI?: unknown } }).getBuiltinModule?.(
      "node:wasi"
    );
    expect(builtin?.WASI).toBe(WASI);
  });

  it("requires options.version === 'preview1'", () => {
    expect(() => new WASI()).toThrow(TypeError);
    expect(() => new WASI({} as unknown as { version: "preview1" })).toThrow(TypeError);
    expect(() => new WASI({ version: "unstable" as "preview1" })).toThrow(TypeError);
  });

  it("exposes all 46 preview1 functions on wasiImport and getImportObject()", () => {
    const wasi = new WASI({ version: "preview1" });
    const keys = Object.keys(wasi.wasiImport);
    expect(keys.length).toBe(46);
    expect(keys).toContain("fd_write");
    expect(keys).toContain("proc_exit");
    expect(keys).toContain("path_open");
    const imp = wasi.getImportObject();
    expect(imp.wasi_snapshot_preview1).toBe(wasi.wasiImport);
  });

  it("runs a WASI command module with args and returnOnExit", () => {
    const dir = mkdtempSync(path.join(tmpdir(), "lexe-wasi-"));
    try {
      writeFileSync(path.join(dir, "hello.txt"), "hi");
      const wasi = new WASI({
        version: "preview1",
        args: ["prog", "--alpha", "beta"],
        env: { FOO: "BAR" },
        preopens: { "/sandbox": dir },
        returnOnExit: true,
      });

      const mod = new WebAssembly.Module(CMD_WASM);
      const inst = new WebAssembly.Instance(mod, wasi.getImportObject() as unknown as WebAssembly.Imports);
      const rc = wasi.start(inst);
      expect(rc).toBe(42);
      const argc = new DataView((inst.exports.memory as WebAssembly.Memory).buffer).getUint32(0, true);
      expect(argc).toBe(3);

      expect(() => wasi.start(inst)).toThrow();
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  it("supports WASI reactor modules via initialize() and env options", () => {
    const wasi = new WASI({
      version: "preview1",
      env: { ONE: "1", TWO: "2" },
    });
    const mod = new WebAssembly.Module(REACTOR_WASM);
    const inst = new WebAssembly.Instance(mod, wasi.getImportObject() as unknown as WebAssembly.Imports);
    expect(wasi.initialize(inst)).toBeUndefined();
    expect((inst.exports.get_envc as () => number)()).toBe(2);
    expect(() => wasi.initialize(inst)).toThrow();
  });

  it("allows overriding a wasiImport entry with a custom JS function", () => {
    const wasi = new WASI({ version: "preview1", returnOnExit: true });
    let intercepted = false;
    (wasi.wasiImport as Record<string, unknown>).args_sizes_get = (argcPtr: number, argvBufPtr: number) => {
      intercepted = true;
      void argcPtr;
      void argvBufPtr;
      return 0;
    };
    const mod = new WebAssembly.Module(CMD_WASM);
    const inst = new WebAssembly.Instance(mod, wasi.getImportObject() as unknown as WebAssembly.Imports);
    const rc = wasi.start(inst);
    expect(intercepted).toBe(true);
    expect(rc).toBe(42);
  });
});
