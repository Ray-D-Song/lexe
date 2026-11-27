import http2, {
  Http2ServerRequest,
  Http2ServerResponse,
  connect,
  constants,
  createSecureServer,
  createServer,
  getDefaultSettings,
  getPackedSettings,
  getUnpackedSettings,
  performServerHandshake,
  sensitiveHeaders,
} from "node:http2";
import { Readable, Stream } from "node:stream";

describe("node:http2 parity", () => {
  it("exports all Node.js http2 top-level members in both ESM and CJS", () => {
    const cjs = require("node:http2");
    const expected = {
      Http2ServerRequest,
      Http2ServerResponse,
      connect,
      constants,
      createSecureServer,
      createServer,
      getDefaultSettings,
      getPackedSettings,
      getUnpackedSettings,
      performServerHandshake,
      sensitiveHeaders,
    };

    for (const [name, val] of Object.entries(expected)) {
      expect(val).toBeDefined();
      expect((http2 as Record<string, unknown>)[name]).toBe(val);
      expect(cjs[name]).toBe(val);
    }

    expect(typeof Http2ServerRequest).toBe("function");
    expect(typeof Http2ServerResponse).toBe("function");
    expect(typeof performServerHandshake).toBe("function");
    expect(Object.getOwnPropertyNames(constants).length).toBe(253);
    expect(constants.HTTP2_HEADER_CONTENT_TYPE).toBe("content-type");
    expect(constants.NGHTTP2_NO_ERROR).toBe(0);
    expect(constants.HTTP_STATUS_OK).toBe(200);
    expect(constants.DEFAULT_SETTINGS_MAX_CONCURRENT_STREAMS).toBe(4294967295);
  });

  it("packs and unpacks HTTP/2 settings frames and rejects invalid lengths", () => {
    const defaults = getDefaultSettings();
    expect(defaults.headerTableSize).toBe(4096);
    expect(defaults.enablePush).toBe(true);
    expect(defaults.maxConcurrentStreams).toBe(4294967295);

    const packed = getPackedSettings({
      headerTableSize: 8192,
      enablePush: false,
      maxConcurrentStreams: 50,
    });
    expect(Buffer.isBuffer(packed)).toBe(true);
    expect(packed.length).toBe(18);

    const unpacked = getUnpackedSettings(packed);
    expect(unpacked.headerTableSize).toBe(8192);
    expect(unpacked.enablePush).toBe(false);
    expect(unpacked.maxConcurrentStreams).toBe(50);

    let badErr: any;
    try {
      getUnpackedSettings(Buffer.from([1, 2, 3]));
    } catch (err) {
      badErr = err;
    }
    expect(badErr?.code).toBe("ERR_HTTP2_INVALID_PACKED_SETTINGS_LENGTH");
  });

  it("passes Http2ServerRequest and Http2ServerResponse instances and sends response headers", async () => {
    let sawReqInstance = false;
    let sawResInstance = false;
    let sawReadable = false;
    let sawStream = false;

    const server = createServer((req, res) => {
      sawReqInstance = req instanceof Http2ServerRequest;
      sawResInstance = res instanceof Http2ServerResponse;
      sawReadable = req instanceof Readable;
      sawStream = res instanceof Stream;

      res.statusCode = 201;
      res.setHeader("x-lexe-h2", "one");
      res.appendHeader("x-lexe-h2", "two");
      expect(res.getHeader("X-Lexe-H2")).toEqual(["one", "two"]);
      res.write("chunk1:");
      res.end("chunk2");
    });

    await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
    const addr = server.address() as { port: number };

    try {
      const result = await new Promise<{ status: unknown; header: unknown; body: string }>(
        (resolve, reject) => {
          const client = connect(`http://127.0.0.1:${addr.port}`);
          client.on("error", reject);
          const req = client.request({
            [constants.HTTP2_HEADER_METHOD]: constants.HTTP2_METHOD_GET,
            [constants.HTTP2_HEADER_PATH]: "/check",
          });
          let status: unknown;
          let header: unknown;
          const chunks: Buffer[] = [];
          req.on("response", (headers) => {
            status = headers[":status"];
            header = headers["x-lexe-h2"];
          });
          req.on("data", (chunk: Buffer) => chunks.push(chunk));
          req.on("end", () => {
            client.close(() => {
              resolve({
                status,
                header,
                body: Buffer.concat(chunks).toString("utf8"),
              });
            });
          });
          req.on("error", reject);
        }
      );

      expect(sawReqInstance).toBe(true);
      expect(sawResInstance).toBe(true);
      expect(sawReadable).toBe(true);
      expect(sawStream).toBe(true);
      expect(result.status).toBe(201);
      expect(result.header).toContain("one");
      expect(result.header).toContain("two");
      expect(result.body).toBe("chunk1:chunk2");
    } finally {
      server.close();
    }
  });

  it("validates performServerHandshake socket argument", () => {
    let err: any;
    try {
      (performServerHandshake as (socket: unknown) => unknown)(null);
    } catch (e) {
      err = e;
    }
    expect(err?.code).toBe("ERR_INVALID_ARG_TYPE");
  });
});
