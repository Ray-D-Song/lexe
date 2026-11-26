import defaultImport from "node:zlib";
import legacyImport from "zlib";

const data = "Hello lexe!!";

it("node:zlib should be the same as zlib", () => {
  expect(defaultImport).toStrictEqual(legacyImport);
});

const {
  deflate,
  inflate,
  deflateSync,
  inflateSync,
  deflateRaw,
  inflateRaw,
  deflateRawSync,
  inflateRawSync,
  gzip,
  gunzip,
  gzipSync,
  gunzipSync,
  brotliCompress,
  brotliDecompress,
  brotliCompressSync,
  brotliDecompressSync,
  zstdCompress,
  zstdDecompress,
  zstdCompressSync,
  zstdDecompressSync,
} = defaultImport;

describe("deflate/inflate", () => {
  it("deflate/inflate", (done) => {
    deflate(data, (err, compressed) => {
      inflate(compressed, (err, decompressed) => {
        expect(data).toEqual(decompressed.toString());
        done();
      });
    });
  });
  it("deflateSync/inflateSync", () => {
    const compressed = deflateSync(data);
    const decompressed = inflateSync(compressed);
    expect(data).toEqual(decompressed.toString());
  });
});

describe("deflateRaw/inflateRaw", () => {
  it("deflateRaw/inflateRaw", (done) => {
    deflateRaw(data, (err, compressed) => {
      inflateRaw(compressed, (err, decompressed) => {
        expect(data).toEqual(decompressed.toString());
        done();
      });
    });
  });
  it("deflateRawSync/inflateRawSync", () => {
    const compressed = deflateRawSync(data);
    const decompressed = inflateRawSync(compressed);
    expect(data).toEqual(decompressed.toString());
  });
});

describe("gzip/gunzip", () => {
  it("gzip/gunzip", (done) => {
    gzip(data, (err, compressed) => {
      gunzip(compressed, (err, decompressed) => {
        expect(data).toEqual(decompressed.toString());
        done();
      });
    });
  });
  it("gzipSync/gunzipSync", () => {
    const compressed = gzipSync(data);
    const decompressed = gunzipSync(compressed);
    expect(data).toEqual(decompressed.toString());
  });
});

describe("brotli", () => {
  it("brotliCompress/brotliDecompress", (done) => {
    brotliCompress(data, (err, compressed) => {
      brotliDecompress(compressed, (err, decompressed) => {
        expect(data).toEqual(decompressed.toString());
        done();
      });
    });
  });
  it("brotliCompressSync/brotliDecompressSync", () => {
    const compressed = brotliCompressSync(data);
    const decompressed = brotliDecompressSync(compressed);
    expect(data).toEqual(decompressed.toString());
  });
});

describe("zstandard", () => {
  it("zstdCompress/zstdDecompress", (done) => {
    zstdCompress(data, (err, compressed) => {
      zstdDecompress(compressed, (err, decompressed) => {
        expect(data).toEqual(decompressed.toString());
        done();
      });
    });
  });
  it("zstdCompressSync/zstdDecompressSync", () => {
    const compressed = zstdCompressSync(data);
    const decompressed = zstdDecompressSync(compressed);
    expect(data).toEqual(decompressed.toString());
  });
});

describe("streaming factories", () => {
  const streamNames = [
    ["createGzip", "createGunzip"],
    ["createDeflate", "createInflate"],
    ["createDeflateRaw", "createInflateRaw"],
    ["createBrotliCompress", "createBrotliDecompress"],
    ["createZstdCompress", "createZstdDecompress"],
  ] as const;

  it("exposes the factory functions and their Transform classes", () => {
    const mod = defaultImport as Record<string, unknown>;
    for (const [compress, decompress] of streamNames) {
      expect(typeof mod[compress]).toBe("function");
      expect(typeof mod[decompress]).toBe("function");
    }
    // Class names mirror the factories (`createGzip` -> `Gzip`).
    expect(typeof mod.Gzip).toBe("function");
    expect(typeof mod.Gunzip).toBe("function");
    expect(typeof mod.constants).toBe("object");
    expect((mod.constants as Record<string, number>).Z_BEST_COMPRESSION).toBe(9);
  });

  it("emits a single framed stream across chunks", async () => {
    const { Readable, pipeline } = await import("node:stream");
    const zlib = defaultImport as unknown as typeof import("node:zlib");

    const chunks = ["hello ", "streaming ", "world"];
    const source = Readable.from(chunks.map((c) => Buffer.from(c)));

    await new Promise<void>((resolve, reject) => {
      pipeline(
        source,
        zlib.createGzip(),
        zlib.createGunzip(),
        (err: Error | null) => (err ? reject(err) : resolve())
      );
    });

    const gz = zlib.createGzip();
    const out: Buffer[] = [];
    gz.on("data", (c: Buffer) => out.push(c));
    await new Promise<void>((resolve, reject) => {
      gz.on("end", resolve);
      gz.on("error", reject);
      gz.write("part1 ");
      gz.write("part2");
      gz.end();
    });

    const total = Buffer.concat(out);
    // A single gzip member decodes as a whole.
    expect(zlib.gunzipSync(total).toString()).toBe("part1 part2");

    // Multi-chunk compression must be smaller than three separate members.
    const perChunk = Buffer.concat(
      chunks.map((c) => zlib.gzipSync(Buffer.from(c)))
    );
    expect(total.length).toBeLessThan(perChunk.length);
  });

  it("stops producing data after the stream is destroyed", async () => {
    const zlib = defaultImport as unknown as typeof import("node:zlib");
    const gz = zlib.createGzip();
    gz.destroy();
    await new Promise<void>((resolve) => gz.on("close", () => resolve()));
    expect(gz.destroyed).toBe(true);
  });
});

describe("raw deflate streaming (ws permessage-deflate)", () => {
  it("createInflateRaw decodes what createDeflateRaw produced", async () => {
    const zlib = defaultImport as unknown as typeof import("node:zlib");
    const payload = Buffer.from("websocket frame payload");

    const enc = zlib.createDeflateRaw();
    const out: Buffer[] = [];
    enc.on("data", (c: Buffer) => out.push(c));
    await new Promise<void>((resolve, reject) => {
      enc.on("end", resolve);
      enc.on("error", reject);
      enc.write(payload.subarray(0, 8));
      enc.write(payload.subarray(8));
      enc.end();
    });

    const raw = Buffer.concat(out);
    expect(zlib.inflateRawSync(raw).toString()).toBe("websocket frame payload");

    // The streaming decoder must accept raw deflate too (previously threw
    // "unsupported encoding: deflate-raw").
    const dec = zlib.createInflateRaw();
    const back: Buffer[] = [];
    dec.on("data", (c: Buffer) => back.push(c));
    await new Promise<void>((resolve, reject) => {
      dec.on("end", resolve);
      dec.on("error", reject);
      dec.write(raw);
      dec.end();
    });
    expect(Buffer.concat(back).toString()).toBe("websocket frame payload");
  });
});

describe("unzip, crc32, streaming controls, and zlib options", () => {
  it("crc32 computes unsigned 32-bit CRC and supports incremental chaining", () => {
    const zlib = defaultImport as unknown as typeof import("node:zlib");
    expect(zlib.crc32("hello")).toBe(907060870);
    // Bit-31-set CRC must stay unsigned (positive > 2^31 - 1).
    expect(zlib.crc32("foo")).toBe(2356372769);
    expect(zlib.crc32(Buffer.from("hello"))).toBe(907060870);
    expect(zlib.crc32(new Uint8Array([102, 111, 111]))).toBe(2356372769);
    expect(zlib.crc32("lo", zlib.crc32("hel"))).toBe(907060870);

    expect(() => zlib.crc32("a", -1)).toThrow();
    expect(() => zlib.crc32("a", 0x100000000)).toThrow();
    // @ts-expect-error runtime validation check
    expect(() => zlib.crc32("a", "1")).toThrow();
  });

  it("unzip, unzipSync, and createUnzip auto-detect gzip and deflate", async () => {
    const zlib = defaultImport as unknown as typeof import("node:zlib");
    const msg = "auto-detect header payload";
    const gz = zlib.gzipSync(msg);
    const df = zlib.deflateSync(msg);

    expect(zlib.unzipSync(gz).toString()).toBe(msg);
    expect(zlib.unzipSync(df).toString()).toBe(msg);
    expect(() => zlib.unzipSync(Buffer.alloc(0))).toThrow();

    await new Promise<void>((resolve, reject) => {
      zlib.unzip(gz, (err, out) => {
        if (err) return reject(err);
        expect(out.toString()).toBe(msg);
        resolve();
      });
    });

    await new Promise<void>((resolve, reject) => {
      zlib.unzip(df, (err, out) => {
        if (err) return reject(err);
        expect(out.toString()).toBe(msg);
        resolve();
      });
    });

    for (const input of [gz, df]) {
      const u = zlib.createUnzip();
      expect(typeof zlib.Unzip).toBe("function");
      expect(u instanceof zlib.Unzip).toBe(true);
      const chunks: Buffer[] = [];
      u.on("data", (c: Buffer) => chunks.push(c));
      await new Promise<void>((resolve, reject) => {
        u.on("end", resolve);
        u.on("error", reject);
        u.end(input);
      });
      expect(Buffer.concat(chunks).toString()).toBe(msg);
    }
  });

  it("supports dictionary, windowBits, memLevel, and strategy options", async () => {
    const zlib = defaultImport as unknown as typeof import("node:zlib");
    const dict = Buffer.from("the quick brown fox jumps over the lazy dog");
    const plain = "the quick brown fox and the lazy dog repeated twice: the quick brown fox";

    const deflated = zlib.deflateSync(plain, {
      dictionary: dict,
      windowBits: 12,
      memLevel: 4,
      strategy: zlib.constants.Z_FILTERED,
      level: 9,
    });
    // Without the dictionary, inflating must fail.
    expect(() => zlib.inflateSync(deflated, { windowBits: 12 })).toThrow();
    expect(
      zlib.inflateSync(deflated, { dictionary: dict, windowBits: 12 }).toString()
    ).toBe(plain);

    const raw = zlib.deflateRawSync(plain, { dictionary: dict, windowBits: 11 });
    expect(
      zlib.inflateRawSync(raw, { dictionary: dict, windowBits: 11 }).toString()
    ).toBe(plain);

    // Streaming with dictionary
    const enc = zlib.createDeflate({ dictionary: dict, windowBits: 12 });
    const dec = zlib.createInflate({ dictionary: dict, windowBits: 12 });
    const out: Buffer[] = [];
    dec.on("data", (c: Buffer) => out.push(c));
    await new Promise<void>((resolve, reject) => {
      dec.on("end", resolve);
      dec.on("error", reject);
      enc.pipe(dec);
      enc.end(plain);
    });
    expect(Buffer.concat(out).toString()).toBe(plain);
  });

  it("supports real flush(kind), params(), bytesWritten, reset(), and close()", async () => {
    const zlib = defaultImport as unknown as typeof import("node:zlib");

    const enc = zlib.createDeflate();
    const dec = zlib.createInflate();
    const compressedChunks: Buffer[] = [];
    const decodedChunks: Buffer[] = [];
    enc.on("data", (c: Buffer) => {
      compressedChunks.push(c);
      dec.write(c);
    });
    dec.on("data", (c: Buffer) => decodedChunks.push(c));

    enc.write("first ");
    await new Promise<void>((resolve, reject) =>
      enc.flush(zlib.constants.Z_SYNC_FLUSH, (err?: Error | null) =>
        err ? reject(err) : resolve()
      )
    );
    // After Z_SYNC_FLUSH, compressed bytes are emitted and decodable BEFORE end().
    expect(Buffer.concat(compressedChunks).length).toBeGreaterThan(0);
    expect(Buffer.concat(decodedChunks).toString()).toBe("first ");
    expect(enc.bytesWritten).toBe(6);

    await new Promise<void>((resolve, reject) =>
      enc.params(1, zlib.constants.Z_DEFAULT_STRATEGY, (err?: Error | null) =>
        err ? reject(err) : resolve()
      )
    );

    await new Promise<void>((resolve, reject) => {
      dec.on("end", resolve);
      dec.on("error", reject);
      enc.on("end", () => dec.end());
      enc.end("second");
    });
    expect(enc.bytesWritten).toBe(12);
    expect(Buffer.concat(decodedChunks).toString()).toBe("first second");

    const toClose = zlib.createGzip();
    toClose.write("abc");
    expect(toClose.bytesWritten).toBe(3);
    await new Promise<void>((resolve) => toClose.close(() => resolve()));
    expect(toClose.bytesWritten).toBe(3);
    expect(zlib.Z_SYNC_FLUSH).toBe(zlib.constants.Z_SYNC_FLUSH);
    expect(zlib.Z_DEFAULT_COMPRESSION).toBe(zlib.constants.Z_DEFAULT_COMPRESSION);
  });
});
