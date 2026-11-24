import defaultImport, { setImmediate as namedSetImmediate } from "node:timers";
import legacyImport from "timers";
import timersPromises from "node:timers/promises";
import { promisify } from "node:util";

it("node:timers should be the same as timers and expose promises", () => {
  expect(defaultImport).toStrictEqual(legacyImport);
  expect(defaultImport.promises).toBe(timersPromises);
  expect(legacyImport.promises).toBe(timersPromises);
});

const { setTimeout } = defaultImport;

it("should set timeout", async () => {
  const start = Date.now();
  await new Promise((resolve) => {
    setTimeout(resolve, 10);
  });
  const end = Date.now();
  expect(end - start >= 10).toBeTruthy();
});

it("should set nested timeout", (done) => {
  setTimeout(() => {
    setTimeout(done, 10);
  }, 10);
});

it("should clear timeout", async () => {
  const start = Date.now();
  let status = "";
  await new Promise<void>((resolve) => {
    const timeout = setTimeout(() => {
      status = "not-cleared";
      resolve();
    }, 5);

    setTimeout(() => {
      status = "cleared";
      resolve();
    }, 10);

    clearTimeout(timeout);
  });
  const end = Date.now();

  expect(end - start >= 10).toBeTruthy();
  expect(status).toEqual("cleared");
});

it("should set interval", async () => {
  const start = Date.now();
  let count = 1;
  await new Promise<void>((resolve) => {
    const interval = setInterval(() => {
      if (count > 4) {
        clearInterval(interval);
        return resolve();
      }
      count++;
    }, 5);
  });
  const end = Date.now();
  expect(end - start >= 10).toBeTruthy();
  expect(count).toEqual(5);
});

it("should clear interval", async () => {
  const start = Date.now();
  let count = 1;
  await new Promise<void>((resolve) => {
    let interval = setInterval(() => {
      if (count == 2) {
        clearInterval(interval);
        return;
      }
      count++;
    }, 5);
    setTimeout(resolve, 20);
  });
  const end = Date.now();
  expect(end - start > 10).toBeTruthy();
  expect(count).toEqual(2);
});

it("should accept any parameter to clear timeout", () => {
  expect(() => {
    clearTimeout(null as any);
    clearTimeout("" as any);
    clearTimeout(true as any);
    clearTimeout({});
  }).not.toThrow();
});

it("should import timers", () => {
  expect(setTimeout).toStrictEqual(globalThis.setTimeout);
});

it("delay is optional", async () => {
  const start = Date.now();
  await new Promise((resolve) => {
    setTimeout(resolve);
  });
  const end = Date.now();
  expect(end - start >= 0).toBeTruthy();
});

it("delay can be negative.", async () => {
  const start = Date.now();
  await new Promise((resolve) => {
    setTimeout(resolve, -1);
  });
  const end = Date.now();
  expect(end - start >= 0).toBeTruthy();
});

it("Timeout exposes hasRef(), ref(), unref(), close(), refresh(), Symbol.dispose, and Symbol.toPrimitive", async () => {
  const t = setTimeout(() => {}, 100);
  expect(typeof t.hasRef).toBe("function");
  expect(t.hasRef()).toBe(true);
  expect(t.unref()).toBe(t);
  expect(t.hasRef()).toBe(false);
  expect(t.ref()).toBe(t);
  expect(t.hasRef()).toBe(true);
  expect(typeof +t).toBe("number");
  expect(t.close()).toBe(t);
  expect(t.refresh()).toBe(t); // no-op after close

  let runs = 0;
  let selfThis: any = null;
  let refCount = 0;
  const onceThenRefresh = setTimeout(function (this: any) {
    selfThis = this;
    runs++;
    if (refCount === 0) {
      refCount++;
      this.refresh();
    }
  }, 10);
  await new Promise((r) => setTimeout(r, 60));
  expect(runs).toBe(2);
  expect(selfThis).toBe(onceThenRefresh);

  let disposedRuns = 0;
  const tDisp = setTimeout(() => {
    disposedRuns++;
  }, 10);
  tDisp[Symbol.dispose]();
  tDisp.refresh();
  await new Promise((r) => setTimeout(r, 30));
  expect(disposedRuns).toBe(0);
});

it("Immediate exposes ref(), unref(), hasRef(), and Symbol.dispose without refresh()", () => {
  const im = namedSetImmediate(() => {});
  expect(typeof im.hasRef).toBe("function");
  expect(im.hasRef()).toBe(true);
  expect(im.unref()).toBe(im);
  expect(im.hasRef()).toBe(false);
  expect(im.ref()).toBe(im);
  expect(im.hasRef()).toBe(true);
  expect(typeof im[Symbol.dispose]).toBe("function");
  expect((im as any).refresh).toBeUndefined();
  im[Symbol.dispose]();
});

it("util.promisify(setTimeout) resolves with value and supports options", async () => {
  const pTimeout = promisify(setTimeout);
  await expect(pTimeout(10, "ok")).resolves.toBe("ok");

  const ac = new AbortController();
  const aborted = pTimeout(5000, "nope", { signal: ac.signal });
  setTimeout(() => ac.abort("stop"), 5);
  await expect(aborted).rejects.toBe("stop");
});
