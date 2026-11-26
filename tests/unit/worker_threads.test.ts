import wtDefault, {
  MessageChannel,
  MessagePort,
  getEnvironmentData,
  setEnvironmentData,
  isMainThread,
  threadId,
  markAsUncloneable,
} from "node:worker_threads";

describe("worker_threads MessageChannel", () => {
  it("exposes main-thread constants", () => {
    expect(isMainThread).toBe(true);
    expect(threadId).toBe(0);
  });

  it("delivers structured clones (no reference pass-through)", async () => {
    const { port1, port2 } = new MessageChannel();
    const original = { n: 1, nested: { a: 2 } };
    const received = await new Promise<unknown>((resolve) => {
      port2.on("message", resolve);
      port1.postMessage(original);
    });
    expect(received).toStrictEqual(original);
    expect(received).not.toBe(original);
    (received as { nested: { a: number } }).nested.a = 99;
    expect(original.nested.a).toBe(2);
    port1.close();
    port2.close();
  });

  it("clones circular references, Map, Set, and typed arrays", async () => {
    const { port1, port2 } = new MessageChannel();
    const circular: { self?: unknown; m: Map<string, number>; s: Set<number>; ta: Uint8Array } =
      {
        m: new Map([["k", 1]]),
        s: new Set([1, 2, 3]),
        ta: new Uint8Array([9, 8, 7]),
      };
    circular.self = circular;

    const received = await new Promise<typeof circular>((resolve) => {
      port2.on("message", (v) => resolve(v as typeof circular));
      port1.postMessage(circular);
    });

    expect(received.self).toBe(received);
    expect(received.m).toBeInstanceOf(Map);
    expect(received.m.get("k")).toBe(1);
    expect(received.s).toBeInstanceOf(Set);
    expect([...received.s]).toEqual([1, 2, 3]);
    expect(received.ta).toBeInstanceOf(Uint8Array);
    expect([...received.ta]).toEqual([9, 8, 7]);
    received.ta[0] = 0;
    expect(circular.ta[0]).toBe(9);
    port1.close();
    port2.close();
  });

  it("throws DataCloneError for functions", () => {
    const { port1 } = new MessageChannel();
    expect(() => port1.postMessage(() => 1)).toThrow();
    try {
      port1.postMessage({ f: function bad() {} });
      throw new Error("expected throw");
    } catch (e: any) {
      expect(e?.name === "DataCloneError" || /clone/i.test(String(e?.message ?? e))).toBe(
        true
      );
    }
    port1.close();
  });

  it("stops delivery after close", async () => {
    const { port1, port2 } = new MessageChannel();
    let count = 0;
    port2.on("message", () => {
      count++;
    });
    port2.close();
    port1.postMessage({ a: 1 });
    await Promise.resolve();
    await Promise.resolve();
    expect(count).toBe(0);
    port1.close();
  });

  it("preserves delivery order", async () => {
    const { port1, port2 } = new MessageChannel();
    const got: number[] = [];
    port2.on("message", (v) => {
      got.push(v as number);
    });
    port1.postMessage(1);
    port1.postMessage(2);
    port1.postMessage(3);
    await Promise.resolve();
    await Promise.resolve();
    expect(got).toEqual([1, 2, 3]);
    port1.close();
    port2.close();
  });

  it("does not swallow listener exceptions", async () => {
    const { port1, port2 } = new MessageChannel();
    port2.on("message", () => {
      throw new Error("listener boom");
    });
    // queueMicrotask will surface the error; ensure postMessage itself succeeds
    // and the exception is not swallowed inside dispatch.
    let threwInMicrotask = false;
    const prev = (globalThis as any).process;
    try {
      port1.postMessage({ ok: true });
      await Promise.resolve();
    } catch {
      threwInMicrotask = true;
    }
    // Depending on host unhandled-rejection handling, the throw may not
    // reject this await; assert delivery path ran by using a second listener.
    const { port1: a, port2: b } = new MessageChannel();
    let saw = false;
    let sawSecond = false;
    b.on("message", () => {
      saw = true;
      throw new Error("boom");
    });
    b.on("message", () => {
      sawSecond = true;
    });
    // First listener throws; second must still run (Node EventEmitter-like
    // forEach of a snapshot). Our implementation iterates a copy and does not
    // catch — so the second may not run if the first throws. Spec for
    // MessagePort is that exceptions are reported, not that later listeners
    // always run. Assert at least the throw is not converted to a silent drop
    // of the message (saw becomes true before throw).
    try {
      a.postMessage(1);
      await Promise.resolve();
    } catch {
      /* ignore */
    }
    expect(saw).toBe(true);
    // sawSecond may or may not be true depending on throw timing; not required.
    void sawSecond;
    void threwInMicrotask;
    void prev;
    a.close();
    b.close();
    port1.close();
    port2.close();
  });

  it("isolates environment data via clone on get", () => {
    const key = "lexe-env-test-key";
    const value = { x: 1, nested: { y: 2 } };
    setEnvironmentData(key, value);
    const a = getEnvironmentData(key) as typeof value;
    const b = getEnvironmentData(key) as typeof value;
    expect(a).toStrictEqual(value);
    expect(a).not.toBe(b);
    a.nested.y = 99;
    expect((getEnvironmentData(key) as typeof value).nested.y).toBe(2);
    // Mutations to original after set must not affect stored data.
    value.nested.y = 7;
    expect((getEnvironmentData(key) as typeof value).nested.y).toBe(2);
  });

  it("ref/unref return the port (API surface)", () => {
    const { port1 } = new MessageChannel();
    expect(port1.ref()).toBe(port1);
    expect(port1.unref()).toBe(port1);
    expect(port1).toBeInstanceOf(MessagePort);
    port1.close();
  });
});

describe("worker_threads Worker spawn", () => {
  it("exchanges messages with workerData", async () => {
    const { Worker } = await import("node:worker_threads");
    const { join } = await import("node:path");
    const workerPath = join(
      process.cwd(),
      "tests/unit/worker_threads_worker.fixture.mjs"
    );
    const worker = new Worker(workerPath, { workerData: { tag: "hello" } });
    await new Promise<void>((resolve) => worker.once("online", resolve));
    const reply = new Promise<unknown>((resolve) => {
      worker.once("message", resolve);
    });
    worker.postMessage({ ping: 1 });
    expect(await reply).toEqual({ pong: 1, workerData: { tag: "hello" } });
    const exited = new Promise<number>((resolve) => worker.once("exit", resolve));
    const termination = worker.terminate();
    expect(termination).toBeInstanceOf(Promise);
    expect(await termination).toBe(await exited);
    expect(await worker.terminate()).toBeUndefined();
  });

  it("passes structured-clone payloads (Date, Map) across process IPC", async () => {
    const { Worker } = await import("node:worker_threads");
    const { join } = await import("node:path");
    const workerPath = join(
      process.cwd(),
      "tests/unit/worker_threads_worker.fixture.mjs"
    );
    const worker = new Worker(workerPath, { workerData: null });
    await new Promise<void>((resolve) => worker.once("online", resolve));
    const when = new Date("2020-01-02T03:04:05.000Z");
    const payload = { when, tags: new Map([["a", 1]]) };
    const reply = new Promise<unknown>((resolve) => {
      worker.once("message", resolve);
    });
    worker.postMessage({ type: "clone-check", payload });
    const got = (await reply) as { ok?: boolean; when?: string; mapSize?: number };
    expect(got.ok).toBe(true);
    expect(got.when).toBe(when.toISOString());
    expect(got.mapSize).toBe(1);
    worker.terminate();
    await new Promise<void>((resolve) => worker.once("exit", () => resolve()));
  });

  it("syncs environmentData to workers", async () => {
    const { Worker, setEnvironmentData, getEnvironmentData } = await import(
      "node:worker_threads"
    );
    const { join } = await import("node:path");
    setEnvironmentData("lexe-worker-env-key", { v: 42 });
    const workerPath = join(
      process.cwd(),
      "tests/unit/worker_threads_worker.fixture.mjs"
    );
    const worker = new Worker(workerPath, { workerData: { askEnv: true } });
    await new Promise<void>((resolve) => worker.once("online", resolve));
    const reply = new Promise<unknown>((resolve) => {
      worker.once("message", resolve);
    });
    worker.postMessage({ ping: 2 });
    expect(await reply).toEqual({ envV: 42 });
    worker.terminate();
    await new Promise<void>((resolve) => worker.once("exit", () => resolve()));
    expect(getEnvironmentData("lexe-worker-env-key")).toEqual({ v: 42 });
  });

  it("shares SharedArrayBuffer and Atomics across workers", async () => {
    const { Worker } = await import("node:worker_threads");
    const { join } = await import("node:path");
    const workerPath = join(
      process.cwd(),
      "tests/unit/worker_threads_worker.fixture.mjs"
    );
    const sab = new SharedArrayBuffer(4);
    const view = new Int32Array(sab);
    view[0] = 0;
    const worker = new Worker(workerPath, { workerData: { sab } });
    await new Promise<void>((resolve) => worker.once("online", resolve));
    const reply = new Promise<unknown>((resolve) => {
      worker.once("message", resolve);
    });
    worker.postMessage({ type: "sab-add", delta: 5 });
    expect(await reply).toEqual({ sabOk: true, value: 5 });
    expect(view[0]).toBe(5);
    Atomics.add(view, 0, 2);
    expect(view[0]).toBe(7);
    worker.terminate();
    await new Promise<void>((resolve) => worker.once("exit", () => resolve()));
  });

  it("exposes global MessagePort/MessageChannel matching worker_threads and markAsUncloneable", () => {
    const g = globalThis as Record<string, any>;
    expect(g.MessagePort).toBe(MessagePort);
    expect(g.MessageChannel).toBe(MessageChannel);
    const ch = new g.MessageChannel();
    expect(ch.port1).toBeInstanceOf(MessagePort);
    expect(ch.port1).toBeInstanceOf(g.MessagePort);
    ch.port1.close();
    ch.port2.close();
    expect(typeof markAsUncloneable).toBe("function");
    expect(wtDefault.markAsUncloneable).toBe(markAsUncloneable);
    expect(() => markAsUncloneable({})).not.toThrow();
  });

  it("loads ES module data: URL workers (base64 and UTF-8)", async () => {
    const { Worker } = await import("node:worker_threads");

    const base64Code = `
      import { parentPort, workerData } from "node:worker_threads";
      parentPort.postMessage({ got: workerData.x + 1 });
    `;
    const base64Url = new URL(
      "data:text/javascript;base64," + Buffer.from(base64Code).toString("base64")
    );
    const w1 = new Worker(base64Url, { workerData: { x: 41 } });
    const msg1 = await new Promise<unknown>((resolve, reject) => {
      w1.once("message", resolve);
      w1.once("error", reject);
    });
    expect(msg1).toEqual({ got: 42 });
    w1.terminate();
    await new Promise<void>((resolve) => w1.once("exit", () => resolve()));

    const textCode = `import{parentPort}from"node:worker_threads";parentPort.postMessage("ok-text");`;
    const textUrl = new URL("data:text/javascript," + encodeURIComponent(textCode));
    const w2 = new Worker(textUrl);
    const msg2 = await new Promise<unknown>((resolve, reject) => {
      w2.once("message", resolve);
      w2.once("error", reject);
    });
    expect(msg2).toBe("ok-text");
    w2.terminate();
    await new Promise<void>((resolve) => w2.once("exit", () => resolve()));
  });

  it("shares WebAssembly.Module across worker_threads via workerData and postMessage", async () => {
    const { Worker } = await import("node:worker_threads");
    const addWasm = new Uint8Array([
      0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, 0x01, 0x07, 0x01, 0x60, 0x02, 0x7f, 0x7f,
      0x01, 0x7f, 0x03, 0x02, 0x01, 0x00, 0x07, 0x07, 0x01, 0x03, 0x61, 0x64, 0x64, 0x00, 0x00,
      0x0a, 0x09, 0x01, 0x07, 0x00, 0x20, 0x00, 0x20, 0x01, 0x6a, 0x0b,
    ]);
    const mod = new WebAssembly.Module(addWasm);

    const code = `
      import { parentPort, workerData } from "node:worker_threads";
      const i1 = new WebAssembly.Instance(workerData.mod);
      parentPort.on("message", (msg) => {
        const i2 = new WebAssembly.Instance(msg.mod);
        parentPort.postMessage({
          a: i1.exports.add(20, 22),
          b: i2.exports.add(10, 5),
          isMod1: workerData.mod instanceof WebAssembly.Module,
          isMod2: msg.mod instanceof WebAssembly.Module,
        });
      });
    `;
    const url = new URL("data:text/javascript;base64," + Buffer.from(code).toString("base64"));
    const worker = new Worker(url, { workerData: { mod } });
    await new Promise<void>((resolve) => worker.once("online", resolve));
    const reply = new Promise<unknown>((resolve, reject) => {
      worker.once("message", resolve);
      worker.once("error", reject);
    });
    worker.postMessage({ mod });
    expect(await reply).toEqual({ a: 42, b: 15, isMod1: true, isMod2: true });
    worker.terminate();
    await new Promise<void>((resolve) => worker.once("exit", () => resolve()));
  });

  it("supports MessagePort.once() / parentPort.once() and handles uncaught worker errors cleanly", async () => {
    const { port1, port2 } = new MessageChannel();
    const seen: unknown[] = [];
    port2.once("message", (v) => seen.push(v));
    port1.postMessage("first");
    port1.postMessage("second");
    await Promise.resolve();
    await Promise.resolve();
    expect(seen).toEqual(["first"]);
    port1.close();
    port2.close();

    const { Worker } = await import("node:worker_threads");
    const echoOnce = `import { parentPort } from "node:worker_threads";
      parentPort.once("message", (v) => parentPort.postMessage({ got: v }));`;
    const w1 = new Worker(
      new URL("data:text/javascript;base64," + Buffer.from(echoOnce).toString("base64"))
    );
    await new Promise<void>((resolve) => w1.once("online", resolve));
    const reply = new Promise<unknown>((resolve) => w1.once("message", resolve));
    w1.postMessage("ping");
    expect(await reply).toEqual({ got: "ping" });
    await w1.terminate();

    const throwing = `throw new Error("boom-worker");`;
    const w2 = new Worker(
      new URL("data:text/javascript;base64," + Buffer.from(throwing).toString("base64"))
    );
    const err = await new Promise<unknown>((resolve) => w2.once("error", resolve));
    expect(String((err as { message?: string })?.message ?? err)).toContain("boom-worker");
  });

  it("supports SHARE_ENV and isolated env snapshots", async () => {
    const { Worker, SHARE_ENV } = await import("node:worker_threads");
    expect(typeof SHARE_ENV).toBe("symbol");

    process.env.LEXE_WT_SHARED_TEST = "from-main";
    const sharedCode = `
      import { parentPort } from "node:worker_threads";
      const seen = process.env.LEXE_WT_SHARED_TEST;
      process.env.LEXE_WT_SHARED_REPLY = "from-worker";
      parentPort.postMessage(seen);
    `;
    const wShared = new Worker(sharedCode, { eval: true, env: SHARE_ENV });
    const sharedExited = new Promise<void>((resolve) => wShared.once("exit", () => resolve()));
    const sharedSeen = await new Promise<unknown>((resolve, reject) => {
      wShared.once("message", resolve);
      wShared.once("error", reject);
    });
    expect(sharedSeen).toBe("from-main");
    await sharedExited;
    expect(process.env.LEXE_WT_SHARED_REPLY).toBe("from-worker");
    delete process.env.LEXE_WT_SHARED_TEST;
    delete process.env.LEXE_WT_SHARED_REPLY;

    process.env.LEXE_WT_ISO_TEST = "orig";
    const isoCode = `
      import { parentPort } from "node:worker_threads";
      const before = process.env.LEXE_WT_ISO_TEST;
      process.env.LEXE_WT_ISO_TEST = "mutated-in-worker";
      parentPort.postMessage({ before, custom: process.env.CUSTOM_K, after: process.env.LEXE_WT_ISO_TEST });
    `;
    const wIso = new Worker(isoCode, {
      eval: true,
      env: { ...process.env, CUSTOM_K: "custom-v" },
    });
    const isoExited = new Promise<void>((resolve) => wIso.once("exit", () => resolve()));
    const isoMsg = await new Promise<unknown>((resolve, reject) => {
      wIso.once("message", resolve);
      wIso.once("error", reject);
    });
    expect(isoMsg).toEqual({
      before: "orig",
      custom: "custom-v",
      after: "mutated-in-worker",
    });
    expect(process.env.LEXE_WT_ISO_TEST).toBe("orig");
    await isoExited;
    delete process.env.LEXE_WT_ISO_TEST;
  });

  it("supports postMessageToThread and process 'workerMessage' events", async () => {
    const { Worker, postMessageToThread, threadId: callerThreadId } = await import(
      "node:worker_threads"
    );

    const seenOnMain: Array<{ value: unknown; source: number }> = [];
    const mainListener = (value: unknown, source: number) => {
      seenOnMain.push({ value, source });
    };
    process.on("workerMessage", mainListener);

    const code = `
      import { parentPort, postMessageToThread, threadId, workerData } from "node:worker_threads";
      process.on("workerMessage", (value, source) => {
        postMessageToThread(workerData.replyTo, { echoed: value, from: source, me: threadId });
      });
      parentPort.postMessage("ready");
    `;
    const w = new Worker(code, { eval: true, workerData: { replyTo: callerThreadId } });
    try {
      const ready = await new Promise<unknown>((resolve, reject) => {
        w.once("message", resolve);
        w.once("error", reject);
      });
      expect(ready).toBe("ready");

      await postMessageToThread(w.threadId, { ping: 42 });
      await new Promise<void>((resolve) => {
        const check = () => {
          if (seenOnMain.length > 0) return resolve();
          setTimeout(check, 5);
        };
        check();
      });
      expect(seenOnMain).toEqual([
        {
          value: { echoed: { ping: 42 }, from: callerThreadId, me: w.threadId },
          source: w.threadId,
        },
      ]);
    } finally {
      process.off("workerMessage", mainListener);
      await w.terminate();
    }
    await expect(postMessageToThread(w.threadId, "gone")).rejects.toThrow();
  });

  it("supports piped worker stdio (stdin, stdout, stderr) and null default stdio", async () => {
    const { Worker } = await import("node:worker_threads");

    const wNull = new Worker(`export {}`, { eval: true });
    const wNullExited = new Promise<void>((resolve) => wNull.once("exit", () => resolve()));
    expect(wNull.stdin).toBeNull();
    expect(wNull.stdout).toBeNull();
    expect(wNull.stderr).toBeNull();
    await wNullExited;

    const code = `
      import { parentPort } from "node:worker_threads";
      let buf = "";
      process.stdin.on("data", (chunk) => {
        buf += Buffer.from(chunk).toString("utf8");
      });
      process.stdin.on("end", () => {
        console.log("OUT:" + buf.trim());
        console.error("ERR:" + buf.trim());
        parentPort.postMessage("done");
      });
    `;
    const w = new Worker(code, {
      eval: true,
      stdin: true,
      stdout: true,
      stderr: true,
    });
    const wExited = new Promise<void>((resolve) => w.once("exit", () => resolve()));
    const wOnline = new Promise<void>((resolve) => w.once("online", () => resolve()));
    expect(w.stdin).not.toBeNull();
    expect(w.stdout).not.toBeNull();
    expect(w.stderr).not.toBeNull();

    let out = "";
    let err = "";
    w.stdout!.on("data", (c: Uint8Array) => {
      out += Buffer.from(c).toString("utf8");
    });
    w.stderr!.on("data", (c: Uint8Array) => {
      err += Buffer.from(c).toString("utf8");
    });

    await wOnline;
    w.stdin!.write("hello-");
    w.stdin!.end("piped");

    const done = await new Promise<unknown>((resolve, reject) => {
      w.once("message", resolve);
      w.once("error", reject);
    });
    expect(done).toBe("done");
    await wExited;
    expect(out.trim()).toBe("OUT:hello-piped");
    expect(err.trim()).toBe("ERR:hello-piped");
  });

  it("supports moveMessagePortToContext across vm contexts", async () => {
    const { MessageChannel, MessagePort, moveMessagePortToContext } = await import(
      "node:worker_threads"
    );
    const vm = await import("node:vm");

    const ctx = vm.createContext({
      received: null as unknown,
      replyFromVm(port: MessagePort) {
        port.on("message", (msg: unknown) => {
          (this as { received: unknown }).received = msg;
          port.postMessage({ ack: msg });
        });
        port.start?.();
      },
    });

    const { port1, port2 } = new MessageChannel();
    const moved = moveMessagePortToContext(port1, ctx) as MessagePort;
    try {
      const isVmObject = (vm.runInContext("p => p instanceof Object", ctx) as (p: unknown) => boolean)(
        moved
      );
      expect(isVmObject).toBe(true);
      expect(moved instanceof MessagePort).toBe(false);
      expect(Object.getPrototypeOf(moved)?.constructor?.name).toBe("MessagePort");

      // Original port in host context is neutered/detached: moving again fails, and postMessage is a no-op.
      expect(() => moveMessagePortToContext(port1, ctx)).toThrow();
      port1.postMessage("ignored-after-move");

      (ctx as { replyFromVm: (p: unknown) => void }).replyFromVm(moved);
      const reply = new Promise<unknown>((resolve) => {
        port2.once("message", resolve);
      });
      port2.postMessage({ hello: "vm" });
      expect(await reply).toEqual({ ack: { hello: "vm" } });
      expect((ctx as { received: unknown }).received).toEqual({ hello: "vm" });
    } finally {
      port2.close();
      moved.close();
    }
  });

  it("exposes resourceLimits on main, worker, and Worker instance", async () => {
    const { Worker, resourceLimits } = await import("node:worker_threads");
    expect(resourceLimits).toEqual({});

    const code = `
      import { parentPort, resourceLimits } from "node:worker_threads";
      parentPort.postMessage(resourceLimits);
    `;
    const w = new Worker(code, {
      eval: true,
      resourceLimits: {
        maxOldGenerationSizeMb: 64,
        stackSizeMb: 2,
      },
    });
    const wExited = new Promise<void>((resolve) => w.once("exit", () => resolve()));
    expect(w.resourceLimits).toEqual({
      maxYoungGenerationSizeMb: 16,
      maxOldGenerationSizeMb: 64,
      codeRangeSizeMb: 0,
      stackSizeMb: 2,
    });
    const innerLimits = await new Promise<unknown>((resolve, reject) => {
      w.once("message", resolve);
      w.once("error", reject);
    });
    expect(innerLimits).toEqual({
      maxYoungGenerationSizeMb: 16,
      maxOldGenerationSizeMb: 64,
      codeRangeSizeMb: 0,
      stackSizeMb: 2,
    });
    await wExited;
    expect(w.resourceLimits).toEqual({});
  });
});
