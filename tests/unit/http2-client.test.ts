import http2 from "node:http2";
import net from "node:net";

it("http2.connect accepts URL authority, createConnection handoff, and streaming body", async () => {
  const server = http2.createServer((req, res) => {
    res.writeHead(200);
    res.end(`echo:${req.method}:${req.url}`);
  });

  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  const addr = server.address() as { port: number };
  const url = new URL(`http://127.0.0.1:${addr.port}`);

  try {
    const body = await new Promise<string>((resolve, reject) => {
      const sock = net.connect({ host: "127.0.0.1", port: String(addr.port) });
      const session = http2.connect(url, {
        createConnection: () => sock,
      });
      session.on("error", reject);
      const req = session.request(
        {
          ":method": "POST",
          ":path": "/submit",
          ":scheme": "http",
          ":authority": url.host,
        },
        { endStream: false }
      );
      req.write("hello-");
      req.end("h2");
      const out: Buffer[] = [];
      let status: unknown;
      req.on("response", (h: Record<string, unknown>) => {
        status = h[":status"];
      });
      req.on("data", (chunk: Buffer) => out.push(chunk));
      req.on("end", () => {
        expect(status).toBe(200);
        session.close(() => {
          sock.destroy();
          resolve(Buffer.concat(out).toString("utf8"));
        });
      });
      req.on("error", reject);
    });
    expect(body).toBe("echo:POST:/submit");
  } finally {
    server.close();
  }
});
