import {
  PerformanceObserver,
  PerformanceEntry,
  PerformanceMark,
  PerformanceMeasure,
  PerformanceResourceTiming,
  createHistogram,
  eventLoopUtilization,
  monitorEventLoopDelay,
  timerify,
  constants,
  performance,
} from "node:perf_hooks";

// The observation layer: entries, observers, histograms and event-loop
// instrumentation. `timeOrigin`/`now` are covered elsewhere and untouched.

const tick = () => new Promise((resolve) => setTimeout(resolve, 5));

it("performance.timeOrigin and now are still there", () => {
  expect(typeof performance.timeOrigin).toBe("number");
  expect(typeof performance.now()).toBe("number");
  expect(performance.now()).toBeGreaterThanOrEqual(0);
});

// --- entries -------------------------------------------------------------

it("mark records a PerformanceMark", () => {
  performance.clearMarks();
  const mark = performance.mark("a-mark");
  expect(mark instanceof PerformanceMark).toBe(true);
  expect(mark instanceof PerformanceEntry).toBe(true);
  expect(mark.name).toBe("a-mark");
  expect(mark.entryType).toBe("mark");
  expect(typeof mark.startTime).toBe("number");
  expect(mark.duration).toBe(0);
  expect(typeof mark.toJSON).toBe("function");
});

it("measure spans two marks", () => {
  performance.clearMarks();
  performance.clearMeasures();
  performance.mark("start");
  performance.mark("end");
  const measure = performance.measure("span", "start", "end");
  expect(measure instanceof PerformanceMeasure).toBe(true);
  expect(measure.entryType).toBe("measure");
  expect(measure.duration).toBeGreaterThanOrEqual(0);
  // It starts where the start mark is.
  expect(measure.startTime).toBe(performance.getEntriesByName("start")[0].startTime);
});

it("measure rejects an unknown mark", () => {
  expect(() => performance.measure("bad", "nope", undefined)).toThrow();

  performance.mark("only-start");
  expect(() => performance.measure("bad2", "only-start", "missing")).toThrow();
});

it("getEntries / byName / byType", () => {
  performance.clearMarks();
  performance.clearMeasures();
  performance.mark("one");
  performance.mark("two");
  performance.measure("m", "one", "two");

  expect(performance.getEntriesByType("mark").map((e: any) => e.name)).toEqual(["one", "two"]);
  expect(performance.getEntriesByType("measure").map((e: any) => e.name)).toEqual(["m"]);
  expect(performance.getEntriesByName("one")).toHaveLength(1);
  expect(performance.getEntriesByName("m", "measure")).toHaveLength(1);
  expect(performance.getEntriesByName("m", "mark")).toHaveLength(0);
  expect(performance.getEntries().length).toBe(3);
});

it("clearMarks and clearMeasures", () => {
  performance.clearMarks();
  performance.clearMeasures();
  performance.mark("x");
  performance.mark("y");
  performance.measure("m", "x", "y");

  performance.clearMarks("x");
  expect(performance.getEntriesByType("mark").map((e: any) => e.name)).toEqual(["y"]);

  performance.clearMarks();
  expect(performance.getEntriesByType("mark")).toHaveLength(0);
  expect(performance.getEntriesByType("measure")).toHaveLength(1);

  performance.clearMeasures();
  expect(performance.getEntriesByType("measure")).toHaveLength(0);
});

// --- observers -----------------------------------------------------------

it("observer callbacks are delivered as a microtask, not synchronously", async () => {
  performance.clearMarks();
  const seen: string[] = [];
  const observer = new PerformanceObserver((list) => {
    for (const entry of list.getEntries()) seen.push(entry.name);
  });
  observer.observe({ entryTypes: ["mark"] });

  performance.mark("async-mark");
  // Still inside the same synchronous stack: the callback must not have run.
  expect(seen).toEqual([]);
  expect(seen.length).toBe(0);

  await tick();
  expect(seen).toEqual(["async-mark"]);
  observer.disconnect();
});

it("observer callback runs before the next macrotask but after the current stack", async () => {
  performance.clearMarks();
  const order: string[] = [];
  const observer = new PerformanceObserver(() => order.push("observer"));
  observer.observe({ entryTypes: ["mark"] });

  performance.mark("ordered");
  order.push("after-mark");
  await Promise.resolve();
  // A microtask observer has already run here.
  expect(order).toEqual(["after-mark", "observer"]);

  await tick();
  expect(order).toEqual(["after-mark", "observer"]);
  observer.disconnect();
});

it("observer subscribes by entryTypes", async () => {
  performance.clearMarks();
  performance.clearMeasures();
  const types: string[] = [];
  const observer = new PerformanceObserver((list) => {
    for (const entry of list.getEntries()) types.push(entry.entryType);
  });
  observer.observe({ entryTypes: ["mark", "measure"] });

  performance.mark("a");
  performance.measure("b");
  await tick();

  observer.disconnect();
  expect(types.sort()).toEqual(["mark", "measure"]);
});

it("observer subscribes by a single type", async () => {
  performance.clearMarks();
  performance.clearMeasures();
  const names: string[] = [];
  const observer = new PerformanceObserver((list) => {
    for (const entry of list.getEntries()) names.push(entry.name);
  });
  observer.observe({ type: "measure" });

  performance.mark("not-seen");
  performance.measure("seen");
  await tick();

  observer.disconnect();
  expect(names).toEqual(["seen"]);
});

it("observe with buffered replays existing entries", async () => {
  performance.clearMeasures();
  performance.mark("s1");
  performance.mark("s2");
  performance.measure("already-there", "s1", "s2");

  const names: string[] = [];
  const observer = new PerformanceObserver((list) => {
    for (const entry of list.getEntries()) names.push(entry.name);
  });
  observer.observe({ type: "measure", buffered: true });
  await tick();
  observer.disconnect();

  expect(names).toEqual(["already-there"]);
});

it("takeRecords drains the queue without invoking the callback", async () => {
  performance.clearMarks();
  const seen: string[] = [];
  const observer = new PerformanceObserver((list) => {
    for (const entry of list.getEntries()) seen.push(entry.name);
  });
  observer.observe({ entryTypes: ["mark"] });

  performance.mark("drained");
  const records = observer.takeRecords();
  expect(records.length).toBeGreaterThan(0);
  expect((records[0] as any).name).toBe("drained");

  await tick();
  // The callback never ran for the drained entry.
  expect(seen).toEqual([]);
  observer.disconnect();
});

it("disconnect stops delivery", async () => {
  performance.clearMarks();
  const seen: string[] = [];
  const observer = new PerformanceObserver((list) => {
    for (const entry of list.getEntries()) seen.push(entry.name);
  });
  observer.observe({ entryTypes: ["mark"] });
  observer.disconnect();

  performance.mark("after-disconnect");
  await tick();
  expect(seen).toEqual([]);
});

it("observer lists expose getEntriesByType and getEntriesByName", async () => {
  performance.clearMarks();
  performance.clearMeasures();
  const captured: any = await new Promise((resolve) => {
    const observer = new PerformanceObserver((list) => {
      observer.disconnect();
      resolve(list);
    });
    observer.observe({ entryTypes: ["mark"] });
    performance.mark("listed");
  });

  expect(typeof captured.getEntries).toBe("function");
  expect(captured.getEntriesByType("mark")).toHaveLength(1);
  expect(captured.getEntriesByName("listed")).toHaveLength(1);
});

it("PerformanceObserver rejects a non-function callback", () => {
  expect(() => new (PerformanceObserver as any)(null)).toThrow();
});

it("observe requires entryTypes or type", () => {
  const observer = new PerformanceObserver(() => {});
  expect(() => observer.observe({})).toThrow();
  observer.disconnect();
});

it("supportedEntryTypes lists the implemented types", () => {
  const types = (PerformanceObserver as any).supportedEntryTypes;
  expect(Array.isArray(types)).toBe(true);
  expect(types).toContain("mark");
  expect(types).toContain("measure");
  expect(types).toContain("resource");
});

// --- resource entries ----------------------------------------------------

it("resource entries can be recorded and observed", async () => {
  performance.clearResourceTimings();
  const seen: any[] = [];
  const observer = new PerformanceObserver((list) => {
    for (const entry of list.getEntries()) seen.push(entry);
  });
  observer.observe({ entryTypes: ["resource"] });

  performance.markResourceTiming({
    name: "https://example.com/x.js",
    initiatorType: "script",
    startTime: performance.now(),
    duration: 3,
  });
  await tick();
  observer.disconnect();

  expect(seen).toHaveLength(1);
  expect(seen[0] instanceof PerformanceResourceTiming).toBe(true);
  expect(seen[0].entryType).toBe("resource");
  expect(seen[0].initiatorType).toBe("script");
});

// --- histogram -----------------------------------------------------------

it("createHistogram computes min, max, mean and count", () => {
  const histogram = createHistogram();
  histogram.record(10);
  histogram.record(20);
  histogram.record(30);

  expect(histogram.count).toBe(3);
  expect(histogram.min).toBe(10);
  expect(histogram.max).toBe(30);
  expect(histogram.mean).toBe(20);
  expect(histogram.stddev).toBeGreaterThan(0);
  expect(histogram.percentile(50)).toBeGreaterThanOrEqual(10);
  expect(histogram.percentile(50)).toBeLessThanOrEqual(30);
});

it("histogram rejects a negative sample and resets", () => {
  const histogram = createHistogram();
  expect(() => histogram.record(-1)).toThrow();

  histogram.record(5);
  expect(histogram.count).toBe(1);
  histogram.reset();
  expect(histogram.count).toBe(0);
  expect(histogram.max).toBe(0);
});

it("histogram percentiles are ordered", () => {
  const histogram = createHistogram();
  for (let i = 1; i <= 100; i++) histogram.record(i);

  expect(() => histogram.percentile(0)).toThrow();
  expect(histogram.percentile(1)).toBe(1);
  expect(histogram.percentile(50)).toBeLessThan(histogram.percentile(90));
  expect(histogram.percentile(100)).toBe(100);
  expect(() => histogram.percentile(101)).toThrow();
});

// --- event loop delay ----------------------------------------------------

it("monitorEventLoopDelay observes a blocking stretch", async () => {
  const delay = monitorEventLoopDelay({ resolution: 5 });
  delay.enable();

  // Block the loop well past the sampling resolution; the monitor must record
  // a delay at least that large.
  const until = Date.now() + 120;
  while (Date.now() < until) {
    // spin
  }
  await tick();
  await tick();
  delay.disable();

  expect(delay.count).toBeGreaterThan(0);
  expect(delay.max).toBeGreaterThan(50);
});

it("monitorEventLoopDelay disable stops sampling", async () => {
  const delay = monitorEventLoopDelay({ resolution: 5 });
  delay.enable();
  await tick();
  delay.disable();
  const afterDisable = delay.count;

  const until = Date.now() + 60;
  while (Date.now() < until) {}
  await tick();

  expect(delay.count).toBe(afterDisable);
});

// --- timerify ------------------------------------------------------------

it("timerify preserves the return value and records a duration", () => {
  const add = timerify((a: number, b: number) => a + b);
  expect(add(2, 3)).toBe(5);
  expect(add.name.startsWith("timerified")).toBe(true);
});

it("timerify works with a function that throws", () => {
  const boom = timerify(() => {
    throw new Error("boom");
  });
  expect(() => boom()).toThrow("boom");
});

it("timerify records a duration histogram visible to an observer", async () => {
  const functions: any[] = [];
  const observer = new PerformanceObserver((list) => {
    for (const entry of list.getEntries()) functions.push(entry);
  });
  observer.observe({ entryTypes: ["function"] });

  const fn = timerify((a: number) => a * 2, { name: "doubler" });
  fn(21);
  await tick();
  observer.disconnect();

  expect(functions.length).toBeGreaterThan(0);
  expect(functions[0].entryType).toBe("function");
  expect(functions[0].name).toBe("doubler");
  expect(functions[0].duration).toBeGreaterThanOrEqual(0);
});

// --- event loop utilization ----------------------------------------------

it("eventLoopUtilization reports idle, active and utilization", () => {
  const usage = eventLoopUtilization();
  expect(typeof usage.idle).toBe("number");
  expect(typeof usage.active).toBe("number");
  expect(typeof usage.utilization).toBe("number");
  expect(usage.utilization).toBeGreaterThanOrEqual(0);
  expect(usage.utilization).toBeLessThanOrEqual(1);
});

it("eventLoopUtilization reflects work between two samples", async () => {
  const before = eventLoopUtilization();

  const until = Date.now() + 60;
  while (Date.now() < until) {}
  await tick();

  const after = eventLoopUtilization();
  expect(after.active).toBeGreaterThanOrEqual(before.active);

  const delta = eventLoopUtilization(after, before);
  expect(delta.active).toBeGreaterThan(0);
  expect(delta.utilization).toBeGreaterThan(0);
  expect(delta.utilization).toBeLessThanOrEqual(1);
});

it("eventLoopUtilization delta is zero for identical samples", () => {
  const sample = eventLoopUtilization();
  const delta = eventLoopUtilization(sample, sample);
  expect(delta.active).toBe(0);
  expect(delta.idle).toBe(0);
  expect(delta.utilization).toBe(0);
});

// --- constants -----------------------------------------------------------

it("constants exposes the GC flags Node defines", () => {
  expect(constants.NODE_PERFORMANCE_GC_MAJOR).toBe(4);
  expect(constants.NODE_PERFORMANCE_GC_MINOR).toBe(1);
  expect(constants.NODE_PERFORMANCE_GC_INCREMENTAL).toBe(8);
  expect(constants.NODE_PERFORMANCE_GC_WEAKCB).toBe(16);
  expect(constants.NODE_PERFORMANCE_GC_FLAGS_NO).toBe(0);
  expect(Object.keys(constants).length).toBeGreaterThanOrEqual(12);
});
