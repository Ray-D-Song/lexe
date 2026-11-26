import fs from "node:fs";
import http from "node:http";
import https from "node:https";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { PassThrough, Readable, finished } from "node:stream";

const TLS_DATA_DIR = path.resolve("libs/lexe_test_tls/data");

const listenServer = (
  server: any
): Promise<{ port: number; close: () => Promise<void> }> =>
  new Promise((resolve) => {
    server.listen(0, "127.0.0.1", () => {
      const addr = server.address();
      resolve({
        port: addr.port,
        close: () =>
          new Promise<void>((res) => {
            try {
              server.close(() => res());
            } catch {
              res();
            }
          }),
      });
    });
  });

const readBody = (stream: any): Promise<string> =>
  new Promise((resolve, reject) => {
    const chunks: Buffer[] = [];
    stream.on("data", (c: any) =>
      chunks.push(Buffer.isBuffer(c) ? c : Buffer.from(c))
    );
    stream.on("end", () => resolve(Buffer.concat(chunks).toString("utf8")));
    stream.on("error", reject);
  });

const rawHttpExchange = (
  port: number,
  payload: string
): Promise<string> =>
  new Promise((resolve, reject) => {
    const sock = net.connect(port, "127.0.0.1", () => {
      sock.write(payload);
    });
    let raw = "";
    sock.on("data", (d: any) => {
      raw += d.toString("latin1");
    });
    sock.on("end", () => resolve(raw));
    sock.on("error", reject);
  });

describe("node:http top-level & OutgoingMessage parity", () => {
  it("exports full 35 METHODS, maxHeaderSize, OutgoingMessage, setMaxIdleHTTPParsers, and globalAgent", () => {
    expect(http.METHODS).toHaveLength(35);
    expect(http.METHODS).toContain("MKCALENDAR");
    expect(http.METHODS).toContain("QUERY");
    expect(http.METHODS).toContain("PROPFIND");
    expect(http.maxHeaderSize).toBe(16384);
    expect(typeof http.setMaxIdleHTTPParsers).toBe("function");
    expect(http.setMaxIdleHTTPParsers(8)).toBeUndefined();

    expect(http.globalAgent).toBeInstanceOf(http.Agent);
    expect(http.globalAgent.keepAlive).toBe(true);
    expect((http.globalAgent as any).protocol).toBe("http:");
    expect((http.globalAgent as any).defaultPort).toBe(80);
    expect(http.globalAgent.maxSockets).toBe(Infinity);
    expect(http.globalAgent.maxFreeSockets).toBe(256);

    const om = new http.OutgoingMessage();
    expect(om.headersSent).toBe(false);
    expect(om.setHeader("X-Custom", "abc")).toBe(om);
    expect(om.appendHeader("X-Custom", "def")).toBe(om);
    expect(om.getHeader("x-custom")).toEqual(["abc", "def"]);
    expect(om.getHeaderNames()).toEqual(["x-custom"]);
    expect(om.getRawHeaderNames()).toEqual(["X-Custom"]);
    expect(Object.getPrototypeOf(om.getHeaders())).toBeNull();
    om.setHeaders(
      new Map([
        ["X-Map", "1"],
        ["X-Arr", ["2", "3"] as any],
      ])
    );
    expect(om.hasHeader("x-map")).toBe(true);
    expect(om.getHeader("x-arr")).toEqual(["2", "3"]);
    om.removeHeader("X-Custom");
    expect(om.hasHeader("x-custom")).toBe(false);
  });

  it("validates header names and values with Node error codes", () => {
    http.validateHeaderName("X-Valid-Token");
    http.validateHeaderValue("X-Valid-Token", "hello\tworld");

    let badNameErr: any;
    try {
      http.validateHeaderName("Bad Header");
    } catch (e) {
      badNameErr = e;
    }
    expect(badNameErr?.code).toBe("ERR_INVALID_HTTP_TOKEN");

    let badCharErr: any;
    try {
      http.validateHeaderValue("x-test", "a\r\nb");
    } catch (e) {
      badCharErr = e;
    }
    expect(badCharErr?.code).toBe("ERR_INVALID_CHAR");

    let undefErr: any;
    try {
      http.validateHeaderValue("x-test", undefined as any);
    } catch (e) {
      undefErr = e;
    }
    expect(undefErr?.code).toBe("ERR_HTTP_INVALID_HEADER_VALUE");
  });
});

describe("ClientRequest parity", () => {
  it("supports header setters/getters, properties, instanceof OutgoingMessage, and URL + options", async () => {
    let seenHeaders: any;
    let seenMethod = "";
    let seenUrl = "";
    const server = http.createServer((req, res) => {
      seenMethod = req.method || "";
      seenUrl = req.url || "";
      seenHeaders = req.headers;
      res.end("ok");
    });
    const { port, close } = await listenServer(server);
    try {
      const req = http.request(
        new URL(`http://127.0.0.1:${port}/initial?q=1`),
        {
          method: "post",
          auth: "user:secret",
          headers: { "X-Init": "one" },
        }
      );
      expect(req).toBeInstanceOf(http.OutgoingMessage);
      expect(req.method).toBe("POST");
      expect(req.path).toBe("/initial?q=1");
      expect(req.host).toBe("127.0.0.1");
      expect(req.protocol).toBe("http:");
      expect(req.reusedSocket).toBe(false);
      expect(req.maxHeadersCount).toBeNull();
      expect(req.headersSent).toBe(false);
      expect(req.writableEnded).toBe(false);

      req.path = "/updated?k=2";
      expect(req.path).toBe("/updated?k=2");

      expect(req.setHeader("X-One", "a")).toBe(req);
      expect(req.appendHeader("X-One", "b")).toBe(req);
      expect(req.getHeader("x-one")).toEqual(["a", "b"]);
      expect(req.hasHeader("X-INIT")).toBe(true);
      expect(req.getHeaderNames()).toContain("x-one");
      expect(req.getRawHeaderNames()).toContain("X-One");
      expect(Object.getPrototypeOf(req.getHeaders())).toBeNull();

      req.setHeaders(new Map([["X-Map", "mapped"]]));
      expect(req.getHeader("x-map")).toBe("mapped");
      req.removeHeader("X-Init");
      expect(req.hasHeader("x-init")).toBe(false);

      expect(req.setNoDelay(true)).toBeUndefined();
      expect(req.setSocketKeepAlive(true, 1000)).toBeUndefined();

      let socketEventArg: any = null;
      let finishFired = false;
      req.on("socket", (s: any) => {
        socketEventArg = s;
      });
      req.on("finish", () => {
        finishFired = true;
      });

      const body = await new Promise<string>((resolve, reject) => {
        req.on("response", async (res: any) => resolve(await readBody(res)));
        req.on("error", reject);
        expect(req.end("payload")).toBe(req);
      });

      expect(body).toBe("ok");
      expect(finishFired).toBe(true);
      expect(socketEventArg).toBe(req.socket);
      expect(req.connection).toBe(req.socket);
      expect(req.headersSent).toBe(true);
      expect(req.writableEnded).toBe(true);
      expect(req.writableFinished).toBe(true);

      expect(seenMethod).toBe("POST");
      expect(seenUrl).toBe("/updated?k=2");
      expect(seenHeaders["x-init"]).toBeUndefined();
      expect(seenHeaders["x-map"]).toBe("mapped");
      expect(seenHeaders.authorization).toBe(
        "Basic " + Buffer.from("user:secret").toString("base64")
      );
      expect(seenHeaders["x-one"]).toContain("a");
      expect(seenHeaders["x-one"]).toContain("b");
    } finally {
      await close();
    }
  });

  it("supports flushHeaders streaming before end() and throws ERR_HTTP_HEADERS_SENT after flush", async () => {
    let sawFirstChunkBeforeEnd = false;
    let bodyReceived = "";
    let allowContinue: () => void = () => {};
    const gotChunk = new Promise<void>((r) => {
      allowContinue = r;
    });

    const server = http.createServer((req, res) => {
      req.on("data", (chunk: any) => {
        bodyReceived += chunk.toString();
        if (bodyReceived.includes("hello ")) {
          sawFirstChunkBeforeEnd = true;
          allowContinue();
        }
      });
      req.on("end", () => res.end(bodyReceived));
    });
    const { port, close } = await listenServer(server);
    try {
      const req = http.request({
        host: "127.0.0.1",
        port,
        method: "POST",
        path: "/stream",
      });
      req.setHeader("X-Stream", "yes");
      req.flushHeaders();
      expect(req.headersSent).toBe(true);

      let sentErr: any;
      try {
        req.setHeader("X-Late", "no");
      } catch (e) {
        sentErr = e;
      }
      expect(sentErr?.code).toBe("ERR_HTTP_HEADERS_SENT");

      req.write("hello ");
      await gotChunk;
      expect(sawFirstChunkBeforeEnd).toBe(true);

      const responseBody = await new Promise<string>((resolve, reject) => {
        req.on("response", async (res: any) => resolve(await readBody(res)));
        req.on("error", reject);
        req.end("world");
      });
      expect(responseBody).toBe("hello world");
    } finally {
      await close();
    }
  });

  it("supports ClientRequest.setTimeout, clearTimeout, and AbortSignal", async () => {
    const server = http.createServer((_req, res) => {
      setTimeout(() => {
        if (!res.destroyed) res.end("slow");
      }, 250);
    });
    const { port, close } = await listenServer(server);
    try {
      // setTimeout fires without auto-destroying until caller destroys
      const timedOut = await new Promise<boolean>((resolve) => {
        const req = http.request({ host: "127.0.0.1", port, path: "/" });
        expect(
          req.setTimeout(30, () => {
            req.destroy();
            resolve(true);
          })
        ).toBe(req);
        req.on("error", () => {});
        req.end();
      });
      expect(timedOut).toBe(true);

      // clearTimeout cancels pending timeout
      const clearedOk = await new Promise<boolean>((resolve, reject) => {
        let fired = false;
        const req = http.request({ host: "127.0.0.1", port, path: "/" });
        req.setTimeout(30, () => {
          fired = true;
        });
        req.clearTimeout();
        req.on("response", async (res: any) => {
          const text = await readBody(res);
          resolve(!fired && text === "slow");
        });
        req.on("error", reject);
        req.end();
      });
      expect(clearedOk).toBe(true);

      // AbortController aborts in-flight request
      const ac = new AbortController();
      const aborted = await new Promise<boolean>((resolve) => {
        const req = http.request({
          host: "127.0.0.1",
          port,
          path: "/",
          signal: ac.signal,
        });
        req.on("error", () => resolve(true));
        req.end();
        setTimeout(() => ac.abort(), 20);
      });
      expect(aborted).toBe(true);
    } finally {
      await close();
    }
  });

  if (process.platform !== "win32") {
    it("supports http.request({ socketPath }) over Unix domain sockets", async () => {
      const sockPath = path.join(
        os.tmpdir(),
        `lexe-http-parity-${process.pid}-${Date.now()}.sock`
      );
      const server = http.createServer((req, res) => {
        res.end("uds:" + req.url);
      });
      await new Promise<void>((r) => server.listen(sockPath, r));
      try {
        const body = await new Promise<string>((resolve, reject) => {
          const req = http.request(
            { socketPath: sockPath, path: "/ipc-test" },
            async (res: any) => resolve(await readBody(res))
          );
          req.on("error", reject);
          req.end();
        });
        expect(body).toBe("uds:/ipc-test");
      } finally {
        await new Promise<void>((r) => server.close(() => r()));
        try {
          fs.unlinkSync(sockPath);
        } catch {}
      }
    });
  }
});

describe("ServerResponse & http.Server parity", () => {
  it("supports ServerResponse header APIs, instanceof OutgoingMessage, strictContentLength, and ERR_HTTP_HEADERS_SENT", async () => {
    const server = http.createServer((req, res) => {
      expect(res).toBeInstanceOf(http.OutgoingMessage);
      expect(res.req).toBe(req);
      expect(res.socket).toBe(req.socket);
      expect((res as any).connection).toBe(req.socket);

      if (req.url === "/strict-err") {
        res.strictContentLength = true;
        res.setHeader("Content-Length", 10);
        let errCode = "";
        try {
          res.end("short");
        } catch (e: any) {
          errCode = e?.code || "";
        }
        res.removeHeader("Content-Length");
        res.strictContentLength = false;
        res.end(errCode);
        return;
      }

      expect(res.setHeader("X-First", "1")).toBe(res);
      expect(res.appendHeader("X-First", "2")).toBe(res);
      res.setHeaders(new Headers([["X-Second", "v2"]]));
      res.setHeaders(new Map([["X-MapCase", "m"]]));
      expect(res.getHeaderNames()).toEqual(["x-first", "x-second", "x-mapcase"]);
      expect(res.getRawHeaderNames()).toEqual(["X-First", "x-second", "X-MapCase"]);
      expect(Object.getPrototypeOf(res.getHeaders())).toBeNull();
      res.writeHead(200, { "x-head": "ok" });

      let sentCode = "";
      try {
        res.setHeader("X-Late", "bad");
      } catch (e: any) {
        sentCode = e?.code || "";
      }
      expect(sentCode).toBe("ERR_HTTP_HEADERS_SENT");
      expect(res.end("done")).toBe(res);
    });
    const { port, close } = await listenServer(server);
    try {
      const r1 = await new Promise<string>((resolve, reject) => {
        http
          .get(`http://127.0.0.1:${port}/strict-err`, async (res: any) =>
            resolve(await readBody(res))
          )
          .on("error", reject);
      });
      expect(r1).toBe("ERR_HTTP_CONTENT_LENGTH_MISMATCH");

      const r2 = await new Promise<string>((resolve, reject) => {
        http
          .get(`http://127.0.0.1:${port}/ok`, async (res: any) =>
            resolve(
              `${res.headers["x-head"]}|${res.headers["x-second"]}|${await readBody(res)}`
            )
          )
          .on("error", reject);
      });
      expect(r2).toBe("ok|v2|done");
    } finally {
      await close();
    }
  });

  it("supports writeEarlyHints and writeProcessing", async () => {
    const server = http.createServer((_req, res) => {
      res.writeEarlyHints(
        { link: "</style.css>; rel=preload; as=style", "x-hint": "1" },
        () => {
          res.writeProcessing(() => {
            res.writeHead(200, { "content-type": "text/plain" });
            res.end("early-ok");
          });
        }
      );
    });
    const { port, close } = await listenServer(server);
    try {
      const raw = await rawHttpExchange(
        port,
        "GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
      );
      expect(raw.startsWith("HTTP/1.1 103 Early Hints\r\n")).toBe(true);
      expect(raw).toContain("Link: </style.css>; rel=preload; as=style");
      expect(raw).toContain("HTTP/1.1 102 Processing\r\n\r\n");
      expect(raw).toContain("HTTP/1.1 200 OK");
      expect(raw).toContain("early-ok");
    } finally {
      await close();
    }
  });

  it("supports checkExpectation event and maxConnections drop event", async () => {
    let sawExpect = "";
    const expectServer = new http.Server((_req, res) => res.end("normal"));
    expectServer.on("checkExpectation", (req: any, res: any) => {
      sawExpect = req.headers.expect;
      res.writeHead(200);
      res.end("custom-expect-handled");
    });
    const eSrv = await listenServer(expectServer);

    const dropped: any[] = [];
    const maxConnServer = http.createServer((_req, _res) => {
      // hold connection open
    });
    maxConnServer.maxConnections = 1;
    maxConnServer.on("drop", (info: any) => dropped.push(info));
    const mSrv = await listenServer(maxConnServer);

    let holdSock: any;
    let extraSock: any;
    try {
      const raw = await rawHttpExchange(
        eSrv.port,
        "POST / HTTP/1.1\r\nHost: localhost\r\nExpect: custom-val\r\nConnection: close\r\n\r\n"
      );
      expect(sawExpect).toBe("custom-val");
      expect(raw).toContain("custom-expect-handled");

      holdSock = net.connect(mSrv.port, "127.0.0.1");
      await new Promise<void>((r) => holdSock.once("connect", r));
      // Wait briefly so server increments connection counter before second connect
      await new Promise((r) => setTimeout(r, 20));
      extraSock = net.connect(mSrv.port, "127.0.0.1");
      extraSock.on("error", () => {});
      await new Promise<void>((r) => setTimeout(r, 50));
      expect(dropped.length).toBeGreaterThanOrEqual(1);
      expect(typeof dropped[0].remotePort).toBe("number");
    } finally {
      try {
        holdSock?.destroy();
        extraSock?.destroy();
        maxConnServer.closeAllConnections();
      } catch {}
      await eSrv.close();
      await mSrv.close();
    }
  });

  it("supports insecureHTTPParser: true (bare LF, multi-space, CTL header, obs-fold, chunked, keep-alive)", async () => {
    const server = http.createServer(
      { insecureHTTPParser: true, maxRequestsPerSocket: 2 },
      (req, res) => {
        const chunks: Buffer[] = [];
        req.on("data", (d: any) => chunks.push(d));
        req.on("end", () => {
          res.end(
            JSON.stringify({
              url: req.url,
              ctl: req.headers["x-ctl"],
              fold: req.headers["x-fold"],
              body: Buffer.concat(chunks).toString(),
              trailer: req.trailers["x-tr"],
            })
          );
        });
      }
    );
    const { port, close } = await listenServer(server);
    try {
      const raw = await rawHttpExchange(
        port,
        "POST  /one  HTTP/1.1\nHost: localhost\nX-Ctl: x\x01y\nX-Fold: a\n b\nTransfer-Encoding: chunked\n\n2\nhi\n0\nX-Tr: ok\n\n" +
          "GET /two HTTP/1.1\nHost: localhost\nConnection: close\n\n"
      );
      expect(raw).toContain('"url":"/one"');
      expect(raw).toContain('"ctl":"x\\u0001y"');
      expect(raw).toContain('"fold":"a b"');
      expect(raw).toContain('"body":"hi"');
      expect(raw).toContain('"trailer":"ok"');
      expect(raw).toContain('"url":"/two"');
    } finally {
      await close();
    }
  });
});

describe("node:https Server, Agent, and globalAgent parity", () => {
  it("exports Server and globalAgent on node:https and runs HTTPS server lifecycle", async () => {
    expect(typeof https.Server).toBe("function");
    expect(https.globalAgent).toBeInstanceOf(https.Agent);
    expect((https.globalAgent as any).protocol).toBe("https:");
    expect((https.globalAgent as any).defaultPort).toBe(443);
    expect((https.globalAgent as any).keepAlive).toBe(true);
    expect(
      (https.globalAgent as any).getName({ host: "example.com", port: 8443 })
    ).toBe("example.com:8443:");

    const cert = fs.readFileSync(path.join(TLS_DATA_DIR, "server.pem"));
    const key = fs.readFileSync(path.join(TLS_DATA_DIR, "server.key"));
    const ca = fs.readFileSync(path.join(TLS_DATA_DIR, "root.pem"));

    const server = new https.Server({ cert, key });
    expect(server).toBeInstanceOf(https.Server);
    server.on("request", (req: any, res: any) => {
      res.writeHead(200, { "x-https": "yes" });
      res.end("https-server:" + req.url);
    });
    const { port, close } = await listenServer(server);
    try {
      expect(server.listening).toBe(true);
      const customAgent = new https.Agent({ ca, keepAlive: true });
      const result = await new Promise<string>((resolve, reject) => {
        https
          .get(
            {
              host: "127.0.0.1",
              port,
              path: "/tls-path",
              agent: customAgent,
            },
            async (res: any) => {
              resolve(`${res.statusCode}|${res.headers["x-https"]}|${await readBody(res)}`);
            }
          )
          .on("error", reject);
      });
      expect(result).toBe("200|yes|https-server:/tls-path");
      customAgent.destroy();
    } finally {
      await close();
      expect(server.listening).toBe(false);
    }
  });

  it("preserves headers passed as 3rd argument when writeHead statusMessage is undefined and supports flat header arrays", async () => {
    const server = http.createServer((req: any, res: any) => {
      if (req.url === "/undef-msg") {
        res.writeHead(401, undefined as any, {
          "content-type": "application/json",
          "x-custom": "ok",
        });
        res.end('{"ok":false}');
        return;
      }
      res.writeHead(200, [
        "content-type",
        "text/plain",
        "x-dup",
        "one",
        "x-dup",
        "two",
      ] as any);
      res.end("array-ok");
    });
    const { port, close } = await listenServer(server);
    try {
      const r1 = await new Promise<{ status: number; headers: any; body: string }>((resolve, reject) => {
        http
          .get({ host: "127.0.0.1", port, path: "/undef-msg" }, async (res: any) => {
            resolve({
              status: res.statusCode,
              headers: res.headers,
              body: await readBody(res),
            });
          })
          .on("error", reject);
      });
      expect(r1.status).toBe(401);
      expect(r1.headers["content-type"]).toBe("application/json");
      expect(r1.headers["x-custom"]).toBe("ok");
      expect(r1.body).toBe('{"ok":false}');

      const r2 = await new Promise<{ status: number; headers: any; body: string }>((resolve, reject) => {
        http
          .get({ host: "127.0.0.1", port, path: "/flat-array" }, async (res: any) => {
            resolve({
              status: res.statusCode,
              headers: res.headers,
              body: await readBody(res),
            });
          })
          .on("error", reject);
      });
      expect(r2.status).toBe(200);
      expect(r2.headers["content-type"]).toBe("text/plain");
      expect(String(r2.headers["x-dup"])).toContain("one");
      expect(String(r2.headers["x-dup"])).toContain("two");
      expect(r2.body).toBe("array-ok");
    } finally {
      await close();
    }
  });

  it("exposes IncomingMessage as a Readable stream supporting finished, pipe, async iteration, and delayed readers", async () => {
    const server = http.createServer(async (req: any, res: any) => {
      expect(req instanceof Readable).toBe(true);
      expect(typeof req.pipe).toBe("function");
      expect(typeof req.closed).toBe("boolean");

      if (req.url === "/delayed-eos") {
        // Yield across timer ticks before attaching finished() and 'data'
        await new Promise((r) => setTimeout(r, 15));
        const chunks: Buffer[] = [];
        await new Promise<void>((resolve, reject) => {
          const cleanup = finished(req, { readable: true, writable: false }, (err?: any) => {
            cleanup();
            if (err) reject(err);
            else resolve();
          });
          req.on("data", (chunk: any) => {
            chunks.push(Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk));
          });
        });
        res.writeHead(200, { "content-type": "application/json" });
        res.end(Buffer.concat(chunks));
        return;
      }

      if (req.url === "/pipe") {
        const pt = new PassThrough();
        const out: Buffer[] = [];
        pt.on("data", (c: any) => out.push(Buffer.isBuffer(c) ? c : Buffer.from(c)));
        pt.on("end", () => res.end(Buffer.concat(out)));
        req.pipe(pt);
        return;
      }

      if (req.url === "/async-iter") {
        const parts: Buffer[] = [];
        for await (const chunk of req) {
          parts.push(Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk));
        }
        res.end(Buffer.concat(parts));
        return;
      }

      if (req.url === "/read-empty") {
        expect(req.read()).toBe(null);
        res.end("empty-ok");
        return;
      }

      res.end("fallback");
    });

    const { port, close } = await listenServer(server);
    try {
      const sendPost = (reqPath: string, payload: string) =>
        new Promise<{ status: number; body: string; resIsReadable: boolean; finishedOk: boolean }>((resolve, reject) => {
          const clientReq = http.request(
            { host: "127.0.0.1", port, path: reqPath, method: "POST" },
            (res: any) => {
              const resIsReadable = res instanceof Readable;
              const chunks: Buffer[] = [];
              const cleanup = finished(res, { readable: true, writable: false }, (err?: any) => {
                cleanup();
                if (err) reject(err);
                else resolve({
                  status: res.statusCode,
                  body: Buffer.concat(chunks).toString("utf8"),
                  resIsReadable,
                  finishedOk: true,
                });
              });
              res.on("data", (c: any) => chunks.push(Buffer.isBuffer(c) ? c : Buffer.from(c)));
            },
          );
          clientReq.on("error", reject);
          clientReq.end(payload);
        });

      const d = await sendPost("/delayed-eos", '{"id":"ses_123"}');
      expect(d.status).toBe(200);
      expect(d.body).toBe('{"id":"ses_123"}');
      expect(d.resIsReadable).toBe(true);
      expect(d.finishedOk).toBe(true);

      const p = await sendPost("/pipe", "pipe-payload");
      expect(p.body).toBe("pipe-payload");

      const a = await sendPost("/async-iter", "iter-payload");
      expect(a.body).toBe("iter-payload");

      const e = await sendPost("/read-empty", "");
      expect(e.body).toBe("empty-ok");
    } finally {
      await close();
    }
  });

  it("keeps streaming SSE responses open past keepAliveTimeout", async () => {
    const server = http.createServer((_req: any, res: any) => {
      res.writeHead(200, { "content-type": "text/event-stream" });
      res.write("data: first\n\n");
      setTimeout(() => {
        if (!res.destroyed) {
          res.end("data: second\n\n");
        }
      }, 220);
    });
    server.keepAliveTimeout = 60;
    const { port, close } = await listenServer(server);
    try {
      const response = await fetch(`http://127.0.0.1:${port}/events`);
      expect(response.status).toBe(200);
      const body = await response.text();
      expect(body).toBe("data: first\n\ndata: second\n\n");
    } finally {
      await close();
    }
  });
});
