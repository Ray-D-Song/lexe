import {
  AsyncLocalStorage,
  createHook,
  executionAsyncId,
} from "node:async_hooks";
import defaultImport from "node:async_hooks";
import legacyImport from "async_hooks";

it("exports AsyncLocalStorage on CJS, node: alias, and default", () => {
  expect(typeof AsyncLocalStorage).toBe("function");
  expect(defaultImport.AsyncLocalStorage).toBe(legacyImport.AsyncLocalStorage);
  expect(
    defaultImport.default?.AsyncLocalStorage ?? defaultImport.AsyncLocalStorage
  ).toBe(AsyncLocalStorage);
});

it("getStore is undefined initially", () => {
  const als = new AsyncLocalStorage<number>();
  expect(als.getStore()).toBeUndefined();
});

it("run sets store synchronously and restores afterward", () => {
  const als = new AsyncLocalStorage<string>();
  const ret = als.run("inner", function (this: unknown, a: number) {
    expect(this).toBeNull();
    expect(a).toBe(1);
    expect(als.getStore()).toBe("inner");
    return "done";
  }, 1);
  expect(ret).toBe("done");
  expect(als.getStore()).toBeUndefined();
});

it("nested run restores outer store", () => {
  const als = new AsyncLocalStorage<string>();
  als.run("outer", () => {
    expect(als.getStore()).toBe("outer");
    als.run("inner", () => {
      expect(als.getStore()).toBe("inner");
    });
    expect(als.getStore()).toBe("outer");
  });
  expect(als.getStore()).toBeUndefined();
});

it("restores store after synchronous throw", () => {
  const als = new AsyncLocalStorage<string>();
  expect(() =>
    als.run("x", () => {
      throw new Error("boom");
    })
  ).toThrow("boom");
  expect(als.getStore()).toBeUndefined();
});

it("enterWith persists until replaced", () => {
  const als = new AsyncLocalStorage<string>();
  als.enterWith("entered");
  expect(als.getStore()).toBe("entered");
  als.enterWith("next");
  expect(als.getStore()).toBe("next");
  als.disable();
  expect(als.getStore()).toBeUndefined();
});

it("exit temporarily clears store", () => {
  const als = new AsyncLocalStorage<string>();
  als.run("v", () => {
    expect(als.getStore()).toBe("v");
    als.exit(() => {
      expect(als.getStore()).toBeUndefined();
    });
    expect(als.getStore()).toBe("v");
  });
});

it("propagates across Promise.then, await, and queueMicrotask", async () => {
  const als = new AsyncLocalStorage<string>();
  await als.run("p", async () => {
    expect(als.getStore()).toBe("p");
    await Promise.resolve();
    expect(als.getStore()).toBe("p");
    await new Promise<void>((resolve) => {
      queueMicrotask(() => {
        expect(als.getStore()).toBe("p");
        resolve();
      });
    });
    expect(als.getStore()).toBe("p");
  });
  expect(als.getStore()).toBeUndefined();
});

it("propagates across setTimeout and setImmediate", async () => {
  const als = new AsyncLocalStorage<string>();
  await als.run("t", async () => {
    const viaTimeout = await new Promise<string | undefined>((resolve) => {
      setTimeout(() => resolve(als.getStore()), 5);
    });
    expect(viaTimeout).toBe("t");
    const viaImmediate = await new Promise<string | undefined>((resolve) => {
      setImmediate(() => resolve(als.getStore()));
    });
    expect(viaImmediate).toBe("t");
  });
});

it("propagates across process.nextTick", async () => {
  const als = new AsyncLocalStorage<string>();
  await als.run("n", async () => {
    const v = await new Promise<string | undefined>((resolve) => {
      process.nextTick(() => resolve(als.getStore()));
    });
    expect(v).toBe("n");
  });
});

it("instances are independent", () => {
  const a = new AsyncLocalStorage<string>();
  const b = new AsyncLocalStorage<number>();
  a.run("A", () => {
    b.run(42, () => {
      expect(a.getStore()).toBe("A");
      expect(b.getStore()).toBe(42);
    });
    expect(b.getStore()).toBeUndefined();
    expect(a.getStore()).toBe("A");
  });
});

it("Promise.all concurrent contexts stay isolated", async () => {
  const als = new AsyncLocalStorage<number>();
  const results = await Promise.all(
    [1, 2, 3, 4, 5].map((id) =>
      als.run(id, async () => {
        await Promise.resolve();
        await new Promise((r) => setTimeout(r, 5 + id));
        await Promise.resolve();
        return als.getStore();
      })
    )
  );
  expect(results).toEqual([1, 2, 3, 4, 5]);
});

it("static bind preserves this and restores captured store", () => {
  const als = new AsyncLocalStorage<string>();
  let bound: (this: { x: number }) => string | undefined;
  als.run("snap", () => {
    bound = AsyncLocalStorage.bind(function (this: { x: number }) {
      expect(this.x).toBe(1);
      return als.getStore();
    });
  });
  expect(als.getStore()).toBeUndefined();
  expect(bound!.call({ x: 1 })).toBe("snap");
});

it("snapshot restores captured store and clean snapshot blocks later outer stores", () => {
  const als = new AsyncLocalStorage<string>();

  let snapWithValue: ReturnType<typeof AsyncLocalStorage.snapshot>;
  als.run("captured", () => {
    snapWithValue = AsyncLocalStorage.snapshot();
  });
  als.enterWith("outer-later");
  expect(snapWithValue!(() => als.getStore())).toBe("captured");
  expect(als.getStore()).toBe("outer-later");
  als.disable();

  // Snapshot taken while no active ALS instances have a current store.
  // An instance created later with a store must be cleared inside the wrapper.
  const snapClean = AsyncLocalStorage.snapshot();
  const later = new AsyncLocalStorage<string>();
  later.enterWith("should-not-leak");
  expect(snapClean(() => later.getStore())).toBeUndefined();
  expect(later.getStore()).toBe("should-not-leak");
});

it("disable clears and allows re-enable via run", () => {
  const als = new AsyncLocalStorage<string>();
  als.run("a", () => {
    als.disable();
    expect(als.getStore()).toBeUndefined();
  });
  als.run("b", () => {
    expect(als.getStore()).toBe("b");
  });
});

it("works without relying on user createHook events", async () => {
  // Unit suite sets LEXE_ASYNC_HOOKS=1, so also verify ALS while a
  // disabled user hook is present does not break propagation.
  const hook = createHook({
    init() {},
  });
  // leave disabled
  const als = new AsyncLocalStorage<string>();
  await als.run("ok", async () => {
    await Promise.resolve();
    expect(als.getStore()).toBe("ok");
  });
  hook.disable();
  expect(executionAsyncId()).toBeGreaterThanOrEqual(1);
});
