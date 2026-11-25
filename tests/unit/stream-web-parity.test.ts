import * as streamWeb from "node:stream/web";
import vm from "node:vm";

const {
  ByteLengthQueuingStrategy,
  CompressionStream,
  CountQueuingStrategy,
  DecompressionStream,
  ReadableByteStreamController,
  ReadableStream,
  ReadableStreamBYOBReader,
  ReadableStreamBYOBRequest,
  ReadableStreamDefaultController,
  ReadableStreamDefaultReader,
  TextDecoderStream,
  TextEncoderStream,
  TransformStream,
  TransformStreamDefaultController,
  WritableStream,
  WritableStreamDefaultController,
  WritableStreamDefaultWriter,
} = streamWeb as any;

async function readAllBytes(readable: any): Promise<Uint8Array> {
  const reader = readable.getReader();
  const chunks: Uint8Array[] = [];
  let total = 0;
  while (true) {
    const { done, value } = await reader.read();
    if (done) break;
    const u8 = value instanceof Uint8Array ? value : new Uint8Array(value);
    chunks.push(u8);
    total += u8.byteLength;
  }
  const out = new Uint8Array(total);
  let offset = 0;
  for (const c of chunks) {
    out.set(c, offset);
    offset += c.byteLength;
  }
  return out;
}

describe("TransformStream spec compliance", () => {
  it("validates readableType/writableType and accepts null/undefined strategies", () => {
    expect(() => new TransformStream({ readableType: "bytes" })).toThrow(RangeError);
    expect(() => new TransformStream({ writableType: "bytes" })).toThrow(RangeError);
    expect(() => new TransformStream({ readableType: null })).toThrow(RangeError);
    expect(() => new TransformStream({ writableType: null })).toThrow(RangeError);

    expect(() => new TransformStream(null)).toThrow(TypeError);
    expect(() => new TransformStream(42)).toThrow(TypeError);

    expect(() => new TransformStream({})).not.toThrow();
    expect(
      () => new TransformStream({ readableType: undefined, writableType: undefined })
    ).not.toThrow();
    expect(() => new TransformStream(undefined, null, null)).not.toThrow();
  });

  it("reads transformer properties in WebIDL dictionary order", () => {
    const order: string[] = [];
    const transformer = {
      get cancel() {
        order.push("cancel");
        return undefined;
      },
      get flush() {
        order.push("flush");
        return undefined;
      },
      get transform() {
        order.push("transform");
        return undefined;
      },
      get start() {
        order.push("start");
        return undefined;
      },
      get writableType() {
        order.push("writableType");
        return undefined;
      },
      get readableType() {
        order.push("readableType");
        return undefined;
      },
    };
    new TransformStream(transformer);
    expect(order).toEqual([
      "readableType",
      "writableType",
      "start",
      "transform",
      "flush",
      "cancel",
    ]);
  });

  it("propagates synchronous start() errors from constructor and async start() rejections to both sides", async () => {
    expect(
      () =>
        new TransformStream({
          start() {
            throw new Error("sync start fail");
          },
        })
    ).toThrow("sync start fail");

    const ts = new TransformStream({
      start() {
        return Promise.reject(new Error("async start fail"));
      },
    });
    const reader = ts.readable.getReader();
    const writer = ts.writable.getWriter();
    await expect(reader.read()).rejects.toThrow("async start fail");
    await expect(writer.closed).rejects.toThrow("async start fail");
  });

  it("errors both sides when async transform() rejects or writable aborts", async () => {
    const ts1 = new TransformStream({
      async transform() {
        throw new Error("async transform fail");
      },
    });
    const r1 = ts1.readable.getReader();
    const w1 = ts1.writable.getWriter();
    // A pending read releases the initial backpressure so the write proceeds.
    const readP = r1.read();
    readP.catch(() => {});
    const writeP = w1.write("a");
    await expect(writeP).rejects.toThrow("async transform fail");
    await expect(readP).rejects.toThrow("async transform fail");

    let cancelReason: unknown;
    const ts2 = new TransformStream({
      cancel(reason: unknown) {
        cancelReason = reason;
      },
    });
    const r2 = ts2.readable.getReader();
    const w2 = ts2.writable.getWriter();
    await w2.abort("aborted-reason");
    expect(cancelReason).toBe("aborted-reason");
    await expect(r2.read()).rejects.toBe("aborted-reason");
  });

  it("throws TypeError on controller.enqueue() after controller.terminate()", async () => {
    let savedCtrl: any;
    const ts = new TransformStream({
      start(c: any) {
        savedCtrl = c;
      },
    });
    savedCtrl.terminate();
    expect(() => savedCtrl.enqueue("after")).toThrow(TypeError);
    await ts.readable.cancel();
  });
});

describe("pipeTo, pipeThrough, and ReadableStream.from parity", () => {
  it("validates pipeTo arguments before locks and supports null/undefined options", async () => {
    const log: string[] = [];
    const rs = new ReadableStream();
    rs.getReader(); // lock rs AFTER destination/options conversion order check

    const fakeDest = {
      get locked() {
        log.push("locked");
        return false;
      },
    };
    await expect(rs.pipeTo(fakeDest)).rejects.toThrow(TypeError);

    const optOrder: string[] = [];
    const writable = new WritableStream();
    writable.getWriter();
    const opts = {
      get signal() {
        optOrder.push("signal");
        return undefined;
      },
      get preventClose() {
        optOrder.push("preventClose");
        return false;
      },
      get preventCancel() {
        optOrder.push("preventCancel");
        return false;
      },
      get preventAbort() {
        optOrder.push("preventAbort");
        return false;
      },
    };
    await expect(rs.pipeTo(writable, opts)).rejects.toThrow(TypeError);
    expect(optOrder).toEqual(["preventAbort", "preventCancel", "preventClose", "signal"]);

    const rsOk = new ReadableStream({
      start(c: any) {
        c.enqueue(1);
        c.close();
      },
    });
    const seen: number[] = [];
    await rsOk.pipeTo(
      new WritableStream({
        write(v: number) {
          seen.push(v);
        },
      }),
      null
    );
    expect(seen).toEqual([1]);

    const rsBadSig = new ReadableStream();
    await expect(rsBadSig.pipeTo(new WritableStream(), { signal: 123 })).rejects.toThrow(TypeError);
  });

  it("does not hang pipeTo when destination write rejects", async () => {
    const rs = new ReadableStream({
      start(c: any) {
        c.enqueue("a");
        c.enqueue("b");
      },
    });
    const ws = new WritableStream({
      write() {
        return Promise.reject(new Error("sink boom"));
      },
    });
    await expect(rs.pipeTo(ws)).rejects.toThrow("sink boom");
  });

  it("accepts null/undefined options in pipeThrough and values(), and reads ReadableStream.from()", async () => {
    const out: number[] = [];
    const rs = ReadableStream.from([10, 20, 30]);
    const piped = rs.pipeThrough(new TransformStream(), null);
    for await (const chunk of piped.values(null)) {
      out.push(chunk);
    }
    expect(out).toEqual([10, 20, 30]);
  });
});

describe("BYOB reader, non-transferable WebAssembly buffers, and SharedArrayBuffer", () => {
  it("validates BYOB read(view, options) min option matching Node v24", async () => {
    const mkStream = () =>
      new ReadableStream({
        type: "bytes",
        start(c: any) {
          c.enqueue(new Uint8Array([1, 2, 3, 4]));
          c.close();
        },
      });

    const r1 = mkStream().getReader({ mode: "byob" });
    const res1 = await r1.read(new Uint8Array(4), undefined);
    expect(Array.from(res1.value)).toEqual([1, 2, 3, 4]);

    const r2 = mkStream().getReader({ mode: "byob" });
    const res2 = await r2.read(new Uint8Array(4), null);
    expect(Array.from(res2.value)).toEqual([1, 2, 3, 4]);

    const r3 = mkStream().getReader({ mode: "byob" });
    const res3 = await r3.read(new Uint8Array(4), { min: undefined });
    expect(Array.from(res3.value)).toEqual([1, 2, 3, 4]);

    const r4 = mkStream().getReader({ mode: "byob" });
    const res4 = await r4.read(new Uint8Array(4), { min: null });
    expect(Array.from(res4.value)).toEqual([1, 2, 3, 4]);

    const badMinTypeError = [0, -1, 1.5, NaN, Infinity, "1", true, 1n];
    for (const min of badMinTypeError) {
      const r = mkStream().getReader({ mode: "byob" });
      await expect(r.read(new Uint8Array(4), { min })).rejects.toThrow(TypeError);
    }

    const rRange = mkStream().getReader({ mode: "byob" });
    await expect(rRange.read(new Uint8Array(4), { min: 5 })).rejects.toThrow(RangeError);
    const rBig = mkStream().getReader({ mode: "byob" });
    await expect(rBig.read(new Uint8Array(4), { min: 2 ** 53 })).rejects.toThrow(RangeError);
  });

  it("rejects SharedArrayBuffer and WebAssembly.Memory buffers in BYOB and ArrayBuffer.transfer()", async () => {
    const sab = new SharedArrayBuffer(16);
    const sabView = new Uint8Array(sab);
    const rs1 = new ReadableStream({ type: "bytes" });
    const reader1 = rs1.getReader({ mode: "byob" });
    await expect(reader1.read(sabView)).rejects.toThrow(TypeError);

    const mem = new WebAssembly.Memory({ initial: 1 });
    const wasmBuf = mem.buffer;
    expect(() => (wasmBuf as any).transfer()).toThrow(TypeError);

    const rs2 = new ReadableStream({
      type: "bytes",
      start(c: any) {
        c.enqueue(new Uint8Array([9]));
      },
    });
    const reader2 = rs2.getReader({ mode: "byob" });
    await expect(reader2.read(new Uint8Array(wasmBuf, 0, 8))).rejects.toThrow(TypeError);

    // Wasm Memory.grow() must still detach its mirror buffer cleanly.
    expect(() => mem.grow(1)).not.toThrow();
    expect(wasmBuf.byteLength).toBe(0);
    expect(mem.buffer.byteLength).toBe(2 * 65536);

    // Normal ArrayBuffer transfer remains functional.
    const normal = new ArrayBuffer(8);
    const transferred = (normal as any).transfer(16);
    expect(normal.byteLength).toBe(0);
    expect(transferred.byteLength).toBe(16);
  });
});

describe("QueuingStrategy and Symbol.toStringTag parity", () => {
  it("coerces highWaterMark via ToNumber and exposes shared size functions", () => {
    const b1 = new ByteLengthQueuingStrategy({ highWaterMark: "4" });
    const b2 = new ByteLengthQueuingStrategy({ highWaterMark: true });
    expect(b1.highWaterMark).toBe(4);
    expect(b2.highWaterMark).toBe(1);
    expect(b1.size).toBe(b2.size);
    expect(b1.size.name).toBe("size");
    expect(b1.size.length).toBe(1);
    expect(b1.size({ byteLength: 7 })).toBe(7);

    const c1 = new CountQueuingStrategy({ highWaterMark: null });
    const c2 = new CountQueuingStrategy({ highWaterMark: { valueOf: () => 3 } });
    expect(c1.highWaterMark).toBe(0);
    expect(c2.highWaterMark).toBe(3);
    expect(c1.size).toBe(c2.size);
    expect(c1.size.name).toBe("size");
    expect(c1.size.length).toBe(0);
    expect(c1.size("anything")).toBe(1);

    expect(() => new ByteLengthQueuingStrategy({ highWaterMark: 1n })).toThrow(TypeError);
    expect(() => new CountQueuingStrategy({ highWaterMark: 1n })).toThrow(TypeError);

    expect(() => new ReadableStream({}, null)).not.toThrow();
    expect(() => new WritableStream({}, null)).not.toThrow();
  });

  it("exposes Symbol.toStringTag across all stream/web classes and prototypes", () => {
    const rs = new ReadableStream();
    const rsReader = rs.getReader();
    rsReader.releaseLock();
    const bs = new ReadableStream({ type: "bytes" });
    const bsReader = bs.getReader({ mode: "byob" });
    const ws = new WritableStream();
    const writer = ws.getWriter();
    let tsCtrl: any;
    const ts = new TransformStream({
      start(c: any) {
        tsCtrl = c;
      },
    });

    const cases: Array<[unknown, string]> = [
      [ReadableStream.prototype, "ReadableStream"],
      [ReadableStreamDefaultReader.prototype, "ReadableStreamDefaultReader"],
      [ReadableStreamBYOBReader.prototype, "ReadableStreamBYOBReader"],
      [ReadableStreamDefaultController.prototype, "ReadableStreamDefaultController"],
      [ReadableByteStreamController.prototype, "ReadableByteStreamController"],
      [ReadableStreamBYOBRequest.prototype, "ReadableStreamBYOBRequest"],
      [WritableStream.prototype, "WritableStream"],
      [WritableStreamDefaultWriter.prototype, "WritableStreamDefaultWriter"],
      [WritableStreamDefaultController.prototype, "WritableStreamDefaultController"],
      [TransformStream.prototype, "TransformStream"],
      [TransformStreamDefaultController.prototype, "TransformStreamDefaultController"],
      [ByteLengthQueuingStrategy.prototype, "ByteLengthQueuingStrategy"],
      [CountQueuingStrategy.prototype, "CountQueuingStrategy"],
      [TextEncoderStream.prototype, "TextEncoderStream"],
      [TextDecoderStream.prototype, "TextDecoderStream"],
      [CompressionStream.prototype, "CompressionStream"],
      [DecompressionStream.prototype, "DecompressionStream"],
      [rs, "ReadableStream"],
      [rsReader, "ReadableStreamDefaultReader"],
      [bsReader, "ReadableStreamBYOBReader"],
      [ws, "WritableStream"],
      [writer, "WritableStreamDefaultWriter"],
      [ts, "TransformStream"],
      [tsCtrl, "TransformStreamDefaultController"],
      [new ByteLengthQueuingStrategy({ highWaterMark: 1 }), "ByteLengthQueuingStrategy"],
      [new CountQueuingStrategy({ highWaterMark: 1 }), "CountQueuingStrategy"],
      [new TextEncoderStream(), "TextEncoderStream"],
      [new TextDecoderStream(), "TextDecoderStream"],
      [new CompressionStream("gzip"), "CompressionStream"],
      [new DecompressionStream("gzip"), "DecompressionStream"],
    ];

    for (const [target, tag] of cases) {
      expect(Object.prototype.toString.call(target)).toBe(`[object ${tag}]`);
    }
  });
});

describe("CompressionStream and DecompressionStream web parity", () => {
  it("validates constructor arguments, ToString coercion, and cross-realm ArrayBuffers", async () => {
    expect(() => new (CompressionStream as any)()).toThrow(TypeError);
    expect(() => new (DecompressionStream as any)()).toThrow(TypeError);
    expect(() => new CompressionStream(Symbol("gzip") as any)).toThrow(TypeError);
    expect(CompressionStream.name).toBe("CompressionStream");
    expect(DecompressionStream.name).toBe("DecompressionStream");

    const cs = new CompressionStream({
      toString() {
        return "gzip";
      },
    } as any);
    const crossRealmBuf = vm.runInNewContext("new Uint8Array([104, 101, 108, 108, 111]).buffer");
    const w = cs.writable.getWriter();
    const outPromise = readAllBytes(cs.readable);
    await w.write(crossRealmBuf);
    await w.close();
    const gz = await outPromise;

    const ds = new DecompressionStream("gzip");
    const dw = ds.writable.getWriter();
    const plainPromise = readAllBytes(ds.readable);
    await dw.write(gz);
    await dw.close();
    const plain = await plainPromise;
    expect(new TextDecoder().decode(plain)).toBe("hello");
  });

  it("decompresses concatenated gzip members (single chunk and split chunks)", async () => {
    const compress = async (text: string): Promise<Uint8Array> => {
      const cs = new CompressionStream("gzip");
      const w = cs.writable.getWriter();
      const outP = readAllBytes(cs.readable);
      await w.write(new TextEncoder().encode(text));
      await w.close();
      return await outP;
    };

    const gz1 = await compress("first-");
    const gz2 = await compress("second");
    const combined = new Uint8Array(gz1.byteLength + gz2.byteLength);
    combined.set(gz1, 0);
    combined.set(gz2, gz1.byteLength);

    // Single combined chunk
    const ds1 = new DecompressionStream("gzip");
    const w1 = ds1.writable.getWriter();
    const p1 = readAllBytes(ds1.readable);
    await w1.write(combined);
    await w1.close();
    expect(new TextDecoder().decode(await p1)).toBe("first-second");

    // Split across member boundary + 1-byte slices
    const ds2 = new DecompressionStream("gzip");
    const w2 = ds2.writable.getWriter();
    const p2 = readAllBytes(ds2.readable);
    for (let i = 0; i < combined.length; i += 3) {
      await w2.write(combined.subarray(i, i + 3));
    }
    await w2.close();
    expect(new TextDecoder().decode(await p2)).toBe("first-second");
  });

  it("throws TypeError on invalid/empty/trailing data for gzip and deflate, while deflate-raw ignores trailing", async () => {
    const compress = async (format: string, text: string): Promise<Uint8Array> => {
      const cs = new CompressionStream(format);
      const w = cs.writable.getWriter();
      const outP = readAllBytes(cs.readable);
      await w.write(new TextEncoder().encode(text));
      await w.close();
      return await outP;
    };

    // Empty close errors with TypeError
    for (const fmt of ["gzip", "deflate", "deflate-raw"]) {
      const ds = new DecompressionStream(fmt);
      const rP = readAllBytes(ds.readable);
      const w = ds.writable.getWriter();
      const cP = w.close();
      await expect(rP).rejects.toThrow(TypeError);
      await expect(cP).rejects.toThrow(TypeError);
    }

    // Trailing bytes on gzip -> TypeError
    const gz = await compress("gzip", "abc");
    const gzTrailing = new Uint8Array(gz.length + 4);
    gzTrailing.set(gz, 0);
    gzTrailing.set([1, 2, 3, 4], gz.length);
    {
      const ds = new DecompressionStream("gzip");
      const rP = readAllBytes(ds.readable);
      const w = ds.writable.getWriter();
      const wP = w.write(gzTrailing).catch(() => {});
      const cP = w.close().catch(() => {});
      await expect(rP).rejects.toThrow(TypeError);
      await wP;
      await cP;
    }

    // Trailing bytes on deflate (single chunk & second chunk) -> TypeError
    const def = await compress("deflate", "abc");
    const defTrailing = new Uint8Array(def.length + 2);
    defTrailing.set(def, 0);
    defTrailing.set([9, 9], def.length);
    {
      const ds = new DecompressionStream("deflate");
      const rP = readAllBytes(ds.readable);
      const w = ds.writable.getWriter();
      const wP = w.write(defTrailing);
      const cP = w.close();
      await expect(rP).rejects.toThrow(TypeError);
      await expect(Promise.all([wP, cP])).rejects.toThrow(TypeError);
    }
    {
      const ds = new DecompressionStream("deflate");
      const rP = readAllBytes(ds.readable);
      const w = ds.writable.getWriter();
      await w.write(def);
      await w.write(new Uint8Array([9, 9]));
      const cP = w.close();
      await expect(rP).rejects.toThrow(TypeError);
      await expect(cP).rejects.toThrow(TypeError);
    }

    // Trailing bytes on deflate-raw -> ignored cleanly matching Compression Streams spec
    const raw = await compress("deflate-raw", "abc");
    const rawTrailing = new Uint8Array(raw.length + 3);
    rawTrailing.set(raw, 0);
    rawTrailing.set([5, 6, 7], raw.length);
    {
      const ds = new DecompressionStream("deflate-raw");
      const rP = readAllBytes(ds.readable);
      const w = ds.writable.getWriter();
      await w.write(rawTrailing);
      await w.close();
      expect(new TextDecoder().decode(await rP)).toBe("abc");
    }
  });
});
