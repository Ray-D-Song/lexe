import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import ffi, { dlopen, getInt32, getRawPointer, suffix, toArrayBuffer } from "node:ffi";
import { spawnCapture } from "./test-utils";

const TIMEOUT = 30000;

describe("node:ffi", () => {
  it("exports named and default bindings with platform suffix", () => {
    const expected =
      process.platform === "darwin" ? "dylib" : process.platform === "win32" ? "dll" : "so";
    expect(suffix).toBe(expected);
    expect(ffi.suffix).toBe(expected);
    expect(typeof dlopen).toBe("function");
    expect(typeof getRawPointer).toBe("function");
    expect(typeof toArrayBuffer).toBe("function");
    expect(typeof getInt32).toBe("function");
    expect(ffi.dlopen).toBe(dlopen);
  });

  it("cannot be imported via bare 'ffi' specifier", async () => {
    const bare = ["f", "f", "i"].join("");
    await expect(import(bare as any)).rejects.toThrow();
  });

  it("supports getRawPointer, toArrayBuffer (copy and zero-copy), and getInt32", () => {
    const raw = new ArrayBuffer(16);
    const u8 = new Uint8Array(raw);
    const sub = u8.subarray(4);
    const buf = Buffer.from(raw, 4, 8);
    const dv = new DataView(raw, 8, 8);

    const pRaw = getRawPointer(raw);
    const pU8 = getRawPointer(u8);
    const pSub = getRawPointer(sub);
    const pBuf = getRawPointer(buf);
    const pDv = getRawPointer(dv);

    expect(typeof pRaw).toBe("bigint");
    expect(pRaw > 0n).toBe(true);
    expect(pU8).toBe(pRaw);
    expect(pSub).toBe(pRaw + 4n);
    expect(pBuf).toBe(pRaw + 4n);
    expect(pDv).toBe(pRaw + 8n);

    const i32 = new Int32Array(raw);
    i32[0] = 100;
    i32[1] = -200;
    i32[2] = 300;
    i32[3] = -400;

    expect(getInt32(pRaw)).toBe(100);
    expect(getInt32(pRaw, 4)).toBe(-200);
    expect(getInt32(pRaw, 8)).toBe(300);

    const copied = toArrayBuffer(pRaw, 16, true);
    const zeroCopy = toArrayBuffer(pRaw, 16, false);
    const emptyAb = toArrayBuffer(0n, 0, false);
    expect(emptyAb.byteLength).toBe(0);

    i32[1] = 777;
    expect(new Int32Array(copied)[1]).toBe(-200);
    expect(new Int32Array(zeroCopy)[1]).toBe(777);

    expect(() => getRawPointer({} as any)).toThrow(TypeError);
    expect(() => getRawPointer("not a buffer" as any)).toThrow(TypeError);
    const detached = raw.transfer(4);
    expect(() => getRawPointer(raw)).toThrow(TypeError);
    expect(getRawPointer(detached) > 0n).toBe(true);
  });

  it("calls process symbols via dlopen(null), runs qsort JS callback, and enforces close()", () => {
    const { lib, functions } = dlopen(null, {
      abs: { arguments: ["i32"], return: "i32" },
      strlen: { arguments: ["string"], return: "u64" },
      qsort: { arguments: ["pointer", "u64", "u64", "pointer"], return: "void" },
    });

    expect(functions.abs(-42)).toBe(42);
    expect(functions.strlen("lexe-ffi")).toBe(8n);

    const arr = new Int32Array([42, -5, 17, 0, 99, 3]);
    let callbackCalls = 0;
    const cmpPtr = lib.registerCallback(
      { arguments: ["pointer", "pointer"], return: "i32" },
      (a: bigint, b: bigint) => {
        callbackCalls += 1;
        return getInt32(a) - getInt32(b);
      }
    );
    expect(typeof cmpPtr).toBe("bigint");
    expect(cmpPtr > 0n).toBe(true);

    functions.qsort(arr, BigInt(arr.length), 4n, cmpPtr);
    expect(callbackCalls > 0).toBe(true);
    expect(Array.from(arr)).toEqual([-5, 0, 3, 17, 42, 99]);

    lib.unregisterCallback(cmpPtr);
    lib.unregisterCallback(cmpPtr); // safe no-op if already unregistered
    lib.close();
    lib.close(); // idempotent

    expect(() => functions.abs(-1)).toThrow();
    // Unregister after close must remain a safe no-op (mirrors @opentui/core teardown order).
    expect(() => lib.unregisterCallback(cmpPtr)).not.toThrow();
  });

  it(
    "exits cleanly when a child process leaves registered FFI callbacks unclosed",
    async () => {
      const root = await mkdtemp(join(tmpdir(), "lexe-test-ffi-"));
      try {
        const script = join(root, "unclosed-cb.mjs");
        await writeFile(
          script,
          `import { dlopen, getInt32 } from "node:ffi";
const { lib, functions } = dlopen(null, {
  qsort: { arguments: ["pointer", "u64", "u64", "pointer"], return: "void" },
});
const arr = new Int32Array([3, 1, 2]);
const cb = lib.registerCallback(
  { arguments: ["pointer", "pointer"], return: "i32" },
  (a, b) => getInt32(a) - getInt32(b)
);
functions.qsort(arr, 3n, 4n, cb);
console.log(Array.from(arr).join(","));
`,
          "utf8"
        );
        const res = await spawnCapture(process.argv0, [script]);
        expect(res.code).toBe(0);
        expect(res.stdout.trim()).toBe("1,2,3");
      } finally {
        await rm(root, { recursive: true, force: true });
      }
    },
    TIMEOUT
  );
});
