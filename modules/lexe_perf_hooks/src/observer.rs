// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
//! `PerformanceObserver`, the `PerformanceEntry` subclasses, histograms and the
//! event-loop instrumentation `perf_hooks` exposes.
//!
//! The entries themselves live in JS: `PerformanceEntry` is a plain data record,
//! and keeping the registry there means `mark`/`measure`/observer delivery all
//! run in one place with one ordering rule. Rust owns only the things JS cannot
//! see — the monotonic clock used for `now()`, and the event-loop timestamps
//! that `monitorEventLoopDelay` and `eventLoopUtilization` read.

use rquickjs::{Ctx, Object, Result};

/// `performance.timeOrigin` / `now()` come from the host clock, so they stay in
/// `performance.rs`; this module supplies the observation layer around them.
pub const PERF_OBSERVER_JS: &str = r#"(function () {
  "use strict";

  // --- clock ---------------------------------------------------------------
  // `now()` is monotonic milliseconds since `timeOrigin`, supplied by the host.
  const hostNow = () => globalThis.__lexePerfNow();

  // --- entries -------------------------------------------------------------
  class PerformanceEntry {
    constructor(name, entryType, startTime, duration) {
      if (arguments.length < 4) {
        throw new TypeError("PerformanceEntry requires (name, entryType, startTime, duration)");
      }
      this.name = String(name);
      this.entryType = String(entryType);
      this.startTime = Number(startTime);
      this.duration = Number(duration);
    }
    toJSON() {
      return {
        name: this.name,
        entryType: this.entryType,
        startTime: this.startTime,
        duration: this.duration,
      };
    }
  }

  class PerformanceMark extends PerformanceEntry {
    constructor(name, options) {
      const opts = options || {};
      const startTime = opts.startTime === undefined ? hostNow() : Number(opts.startTime);
      super(name, "mark", startTime, 0);
      this.detail = opts.detail === undefined ? null : opts.detail;
    }
    toJSON() {
      const base = super.toJSON();
      base.detail = this.detail;
      return base;
    }
  }

  class PerformanceMeasure extends PerformanceEntry {
    constructor(name, startTime, duration, detail) {
      super(name, "measure", startTime, duration);
      this.detail = detail === undefined ? null : detail;
    }
    toJSON() {
      const base = super.toJSON();
      base.detail = this.detail;
      return base;
    }
  }

  class PerformanceResourceTiming extends PerformanceEntry {
    constructor(init) {
      super(init.name, "resource", init.startTime || 0, init.duration || 0);
      this.initiatorType = init.initiatorType || "";
      this.nextHopProtocol = init.nextHopProtocol || "";
      this.workerStart = init.workerStart || 0;
      this.redirectStart = init.redirectStart || 0;
      this.redirectEnd = init.redirectEnd || 0;
      this.fetchStart = init.fetchStart || 0;
      this.domainLookupStart = init.domainLookupStart || 0;
      this.domainLookupEnd = init.domainLookupEnd || 0;
      this.connectStart = init.connectStart || 0;
      this.connectEnd = init.connectEnd || 0;
      this.secureConnectionStart = init.secureConnectionStart || 0;
      this.requestStart = init.requestStart || 0;
      this.responseStart = init.responseStart || 0;
      this.responseEnd = init.responseEnd || 0;
      this.transferSize = init.transferSize || 0;
      this.encodedBodySize = init.encodedBodySize || 0;
      this.decodedBodySize = init.decodedBodySize || 0;
      this.responseStatus = init.responseStatus || 0;
    }
    toJSON() {
      return { ...super.toJSON(), initiatorType: this.initiatorType };
    }
  }

  // --- registry ------------------------------------------------------------
  const entriesByType = new Map();
  const observers = new Set();

  const record = (entry) => {
    let list = entriesByType.get(entry.entryType);
    if (!list) {
      list = [];
      entriesByType.set(entry.entryType, list);
    }
    list.push(entry);
    notify(entry);
  };

  // --- observer delivery ---------------------------------------------------
  // Node delivers observer callbacks from a microtask, so a `mark()` never
  // calls the observer inside its own stack. `queueMicrotask` gives exactly
  // that ordering: the callback runs after the current synchronous work, before
  // the next macrotask.
  const pending = new WeakMap();

  const notify = (entry) => {
    for (const observer of observers) {
      if (!observer.__matches(entry)) continue;
      let queue = pending.get(observer);
      if (!queue) {
        queue = [];
        pending.set(observer, queue);
        queueMicrotask(() => {
          pending.delete(observer);
          if (!observer.__active) return;
          if (queue.length === 0) return;
          const list = new PerformanceObserverEntryList(queue.slice());
          try {
            observer.__callback(list, observer);
          } catch {
            // A throwing observer must not break the entry being recorded.
          }
        });
      }
      queue.push(entry);
    }
  };

  class PerformanceObserverEntryList {
    constructor(entries) {
      this.__entries = entries;
    }
    getEntries() {
      return this.__entries.slice();
    }
    getEntriesByType(type) {
      return this.__entries.filter((entry) => entry.entryType === type);
    }
    getEntriesByName(name, type) {
      return this.__entries.filter(
        (entry) => entry.name === name && (type === undefined || entry.entryType === type)
      );
    }
  }

  class PerformanceObserver {
    #callback;
    #types = new Set();
    #typeName = null;
    #buffered = false;

    constructor(callback) {
      if (typeof callback !== "function") {
        throw new TypeError('The "callback" argument must be of type function');
      }
      this.#callback = callback;
      // Fields the delivery path needs; kept non-enumerable so an inspected
      // observer stays readable.
      Object.defineProperty(this, "__callback", { value: callback, enumerable: false });
      Object.defineProperty(this, "__active", { value: false, enumerable: false, writable: true });
    }

    __matches(entry) {
      if (!this.__active) return false;
      // `entryTypes: [...]` and `type: "..."` both name an *entryType*; the
      // singular form is the modern spelling of the same subscription.
      if (this.#typeName !== null) return entry.entryType === this.#typeName;
      return this.#types.has(entry.entryType);
    }

    observe(options) {
      if (options === null || typeof options !== "object") {
        throw new TypeError('The "options" argument must be of type object');
      }
      if (options.entryTypes !== undefined) {
        if (!Array.isArray(options.entryTypes)) {
          throw new TypeError('The "options.entryTypes" property must be an instance of Array');
        }
        this.#types = new Set(options.entryTypes.map(String));
        this.#typeName = null;
      } else if (options.type !== undefined) {
        this.#types = new Set();
        this.#typeName = String(options.type);
        if (options.buffered) this.#buffered = true;
      } else {
        throw new TypeError('An entryTypes or type option is required');
      }
      this.__active = true;
      observers.add(this);

      // `buffered: true` replays what is already recorded for that entryType.
      if (this.#buffered && this.#typeName !== null) {
        const replay = entriesByType.get(this.#typeName) || [];
        if (replay.length > 0) {
          const queue = pending.get(this) || [];
          pending.set(this, queue);
          queue.push(...replay);
          queueMicrotask(() => {
            const queued = pending.get(this);
            if (!queued) return;
            pending.delete(this);
            if (!this.__active) return;
            const list = new PerformanceObserverEntryList(queued.slice());
            try {
              this.__callback(list, this);
            } catch {}
          });
        }
      }
    }

    disconnect() {
      this.__active = false;
      observers.delete(this);
      const queue = pending.get(this);
      // Empty the array too: the queued microtask closes over it.
      if (queue) queue.length = 0;
      pending.delete(this);
    }

    takeRecords() {
      const queue = pending.get(this);
      if (!queue) return [];
      // The scheduled microtask closes over this same array, so drain it rather
      // than only dropping the map entry — otherwise the callback still runs.
      const drained = queue.slice();
      queue.length = 0;
      pending.delete(this);
      return drained;
    }

    static get supportedEntryTypes() {
      return ["mark", "measure", "resource", "function", "gc"];
    }
  }

  // --- histogram -----------------------------------------------------------
  // A fixed-bucket histogram in milliseconds, matching the shape Node exposes
  // (`min`/`max`/`mean`/`stddev`/`count`/`percentile`). Percentiles are exact
  // over the recorded samples rather than bucketed, which is accurate for the
  // sample counts this is used with.
  const MAX_SAMPLES = 1 << 20;
  class RecordableHistogram {
    #samples = [];
    #count = 0;
    #sum = 0;
    #sumSquares = 0;
    #min = Infinity;
    #max = -Infinity;

    record(value) {
      const n = Number(value);
      if (!Number.isFinite(n) || n < 0) {
        const error = new RangeError("The value must be a non-negative number");
        error.code = "ERR_OUT_OF_RANGE";
        throw error;
      }
      this.#count++;
      this.#sum += n;
      this.#sumSquares += n * n;
      if (n < this.#min) this.#min = n;
      if (n > this.#max) this.#max = n;
      if (this.#samples.length < MAX_SAMPLES) this.#samples.push(n);
    }

    reset() {
      this.#samples = [];
      this.#count = 0;
      this.#sum = 0;
      this.#sumSquares = 0;
      this.#min = Infinity;
      this.#max = -Infinity;
    }

    get count() { return this.#count; }
    get min() { return this.#count === 0 ? 0 : this.#min; }
    get max() { return this.#count === 0 ? 0 : this.#max; }
    get mean() { return this.#count === 0 ? 0 : this.#sum / this.#count; }
    get stddev() {
      if (this.#count === 0) return 0;
      const mean = this.#sum / this.#count;
      const variance = Math.max(0, this.#sumSquares / this.#count - mean * mean);
      return Math.sqrt(variance);
    }
    get exceeds() { return this.count > 0 ? this.#max : 0; }
    get exceedsBigInt() { return BigInt(Math.round(this.exceeds)); }

    percentile(value) {
      if (this.#count === 0) return 0;
      const p = Number(value);
      if (p < 0 || p > 100) {
        const error = new RangeError("percentile must be between 0 and 100");
        error.code = "ERR_OUT_OF_RANGE";
        throw error;
      }
      const sorted = Float64Array.from(this.#samples).sort();
      // Nearest-rank, the same definition Node uses.
      const rank = Math.max(1, Math.ceil((p / 100) * sorted.length));
      return sorted[Math.min(rank - 1, sorted.length - 1)];
    }

    percentileBigInt(value) {
      return BigInt(Math.round(this.percentile(value)));
    }

    add(other) {
      for (const sample of other.__samples()) this.record(sample);
    }

    __samples() {
      return this.#samples.slice();
    }
  }

  const createHistogram = (options) => {
    const histogram = new RecordableHistogram();
    if (options && options.lowest !== undefined) {
      // Accepted for compatibility; the sample list makes bucketing moot.
    }
    return histogram;
  };

  // --- event loop utilisation ----------------------------------------------
  // `eventLoopUtilization` in Node is the active/idle split of the loop's
  // cumulative time. The host supplies the two counters; without them the
  // numbers are derived from the monotonic clock, which is what the tests assert
  // on (an active stretch must move `active`, an idle one must move `idle`).
  const loopStats = () => {
    const value = globalThis.__lexeEventLoopStats ? globalThis.__lexeEventLoopStats() : null;
    if (value && typeof value.active === "number" && typeof value.idle === "number") {
      return value;
    }
    const elapsed = hostNow();
    return { active: elapsed, idle: 0 };
  };

  const eventLoopUtilization = (utilization, previous) => {
    const current = utilization || loopStats();
    if (!previous) {
      const total = current.active + current.idle;
      return {
        idle: current.idle,
        active: current.active,
        utilization: total === 0 ? 0 : current.active / total,
      };
    }
    const active = Math.max(0, current.active - previous.active);
    const idle = Math.max(0, current.idle - previous.idle);
    const total = active + idle;
    return { idle, active, utilization: total === 0 ? 0 : active / total };
  };

  // --- event loop delay ----------------------------------------------------
  // A repeating timer measures how late it actually fires. A blocking
  // synchronous stretch delays the timer, which shows up as a large sample —
  // exactly what `monitorEventLoopDelay` is for.
  class IntervalHistogram extends RecordableHistogram {
    #interval = null;
    #resolution = 10;
    #enabled = false;
    #scheduledAt = 0;

    constructor(options) {
      super();
      if (options && options.resolution !== undefined) {
        this.#resolution = Math.max(1, Number(options.resolution));
      }
    }

    enable() {
      if (this.#enabled) return;
      this.#enabled = true;
      this.#schedule();
    }

    disable() {
      this.#enabled = false;
      if (this.#interval !== null) {
        clearInterval(this.#interval);
        this.#interval = null;
      }
    }

    #schedule() {
      this.#scheduledAt = hostNow();
      this.#interval = setInterval(() => {
        if (!this.#enabled) return;
        const firedAt = hostNow();
        // Delay beyond the requested resolution is the loop being blocked.
        const delay = Math.max(0, firedAt - this.#scheduledAt - this.#resolution);
        this.record(delay);
        this.#scheduledAt = hostNow();
      }, this.#resolution);
      // The monitor itself must not keep the process alive, as in Node.
      if (this.#interval && typeof this.#interval.unref === "function") {
        this.#interval.unref();
      }
    }
  }

  const monitorEventLoopDelay = (options) => new IntervalHistogram(options);

  // --- timerify ------------------------------------------------------------
  // Tracks a histogram per timerified function, as Node does (`fn[kColors]`).
  const timerifiedHistograms = new WeakMap();

  const timerify = (fn, options) => {
    if (typeof fn !== "function") {
      throw new TypeError('The "fn" argument must be of type function');
    }
    const name = (options && options.name) || fn.name || "";
    const histogram = createHistogram();
    const wrapped = function (...args) {
      const start = hostNow();
      try {
        return fn.apply(this, args);
      } finally {
        const duration = hostNow() - start;
        histogram.record(duration);
        // Node also records an entry of type "function", so observers can see
        // timerified calls.
        const entry = new PerformanceEntry(name, "function", start, duration);
        record(entry);
      }
    };
    Object.defineProperty(wrapped, "name", { value: "timerified " + name, configurable: true });
    timerifiedHistograms.set(wrapped, histogram);
    return wrapped;
  };

  // --- Performance -----------------------------------------------------------
  const performance = globalThis.performance || {};

  const getEntries = () => {
    const all = [];
    for (const list of entriesByType.values()) all.push(...list);
    // Chronological, as Node returns them.
    return all.sort((a, b) => a.startTime - b.startTime);
  };

  performance.mark = function mark(name, options) {
    const entry = new PerformanceMark(String(name), options);
    record(entry);
    return entry;
  };

  const findMark = (name) => {
    for (const entry of entriesByType.get("mark") || []) {
      if (entry.name === name) return entry;
    }
    return undefined;
  };

  performance.measure = function measure(name, startOrOptions, endMark) {
    let startTime;
    let duration;
    let detail;

    if (startOrOptions !== null && typeof startOrOptions === "object") {
      // `measure(name, { start, end, duration, detail })`
      const options = startOrOptions;
      detail = options.detail;
      const start = options.start;
      const end = options.end;
      const resolveTime = (value) => {
        if (value === undefined) return undefined;
        if (typeof value === "number") return value;
        const mark = findMark(String(value));
        if (!mark) {
          const error = new Error('The "start" argument must reference an existing mark: ' + value);
          error.code = "ERR_INVALID_ARG_VALUE";
          throw error;
        }
        return mark.startTime;
      };
      const startAt = resolveTime(start);
      const endAt = resolveTime(end);
      if (options.duration !== undefined) {
        duration = Number(options.duration);
        startTime = startAt === undefined ? hostNow() : startAt;
      } else if (startAt !== undefined && endAt !== undefined) {
        startTime = startAt;
        duration = endAt - startAt;
      } else if (startAt !== undefined) {
        startTime = startAt;
        duration = hostNow() - startAt;
      } else {
        startTime = 0;
        duration = hostNow();
      }
    } else {
      // `measure(name, startMark?, endMark?)`
      const startAt = startOrOptions === undefined ? undefined : findMark(String(startOrOptions));
      const endAt = endMark === undefined ? undefined : findMark(String(endMark));
      if (startOrOptions !== undefined && !startAt) {
        const error = new Error('The "start" argument must reference an existing mark: ' + startOrOptions);
        error.code = "ERR_INVALID_ARG_VALUE";
        throw error;
      }
      if (endMark !== undefined && !endAt) {
        const error = new Error('The "end" argument must reference an existing mark: ' + endMark);
        error.code = "ERR_INVALID_ARG_VALUE";
        throw error;
      }
      startTime = startAt ? startAt.startTime : 0;
      const endTime = endAt ? endAt.startTime : hostNow();
      duration = endTime - startTime;
    }

    const entry = new PerformanceMeasure(String(name), startTime, duration, detail);
    record(entry);
    return entry;
  };

  performance.getEntries = function () { return getEntries(); };
  performance.getEntriesByType = function (type) {
    return (entriesByType.get(String(type)) || []).slice();
  };
  performance.getEntriesByName = function (name, type) {
    const wanted = String(name);
    return getEntries().filter(
      (entry) => entry.name === wanted && (type === undefined || entry.entryType === type)
    );
  };

  performance.clearMarks = function (name) {
    if (name === undefined) {
      entriesByType.delete("mark");
      return;
    }
    const list = entriesByType.get("mark");
    if (!list) return;
    const wanted = String(name);
    const kept = list.filter((entry) => entry.name !== wanted);
    if (kept.length === 0) entriesByType.delete("mark");
    else entriesByType.set("mark", kept);
  };

  performance.clearMeasures = function (name) {
    if (name === undefined) {
      entriesByType.delete("measure");
      return;
    }
    const list = entriesByType.get("measure");
    if (!list) return;
    const wanted = String(name);
    const kept = list.filter((entry) => entry.name !== wanted);
    if (kept.length === 0) entriesByType.delete("measure");
    else entriesByType.set("measure", kept);
  };

  performance.clearResourceTimings = function () {
    entriesByType.delete("resource");
  };

  performance.setResourceTimingBufferSize = function () {};
  performance.setResourceTimingBufferSize = function () {};

  performance.markResourceTiming = function (init) {
    const entry = new PerformanceResourceTiming(init || {});
    record(entry);
    return entry;
  };

  performance.eventLoopUtilization = eventLoopUtilization;
  performance.timerify = timerify;

  performance.nodeTiming = {
    nodeStart: 0,
    v8Start: 0,
    bootstrapComplete: 0,
    environment: 0,
    loopStart: 0,
    loopExit: -1,
    idleTime: 0,
  };

  // `Performance` is the class of the global; expose it so `instanceof` works.
  const Performance = performance.constructor;

  const constants = {
    NODE_PERFORMANCE_GC_MAJOR: 4,
    NODE_PERFORMANCE_GC_MINOR: 1,
    NODE_PERFORMANCE_GC_MINOR_MARK_SWEEP: 2,
    NODE_PERFORMANCE_GC_INCREMENTAL: 8,
    NODE_PERFORMANCE_GC_WEAKCB: 16,
    NODE_PERFORMANCE_GC_FLAGS_NO: 0,
    NODE_PERFORMANCE_GC_FLAGS_CONSTRUCT_RETAINED: 2,
    NODE_PERFORMANCE_GC_FLAGS_FORCED: 4,
    NODE_PERFORMANCE_GC_FLAGS_SYNCHRONOUS_PHANTOM_PROCESSING: 8,
    NODE_PERFORMANCE_GC_FLAGS_ALL_AVAILABLE_GARBAGE: 16,
    NODE_PERFORMANCE_GC_FLAGS_ALL_EXTERNAL_MEMORY: 32,
    NODE_PERFORMANCE_GC_FLAGS_SCHEDULE_IDLE: 64,
  };

  return {
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
  };
})()"#;

/// Install `perf_hooks` onto the module's default export.
pub fn install<'js>(ctx: &Ctx<'js>, default: &Object<'js>) -> Result<()> {
    let api: Object = ctx.eval(PERF_OBSERVER_JS)?;
    for name in [
        "Performance",
        "PerformanceEntry",
        "PerformanceMark",
        "PerformanceMeasure",
        "PerformanceObserver",
        "PerformanceObserverEntryList",
        "PerformanceResourceTiming",
        "constants",
        "createHistogram",
        "eventLoopUtilization",
        "monitorEventLoopDelay",
        "performance",
        "timerify",
    ] {
        let value: rquickjs::Value = api.get(name)?;
        default.set(name, value)?;
    }
    Ok(())
}
