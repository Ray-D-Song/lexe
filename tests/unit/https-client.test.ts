import fs from "node:fs";
import path from "node:path";
import https from "node:https";
import tls from "node:tls";
import http from "node:http";

// `https.get`/`https.request` are the only client entry points that need TLS at
// all, and each case below failed on main:
//   - the URI defaulted to `http:`, so a TLS port got a plaintext request;
//   - per-request `ca`/`rejectUnauthorized` were dropped, because the shared
//     pooled client is built once from the process-wide TLS config;
//   - transport failures were thrown instead of emitted as `error`.
//
// These cases used to run in a child process, because a listener left in the
// test worker made the worker exit non-zero at teardown. That was a harness
// gate, not a test problem, and it is gone: the worker now exits as soon as it
// has reported. So the listener lives here, and every case closes what it opens.

const DATA_DIR = path.resolve("libs/raster_runtime_test_tls/data");

/// A TLS listener that answers one request with a fixed HTTP/1.1 response,
/// echoing any request body back as `got:<body>`.
const startTlsServer = async (): Promise<{
  port: number;
  close: () => Promise<void>;
}> => {
  const server: any = tls.createServer(
    {
      key: fs.readFileSync(path.join(DATA_DIR, "server.key")),
      cert: fs.readFileSync(path.join(DATA_DIR, "server.pem")),
    },
    (socket: any) => {
      socket.on("error", () => {});
      let raw = "";
      const onData = (chunk: any) => {
        raw += chunk.toString();
        const headerEnd = raw.indexOf("\r\n\r\n");
        if (headerEnd === -1) return;
        const head = raw.slice(0, headerEnd).toLowerCase();
        const rest = raw.slice(headerEnd + 4);
        const method = head.split(" ")[0].toUpperCase();
        const declared = head.includes("content-length:")
          ? Number(head.split("content-length:")[1].split("\r\n")[0])
          : 0;
        const finish = (body: string) => {
          socket.removeListener("data", onData);
          const payload = "got:" + body;
          socket.end(
            [
              "HTTP/1.1 200 OK",
              "content-type: text/plain",
              "x-method: " + method,
              "content-length: " + Buffer.byteLength(payload),
              "connection: close",
              "",
              payload,
            ].join("\r\n"),
            () => socket.destroy()
          );
        };
        if (declared === 0) return finish(rest);
        if (rest.length >= declared) return finish(rest.slice(0, declared));
        socket.on("data", (more: any) => {
          raw += more.toString();
          const body = raw.slice(headerEnd + 4);
          if (body.length >= declared) finish(body.slice(0, declared));
        });
      };
      socket.on("data", onData);
    }
  );

  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  const port = server.address().port as number;
  const close = () =>
    new Promise<void>((resolve) => {
      try {
        server.close(() => resolve());
      } catch {
        resolve();
      }
    });
  return { port, close };
};

/// Collect a response body to completion.
const readBody = (res: any): Promise<string> =>
  new Promise((resolve, reject) => {
    let body = "";
    res.on("data", (chunk: any) => (body += chunk));
    res.on("end", () => resolve(body));
    res.on("error", reject);
  });

it("https.get defaults to https: and reads the response body", async () => {
  const { port, close } = await startTlsServer();
  try {
    const result = await new Promise<string>((resolve) => {
      https
        .get(
          { port, host: "127.0.0.1", rejectUnauthorized: false, path: "/" },
          async (res: any) =>
            resolve(`${res.statusCode}|${await readBody(res)}`)
        )
        .on("error", (e: any) => resolve("ERR|" + e.message));
    });
    expect(result).toBe("200|got:");
  } finally {
    await close();
  }
}, 30000);

it("https.request sends a request body and reads the response body", async () => {
  const { port, close } = await startTlsServer();
  try {
    const payload = "payload";
    const result = await new Promise<string>((resolve) => {
      const req = https.request(
        {
          port,
          host: "127.0.0.1",
          rejectUnauthorized: false,
          path: "/",
          method: "POST",
          headers: { "content-length": payload.length },
        },
        async (res: any) =>
          resolve(
            `${res.statusCode}|${res.headers["x-method"]}|${await readBody(res)}`
          )
      );
      req.on("error", (e: any) => resolve("ERR|" + e.message));
      req.write(payload);
      req.end();
    });
    expect(result).toBe("200|POST|got:payload");
  } finally {
    await close();
  }
}, 30000);

it("https.get honours a per-request `ca`", async () => {
  const { port, close } = await startTlsServer();
  try {
    const result = await new Promise<string>((resolve) => {
      https
        .get(
          {
            port,
            host: "127.0.0.1",
            ca: fs.readFileSync(path.join(DATA_DIR, "root.pem")),
            path: "/",
          },
          async (res: any) =>
            resolve(`${res.statusCode}|${await readBody(res)}`)
        )
        .on("error", (e: any) => resolve("ERR|" + e.message));
    });
    expect(result).toBe("200|got:");
  } finally {
    await close();
  }
}, 30000);

it("https.get reports verification failures through the error event", async () => {
  // No `ca` and no `rejectUnauthorized: false`: the self-signed chain must be
  // rejected, and the failure must arrive as an `error` event rather than as an
  // uncaught throw (which is how it used to escape).
  const { port, close } = await startTlsServer();
  try {
    const result = await new Promise<string>((resolve) => {
      const req = https.get({ port, host: "127.0.0.1", path: "/" });
      req.on("response", () => resolve("UNEXPECTED_RESPONSE"));
      req.on("error", (e: any) => resolve("ERROR_EVENT|" + typeof e.message));
    });
    expect(result).toBe("ERROR_EVENT|string");
  } finally {
    await close();
  }
}, 30000);

it("plain http.get is unaffected", async () => {
  const server: any = http.createServer((_req: any, res: any) => {
    res.writeHead(200, { "content-type": "text/plain" });
    res.end("plain");
  });
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  const port = server.address().port;
  try {
    const result = await new Promise<string>((resolve) => {
      http
        .get({ port, host: "127.0.0.1", path: "/" }, async (res: any) =>
          resolve(`${res.statusCode}|${await readBody(res)}`)
        )
        .on("error", (e: any) => resolve("ERR|" + e.message));
    });
    expect(result).toBe("200|plain");
  } finally {
    await new Promise<void>((resolve) => server.close(() => resolve()));
  }
}, 30000);
