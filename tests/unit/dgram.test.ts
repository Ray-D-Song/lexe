import dgram from "dgram";
import defaultImport from "node:dgram";

describe("dgram module", () => {
  it("node:dgram should be the same as dgram", () => {
    expect(defaultImport).toStrictEqual(dgram);
  });

  it("should export createSocket function", () => {
    expect(typeof dgram.createSocket).toBe("function");
  });

  it("should export Socket class", () => {
    expect(typeof dgram.Socket).toBe("function");
  });

  it("should create a UDP4 socket", () => {
    const socket = dgram.createSocket("udp4");
    expect(socket).toBeDefined();
  });

  it("should create a UDP6 socket", () => {
    const socket = dgram.createSocket("udp6");
    expect(socket).toBeDefined();
  });

  it("should create a socket with options object", () => {
    const socket = dgram.createSocket({ type: "udp4" });
    expect(socket).toBeDefined();
  });

  it("should bind and close a socket", async () => {
    const socket = dgram.createSocket("udp4");

    await new Promise<void>((resolve) => {
      socket.bind(() => {
        resolve();
      });
    });

    const address = socket.address();
    expect(address).toBeDefined();
    expect(address.port).toBeGreaterThan(0);
    expect(address.family).toBe("IPv4");

    await new Promise<void>((resolve) => {
      socket.close(() => {
        resolve();
      });
    });
  });

  it("should send and receive messages", async () => {
    const server = dgram.createSocket("udp4");
    const client = dgram.createSocket("udp4");

    const testMessage = "Hello, UDP!";

    await new Promise<void>((resolve, reject) => {
      let messageReceived = false;

      server.on("message", (msg, rinfo) => {
        try {
          expect(msg.toString()).toBe(testMessage);
          expect(rinfo.address).toBeDefined();
          expect(rinfo.port).toBeGreaterThan(0);
          expect(rinfo.family).toBe("IPv4");
          messageReceived = true;
        } catch (err) {
          reject(err);
        }
      });

      server.on("listening", () => {
        const address = server.address();

        client.send(testMessage, address.port, "127.0.0.1", (err) => {
          if (err) {
            reject(err);
          }
        });

        // Wait a bit for message to arrive
        setTimeout(() => {
          try {
            expect(messageReceived).toBe(true);
            client.close();
            server.close();
            resolve();
          } catch (err) {
            reject(err);
          }
        }, 100);
      });

      server.on("error", (err) => {
        reject(err);
      });

      server.bind(0); // Bind to random port
    });
  }, 5000);

  it("should emit listening event", async () => {
    const socket = dgram.createSocket("udp4");

    await new Promise<void>((resolve) => {
      socket.on("listening", () => {
        const address = socket.address();
        expect(address).toBeDefined();
        socket.close();
        resolve();
      });

      socket.bind(0);
    });
  });

  it("should emit close event", async () => {
    const socket = dgram.createSocket("udp4");

    await new Promise<void>((resolve) => {
      socket.on("close", () => {
        resolve();
      });

      socket.bind(() => {
        socket.close();
      });
    });
  });

  it("supports createSocket callback and reports rinfo.size", async () => {
    const serverMessages: Array<{ text: string; size: number; family: string }> = [];
    const server = dgram.createSocket("udp4", (msg, rinfo) => {
      serverMessages.push({
        text: msg.toString(),
        size: rinfo.size,
        family: rinfo.family,
      });
    });
    await new Promise<void>((r) => server.bind(0, "127.0.0.1", r));
    const port = server.address().port;

    const client = dgram.createSocket("udp4");
    await new Promise<void>((r) => client.send("ping-size", port, "127.0.0.1", () => r()));
    await new Promise((r) => setTimeout(r, 40));

    expect(serverMessages).toEqual([{ text: "ping-size", size: 9, family: "IPv4" }]);
    await server[Symbol.asyncDispose]();
    await client[Symbol.asyncDispose]();
  });

  it("supports connect, remoteAddress, send overloads, disconnect, and sending to a new peer", async () => {
    const peerA = dgram.createSocket("udp4");
    const peerB = dgram.createSocket("udp4");
    const gotA: string[] = [];
    const gotB: string[] = [];
    peerA.on("message", (msg) => gotA.push(msg.toString()));
    peerB.on("message", (msg) => gotB.push(msg.toString()));

    await new Promise<void>((r) => peerA.bind(0, "127.0.0.1", r));
    await new Promise<void>((r) => peerB.bind(0, "127.0.0.1", r));
    const portA = peerA.address().port;
    const portB = peerB.address().port;

    const client = dgram.createSocket("udp4");
    expect(() => client.remoteAddress()).toThrow();
    try {
      client.remoteAddress();
    } catch (e: unknown) {
      expect((e as { code?: string }).code).toBe("ERR_SOCKET_DGRAM_NOT_CONNECTED");
    }
    try {
      client.disconnect();
    } catch (e: unknown) {
      expect((e as { code?: string }).code).toBe("ERR_SOCKET_DGRAM_NOT_CONNECTED");
    }
    try {
      client.connect(0);
    } catch (e: unknown) {
      expect((e as { code?: string }).code).toBe("ERR_SOCKET_BAD_PORT");
    }

    let connectEventFired = false;
    client.on("connect", () => {
      connectEventFired = true;
    });
    await new Promise<void>((r) => client.connect(portA, "127.0.0.1", r));
    expect(connectEventFired).toBe(true);
    expect(client.remoteAddress()).toEqual({
      address: "127.0.0.1",
      family: "IPv4",
      port: portA,
    });

    try {
      client.connect(portB, "127.0.0.1");
      throw new Error("duplicate connect should fail");
    } catch (e: unknown) {
      expect((e as { code?: string }).code).toBe("ERR_SOCKET_DGRAM_IS_CONNECTED");
    }

    try {
      client.send(Buffer.from("abcd"), 0, 2, portA, "127.0.0.1");
      throw new Error("connected send with destination should fail");
    } catch (e: unknown) {
      expect((e as { code?: string }).code).toBe("ERR_SOCKET_DGRAM_IS_CONNECTED");
    }

    await new Promise<void>((r, rej) =>
      client.send("one", (err) => (err ? rej(err) : r()))
    );
    await new Promise<void>((r, rej) =>
      client.send(Buffer.from("xxtwoyy"), 2, 3, (err) => (err ? rej(err) : r()))
    );
    await new Promise<void>((r, rej) =>
      client.send(["thr", Buffer.from("ee")], (err) => (err ? rej(err) : r()))
    );

    // Queue counters drain back to 0 once sent.
    expect(client.getSendQueueCount()).toBe(0);
    expect(client.getSendQueueSize()).toBe(0);

    client.disconnect();
    try {
      client.remoteAddress();
      throw new Error("remoteAddress after disconnect should fail");
    } catch (e: unknown) {
      expect((e as { code?: string }).code).toBe("ERR_SOCKET_DGRAM_NOT_CONNECTED");
    }

    // Must actually reach peerB after OS disconnect.
    await new Promise<void>((r, rej) =>
      client.send("to-b", portB, "127.0.0.1", (err) => (err ? rej(err) : r()))
    );
    await new Promise((r) => setTimeout(r, 50));

    expect(gotA).toEqual(["one", "two", "three"]);
    expect(gotB).toEqual(["to-b"]);

    await client[Symbol.asyncDispose]();
    await client[Symbol.asyncDispose]();
    await peerA[Symbol.asyncDispose]();
    await peerB[Symbol.asyncDispose]();
  });

  it("supports broadcast, TTL, multicast, buffer size controls, options, and AbortSignal", async () => {
    const unbound = dgram.createSocket("udp4");
    for (const fn of [
      () => unbound.getRecvBufferSize(),
      () => unbound.getSendBufferSize(),
      () => unbound.setRecvBufferSize(32768),
      () => unbound.setSendBufferSize(32768),
    ]) {
      try {
        fn();
        throw new Error("expected ERR_SOCKET_BUFFER_SIZE");
      } catch (e: unknown) {
        expect((e as { code?: string }).code).toBe("ERR_SOCKET_BUFFER_SIZE");
      }
    }
    for (const fn of [() => unbound.setBroadcast(true), () => unbound.setTTL(64)]) {
      try {
        fn();
        throw new Error("expected EBADF");
      } catch (e: unknown) {
        expect((e as { code?: string }).code).toBe("EBADF");
      }
    }
    unbound.close();

    const ac = new AbortController();
    const sock = dgram.createSocket({
      type: "udp4",
      reuseAddr: true,
      recvBufferSize: 65536,
      sendBufferSize: 65536,
      signal: ac.signal,
    });
    await new Promise<void>((r) => sock.bind(0, "127.0.0.1", r));

    expect(sock.setBroadcast(true)).toBeUndefined();
    expect(sock.setTTL(64)).toBe(64);
    expect(sock.setMulticastTTL(2)).toBe(2);
    expect(sock.setMulticastLoopback(true)).toBe(true);
    expect(sock.setMulticastInterface("127.0.0.1")).toBeUndefined();

    try {
      sock.setTTL(0);
      throw new Error("expected EINVAL");
    } catch (e: unknown) {
      expect((e as { code?: string }).code).toBe("EINVAL");
    }
    try {
      sock.setMulticastTTL(-1);
      throw new Error("expected EINVAL");
    } catch (e: unknown) {
      expect((e as { code?: string }).code).toBe("EINVAL");
    }

    sock.addMembership("224.0.0.1", "127.0.0.1");
    sock.dropMembership("224.0.0.1", "127.0.0.1");

    sock.addSourceSpecificMembership("127.0.0.1", "232.0.0.1", "127.0.0.1");
    sock.dropSourceSpecificMembership("127.0.0.1", "232.0.0.1", "127.0.0.1");

    sock.setRecvBufferSize(131072);
    sock.setSendBufferSize(131072);
    expect(sock.getRecvBufferSize()).toBeGreaterThan(0);
    expect(sock.getSendBufferSize()).toBeGreaterThan(0);

    const closed = new Promise<void>((r) => sock.once("close", r));
    ac.abort();
    await closed;
  });

  it("supports IPv6 udp6 bind and loopback datagrams", async () => {
    const server = dgram.createSocket({ type: "udp6", ipv6Only: true });
    const client = dgram.createSocket("udp6");
    const recvd: Array<{ text: string; size: number; family: string }> = [];
    server.on("message", (msg, rinfo) => {
      recvd.push({
        text: msg.toString(),
        size: rinfo.size,
        family: rinfo.family,
      });
    });

    await new Promise<void>((r) => server.bind(0, "::1", r));
    const port = server.address().port;
    expect(server.address().family).toBe("IPv6");

    await new Promise<void>((r, rej) =>
      client.send("hello-v6", port, "::1", (err) => (err ? rej(err) : r()))
    );
    await new Promise((r) => setTimeout(r, 40));

    expect(recvd).toEqual([{ text: "hello-v6", size: 8, family: "IPv6" }]);
    await client[Symbol.asyncDispose]();
    await server[Symbol.asyncDispose]();
  });
});
