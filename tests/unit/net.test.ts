import defaultImport from "node:net";
import legacyImport from "net";

it("node:net should be the same as net", () => {
  expect(defaultImport).toStrictEqual(legacyImport);
});

describe("net.isIP", () => {
  const { isIP } = defaultImport;

  it("classifies IPv4 and IPv6", () => {
    expect(isIP("127.0.0.1")).toBe(4);
    expect(isIP("::1")).toBe(6);
    expect(isIP("2001:db8::1")).toBe(6);
  });

  it("accepts IPv6 zone IDs", () => {
    expect(isIP("fe80::1%lo0")).toBe(6);
  });

  it("rejects illegal zone IDs (Node parity)", () => {
    // Empty zone, multiple zones, IPv4 with zone → 0
    expect(isIP("fe80::1%")).toBe(0);
    expect(isIP("fe80::1%a%b")).toBe(0);
    expect(isIP("127.0.0.1%x")).toBe(0);
    // Disallowed zone characters (Node returns 0)
    expect(isIP("fe80::1%a/b")).toBe(0);
    expect(isIP("fe80::1%a_b")).toBe(0);
    expect(isIP("fe80::1%🚀")).toBe(0);
    // Allowed zone charset: alnum, '.', ':', '-'
    expect(isIP("fe80::1%a:b")).toBe(6);
    expect(isIP("fe80::1%a.b")).toBe(6);
    expect(isIP("fe80::1%a-b")).toBe(6);
  });

  it("rejects hostnames, non-strings, empty, and whitespace", () => {
    expect(isIP("localhost")).toBe(0);
    expect(isIP("example.com")).toBe(0);
    expect(isIP("")).toBe(0);
    expect(isIP(" 127.0.0.1")).toBe(0);
    expect(isIP("127.0.0.1 ")).toBe(0);
    expect(isIP(null as any)).toBe(0);
    expect(isIP(undefined as any)).toBe(0);
    expect(isIP(4 as any)).toBe(0);
  });
});

describe("net.Server address()", () => {
  const { createServer } = defaultImport;

  it("returns null before listen (Node parity)", () => {
    const server = createServer();
    // Node returns null, not undefined: code that probes the port before
    // listening relies on the two being distinguishable.
    expect(server.address()).toBeNull();
    expect(server.address()).not.toBeUndefined();
  });

  it("returns address/family/port inside the listen callback", async () => {
    const server = createServer();
    const addr: any = await new Promise((resolve) => {
      server.listen(0, "127.0.0.1", () => resolve(server.address()));
    });
    expect(addr).not.toBeNull();
    expect(addr.address).toBe("127.0.0.1");
    expect(addr.family).toBe("IPv4");
    expect(typeof addr.port).toBe("number");
    expect(addr.port).toBeGreaterThan(0);
    await new Promise<void>((resolve) => server.close(() => resolve()));
  });

  it("returns null again after close (Node parity)", async () => {
    const server = createServer();
    await new Promise<void>((resolve) => {
      server.listen(0, "127.0.0.1", () => resolve());
    });
    expect(server.address()).not.toBeNull();
    await new Promise<void>((resolve) => server.close(() => resolve()));
    expect(server.address()).toBeNull();
  });

  it("tracks listening state, chains listen/close, and returns null err from getConnections", async () => {
    const server = createServer();
    expect(server.listening).toBe(false);
    const listenRet = await new Promise<any>((resolve) => {
      const r = server.listen({ port: 0, host: "127.0.0.1", backlog: 16 }, () => resolve(r));
    });
    expect(listenRet).toBe(server);
    expect(server.listening).toBe(true);

    const connInfo = await new Promise<{ err: any; count: number }>((resolve) => {
      server.getConnections((err, count) => resolve({ err, count }));
    });
    expect(connInfo).toEqual({ err: null, count: 0 });

    const closeRet = await new Promise<any>((resolve) => {
      const r = server.close(() => resolve(r));
    });
    expect(closeRet).toBe(server);
    expect(server.listening).toBe(false);
  });
});

describe("net.Socket timeout, resetAndDestroy, and destroySoon", () => {
  const { createServer, connect, Socket } = defaultImport;

  it("validates setTimeout arguments and fires idle timeout", async () => {
    const idleSock = new Socket();
    expect(idleSock.timeout).toBeUndefined();
    expect(() => (idleSock as any).setTimeout(undefined)).toThrow(TypeError);
    expect(() => (idleSock as any).setTimeout("50")).toThrow(TypeError);
    expect(() => idleSock.setTimeout(-1)).toThrow(RangeError);
    expect(() => idleSock.setTimeout(Infinity)).toThrow(RangeError);

    const server = createServer((conn) => {
      conn.resume();
    });
    await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
    const port = (server.address() as any).port;

    const client = connect(port, "127.0.0.1");
    await new Promise<void>((resolve) => client.on("connect", resolve));

    let timeouts = 0;
    await new Promise<void>((resolve) => {
      const ret = client.setTimeout(35, () => {
        timeouts++;
        resolve();
      });
      expect(ret).toBe(client);
      expect(client.timeout).toBe(35);
    });
    expect(timeouts).toBe(1);

    // Disabling timeout via 0 clears any pending timer
    client.setTimeout(0);
    expect(client.timeout).toBe(0);
    await new Promise((r) => setTimeout(r, 60));
    expect(timeouts).toBe(1);

    client.destroy();
    await new Promise<void>((resolve) => server.close(() => resolve()));
  });

  it("resets idle timer when sending or receiving data", async () => {
    let serverConn: any;
    const server = createServer((conn) => {
      serverConn = conn;
      conn.resume();
    });
    await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
    const port = (server.address() as any).port;

    const client = connect(port, "127.0.0.1");
    await new Promise<void>((resolve) => client.on("connect", resolve));

    let firedAt = 0;
    const start = Date.now();
    client.setTimeout(80, () => {
      firedAt = Date.now();
    });

    await new Promise((r) => setTimeout(r, 45));
    client.write("ping");
    await new Promise((r) => setTimeout(r, 45));
    serverConn.write("pong");
    await new Promise((r) => setTimeout(r, 45));
    // Active pings kept resetting the 80ms timer past 135ms total
    expect(firedAt).toBe(0);

    // Once quiet for >80ms, it fires
    await new Promise<void>((resolve) => client.once("timeout", () => resolve()));
    expect(Date.now() - start).toBeGreaterThanOrEqual(170);

    client.destroy();
    await new Promise<void>((resolve) => server.close(() => resolve()));
  });

  it("handles destroySoon() and resetAndDestroy()", async () => {
    const received: Buffer[] = [];
    const serverConns: any[] = [];
    const serverErrors: any[] = [];
    const server = createServer((conn) => {
      serverConns.push(conn);
      conn.on("data", (chunk) => received.push(Buffer.from(chunk)));
      conn.on("error", (err) => serverErrors.push(err));
    });
    await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
    const port = (server.address() as any).port;

    // 1. Unconnected resetAndDestroy emits ERR_SOCKET_CLOSED
    const unconnected = new Socket();
    const unconnErr = await new Promise<any>((resolve) => {
      unconnected.on("error", resolve);
      const ret = unconnected.resetAndDestroy();
      expect(ret).toBe(unconnected);
    });
    expect(unconnErr?.code).toBe("ERR_SOCKET_CLOSED");
    expect(unconnected.destroyed).toBe(true);

    // 2. destroySoon() flushes write before destroying
    const s1 = connect(port, "127.0.0.1");
    await new Promise<void>((resolve) => s1.on("connect", resolve));
    s1.write("hello-destroy-soon");
    await new Promise<void>((resolve) => {
      s1.once("close", () => resolve());
      s1.destroySoon();
    });
    await new Promise((r) => setTimeout(r, 30));
    expect(Buffer.concat(received).toString()).toBe("hello-destroy-soon");
    expect(s1.destroyed).toBe(true);
    expect(s1.writableEnded).toBe(true);

    // 3. Connected resetAndDestroy()
    const s2 = connect(port, "127.0.0.1");
    await new Promise<void>((resolve) => s2.on("connect", resolve));
    await new Promise<void>((resolve) => {
      s2.once("close", () => resolve());
      const ret = s2.resetAndDestroy();
      expect(ret).toBe(s2);
    });
    expect(s2.destroyed).toBe(true);
    await new Promise((r) => setTimeout(r, 50));
    expect(serverConns.length).toBe(2);
    expect(serverErrors.length).toBeGreaterThanOrEqual(1);

    await new Promise<void>((resolve) => server.close(() => resolve()));
  });
});

describe("net Happy-Eyeballs and Server options", () => {
  const { createServer, connect } = defaultImport;

  it("supports Happy-Eyeballs autoSelectFamily and records attempted addresses", async () => {
    const server = createServer((conn) => {
      conn.end("ok");
    });
    await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
    const port = (server.address() as any).port;

    const sock = connect({
      host: "localhost",
      port,
      autoSelectFamily: true,
      autoSelectFamilyAttemptTimeout: 25,
    });
    const payload = await new Promise<string>((resolve, reject) => {
      const chunks: Buffer[] = [];
      sock.on("data", (c) => chunks.push(Buffer.from(c)));
      sock.on("end", () => resolve(Buffer.concat(chunks).toString()));
      sock.on("error", reject);
    });
    expect(payload).toBe("ok");
    expect(Array.isArray(sock.autoSelectFamilyAttemptedAddresses)).toBe(true);
    expect(sock.autoSelectFamilyAttemptedAddresses.length).toBeGreaterThanOrEqual(1);
    expect(sock.autoSelectFamilyAttemptedAddresses.some((a) => a.includes(`127.0.0.1:${port}`))).toBe(
      true
    );

    await new Promise<void>((resolve) => server.close(() => resolve()));
  });

  it("supports pauseOnConnect, noDelay, keepAlive, highWaterMark, and maxConnections drop", async () => {
    let pausedOnConnect = false;
    let bytesReadOnConnect = -1;
    let bytesReadWhilePaused = -1;
    let readableLenWhilePaused = -1;
    let firstConn: any;
    let seenData = "";
    let acceptedHwms: number[] = [];

    const server = createServer(
      {
        pauseOnConnect: true,
        noDelay: true,
        keepAlive: true,
        keepAliveInitialDelay: 1000,
        highWaterMark: 4096,
      },
      (conn) => {
        acceptedHwms.push(conn.writableHighWaterMark);
        if (!firstConn) firstConn = conn;
        pausedOnConnect = conn.isPaused();
        bytesReadOnConnect = conn.bytesRead;
        setTimeout(() => {
          bytesReadWhilePaused = conn.bytesRead;
          readableLenWhilePaused = conn.readableLength;
          conn.on("data", (chunk) => {
            seenData += Buffer.from(chunk).toString();
          });
          conn.resume();
        }, 40);
      }
    );
    server.maxConnections = 1;
    expect(server.maxConnections).toBe(1);

    await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
    const port = (server.address() as any).port;

    const dropPromise = new Promise<any>((resolve) => {
      server.once("drop", resolve);
    });

    const c1 = connect(port, "127.0.0.1");
    await new Promise<void>((resolve) => c1.on("connect", resolve));
    c1.write("buffered-while-paused");

    // 2nd connection exceeds maxConnections=1 and must emit "drop"
    const c2 = connect(port, "127.0.0.1");
    c2.on("error", () => {});
    const dropped = await dropPromise;
    expect(dropped.localAddress).toBe("127.0.0.1");
    expect(dropped.localPort).toBe(port);
    expect(typeof dropped.remotePort).toBe("number");

    await new Promise((r) => setTimeout(r, 90));
    expect(pausedOnConnect).toBe(true);
    expect(bytesReadOnConnect).toBe(0);
    expect(bytesReadWhilePaused).toBe(0);
    expect(readableLenWhilePaused).toBe(0);
    expect(seenData).toBe("buffered-while-paused");
    expect(acceptedHwms).toEqual([4096]);

    c1.destroy();
    c2.destroy();
    firstConn?.destroy();
    await new Promise<void>((resolve) => server.close(() => resolve()));
  });
});
