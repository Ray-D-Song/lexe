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
