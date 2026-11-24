import defaultImport, {
  EventEmitterAsyncResource,
  captureRejectionSymbol,
  errorMonitor,
} from "node:events";
import { AsyncLocalStorage } from "node:async_hooks";
import legacyImport from "events";

it("node:events should be the same as events", () => {
  expect(defaultImport).toStrictEqual(legacyImport);
});

const sleep = (millis: number) => new Promise((cb) => setTimeout(cb, millis));

const { EventEmitter } = defaultImport;
// Named helper exports (module-level functions, plus well-known symbols).
const Events = defaultImport as unknown as {
  once: (
    emitter: any,
    name: string,
    options?: { signal?: AbortSignal }
  ) => Promise<unknown[]>;
  getEventListeners: (emitter: any, name: string) => Function[];
  addAbortListener: (
    signal: AbortSignal,
    listener: () => void
  ) => { [Symbol.dispose](): void };
};

describe("EventEmitter", () => {
  it("should use custom EventEmitter", () => {
    let called = 0;
    const symbolA = Symbol();
    const symbolB = Symbol();
    const symbolC = Symbol();
    const callback = () => {
      called++;
    };

    class MyEmitter extends EventEmitter {}
    const myEmitter = new MyEmitter();
    const myEmitter2 = new MyEmitter();

    myEmitter.once("event", function (a, b) {
      expect(a).toEqual("a");
      expect(b).toEqual("b");
      // @ts-ignore
      expect(this instanceof MyEmitter).toBeTruthy();
      // @ts-ignore
      expect(this === myEmitter).toBeTruthy();
      // @ts-ignore
      expect(this !== myEmitter2).toBeTruthy();
      called++;
    });

    myEmitter.on(symbolA, callback);
    myEmitter.on(symbolB, callback);
    myEmitter.on(symbolC, callback);

    myEmitter.emit("event", "a", "b");
    myEmitter.emit(symbolA);
    myEmitter.emit(symbolB);
    myEmitter.emit(symbolC);

    expect(called).toEqual(4);
    expect(myEmitter.eventNames()).toEqual([symbolA, symbolB, symbolC]);

    myEmitter.off(symbolB, callback);

    myEmitter.emit("event", "a", "b");
    myEmitter.emit(symbolA);
    myEmitter.emit(symbolB);
    myEmitter.emit(symbolC);

    expect(called).toEqual(6);
    expect(myEmitter.eventNames()).toEqual([symbolA, symbolC]);
  });

  it("should prepend event listeners", async () => {
    const myEmitter = new EventEmitter();

    const eventsArray: string[] = [];

    myEmitter.addListener("event", () => {
      eventsArray.push("added first");
    });
    myEmitter.prependListener("event", () => {
      eventsArray.push("added to beginning");
    });
    myEmitter.addListener("event", () => {
      eventsArray.push("last");
    });
    myEmitter.prependListener("event", () => {
      eventsArray.push("even before that");
    });

    myEmitter.emit("event");

    expect(eventsArray).toEqual([
      "even before that",
      "added to beginning",
      "added first",
      "last",
    ]);
  });

  it("should handle crash in event handler", () => {
    const emitter = new EventEmitter();

    emitter.on("data", () => {
      throw new Error("error");
    });

    expect(() => {
      emitter.emit("data", 123);
    }).toThrow();
  });

  it("should handle events emitted recursively", (done) => {
    const ee = new EventEmitter();

    ee.on("test", () => {
      ee.emit("test2");
    });

    ee.on("test2", done);

    ee.emit("test");
  });
});

describe("AbortSignal & AbortController", () => {
  it("should set abort reason on AbortSignal", () => {
    const abortController = new AbortController();
    const signal = abortController.signal;

    abortController.abort("cancelled");

    expect(signal.aborted).toEqual(true);
    expect(signal.reason).toEqual("cancelled");
  });

  it("should throw DomException on timeout", async () => {
    const signal = AbortSignal.timeout(5);
    expect(signal.aborted).toBe(false);

    await sleep(10);
    expect(signal.aborted).toBe(true);
    //@ts-ignore
    expect(signal.reason).toBeInstanceOf(DOMException);
    expect(signal.reason.name).toBe("TimeoutError");
  });

  it("should abort if any signal is aborted asynchronously", async () => {
    let signal = AbortSignal.timeout(5);
    let ctrl = new AbortController();
    //@ts-ignore
    let new_signal: AbortSignal = AbortSignal.any([signal, ctrl.signal]);

    expect(new_signal.aborted).toBe(false);

    await sleep(10);
    expect(new_signal.aborted).toBe(true);
  });

  it("should only emit aborted once", () => {
    let ctrl = new AbortController();
    let count = 0;
    ctrl.signal.onabort = () => {
      count++;
    };
    expect(ctrl.signal.onabort).toEqual(expect.any(Function));
    ctrl.abort();
    expect(ctrl.signal.onabort).toEqual(expect.any(Function)); //keep listener
    ctrl.abort();
    ctrl.abort();
    expect(count).toBe(1);
  });
});

describe("EventTarget", () => {
  it("should execute event listeners", () => {
    const myTarget = new EventTarget();

    const eventsArray: string[] = [];

    myTarget.addEventListener("event", () => {
      eventsArray.push("1st");
    });
    myTarget.addEventListener(
      "event",
      () => {
        eventsArray.push("2nd");
      },
      { once: true }
    );

    myTarget.dispatchEvent(new CustomEvent("event"));
    expect(eventsArray).toEqual(["1st", "2nd"]);

    myTarget.dispatchEvent(new CustomEvent("event"));
    expect(eventsArray).toEqual(["1st", "2nd", "1st"]);
  });
});

describe("Event", () => {
  it("globalThis should have a Event", () => {
    const myEvent = new Event("test");

    expect(myEvent.type).toEqual("test");
    expect(myEvent.bubbles).toBeFalsy();
    expect(myEvent.cancelable).toBeFalsy();
    expect(myEvent.composed).toBeFalsy();
  });
  it("Event should have options", () => {
    const myEvent = new Event("test", {
      bubbles: true,
      cancelable: true,
      composed: true,
    });

    expect(myEvent.bubbles).toBeTruthy();
    expect(myEvent.cancelable).toBeTruthy();
    expect(myEvent.composed).toBeTruthy();
  });
});

describe("EventEmitter meta-events", () => {
  it("emits newListener for newListener and removeListener event names", () => {
    const ee = new EventEmitter();
    const seen: Array<{ type: any; isFn: boolean }> = [];
    ee.on("newListener", (type: any, listener: any) => {
      seen.push({ type, isFn: typeof listener === "function" });
    });

    const nl = () => {};
    const rl = () => {};
    ee.on("newListener", nl);
    ee.on("removeListener", rl);

    expect(seen.some((s) => s.type === "newListener" && s.isFn)).toBe(true);
    expect(seen.some((s) => s.type === "removeListener" && s.isFn)).toBe(true);
  });

  it("emits removeListener for removeListener event name", () => {
    const ee = new EventEmitter();
    const removed: any[] = [];
    const track = (type: any) => {
      removed.push(type);
    };
    ee.on("removeListener", track);
    const target = () => {};
    ee.on("removeListener", target);
    ee.removeListener("removeListener", target);
    expect(removed).toContain("removeListener");
  });

  it("propagates exceptions from newListener handlers", () => {
    const ee = new EventEmitter();
    ee.on("newListener", () => {
      throw new Error("newListener boom");
    });
    expect(() => {
      ee.on("data", () => {});
    }).toThrow("newListener boom");
  });

  it("propagates exceptions from removeListener handlers", () => {
    const ee = new EventEmitter();
    ee.on("removeListener", () => {
      throw new Error("removeListener boom");
    });
    const fn = () => {};
    ee.on("data", fn);
    expect(() => {
      ee.removeListener("data", fn);
    }).toThrow("removeListener boom");
  });

  it("once auto-remove triggers removeListener", () => {
    const ee = new EventEmitter();
    const removed: any[] = [];
    ee.on("removeListener", (type: any, listener: any) => {
      removed.push({ type, listener });
    });
    const onceFn = () => {};
    ee.once("tick", onceFn);
    ee.emit("tick");
    expect(removed.length).toBe(1);
    expect(removed[0].type).toBe("tick");
    expect(removed[0].listener).toBe(onceFn);
    expect(ee.listenerCount("tick")).toBe(0);
  });

  it("keeps later once listeners when an earlier listener throws", () => {
    const ee = new EventEmitter();
    const onceFn = () => {};
    ee.on("x", () => {
      throw new Error("boom");
    });
    ee.once("x", onceFn);
    expect(ee.listenerCount("x")).toBe(2);
    expect(() => ee.emit("x")).toThrow("boom");
    // Node: once not yet invoked → still registered
    expect(ee.listenerCount("x")).toBe(2);
    expect(ee.listeners("x")).toContain(onceFn);
  });

  it("recursive emit does not run a once listener twice", () => {
    // Node onceWrapper: shared fired flag — recursive emit during once body
    // must not re-invoke the same once from the outer snapshot.
    const ee = new EventEmitter();
    let calls = 0;
    ee.once("x", () => {
      calls++;
      ee.emit("x");
    });
    ee.on("x", () => {
      /* permanent listener so recursive emit is not empty */
    });
    ee.emit("x");
    expect({ calls, listeners: ee.listenerCount("x") }).toEqual({
      calls: 1,
      listeners: 1,
    });
  });

  it("on+once same callback: three invocations then only on remains", () => {
    const ee = new EventEmitter();
    let n = 0;
    const f = () => {
      n++;
    };
    ee.on("x", f);
    ee.once("x", f);
    ee.emit("x");
    ee.emit("x");
    expect(n).toBe(3);
    expect(ee.listenerCount("x")).toBe(1);
    expect(ee.listeners("x")).toEqual([f]);
  });

  it("two once same callback: first emit runs twice, second emit zero", () => {
    const ee = new EventEmitter();
    let n = 0;
    const f = () => {
      n++;
    };
    ee.once("x", f);
    ee.once("x", f);
    ee.emit("x");
    expect(n).toBe(2);
    ee.emit("x");
    expect(n).toBe(2);
    expect(ee.listenerCount("x")).toBe(0);
  });

  it("removeListener removes the most recently registered match", () => {
    const ee = new EventEmitter();
    const order: number[] = [];
    const f = () => {
      order.push(1);
    };
    const g = () => {
      order.push(2);
    };
    ee.on("x", f);
    ee.on("x", g);
    ee.on("x", f);
    ee.removeListener("x", f);
    ee.emit("x");
    // Remaining: first f, then g
    expect(order).toEqual([1, 2]);
    expect(ee.listenerCount("x")).toBe(2);
  });

  it("removeAllListeners(event) emits removeListener in reverse registration order", () => {
    const ee = new EventEmitter();
    const seen: string[] = [];
    ee.on("removeListener", (_type: any, listener: any) => {
      seen.push(listener.name || listener._name || "?");
    });
    const a = function a() {};
    const b = function b() {};
    const c = function c() {};
    (a as any)._name = "a";
    (b as any)._name = "b";
    (c as any)._name = "c";
    // prepend c, then on a, on b → list [c, a, b]
    ee.prependListener("x", c);
    ee.on("x", a);
    ee.on("x", b);
    ee.removeAllListeners("x");
    // reverse remove: b, a, c
    expect(seen).toEqual(["b", "a", "c"]);
    expect(ee.listenerCount("x")).toBe(0);
  });

  it("removeAllListeners() processes other events before removeListener itself", () => {
    const ee = new EventEmitter();
    const seen: string[] = [];
    ee.on("removeListener", (type: any) => {
      seen.push(String(type));
    });
    ee.on("x", () => {});
    ee.on("y", () => {});
    ee.removeAllListeners();
    // x and y removed first (order among them is event-list order), removeListener last
    expect(seen.includes("x")).toBe(true);
    expect(seen.includes("y")).toBe(true);
    // After full clear, no listeners remain including removeListener
    expect(ee.listenerCount("removeListener")).toBe(0);
    expect(ee.listenerCount("x")).toBe(0);
    expect(ee.listenerCount("y")).toBe(0);
  });
});

describe("EventEmitter.on async iterator", () => {
  it("is exported as EventEmitter.on and events.on", () => {
    expect(typeof EventEmitter.on).toBe("function");
    expect(typeof (defaultImport as any).on).toBe("function");
  });

  it("always yields args as an array (including single arg)", async () => {
    const ee = new EventEmitter();
    const iter = EventEmitter.on(ee, "data");
    const p = iter.next();
    ee.emit("data", 1);
    const r1 = await p;
    expect(r1).toEqual({ value: [1], done: false });

    const p2 = iter.next();
    ee.emit("data", 1, 2);
    const r2 = await p2;
    expect(r2).toEqual({ value: [1, 2], done: false });
    await iter.return();
  });

  it("serves concurrent next() in FIFO order", async () => {
    const ee = new EventEmitter();
    const iter = EventEmitter.on(ee, "data");
    const p1 = iter.next();
    const p2 = iter.next();
    const p3 = iter.next();
    ee.emit("data", "a");
    ee.emit("data", "b");
    ee.emit("data", "c");
    expect(await p1).toEqual({ value: ["a"], done: false });
    expect(await p2).toEqual({ value: ["b"], done: false });
    expect(await p3).toEqual({ value: ["c"], done: false });
    await iter.return();
  });

  it("rejects all waiters on error and subsequent next()", async () => {
    const ee = new EventEmitter();
    const iter = EventEmitter.on(ee, "data");
    const p1 = iter.next();
    const p2 = iter.next();
    const boom = new Error("fail");
    ee.emit("error", boom);
    await expect(p1).rejects.toBe(boom);
    await expect(p2).rejects.toBe(boom);
    await expect(iter.next()).rejects.toBe(boom);
  });

  it("completes waiters with done:true on close events", async () => {
    const ee = new EventEmitter();
    const iter = EventEmitter.on(ee, "data", { close: ["end"] });
    const p = iter.next();
    ee.emit("end");
    expect(await p).toEqual({ value: undefined, done: true });
    expect(await iter.next()).toEqual({ value: undefined, done: true });
  });

  it("rejects on AbortSignal abort", async () => {
    const ee = new EventEmitter();
    const ac = new AbortController();
    const iter = EventEmitter.on(ee, "data", { signal: ac.signal });
    const p = iter.next();
    ac.abort();
    await expect(p).rejects.toMatchObject({ name: "AbortError" });
  });

  it("throws when signal is already aborted", () => {
    const ee = new EventEmitter();
    const ac = new AbortController();
    ac.abort();
    expect(() => EventEmitter.on(ee, "data", { signal: ac.signal })).toThrow();
  });

  it("return() cleans up and completes", async () => {
    const ee = new EventEmitter();
    const iter = EventEmitter.on(ee, "data");
    const p = iter.next();
    const ret = await iter.return();
    expect(ret).toEqual({ value: undefined, done: true });
    expect(await p).toEqual({ value: undefined, done: true });
    // After return, further emits should not be queued
    ee.emit("data", 1);
    expect(await iter.next()).toEqual({ value: undefined, done: true });
  });

  it("throw() cleans up then rejects", async () => {
    const ee = new EventEmitter();
    const iter = EventEmitter.on(ee, "data");
    const p = iter.next();
    const err = new Error("iterator throw");
    await expect(iter.throw(err)).rejects.toBe(err);
    await expect(p).rejects.toBe(err);
  });

  it("calls pause/resume once around watermarks without dropping events", async () => {
    let pauseCount = 0;
    let resumeCount = 0;
    const ee = new EventEmitter() as EventEmitter & {
      pause: () => void;
      resume: () => void;
    };
    ee.pause = () => {
      pauseCount++;
    };
    ee.resume = () => {
      resumeCount++;
    };

    const hwm = 3;
    const lwm = 1;
    const iter = EventEmitter.on(ee, "data", {
      highWaterMark: hwm,
      lowWaterMark: lwm,
    });

    // Queue past HWM without consuming
    for (let i = 0; i < 5; i++) {
      ee.emit("data", i);
    }
    expect(pauseCount).toBe(1);

    // Drain one by one; no events lost
    const values: number[][] = [];
    for (let i = 0; i < 5; i++) {
      const r = await iter.next();
      values.push(r.value as number[]);
    }
    expect(values).toEqual([[0], [1], [2], [3], [4]]);
    // Resume called once when queue falls to LWM
    expect(resumeCount).toBe(1);
    // Further emit after drain below LWM should not re-pause until HWM again
    ee.emit("data", 99);
    expect(pauseCount).toBe(1);
    await iter.return();
  });

  it("validates options types", () => {
    const ee = new EventEmitter();
    expect(() => EventEmitter.on(ee, "data", "bad" as any)).toThrow(TypeError);
    expect(() =>
      EventEmitter.on(ee, "data", { highWaterMark: 0 } as any)
    ).toThrow(TypeError);
    expect(() =>
      EventEmitter.on(ee, "data", { lowWaterMark: -1 } as any)
    ).toThrow(TypeError);
    expect(() =>
      EventEmitter.on(ee, "data", { close: "end" as any })
    ).toThrow(TypeError);
    expect(() =>
      EventEmitter.on(ee, "data", { signal: {} as any })
    ).toThrow(TypeError);
  });
});

describe("module-level helpers", () => {
  it("exports the Node helper surface", () => {
    const mod = Events as unknown as Record<string, unknown>;
    for (const name of [
      "once",
      "getEventListeners",
      "getMaxListeners",
      "setMaxListeners",
      "addAbortListener",
    ]) {
      expect(typeof mod[name]).toBe("function");
    }
    expect(typeof mod.errorMonitor).toBe("symbol");
    expect(typeof mod.captureRejectionSymbol).toBe("symbol");
    expect(mod.defaultMaxListeners).toBe(10);
  });

  it("once resolves with the emitted arguments", async () => {
    const ee = new EventEmitter();
    const pending = Events.once(ee, "ping");
    setTimeout(() => ee.emit("ping", 1, "two"), 5);
    await expect(pending).resolves.toEqual([1, "two"]);
    // The listener must be gone afterwards.
    expect(ee.listenerCount("ping")).toBe(0);
  });

  it("once rejects when an error is emitted first", async () => {
    const ee = new EventEmitter();
    const pending = Events.once(ee, "never");
    setTimeout(() => ee.emit("error", new Error("boom")), 5);
    await expect(pending).rejects.toThrow("boom");
  });

  it("once rejects with AbortError when the signal aborts", async () => {
    const ee = new EventEmitter();
    const ac = new AbortController();
    const pending = Events.once(ee, "x", { signal: ac.signal });
    setTimeout(() => ac.abort(), 5);
    await expect(pending).rejects.toMatchObject({
      name: "AbortError",
      code: "ABORT_ERR",
    });
  });

  it("once rejects immediately for an already-aborted signal", async () => {
    const ac = new AbortController();
    ac.abort();
    await expect(
      Events.once(new EventEmitter(), "x", { signal: ac.signal })
    ).rejects.toMatchObject({ name: "AbortError" });
  });

  it("getEventListeners returns a copy of the listener list", () => {
    const ee = new EventEmitter();
    const fn = () => {};
    ee.on("a", fn);
    const listeners = Events.getEventListeners(ee, "a");
    expect(listeners.length).toBe(1);
    expect(listeners[0]).toBe(fn);
    // Mutating the returned array must not affect the emitter.
    listeners.length = 0;
    expect(ee.listenerCount("a")).toBe(1);
  });

  it("addAbortListener fires on abort and exposes Symbol.dispose", async () => {
    const ac = new AbortController();
    let fired = 0;
    const handle = Events.addAbortListener(ac.signal, () => fired++);
    expect(typeof handle[Symbol.dispose]).toBe("function");
    ac.abort();
    await new Promise((r) => setTimeout(r, 10));
    expect(fired).toBe(1);
  });

  it("addAbortListener defers an already-aborted signal to a microtask", async () => {
    const ac = new AbortController();
    ac.abort();
    let fired = 0;
    Events.addAbortListener(ac.signal, () => fired++);
    expect(fired).toBe(0);
    await Promise.resolve();
    await new Promise((r) => setTimeout(r, 5));
    expect(fired).toBe(1);
  });
});

describe("max listener bookkeeping", () => {
  it("exposes setMaxListeners/getMaxListeners/rawListeners on the prototype", () => {
    const ee = new EventEmitter();
    for (const name of ["setMaxListeners", "getMaxListeners", "rawListeners"]) {
      expect(typeof (EventEmitter.prototype as any)[name]).toBe("function");
    }
  });

  it("setMaxListeners(0) is accepted (framework bootstrap idiom)", () => {
    // merge-stream / jest-worker / Next call this unconditionally.
    const ee = new EventEmitter();
    expect(() => ee.setMaxListeners(0)).not.toThrow();
    expect(ee.getMaxListeners()).toBe(0);
  });

  it("getMaxListeners defaults to defaultMaxListeners", () => {
    const ee = new EventEmitter();
    expect(ee.getMaxListeners()).toBe(10);
    ee.setMaxListeners(3);
    expect(ee.getMaxListeners()).toBe(3);
  });

  it("rawListeners returns the registered listeners", () => {
    const ee = new EventEmitter();
    const fn = () => {};
    ee.on("a", fn);
    expect(ee.rawListeners("a")).toEqual([fn]);
  });

  it("rejects invalid maxListener values", () => {
    const ee = new EventEmitter();
    expect(() => ee.setMaxListeners(-1 as any)).toThrow();
    expect(() => ee.setMaxListeners("x" as any)).toThrow();
  });
});

describe("errorMonitor and unhandled error semantics", () => {
  it("invokes errorMonitor listeners before error listeners and before throwing", () => {
    const ee = new EventEmitter();
    const order: string[] = [];
    ee.on(errorMonitor, (err: any) => {
      order.push("monitor:" + err.message);
    });
    expect(() => ee.emit("error", new Error("unhandled"))).toThrow("unhandled");
    expect(order).toEqual(["monitor:unhandled"]);

    ee.on("error", (err: any) => {
      order.push("handler:" + err.message);
    });
    expect(ee.emit("error", new Error("handled"))).toBe(true);
    expect(order).toEqual(["monitor:unhandled", "monitor:handled", "handler:handled"]);
  });

  it("throws ERR_UNHANDLED_ERROR when emitting non-Error error events without a listener", () => {
    const ee = new EventEmitter();
    let caughtUndef: any;
    try {
      ee.emit("error");
    } catch (e) {
      caughtUndef = e;
    }
    expect(caughtUndef).toBeInstanceOf(Error);
    expect(caughtUndef.code).toBe("ERR_UNHANDLED_ERROR");
    expect(caughtUndef.message).toContain("undefined");

    let caughtStr: any;
    try {
      ee.emit("error", "boom");
    } catch (e) {
      caughtStr = e;
    }
    expect(caughtStr).toBeInstanceOf(Error);
    expect(caughtStr.code).toBe("ERR_UNHANDLED_ERROR");
    expect(caughtStr.context).toBe("boom");
    expect(caughtStr.message).toContain("'boom'");
  });
});

describe("captureRejections", () => {
  it("routes rejected listener promises to captureRejectionSymbol or error event", async () => {
    const eeCustom = new EventEmitter({ captureRejections: true });
    let captured: any[] | null = null;
    (eeCustom as any)[captureRejectionSymbol] = (err: Error, ev: string, arg: number) => {
      captured = [err.message, ev, arg];
    };
    eeCustom.on("work", async (n: number) => {
      throw new Error("fail-" + n);
    });
    eeCustom.emit("work", 7);
    await new Promise((r) => setTimeout(r, 10));
    expect(captured).toEqual(["fail-7", "work", 7]);

    const eeFallback = new EventEmitter({ captureRejections: true });
    let errMsg = "";
    eeFallback.on("error", (err: Error) => {
      errMsg = err.message;
    });
    eeFallback.on("work", async () => {
      throw new Error("fallback-err");
    });
    eeFallback.emit("work");
    await new Promise((r) => setTimeout(r, 10));
    expect(errMsg).toBe("fallback-err");
  });

  it("supports EventEmitter.captureRejections global default and rejects invalid values", async () => {
    const prev = (EventEmitter as any).captureRejections;
    try {
      expect(prev).toBe(false);
      expect(() => new EventEmitter({ captureRejections: "yes" as any })).toThrow(TypeError);
      expect(() => {
        (EventEmitter as any).captureRejections = 123;
      }).toThrow(TypeError);

      (EventEmitter as any).captureRejections = true;
      const ee = new EventEmitter();
      let seen = "";
      ee.on("error", (e: Error) => {
        seen = e.message;
      });
      ee.on("ping", () => Promise.reject(new Error("global-default")));
      ee.emit("ping");
      await new Promise((r) => setTimeout(r, 10));
      expect(seen).toBe("global-default");
    } finally {
      (EventEmitter as any).captureRejections = prev;
    }
  });
});

describe("EventEmitterAsyncResource", () => {
  it("preserves AsyncLocalStorage context across emit() and exposes resource metadata", () => {
    const als = new AsyncLocalStorage<{ req: number }>();
    const resource = als.run({ req: 42 }, () => new EventEmitterAsyncResource({ name: "MySource" }));
    expect(resource).toBeInstanceOf(EventEmitter);
    expect(resource).toBeInstanceOf(EventEmitterAsyncResource);
    expect(typeof resource.asyncId).toBe("number");
    expect(typeof resource.triggerAsyncId).toBe("number");
    expect(resource.asyncResource).toBeTruthy();

    let observed: any = null;
    resource.on("msg", (payload: string) => {
      observed = { payload, store: als.getStore() };
    });

    als.exit(() => {
      expect(als.getStore()).toBeUndefined();
      resource.emit("msg", "hello");
    });

    expect(observed).toEqual({ payload: "hello", store: { req: 42 } });
    expect(resource.emitDestroy()).toBe(resource);

    const byStr = new EventEmitterAsyncResource("StringName");
    expect(typeof byStr.asyncId).toBe("number");
    byStr.emitDestroy();

    expect(() => new EventEmitterAsyncResource({} as any)).toThrow(TypeError);
  });
});
