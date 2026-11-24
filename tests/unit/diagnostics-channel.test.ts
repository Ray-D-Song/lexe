import diagnostics, {
  Channel,
  channel,
  subscribe,
  unsubscribe,
  hasSubscribers,
  tracingChannel,
} from "node:diagnostics_channel";
import legacy from "diagnostics_channel";
import { AsyncLocalStorage } from "node:async_hooks";
import Module from "node:module";
import dgram from "node:dgram";
import http from "node:http";
import net from "node:net";

it("registers diagnostics_channel as a builtin module", () => {
  expect(Module.isBuiltin("diagnostics_channel")).toBe(true);
  expect(require("diagnostics_channel")).toBe(diagnostics);
  expect(require("node:diagnostics_channel")).toBe(diagnostics);
  expect(legacy).toBe(diagnostics);
  expect(typeof tracingChannel).toBe("function");
});

it("returns singleton channels and publishes in order", () => {
  const first = channel("test-channel");
  const second = channel("test-channel");
  expect(first).toBe(second);
  expect(first instanceof Channel).toBe(true);
  expect(null instanceof Channel).toBe(false);
  expect(("str" as any) instanceof Channel).toBe(false);

  const seen: unknown[] = [];
  first.subscribe((message, name) => {
    seen.push([message, name]);
  });
  expect(first instanceof Channel).toBe(true);
  first.publish("one");
  first.publish("two");
  expect(seen).toEqual([
    ["one", "test-channel"],
    ["two", "test-channel"],
  ]);
  expect(first.hasSubscribers).toBe(true);
  expect(hasSubscribers("test-channel")).toBe(true);
});

it("supports symbol channel names and validates channel name types", () => {
  const sym = Symbol("diag.sym");
  const ch = channel(sym);
  const seen: unknown[] = [];
  ch.subscribe((msg, name) => seen.push([msg, name]));
  ch.publish(42);
  expect(seen).toEqual([[42, sym]]);
  expect(() => channel(123 as any)).toThrow(TypeError);
});

it("supports subscribe helpers and unsubscribe", () => {
  const fn = () => {};
  subscribe("helper-channel", fn);
  expect(hasSubscribers("helper-channel")).toBe(true);
  expect(unsubscribe("helper-channel", fn)).toBe(true);
  expect(hasSubscribers("helper-channel")).toBe(false);
  expect(unsubscribe("missing-channel", fn)).toBe(false);
});

it("continues notifying subscribers when one unsubscribes during publish", () => {
  const ch = channel("unsubscribe-during-publish");
  const seen: string[] = [];
  const a = () => {
    seen.push("a");
    ch.unsubscribe(a);
  };
  const b = () => {
    seen.push("b");
  };
  ch.subscribe(a);
  ch.subscribe(b);
  ch.publish("payload");
  expect(seen).toEqual(["a", "b"]);
});

it("supports AsyncLocalStorage bindStore, unbindStore, and runStores", () => {
  const ch = channel("store-binding");
  const als1 = new AsyncLocalStorage<{ wrapped: number }>();
  const als2 = new AsyncLocalStorage<{ raw: number }>();
  const events: unknown[] = [];

  expect(ch.hasSubscribers).toBe(false);
  ch.bindStore(als1, (ctx: { raw: number }) => ({ wrapped: ctx.raw * 10 }));
  ch.bindStore(als2);
  expect(ch.hasSubscribers).toBe(true);
  expect(hasSubscribers("store-binding")).toBe(true);

  const sub = (msg: any) => {
    events.push(["sub", msg, als1.getStore(), als2.getStore()]);
  };
  ch.subscribe(sub);

  const ret = ch.runStores(
    { raw: 3 },
    function (this: { mult: number }, extra: number) {
      events.push(["fn", als1.getStore(), als2.getStore(), this.mult, extra]);
      return als1.getStore()!.wrapped + this.mult + extra;
    },
    { mult: 5 },
    7
  );

  expect(ret).toBe(42);
  expect(events).toEqual([
    ["sub", { raw: 3 }, { wrapped: 30 }, { raw: 3 }],
    ["fn", { wrapped: 30 }, { raw: 3 }, 5, 7],
  ]);

  expect(ch.unbindStore(als1)).toBe(true);
  expect(ch.unbindStore(als1)).toBe(false);
  expect(ch.unbindStore(als2)).toBe(true);
  expect(ch.hasSubscribers).toBe(true); // subscriber still attached

  expect(ch.unsubscribe(sub)).toBe(true);
  expect(ch.hasSubscribers).toBe(false);
});

it("schedules subscriber and transform errors via process.nextTick without dropping execution", () => {
  const ch = channel("error-rethrow-channel");
  const err = new Error("sub boom");
  const transformErr = new Error("transform boom");
  let nextCalled = false;
  const scheduled: Array<() => void> = [];

  ch.subscribe(() => {
    throw err;
  });
  ch.subscribe(() => {
    nextCalled = true;
  });

  const origNextTick = process.nextTick;
  process.nextTick = ((cb: (...args: any[]) => void, ...args: any[]) => {
    scheduled.push(() => cb(...args));
  }) as typeof process.nextTick;

  try {
    ch.publish("go");
    expect(nextCalled).toBe(true);
    expect(scheduled.length).toBe(1);
    expect(() => scheduled[0]()).toThrow("sub boom");

    scheduled.length = 0;
    const txCh = channel("transform-error-channel");
    const als = new AsyncLocalStorage<unknown>();
    txCh.bindStore(als, () => {
      throw transformErr;
    });
    const out = txCh.runStores({ x: 1 }, () => "ran-fn");
    expect(out).toBe("ran-fn");
    expect(scheduled.length).toBe(1);
    expect(() => scheduled[0]()).toThrow("transform boom");
    txCh.unbindStore(als);
  } finally {
    process.nextTick = origNextTick;
  }
});

it("releases inactive unreferenced channels from the weak channel map", async () => {
  let ref: WeakRef<Channel>;
  {
    const ch = channel("weak-gc-test");
    ref = new WeakRef(ch);
    expect(ref.deref()).toBe(ch);
  }
  const gc = (globalThis as any).__gc;
  if (typeof gc === "function") {
    for (let i = 0; i < 6; i++) {
      gc();
      await Promise.resolve();
    }
    if (ref.deref() === undefined) {
      const recreated = channel("weak-gc-test");
      expect(recreated instanceof Channel).toBe(true);
    }
  }
});

describe("TracingChannel", () => {
  it("constructs from string or 5-channel object and validates inputs", () => {
    const tc = tracingChannel("sample.tc");
    expect(tc.start).toBe(channel("tracing:sample.tc:start"));
    expect(tc.end).toBe(channel("tracing:sample.tc:end"));
    expect(tc.asyncStart).toBe(channel("tracing:sample.tc:asyncStart"));
    expect(tc.asyncEnd).toBe(channel("tracing:sample.tc:asyncEnd"));
    expect(tc.error).toBe(channel("tracing:sample.tc:error"));
    expect(Object.keys(tc)).toEqual([]);
    expect(tc.hasSubscribers).toBe(false);

    const desc = Object.getOwnPropertyDescriptor(tc, "start")!;
    expect(desc.writable).toBe(false);
    expect(desc.enumerable).toBe(false);
    expect(desc.configurable).toBe(false);

    const custom = tracingChannel({
      start: channel("c:start"),
      end: channel("c:end"),
      asyncStart: channel("c:asyncStart"),
      asyncEnd: channel("c:asyncEnd"),
      error: channel("c:error"),
    });
    expect(custom.start.name).toBe("c:start");

    expect(() => tracingChannel(123 as any)).toThrow(TypeError);
    expect(() => tracingChannel(null as any)).toThrow(TypeError);
    expect(() => tracingChannel({ start: channel("only-one") } as any)).toThrow(TypeError);
    expect(() => tracingChannel({ start: "bad" } as any)).toThrow(TypeError);
  });

  it("traces traceSync success, errors, and subscribe/unsubscribe", () => {
    const tc = tracingChannel("sync.tc");
    const log: unknown[] = [];
    const handlers = {
      start: (c: any) => log.push(["start", c.id, "result" in c, "error" in c]),
      end: (c: any) => log.push(["end", c.id, c.result, c.error?.message]),
      error: (c: any) => log.push(["error", c.id, c.error?.message]),
    };

    tc.subscribe(handlers);
    expect(tc.hasSubscribers).toBe(true);

    const out = tc.traceSync(
      function (this: { base: number }, a: number, b: number) {
        return this.base + a + b;
      },
      { id: 1 },
      { base: 10 },
      20,
      12
    );
    expect(out).toBe(42);
    expect(log).toEqual([
      ["start", 1, false, false],
      ["end", 1, 42, undefined],
    ]);

    log.length = 0;
    expect(() =>
      tc.traceSync(
        () => {
          throw new Error("sync-err");
        },
        { id: 2 }
      )
    ).toThrow("sync-err");
    expect(log).toEqual([
      ["start", 2, false, false],
      ["error", 2, "sync-err"],
      ["end", 2, undefined, "sync-err"],
    ]);

    expect(tc.unsubscribe(handlers)).toBe(true);
    expect(tc.unsubscribe(handlers)).toBe(false);
    expect(tc.hasSubscribers).toBe(false);
  });

  it("traces tracePromise resolved, rejected, sync throw, and bound stores", async () => {
    const tc = tracingChannel("promise.tc");
    const als = new AsyncLocalStorage<{ req: string }>();
    const log: unknown[] = [];
    const handlers = {
      start: (c: any) => log.push(["start", c.req]),
      end: (c: any) => log.push(["end", c.req, c.result]),
      asyncStart: (c: any) => log.push(["asyncStart", c.req, c.result, c.error?.message]),
      asyncEnd: (c: any) => log.push(["asyncEnd", c.req, c.result, c.error?.message]),
      error: (c: any) => log.push(["error", c.req, c.error?.message]),
    };
    tc.subscribe(handlers);
    tc.start.bindStore(als, (c: any) => ({ req: c.req }));

    let storeInside: unknown;
    let storeAfterAwait: unknown;
    const res = await tc.tracePromise(
      async (x: number) => {
        storeInside = als.getStore();
        await Promise.resolve();
        storeAfterAwait = als.getStore();
        return x * 3;
      },
      { req: "ok" },
      null,
      7
    );
    expect(res).toBe(21);
    expect(storeInside).toEqual({ req: "ok" });
    expect(storeAfterAwait).toEqual({ req: "ok" });
    expect(log).toEqual([
      ["start", "ok"],
      ["end", "ok", undefined],
      ["asyncStart", "ok", 21, undefined],
      ["asyncEnd", "ok", 21, undefined],
    ]);

    log.length = 0;
    await expect(
      tc.tracePromise(async () => {
        throw new Error("async-fail");
      }, { req: "rej" })
    ).rejects.toThrow("async-fail");
    expect(log).toEqual([
      ["start", "rej"],
      ["end", "rej", undefined],
      ["error", "rej", "async-fail"],
      ["asyncStart", "rej", undefined, "async-fail"],
      ["asyncEnd", "rej", undefined, "async-fail"],
    ]);

    log.length = 0;
    expect(() =>
      tc.tracePromise(() => {
        throw new Error("sync-fail");
      }, { req: "sync" })
    ).toThrow("sync-fail");
    expect(log).toEqual([
      ["start", "sync"],
      ["error", "sync", "sync-fail"],
      ["end", "sync", undefined],
    ]);

    tc.start.unbindStore(als);
    tc.unsubscribe(handlers);
  });

  it("traces traceCallback with AsyncLocalStorage and error handling", (done) => {
    const tc = tracingChannel("callback.tc");
    const als = new AsyncLocalStorage<{ tag: string }>();
    const log: unknown[] = [];
    const handlers = {
      start: (c: any) => log.push(["start", c.tag]),
      end: (c: any) => log.push(["end", c.tag]),
      asyncStart: (c: any) => log.push(["asyncStart", c.tag, c.result, c.error?.message]),
      asyncEnd: (c: any) => log.push(["asyncEnd", c.tag, c.result, c.error?.message]),
      error: (c: any) => log.push(["error", c.tag, c.error?.message]),
    };
    tc.subscribe(handlers);
    tc.start.bindStore(als, (c: any) => ({ tag: c.tag }));

    tc.traceCallback(
      (n: number, cb: (err: Error | null, val?: number) => void) => {
        setTimeout(() => cb(null, n + 1), 1);
      },
      -1,
      { tag: "cb1" },
      null,
      41,
      (_err: Error | null, val?: number) => {
        log.push(["cb", val, als.getStore()]);
        // schedule assertion after asyncEnd fires in finally
        setTimeout(() => {
          expect(log).toEqual([
            ["start", "cb1"],
            ["end", "cb1"],
            ["asyncStart", "cb1", 42, undefined],
            ["cb", 42, { tag: "cb1" }],
            ["asyncEnd", "cb1", 42, undefined],
          ]);
          tc.start.unbindStore(als);
          tc.unsubscribe(handlers);
          done();
        }, 0);
      }
    );
  });
});

describe("Built-in diagnostic channels", () => {
  it("publishes udp.socket, net.client.socket, net.server.socket, and http.* channels", async () => {
    const events: Array<[string, string[]]> = [];
    const watched = [
      "udp.socket",
      "net.client.socket",
      "net.server.socket",
      "http.client.request.created",
      "http.client.request.start",
      "http.client.request.error",
      "http.client.response.finish",
      "http.server.response.created",
      "http.server.request.start",
      "http.server.response.finish",
    ];
    const subs = new Map<string, (msg: any, name: string) => void>();
    for (const name of watched) {
      const handler = (msg: any, chName: string) => {
        events.push([chName, Object.keys(msg || {}).sort()]);
      };
      subs.set(name, handler);
      subscribe(name, handler);
    }

    try {
      // UDP socket
      const udp = dgram.createSocket("udp4");
      await new Promise<void>((resolve) => udp.close(resolve));

      // Net server & client
      await new Promise<void>((resolve, reject) => {
        const tcpServer = net.createServer((socket) => {
          socket.end();
        });
        tcpServer.on("error", reject);
        tcpServer.listen(0, "127.0.0.1", () => {
          const port = (tcpServer.address() as net.AddressInfo).port;
          const client = net.connect({ port, host: "127.0.0.1" });
          client.on("error", reject);
          client.on("close", () => tcpServer.close(() => resolve()));
        });
      });

      // HTTP server & client
      await new Promise<void>((resolve, reject) => {
        const server = http.createServer((_req, res) => {
          res.end("ok");
        });
        server.on("error", reject);
        server.listen(0, "127.0.0.1", () => {
          const port = (server.address() as net.AddressInfo).port;
          const req = http.get(`http://127.0.0.1:${port}/`, (res) => {
            res.on("data", () => {});
            res.on("end", () => server.close(() => resolve()));
          });
          req.on("error", reject);
        });
      });

      // HTTP client error (destroy)
      await new Promise<void>((resolve) => {
        const req = http.request("http://127.0.0.1:1/");
        req.on("error", () => resolve());
        req.destroy(new Error("forced"));
      });

      expect(events).toContainEqual(["udp.socket", ["socket"]]);
      expect(events).toContainEqual(["net.client.socket", ["socket"]]);
      expect(events).toContainEqual(["net.server.socket", ["socket"]]);
      expect(events).toContainEqual(["http.client.request.created", ["request"]]);
      expect(events).toContainEqual(["http.client.request.start", ["request"]]);
      expect(events).toContainEqual(["http.client.request.error", ["error", "request"]]);
      expect(events).toContainEqual(["http.client.response.finish", ["request", "response"]]);
      expect(events).toContainEqual(["http.server.response.created", ["request", "response"]]);
      expect(events).toContainEqual([
        "http.server.request.start",
        ["request", "response", "server", "socket"],
      ]);
      expect(events).toContainEqual([
        "http.server.response.finish",
        ["request", "response", "server", "socket"],
      ]);
    } finally {
      for (const [name, handler] of subs) {
        unsubscribe(name, handler);
      }
    }
  });
});
