import Module from "node:module";
import v8 from "node:v8";
import legacy from "v8";
import {
  DefaultDeserializer,
  DefaultSerializer,
  Deserializer,
  Serializer,
  cachedDataVersionTag,
  deserialize,
  getHeapCodeStatistics,
  getHeapSpaceStatistics,
  getHeapStatistics,
  serialize,
  setFlagsFromString,
} from "node:v8";

it("node:v8 should match v8", () => {
  expect(v8.getHeapStatistics).toBe(legacy.getHeapStatistics);
});

it("Module.isBuiltin recognizes v8", () => {
  expect(Module.isBuiltin("v8")).toBe(true);
  expect(Module.isBuiltin("node:v8")).toBe(true);
  expect(Module.builtinModules).toContain("v8");
});

function assertSafeNumber(v: unknown, label: string) {
  expect(typeof v).toBe("number");
  expect(Number.isFinite(v as number)).toBeTruthy();
  expect(v as number).toBeGreaterThanOrEqual(0);
  expect(v as number).toBeLessThanOrEqual(Number.MAX_SAFE_INTEGER);
  if ((v as number) < 0) {
    throw new Error(`${label} negative`);
  }
}

it("getHeapStatistics returns finite non-negative fields", () => {
  const stats = getHeapStatistics();
  const keys = [
    "total_heap_size",
    "total_heap_size_executable",
    "total_physical_size",
    "total_available_size",
    "used_heap_size",
    "heap_size_limit",
    "malloced_memory",
    "peak_malloced_memory",
    "does_zap_garbage",
    "number_of_native_contexts",
    "number_of_detached_contexts",
    "total_global_handles_size",
    "used_global_handles_size",
    "external_memory",
    "total_allocated_bytes",
  ] as const;

  for (const key of keys) {
    assertSafeNumber(stats[key], key);
  }

  expect(stats.heap_size_limit).toBeGreaterThanOrEqual(stats.used_heap_size);
  expect(stats.total_available_size).toBeGreaterThanOrEqual(0);
  // Unconfigured QuickJS limit must not report as 0.
  expect(stats.heap_size_limit).toBeGreaterThan(0);
});

it("getHeapSpaceStatistics returns a single quickjs space", () => {
  const spaces = getHeapSpaceStatistics();
  expect(Array.isArray(spaces)).toBeTruthy();
  expect(spaces.length).toBe(1);
  expect(spaces[0].space_name).toBe("quickjs");
  assertSafeNumber(spaces[0].space_size, "space_size");
  assertSafeNumber(spaces[0].space_used_size, "space_used_size");
  assertSafeNumber(spaces[0].space_available_size, "space_available_size");
  assertSafeNumber(spaces[0].physical_space_size, "physical_space_size");
});

it("getHeapCodeStatistics returns four safe fields", () => {
  const code = getHeapCodeStatistics();
  assertSafeNumber(code.code_and_metadata_size, "code_and_metadata_size");
  assertSafeNumber(code.bytecode_and_metadata_size, "bytecode_and_metadata_size");
  assertSafeNumber(code.external_script_source_size, "external_script_source_size");
  assertSafeNumber(code.cpu_profiler_metadata_size, "cpu_profiler_metadata_size");
});

it("setFlagsFromString is a no-op for strings and TypeError otherwise", () => {
  expect(setFlagsFromString("--example")).toBeUndefined();
  expect(() => setFlagsFromString(42 as unknown as string)).toThrow(TypeError);
  expect(() => (setFlagsFromString as Function)()).toThrow(TypeError);
});

it("cachedDataVersionTag returns a stable positive uint32", () => {
  const tag = cachedDataVersionTag();
  expect(Number.isInteger(tag)).toBe(true);
  expect(tag).toBeGreaterThan(0);
  expect(tag).toBeLessThanOrEqual(0xffffffff);
  expect(cachedDataVersionTag()).toBe(tag);
});

it("serialize and deserialize roundtrip structured values as Buffer", () => {
  const cyclic: Record<string, unknown> = {
    n: 42,
    big: 12345678901234567890n,
    date: new Date("2025-01-01T00:00:00.000Z"),
    re: /abc/gi,
    map: new Map<unknown, unknown>([["k", new Set([1, 2, 3])]]),
    bytes: new Uint8Array([10, 20, 30]),
  };
  cyclic.self = cyclic;

  const buf = serialize(cyclic);
  expect(Buffer.isBuffer(buf)).toBe(true);

  const out = deserialize(buf) as typeof cyclic;
  expect(out).not.toBe(cyclic);
  expect(out.self).toBe(out);
  expect(out.n).toBe(42);
  expect(out.big).toBe(12345678901234567890n);
  expect((out.date as Date).toISOString()).toBe("2025-01-01T00:00:00.000Z");
  expect((out.re as RegExp).source).toBe("abc");
  expect((out.re as RegExp).flags).toBe("gi");
  expect([...((out.map as Map<unknown, Set<number>>).get("k")!)]).toEqual([1, 2, 3]);
  expect([...(out.bytes as Uint8Array)]).toEqual([10, 20, 30]);

  expect(() => serialize(() => {})).toThrow();
});

it("Serializer/Deserializer and Default* support primitives, ArrayBuffer transfer, and custom host objects", () => {
  const ser = new DefaultSerializer();
  ser.writeHeader();
  ser.writeUint32(300);
  ser.writeUint64(0x01020304, 0x05060708);
  ser.writeDouble(3.5);
  ser.writeRawBytes(Buffer.from("hi"));
  const ab = new ArrayBuffer(4);
  new Uint8Array(ab).set([7, 8, 9, 10]);
  ser.transferArrayBuffer(0, ab);
  ser.writeValue(ab);
  ser.writeValue({ ok: true });
  const encoded = ser.releaseBuffer();
  expect(Buffer.isBuffer(encoded)).toBe(true);

  const des = new DefaultDeserializer(encoded);
  expect(des.getWireFormatVersion()).toBe(0);
  expect(des.readHeader()).toBe(true);
  expect(des.getWireFormatVersion()).toBe(15);
  expect(des.readUint32()).toBe(300);
  expect(des.readUint64()).toEqual([0x01020304, 0x05060708]);
  expect(des.readDouble()).toBe(3.5);
  const raw = des.readRawBytes(2);
  expect(Buffer.isBuffer(raw)).toBe(true);
  expect(raw.toString()).toBe("hi");

  const movedAb = new ArrayBuffer(4);
  new Uint8Array(movedAb).set([7, 8, 9, 10]);
  des.transferArrayBuffer(0, movedAb);
  expect(des.readValue()).toBe(movedAb);
  expect(des.readValue()).toEqual({ ok: true });

  class CustomSerializer extends Serializer {
    constructor() {
      super();
      (this as unknown as { _setTreatArrayBufferViewsAsHostObjects(v: boolean): void })._setTreatArrayBufferViewsAsHostObjects(true);
    }
    _writeHostObject(obj: unknown) {
      if (obj instanceof Int16Array) {
        this.writeUint32(obj[0] >>> 0);
        this.writeUint32(obj[1] >>> 0);
        return;
      }
      throw new Error("bad host object");
    }
  }
  class CustomDeserializer extends Deserializer {
    _readHostObject() {
      return new Int16Array([this.readUint32(), this.readUint32()]);
    }
  }

  const cs = new CustomSerializer();
  cs.writeHeader();
  cs.writeValue(new Int16Array([11, 29]));
  const cbuf = cs.releaseBuffer();
  const cd = new CustomDeserializer(cbuf);
  cd.readHeader();
  const pt = cd.readValue() as Int16Array;
  expect(pt).toBeInstanceOf(Int16Array);
  expect([...pt]).toEqual([11, 29]);
});
