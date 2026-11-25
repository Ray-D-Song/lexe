import http from "node:http";
import dns from "node:dns";
import net from "node:net";
import {
  Performance,
  PerformanceEntry,
  PerformanceMark,
  PerformanceMeasure,
  PerformanceObserver,
  PerformanceObserverEntryList,
  PerformanceResourceTiming,
  constants,
  createHistogram,
  eventLoopUtilization,
  monitorEventLoopDelay,
  performance,
  timerify,
} from "node:perf_hooks";

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

describe("node:perf_hooks parity", () => {
  it("exposes Web Performance globals matching node:perf_hooks exports", () => {
    expect((globalThis as any).Performance).toBe(Performance);
    expect((globalThis as any).PerformanceEntry).toBe(PerformanceEntry);
    expect((globalThis as any).PerformanceMark).toBe(PerformanceMark);
    expect((globalThis as any).PerformanceMeasure).toBe(PerformanceMeasure);
    expect((globalThis as any).PerformanceObserver).toBe(PerformanceObserver);
    expect((globalThis as any).PerformanceObserverEntryList).toBe(PerformanceObserverEntryList);
    expect((globalThis as any).PerformanceResourceTiming).toBe(PerformanceResourceTiming);
    expect(PerformanceObserver.supportedEntryTypes).toEqual(
      expect.arrayContaining([
        "dns",
        "function",
        "gc",
        "http",
        "http2",
        "mark",
        "measure",
        "net",
        "resource",
      ])
    );
  });

  it("provides live performance.nodeTiming and performance.toJSON()", () => {
    const nt: any = performance.nodeTiming;
    expect(nt instanceof PerformanceEntry).toBe(true);
    expect(nt.name).toBe("node");
    expect(nt.entryType).toBe("node");
    expect(nt.startTime).toBe(0);
    expect(nt.duration).toBeGreaterThanOrEqual(0);
    expect(nt.nodeStart).toBeGreaterThanOrEqual(0);
    expect(nt.v8Start).toBeGreaterThanOrEqual(nt.nodeStart);
    expect(nt.environment).toBeGreaterThanOrEqual(nt.v8Start);
    expect(nt.bootstrapComplete).toBeGreaterThanOrEqual(nt.environment);
    expect(nt.loopStart).toBeGreaterThanOrEqual(0);
    expect(nt.loopExit).toBe(-1);
    expect(nt.idleTime).toBeGreaterThanOrEqual(0);
    expect(typeof nt.uvMetricsInfo.loopCount).toBe("number");
    expect(typeof nt.uvMetricsInfo.events).toBe("number");

    const json = nt.toJSON();
    expect(json.name).toBe("node");
    expect(json.entryType).toBe("node");
    expect(json.bootstrapComplete).toBe(nt.bootstrapComplete);

    const perfJson = performance.toJSON();
    expect(perfJson.timeOrigin).toBe(performance.timeOrigin);
    expect(perfJson.nodeTiming).toBe(nt);
    expect(typeof perfJson.eventLoopUtilization.utilization).toBe("number");
  });

  it("supports real HdrHistogram bucketing, BigInt getters, recordDelta, add, and validation", () => {
    const empty = createHistogram();
    expect(empty.count).toBe(0);
    expect(empty.countBigInt).toBe(0n);
    expect(empty.minBigInt).toBe(9223372036854775807n);
    expect(empty.max).toBe(0);
    expect(Number.isNaN(empty.mean)).toBe(true);
    expect(Number.isNaN(empty.stddev)).toBe(true);
    expect(empty.percentiles.get(100)).toBe(0);
    expect(empty.percentilesBigInt.get(100)).toBe(0n);

    const h1 = createHistogram({ lowest: 1, highest: 1_000_000, figures: 1 });
    h1.record(50_000);
    expect(h1.max).toBe(51_199);

    const h3 = createHistogram({ lowest: 1, highest: 1_000_000, figures: 3 });
    h3.record(50_000);
    expect(h3.max).toBe(50_015);
    expect(h3.maxBigInt).toBe(50_015n);

    // Out-of-bounds increments exceeds without adding to count.
    const capped = createHistogram({ lowest: 1, highest: 1000, figures: 3 });
    capped.record(100);
    capped.record(10000);
    expect(capped.count).toBe(1);
    expect(capped.exceeds).toBe(1);
    expect(capped.exceedsBigInt).toBe(1n);

    // BigInt + recordDelta + add + toJSON
    capped.record(200n);
    expect(capped.count).toBe(2);
    expect(capped.percentile(50)).toBe(100);
    expect(capped.percentileBigInt(100)).toBe(200n);
    const json = capped.toJSON();
    expect(json.count).toBe(2);
    expect(json.exceeds).toBe(1);
    expect(json.percentiles["100"]).toBe(200);

    const deltaHist = createHistogram();
    deltaHist.recordDelta();
    const tEnd = Date.now() + 3;
    while (Date.now() < tEnd) {}
    deltaHist.recordDelta();
    expect(deltaHist.count).toBe(1);
    expect(deltaHist.max).toBeGreaterThan(0);

    const other = createHistogram({ lowest: 1, highest: 1000, figures: 3 });
    other.record(300);
    capped.add(other);
    expect(capped.count).toBe(3);
    expect(capped.max).toBe(300);

    // Validation error codes match Node.js
    const expectCode = (fn: () => unknown, code: string) => {
      let caught: any = null;
      try {
        fn();
      } catch (e) {
        caught = e;
      }
      expect(caught).not.toBeNull();
      expect(caught.code).toBe(code);
    };
    expectCode(() => createHistogram({ lowest: 0 }), "ERR_OUT_OF_RANGE");
    expectCode(() => createHistogram({ figures: 6 }), "ERR_OUT_OF_RANGE");
    expectCode(() => createHistogram({ lowest: 10, highest: 15 }), "ERR_OUT_OF_RANGE");
    expectCode(() => h3.record(1.5), "ERR_OUT_OF_RANGE");
    expectCode(() => h3.record(0), "ERR_OUT_OF_RANGE");
    expectCode(() => (h3 as any).record("10"), "ERR_INVALID_ARG_TYPE");
    expectCode(() => h3.percentile(0), "ERR_OUT_OF_RANGE");
    expectCode(() => h3.percentile(101), "ERR_OUT_OF_RANGE");
  });

  it("provides native nanosecond ELDHistogram for monitorEventLoopDelay", async () => {
    const eld = monitorEventLoopDelay({ resolution: 5 });
    expect("record" in eld).toBe(false);
    expect(typeof eld.reset).toBe("function");
    expect(eld.enable()).toBe(true);
    expect(eld.enable()).toBe(false);

    const until = Date.now() + 45;
    while (Date.now() < until) {}
    await sleep(15);

    expect(eld.disable()).toBe(true);
    expect(eld.disable()).toBe(false);
    expect(eld.count).toBeGreaterThan(0);
    // Samples are nanoseconds (well over 1ms = 1_000_000ns after 45ms spin).
    expect(eld.max).toBeGreaterThan(10_000_000);
    expect(eld.maxBigInt).toBeGreaterThan(10_000_000n);

    expect(() => monitorEventLoopDelay({ resolution: 0 })).toThrow();
    expect(() => monitorEventLoopDelay({ resolution: "5" as any })).toThrow();
  });

  it("tracks real active and idle time in eventLoopUtilization", async () => {
    const s0 = performance.eventLoopUtilization();
    const spinUntil = Date.now() + 40;
    while (Date.now() < spinUntil) {}
    const s1 = performance.eventLoopUtilization(undefined, s0);
    expect(s1.active).toBeGreaterThan(20);

    const s2 = performance.eventLoopUtilization();
    await sleep(45);
    const s3 = performance.eventLoopUtilization(undefined, s2);
    expect(s3.idle).toBeGreaterThan(20);
    expect(performance.nodeTiming.idleTime).toBeGreaterThan(20);
  });

  it("automatically observes GC events via PerformanceObserver", async () => {
    const entries: any[] = [];
    const obs = new PerformanceObserver((list) => {
      entries.push(...list.getEntries());
    });
    obs.observe({ entryTypes: ["gc"] });

    (globalThis as any).__gc();
    await sleep(10);
    obs.disconnect();

    expect(entries.length).toBeGreaterThanOrEqual(1);
    const ev = entries.find((e) => e.entryType === "gc");
    expect(ev).toBeDefined();
    expect(ev.name).toBe("gc");
    expect(ev.duration).toBeGreaterThanOrEqual(0);
    expect(ev.detail.kind).toBe(constants.NODE_PERFORMANCE_GC_MAJOR);
    expect(ev.detail.flags).toBe(constants.NODE_PERFORMANCE_GC_FLAGS_FORCED);
    // GC entries are observer-only, not retained in performance.getEntries()
    expect(performance.getEntriesByType("gc")).toHaveLength(0);
  });

  it("automatically records fetch resource timings and enforces buffer limits", async () => {
    const srv = http.createServer((req, res) => {
      res.writeHead(200, { "content-type": "text/plain" });
      res.end("hello-perf");
    });

    await new Promise<void>((resolve) => srv.listen(0, "127.0.0.1", resolve));
    const port = (srv.address() as any).port;

    performance.clearResourceTimings();
    performance.setResourceTimingBufferSize(1);

    let fullCount = 0;
    const onProp = () => {
      fullCount++;
    };
    performance.onresourcetimingbufferfull = onProp;

    const observed: any[] = [];
    const obs = new PerformanceObserver((list) => {
      observed.push(...list.getEntries());
    });
    obs.observe({ entryTypes: ["resource"] });

    const r1 = await fetch(`http://127.0.0.1:${port}/one`);
    await r1.text();
    const r2 = await fetch(`http://127.0.0.1:${port}/two`);
    const reader = r2.body!.getReader();
    while (true) {
      const { done } = await reader.read();
      if (done) break;
    }

    await sleep(20);
    obs.disconnect();
    performance.onresourcetimingbufferfull = null;
    performance.setResourceTimingBufferSize(250);
    srv.close();

    expect(observed.length).toBe(2);
    expect(observed[0] instanceof PerformanceResourceTiming).toBe(true);
    expect(observed[0].name).toBe(`http://127.0.0.1:${port}/one`);
    expect(observed[0].initiatorType).toBe("fetch");
    expect(observed[0].responseStatus).toBe(200);
    expect(observed[0].encodedBodySize).toBe(10);
    expect(observed[0].decodedBodySize).toBe(10);
    expect(observed[0].transferSize).toBeGreaterThanOrEqual(10);
    expect(observed[0].responseEnd).toBeGreaterThanOrEqual(observed[0].fetchStart);

    // Buffer limit of 1 retained only the first entry and fired bufferfull event.
    expect(performance.getEntriesByType("resource")).toHaveLength(1);
    expect(fullCount).toBe(1);

    // 7-argument Node 24 markResourceTiming signature
    const manual = (performance as any).markResourceTiming(
      {
        startTime: 1,
        endTime: 6,
        redirectStartTime: 0,
        redirectEndTime: 0,
        postRedirectStartTime: 1,
        finalServiceWorkerStartTime: 0,
        finalNetworkRequestStartTime: 2,
        finalNetworkResponseStartTime: 4,
        encodedBodySize: 12,
        decodedBodySize: 12,
      },
      "http://example.test/manual",
      "fetch",
      globalThis,
      "",
      { status: 201 },
      201
    );
    expect(manual.name).toBe("http://example.test/manual");
    expect(manual.duration).toBe(5);
    expect(manual.responseStatus).toBe(201);
  });

  it("automatically emits http, dns, and net entries when observed", async () => {
    const srv = http.createServer((_req, res) => {
      res.writeHead(200, { "x-perf": "1" });
      res.end("ok");
    });
    await new Promise<void>((resolve) => srv.listen(0, "127.0.0.1", resolve));
    const port = (srv.address() as any).port;

    const seen: any[] = [];
    const obs = new PerformanceObserver((list) => {
      seen.push(...list.getEntries());
    });
    obs.observe({ entryTypes: ["http", "dns", "net"] });

    await new Promise<void>((resolve) => {
      http.get(`http://127.0.0.1:${port}/ping`, (res) => {
        res.resume();
        res.on("end", resolve);
      });
    });

    await dns.promises.lookup("localhost");

    await new Promise<void>((resolve) => {
      const sock = net.connect(port, "127.0.0.1", () => {
        sock.end();
      });
      sock.on("close", resolve);
    });

    await sleep(20);
    obs.disconnect();
    srv.close();

    expect(seen.some((e) => e.entryType === "http" && e.name === "HttpRequest")).toBe(true);
    expect(seen.some((e) => e.entryType === "http" && e.name === "HttpClient")).toBe(true);
    expect(seen.some((e) => e.entryType === "dns" && e.name === "lookup")).toBe(true);
    expect(seen.some((e) => e.entryType === "net" && e.name === "connect")).toBe(true);
    expect(performance.getEntriesByType("http")).toHaveLength(0);
  });

  it("records nanosecond samples into timerify({ histogram }) and attaches args detail", async () => {
    const hist = createHistogram();
    const seen: any[] = [];
    const obs = new PerformanceObserver((list) => {
      seen.push(...list.getEntries());
    });
    obs.observe({ entryTypes: ["function"] });

    const fn = timerify((a: number, b: number) => a + b, { histogram: hist });
    expect(fn(3, 4)).toBe(7);

    await sleep(10);
    obs.disconnect();

    expect(hist.count).toBe(1);
    expect(hist.min).toBeGreaterThanOrEqual(1);
    expect(seen.length).toBe(1);
    expect(seen[0].detail).toEqual([3, 4]);
    expect(seen[0][0]).toBe(3);
    expect(seen[0][1]).toBe(4);
  });
});
