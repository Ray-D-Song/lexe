// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
//! `PerformanceObserver`, `PerformanceEntry` subclasses, native HdrHistogram /
//! ELDHistogram wrappers, `PerformanceNodeTiming`, and global Web Performance APIs.

use lexe_events::Emitter;
use lexe_utils::perf::{self};
use rquickjs::{prelude::Func, Class, Ctx, Function, Object, Result, Symbol, Value};

use crate::histogram::NativeHistogram;
use crate::performance::Performance;

pub const PERF_OBSERVER_JS: &str = r#"(function () {
  "use strict";

  const symNative = Symbol.for("lexe.perfNative");
  const symInternal = Symbol.for("lexe.perfInternal");
  const symExports = Symbol.for("lexe.perfExports");

  const native = globalThis[symNative];
  const hostNow = () => native.now();

  const makeErr = (Ctor, code, msg) => {
    const err = new Ctor(msg);
    err.code = code;
    return err;
  };

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
      this.deliveryType = init.deliveryType || "";
      this.responseStatus = init.responseStatus || 0;
    }
    toJSON() {
      return {
        ...super.toJSON(),
        initiatorType: this.initiatorType,
        nextHopProtocol: this.nextHopProtocol,
        workerStart: this.workerStart,
        redirectStart: this.redirectStart,
        redirectEnd: this.redirectEnd,
        fetchStart: this.fetchStart,
        domainLookupStart: this.domainLookupStart,
        domainLookupEnd: this.domainLookupEnd,
        connectStart: this.connectStart,
        connectEnd: this.connectEnd,
        secureConnectionStart: this.secureConnectionStart,
        requestStart: this.requestStart,
        responseStart: this.responseStart,
        responseEnd: this.responseEnd,
        transferSize: this.transferSize,
        encodedBodySize: this.encodedBodySize,
        decodedBodySize: this.decodedBodySize,
        deliveryType: this.deliveryType,
        responseStatus: this.responseStatus,
      };
    }
  }

  class PerformanceNodeEntry extends PerformanceEntry {
    constructor(name, entryType, startTime, duration, detail) {
      super(name, entryType, startTime, duration);
      if (detail !== undefined) {
        this.detail = detail;
      }
    }
    toJSON() {
      const base = super.toJSON();
      if ("detail" in this) {
        base.detail = this.detail;
      }
      return base;
    }
  }

  class PerformanceNodeTiming extends PerformanceEntry {
    constructor() {
      super("node", "node", 0, 0);
      const snap = () => native.nodeTimingSnapshot();
      Object.defineProperties(this, {
        duration: { enumerable: true, configurable: true, get: () => hostNow() },
        nodeStart: { enumerable: true, configurable: true, get: () => snap().nodeStart },
        v8Start: { enumerable: true, configurable: true, get: () => snap().v8Start },
        bootstrapComplete: {
          enumerable: true,
          configurable: true,
          get: () => snap().bootstrapComplete,
        },
        environment: { enumerable: true, configurable: true, get: () => snap().environment },
        loopStart: { enumerable: true, configurable: true, get: () => snap().loopStart },
        loopExit: { enumerable: true, configurable: true, get: () => snap().loopExit },
        idleTime: { enumerable: true, configurable: true, get: () => snap().idleTime },
        uvMetricsInfo: {
          enumerable: false,
          configurable: true,
          get: () => {
            const s = snap();
            return {
              loopCount: s.loopCount,
              events: s.events,
              eventsWaiting: s.eventsWaiting,
            };
          },
        },
      });
    }
    toJSON() {
      const s = native.nodeTimingSnapshot();
      return {
        name: "node",
        entryType: "node",
        startTime: 0,
        duration: hostNow(),
        nodeStart: s.nodeStart,
        v8Start: s.v8Start,
        bootstrapComplete: s.bootstrapComplete,
        environment: s.environment,
        loopStart: s.loopStart,
        loopExit: s.loopExit,
        idleTime: s.idleTime,
      };
    }
  }

  // --- registry & observer mask --------------------------------------------
  const ENTRY_MASKS = {
    mark: 1 << 0,
    measure: 1 << 1,
    resource: 1 << 2,
    function: 1 << 3,
    gc: 1 << 4,
    http: 1 << 5,
    http2: 1 << 6,
    dns: 1 << 7,
    net: 1 << 8,
  };
  const TIMELINE_TYPES = new Set(["mark", "measure", "resource"]);

  const entriesByType = new Map();
  const observers = new Set();
  let resourceTimingBufferSize = 250;
  let bufferFullPending = false;

  const scheduleBufferFullEvent = () => {
    if (bufferFullPending) return;
    bufferFullPending = true;
    queueMicrotask(() => {
      bufferFullPending = false;
      try {
        const evt =
          typeof Event === "function"
            ? new Event("resourcetimingbufferfull")
            : { type: "resourcetimingbufferfull" };
        performance.dispatchEvent(evt);
      } catch {}
    });
  };

  const syncObserverMask = () => {
    let mask = 0;
    for (const obs of observers) {
      if (!obs.__active) continue;
      if (obs.__typeName !== null) {
        mask |= ENTRY_MASKS[obs.__typeName] || 0;
      } else if (obs.__types) {
        for (const t of obs.__types) {
          mask |= ENTRY_MASKS[t] || 0;
        }
      }
    }
    native.setObserverMask(mask >>> 0);
  };

  const record = (entry) => {
    if (TIMELINE_TYPES.has(entry.entryType)) {
      if (entry.entryType === "resource") {
        const list = entriesByType.get("resource") || [];
        if (list.length < resourceTimingBufferSize) {
          list.push(entry);
          entriesByType.set("resource", list);
        } else {
          scheduleBufferFullEvent();
        }
      } else {
        let list = entriesByType.get(entry.entryType);
        if (!list) {
          list = [];
          entriesByType.set(entry.entryType, list);
        }
        list.push(entry);
      }
    }
    notify(entry);
  };

  // --- observer delivery ---------------------------------------------------
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
          } catch {}
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

    constructor(callback) {
      if (typeof callback !== "function") {
        throw makeErr(TypeError, "ERR_INVALID_ARG_TYPE", 'The "callback" argument must be of type function');
      }
      this.#callback = callback;
      Object.defineProperty(this, "__callback", { value: callback, enumerable: false });
      Object.defineProperty(this, "__active", { value: false, enumerable: false, writable: true });
      Object.defineProperty(this, "__types", { value: new Set(), enumerable: false, writable: true });
      Object.defineProperty(this, "__typeName", { value: null, enumerable: false, writable: true });
      Object.defineProperty(this, "__buffered", { value: false, enumerable: false, writable: true });
    }

    __matches(entry) {
      if (!this.__active) return false;
      if (this.__typeName !== null) return entry.entryType === this.__typeName;
      return this.__types.has(entry.entryType);
    }

    observe(options) {
      if (options === null || typeof options !== "object") {
        throw makeErr(TypeError, "ERR_INVALID_ARG_TYPE", 'The "options" argument must be of type object');
      }
      if (options.entryTypes !== undefined) {
        if (!Array.isArray(options.entryTypes)) {
          throw makeErr(
            TypeError,
            "ERR_INVALID_ARG_TYPE",
            'The "options.entryTypes" property must be an instance of Array'
          );
        }
        this.__types = new Set(options.entryTypes.map(String));
        this.__typeName = null;
      } else if (options.type !== undefined) {
        this.__types = new Set();
        this.__typeName = String(options.type);
        this.__buffered = Boolean(options.buffered);
      } else {
        throw new TypeError("An entryTypes or type option is required");
      }
      this.__active = true;
      observers.add(this);
      syncObserverMask();

      if (this.__buffered && this.__typeName !== null) {
        const replay = entriesByType.get(this.__typeName) || [];
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
      syncObserverMask();
      const queue = pending.get(this);
      if (queue) queue.length = 0;
      pending.delete(this);
    }

    takeRecords() {
      native.flushGc();
      const queue = pending.get(this);
      if (!queue) return [];
      const drained = queue.slice();
      queue.length = 0;
      pending.delete(this);
      return drained;
    }

    static get supportedEntryTypes() {
      return ["dns", "function", "gc", "http", "http2", "mark", "measure", "net", "resource"];
    }
  }

  // --- histograms ----------------------------------------------------------
  const nativeHistograms = new WeakMap();
  const requireNativeHist = (inst) => {
    const h = nativeHistograms.get(inst);
    if (!h) {
      throw makeErr(TypeError, "ERR_INVALID_THIS", "Value of \"this\" must be of type Histogram");
    }
    return h;
  };

  class Histogram {
    constructor(nativeHist) {
      if (!nativeHist) {
        throw makeErr(TypeError, "ERR_ILLEGAL_CONSTRUCTOR", "Illegal constructor");
      }
      nativeHistograms.set(this, nativeHist);
    }

    get count() {
      return requireNativeHist(this).count;
    }
    get countBigInt() {
      return requireNativeHist(this).countBigint;
    }
    get min() {
      return requireNativeHist(this).min;
    }
    get minBigInt() {
      return requireNativeHist(this).minBigint;
    }
    get max() {
      return requireNativeHist(this).max;
    }
    get maxBigInt() {
      return requireNativeHist(this).maxBigint;
    }
    get mean() {
      return requireNativeHist(this).mean;
    }
    get stddev() {
      return requireNativeHist(this).stddev;
    }
    get exceeds() {
      return requireNativeHist(this).exceeds;
    }
    get exceedsBigInt() {
      return requireNativeHist(this).exceedsBigint;
    }

    percentile(value) {
      const h = requireNativeHist(this);
      if (typeof value !== "number") {
        throw makeErr(TypeError, "ERR_INVALID_ARG_TYPE", 'The "percentile" argument must be of type number');
      }
      if (!(value > 0 && value <= 100)) {
        throw makeErr(RangeError, "ERR_OUT_OF_RANGE", 'The value of "percentile" is out of range. It must be > 0 && <= 100');
      }
      return h.percentile(value);
    }

    percentileBigInt(value) {
      const h = requireNativeHist(this);
      if (typeof value !== "number") {
        throw makeErr(TypeError, "ERR_INVALID_ARG_TYPE", 'The "percentile" argument must be of type number');
      }
      if (!(value > 0 && value <= 100)) {
        throw makeErr(RangeError, "ERR_OUT_OF_RANGE", 'The value of "percentile" is out of range. It must be > 0 && <= 100');
      }
      return h.percentileBigint(value);
    }

    get percentiles() {
      return new Map(requireNativeHist(this).percentilesPairs());
    }

    get percentilesBigInt() {
      return new Map(requireNativeHist(this).percentilesBigintPairs());
    }

    reset() {
      requireNativeHist(this).reset();
    }

    toJSON() {
      const percentiles = {};
      for (const [k, v] of this.percentiles) {
        percentiles[String(k)] = v;
      }
      return {
        count: this.count,
        min: this.min,
        max: this.max,
        mean: this.mean,
        exceeds: this.exceeds,
        stddev: this.stddev,
        percentiles,
      };
    }
  }

  class RecordableHistogram extends Histogram {
    record(val) {
      const h = requireNativeHist(this);
      if (typeof val === "number") {
        if (!Number.isInteger(val)) {
          throw makeErr(RangeError, "ERR_OUT_OF_RANGE", 'The value of "val" is out of range. It must be an integer.');
        }
        if (val < 1 || val > Number.MAX_SAFE_INTEGER) {
          throw makeErr(
            RangeError,
            "ERR_OUT_OF_RANGE",
            'The value of "val" is out of range. It must be >= 1 && <= 9007199254740991.'
          );
        }
      } else if (typeof val === "bigint") {
        if (val < 1n || val > 9223372036854775807n) {
          throw makeErr(RangeError, "ERR_OUT_OF_RANGE", "value is out of range");
        }
      } else {
        throw makeErr(TypeError, "ERR_INVALID_ARG_TYPE", 'The "val" argument must be of type number');
      }
      h.record(val);
    }

    recordDelta() {
      requireNativeHist(this).recordDelta();
    }

    add(other) {
      const selfNative = requireNativeHist(this);
      if (!(other instanceof RecordableHistogram)) {
        throw makeErr(
          TypeError,
          "ERR_INVALID_ARG_TYPE",
          'The "other" argument must be an instance of RecordableHistogram'
        );
      }
      selfNative.add(requireNativeHist(other));
    }
  }

  class ELDHistogram extends Histogram {
    enable() {
      return requireNativeHist(this).enable();
    }
    disable() {
      return requireNativeHist(this).disable();
    }
  }

  const validateIntOpt = (value, name, min, max) => {
    if (typeof value !== "number") {
      throw makeErr(
        TypeError,
        "ERR_INVALID_ARG_TYPE",
        `The "options.${name}" property must be of type number`
      );
    }
    if (!Number.isInteger(value)) {
      throw makeErr(
        RangeError,
        "ERR_OUT_OF_RANGE",
        `The value of "options.${name}" is out of range. It must be an integer.`
      );
    }
    if (value < min || value > max) {
      throw makeErr(
        RangeError,
        "ERR_OUT_OF_RANGE",
        `The value of "options.${name}" is out of range. It must be >= ${min} && <= ${max}.`
      );
    }
  };

  const createHistogram = (options) => {
    if (options !== undefined && (options === null || typeof options !== "object")) {
      throw makeErr(TypeError, "ERR_INVALID_ARG_TYPE", 'The "options" argument must be of type object');
    }
    const opts = options || {};
    const lowest = opts.lowest === undefined ? 1 : opts.lowest;
    const highest = opts.highest === undefined ? Number.MAX_SAFE_INTEGER : opts.highest;
    const figures = opts.figures === undefined ? 3 : opts.figures;

    validateIntOpt(lowest, "lowest", 1, Number.MAX_SAFE_INTEGER);
    const minHighest = Math.min(Number.MAX_SAFE_INTEGER, 2 * lowest);
    if (typeof highest !== "number") {
      throw makeErr(TypeError, "ERR_INVALID_ARG_TYPE", 'The "options.highest" property must be of type number');
    }
    if (!Number.isInteger(highest) || highest < 2 * lowest || highest > Number.MAX_SAFE_INTEGER) {
      throw makeErr(
        RangeError,
        "ERR_OUT_OF_RANGE",
        `The value of "options.highest" is out of range. It must be >= ${minHighest} && <= ${Number.MAX_SAFE_INTEGER}.`
      );
    }
    validateIntOpt(figures, "figures", 1, 5);

    const h = native.createHistogram(lowest, highest, figures);
    return new RecordableHistogram(h);
  };

  const monitorEventLoopDelay = (options) => {
    if (options !== undefined && (options === null || typeof options !== "object")) {
      throw makeErr(TypeError, "ERR_INVALID_ARG_TYPE", 'The "options" argument must be of type object');
    }
    const opts = options || {};
    const resolution = opts.resolution === undefined ? 10 : opts.resolution;
    validateIntOpt(resolution, "resolution", 1, Number.MAX_SAFE_INTEGER);
    const h = native.createEld(resolution);
    return new ELDHistogram(h);
  };

  // --- event loop utilization ----------------------------------------------
  const eventLoopUtilization = (utilization, previous) => {
    const current = utilization || native.eventLoopStats();
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

  // --- timerify ------------------------------------------------------------
  const timerify = (fn, options) => {
    if (typeof fn !== "function") {
      throw makeErr(TypeError, "ERR_INVALID_ARG_TYPE", 'The "fn" argument must be of type function');
    }
    if (options !== undefined && (options === null || typeof options !== "object")) {
      throw makeErr(TypeError, "ERR_INVALID_ARG_TYPE", 'The "options" argument must be of type object');
    }
    let histogram = null;
    if (options && options.histogram !== undefined) {
      if (!(options.histogram instanceof RecordableHistogram)) {
        throw makeErr(
          TypeError,
          "ERR_INVALID_ARG_TYPE",
          'The "options.histogram" property must be an instance of RecordableHistogram'
        );
      }
      histogram = options.histogram;
    }
    const displayName = (options && options.name) || fn.name || "";
    const wrapped = function (...args) {
      const start = hostNow();
      try {
        return fn.apply(this, args);
      } finally {
        const duration = Math.max(0, hostNow() - start);
        if (histogram !== null) {
          histogram.record(Math.max(1, Math.round(duration * 1e6)));
        }
        const entry = new PerformanceNodeEntry(displayName, "function", start, duration, args);
        for (let i = 0; i < args.length; i++) {
          entry[i] = args[i];
        }
        notify(entry);
      }
    };
    Object.defineProperty(wrapped, "name", {
      value: "timerified " + (fn.name || ""),
      configurable: true,
    });
    return wrapped;
  };

  // --- Performance ---------------------------------------------------------
  const performance = globalThis.performance;
  const proto = Object.getPrototypeOf(performance);
  let Performance = proto.constructor;
  if (!Performance || Performance === Object) {
    Performance = function Performance() {
      throw makeErr(TypeError, "ERR_ILLEGAL_CONSTRUCTOR", "Illegal constructor");
    };
    Object.defineProperty(Performance, "prototype", {
      value: proto,
      writable: false,
      enumerable: false,
      configurable: false,
    });
    Object.defineProperty(proto, "constructor", {
      value: Performance,
      writable: true,
      enumerable: false,
      configurable: true,
    });
  }
  if (typeof EventTarget === "function" && Object.getPrototypeOf(proto) === Object.prototype) {
    Object.setPrototypeOf(proto, EventTarget.prototype);
  }

  const getEntries = () => {
    const all = [];
    for (const list of entriesByType.values()) all.push(...list);
    return all.sort((a, b) => a.startTime - b.startTime);
  };

  const findMark = (name) => {
    for (const entry of entriesByType.get("mark") || []) {
      if (entry.name === name) return entry;
    }
    return undefined;
  };

  const nodeTiming = new PerformanceNodeTiming();
  let bufferFullHandler = null;
  let bufferFullWrapper = null;

  Object.defineProperties(proto, {
    mark: {
      writable: true,
      configurable: true,
      value: function mark(name, options) {
        const entry = new PerformanceMark(String(name), options);
        record(entry);
        return entry;
      },
    },
    measure: {
      writable: true,
      configurable: true,
      value: function measure(name, startOrOptions, endMark) {
        let startTime;
        let duration;
        let detail;

        if (startOrOptions !== null && typeof startOrOptions === "object") {
          const options = startOrOptions;
          detail = options.detail;
          const resolveTime = (value) => {
            if (value === undefined) return undefined;
            if (typeof value === "number") return value;
            const mark = findMark(String(value));
            if (!mark) {
              throw makeErr(
                Error,
                "ERR_INVALID_ARG_VALUE",
                'The "start" argument must reference an existing mark: ' + value
              );
            }
            return mark.startTime;
          };
          const startAt = resolveTime(options.start);
          const endAt = resolveTime(options.end);
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
          const startAt =
            startOrOptions === undefined ? undefined : findMark(String(startOrOptions));
          const endAt = endMark === undefined ? undefined : findMark(String(endMark));
          if (startOrOptions !== undefined && !startAt) {
            throw makeErr(
              Error,
              "ERR_INVALID_ARG_VALUE",
              'The "start" argument must reference an existing mark: ' + startOrOptions
            );
          }
          if (endMark !== undefined && !endAt) {
            throw makeErr(
              Error,
              "ERR_INVALID_ARG_VALUE",
              'The "end" argument must reference an existing mark: ' + endMark
            );
          }
          startTime = startAt ? startAt.startTime : 0;
          const endTime = endAt ? endAt.startTime : hostNow();
          duration = endTime - startTime;
        }

        const entry = new PerformanceMeasure(String(name), startTime, duration, detail);
        record(entry);
        return entry;
      },
    },
    getEntries: {
      writable: true,
      configurable: true,
      value: function getEntriesFn() {
        return getEntries();
      },
    },
    getEntriesByType: {
      writable: true,
      configurable: true,
      value: function getEntriesByType(type) {
        return (entriesByType.get(String(type)) || []).slice();
      },
    },
    getEntriesByName: {
      writable: true,
      configurable: true,
      value: function getEntriesByName(name, type) {
        const wanted = String(name);
        return getEntries().filter(
          (entry) => entry.name === wanted && (type === undefined || entry.entryType === type)
        );
      },
    },
    clearMarks: {
      writable: true,
      configurable: true,
      value: function clearMarks(name) {
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
      },
    },
    clearMeasures: {
      writable: true,
      configurable: true,
      value: function clearMeasures(name) {
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
      },
    },
    clearResourceTimings: {
      writable: true,
      configurable: true,
      value: function clearResourceTimings() {
        entriesByType.delete("resource");
      },
    },
    setResourceTimingBufferSize: {
      writable: true,
      configurable: true,
      value: function setResourceTimingBufferSize(maxSize) {
        const n = Number(maxSize);
        resourceTimingBufferSize =
          Number.isFinite(n) && n >= 0 ? Math.min(0xffffffff, Math.floor(n)) : 0;
      },
    },
    markResourceTiming: {
      writable: true,
      configurable: true,
      value: function markResourceTiming(
        timingInfo,
        requestedUrl,
        initiatorType,
        _global,
        _cacheMode,
        bodyInfo,
        responseStatus
      ) {
        if (arguments.length === 1 && timingInfo && typeof timingInfo === "object" && "name" in timingInfo) {
          const entry = new PerformanceResourceTiming(timingInfo);
          record(entry);
          return entry;
        }
        const info = timingInfo || {};
        const startTime = Number(info.startTime || 0);
        const endTime = Number(info.endTime ?? startTime);
        const duration = Math.max(0, endTime - startTime);
        const body = bodyInfo || {};
        const init = {
          name: String(requestedUrl || ""),
          initiatorType: String(initiatorType || "other"),
          startTime,
          duration,
          workerStart: Number(info.finalServiceWorkerStartTime || 0),
          redirectStart: Number(info.redirectStartTime || 0),
          redirectEnd: Number(info.redirectEndTime || 0),
          fetchStart: Number(info.postRedirectStartTime || startTime),
          domainLookupStart: Number(info.domainLookupStart || startTime),
          domainLookupEnd: Number(info.domainLookupEnd || startTime),
          connectStart: Number(info.connectStart || startTime),
          connectEnd: Number(info.connectEnd || startTime),
          secureConnectionStart: Number(info.secureConnectionStart || startTime),
          requestStart: Number(info.finalNetworkRequestStartTime || startTime),
          responseStart: Number(info.finalNetworkResponseStartTime || endTime),
          responseEnd: endTime,
          encodedBodySize: Number(info.encodedBodySize ?? body.encodedBodySize ?? 0),
          decodedBodySize: Number(info.decodedBodySize ?? body.decodedBodySize ?? 0),
          transferSize: Number(info.transferSize ?? 0),
          responseStatus: Number(responseStatus ?? body.status ?? 0),
        };
        const entry = new PerformanceResourceTiming(init);
        record(entry);
        return entry;
      },
    },
    eventLoopUtilization: {
      writable: true,
      configurable: true,
      value: eventLoopUtilization,
    },
    timerify: {
      writable: true,
      configurable: true,
      value: timerify,
    },
    nodeTiming: {
      enumerable: false,
      configurable: true,
      get: () => nodeTiming,
    },
    onresourcetimingbufferfull: {
      enumerable: true,
      configurable: true,
      get: () => bufferFullHandler,
      set: (fn) => {
        if (bufferFullWrapper) {
          performance.removeEventListener("resourcetimingbufferfull", bufferFullWrapper);
          bufferFullWrapper = null;
        }
        bufferFullHandler = typeof fn === "function" ? fn : null;
        if (bufferFullHandler) {
          const targetFn = bufferFullHandler;
          bufferFullWrapper = function (event) {
            return targetFn.call(performance, event);
          };
          performance.addEventListener("resourcetimingbufferfull", bufferFullWrapper);
        }
      },
    },
    toJSON: {
      writable: true,
      configurable: true,
      value: function toJSON() {
        return {
          nodeTiming: this.nodeTiming,
          timeOrigin: this.timeOrigin,
          eventLoopUtilization: this.eventLoopUtilization(),
        };
      },
    },
  });

  // Expose Web Performance globals matching Node 24.
  for (const [k, v] of [
    ["Performance", Performance],
    ["PerformanceEntry", PerformanceEntry],
    ["PerformanceMark", PerformanceMark],
    ["PerformanceMeasure", PerformanceMeasure],
    ["PerformanceObserver", PerformanceObserver],
    ["PerformanceObserverEntryList", PerformanceObserverEntryList],
    ["PerformanceResourceTiming", PerformanceResourceTiming],
  ]) {
    Object.defineProperty(globalThis, k, {
      value: v,
      writable: true,
      enumerable: false,
      configurable: true,
    });
  }

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
    NODE_PERFORMANCE_ENTRY_TYPE_GC: 0,
    NODE_PERFORMANCE_ENTRY_TYPE_HTTP: 1,
    NODE_PERFORMANCE_ENTRY_TYPE_HTTP2: 2,
    NODE_PERFORMANCE_ENTRY_TYPE_NET: 3,
    NODE_PERFORMANCE_ENTRY_TYPE_DNS: 4,
    NODE_PERFORMANCE_MILESTONE_TIME_ORIGIN_TIMESTAMP: 0,
    NODE_PERFORMANCE_MILESTONE_TIME_ORIGIN: 1,
    NODE_PERFORMANCE_MILESTONE_ENVIRONMENT: 2,
    NODE_PERFORMANCE_MILESTONE_NODE_START: 3,
    NODE_PERFORMANCE_MILESTONE_V8_START: 4,
    NODE_PERFORMANCE_MILESTONE_LOOP_START: 5,
    NODE_PERFORMANCE_MILESTONE_LOOP_EXIT: 6,
    NODE_PERFORMANCE_MILESTONE_BOOTSTRAP_COMPLETE: 7,
  };

  Object.defineProperty(globalThis, symInternal, {
    value: {
      emitGc(startTime, duration, kind, flags) {
        notify(new PerformanceNodeEntry("gc", "gc", startTime, duration, { kind, flags }));
      },
      emitResource(init) {
        record(new PerformanceResourceTiming(init));
      },
      emitNodeEntry(name, entryType, startTime, duration, detail) {
        notify(new PerformanceNodeEntry(name, entryType, startTime, duration, detail));
      },
    },
    writable: true,
    enumerable: false,
    configurable: true,
  });

  const exportsObj = {
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

  Object.defineProperty(globalThis, symExports, {
    value: exportsObj,
    writable: true,
    enumerable: false,
    configurable: true,
  });

  return exportsObj;
})()"#;

fn native_node_timing_snapshot<'js>(ctx: Ctx<'js>) -> Result<Object<'js>> {
    let snap = perf::node_timing_snapshot(perf::rt_key_for_ctx(&ctx));
    let obj = Object::new(ctx.clone())?;
    obj.set("nodeStart", snap.node_start)?;
    obj.set("v8Start", snap.v8_start)?;
    obj.set("bootstrapComplete", snap.bootstrap_complete)?;
    obj.set("environment", snap.environment)?;
    obj.set("loopStart", snap.loop_start)?;
    obj.set("loopExit", snap.loop_exit)?;
    obj.set("idleTime", snap.idle_time)?;
    obj.set("loopCount", snap.loop_count as f64)?;
    obj.set("events", snap.events as f64)?;
    obj.set("eventsWaiting", snap.events_waiting as f64)?;
    Ok(obj)
}

fn native_event_loop_stats<'js>(ctx: Ctx<'js>) -> Result<Object<'js>> {
    let (idle, active) = perf::event_loop_utilization_ms(perf::rt_key_for_ctx(&ctx));
    let obj = Object::new(ctx)?;
    obj.set("idle", idle)?;
    obj.set("active", active)?;
    Ok(obj)
}

fn native_set_observer_mask<'js>(ctx: Ctx<'js>, mask: u32) {
    perf::set_observer_mask(perf::rt_key_for_ctx(&ctx), mask);
}

fn native_flush_gc<'js>(ctx: Ctx<'js>) {
    perf::flush_pending_gc(&ctx);
}

fn native_create_histogram<'js>(
    ctx: Ctx<'js>,
    lowest: f64,
    highest: f64,
    figures: f64,
) -> Result<Value<'js>> {
    let hist = NativeHistogram::new_recordable(&ctx, lowest, highest, figures)?;
    crate::histogram::into_class_value(&ctx, hist)
}

fn native_create_eld<'js>(ctx: Ctx<'js>, resolution: f64) -> Result<Value<'js>> {
    let hist = NativeHistogram::new_eld(&ctx, resolution)?;
    crate::histogram::into_class_value(&ctx, hist)
}

fn install_native_bridge<'js>(ctx: &Ctx<'js>) -> Result<()> {
    let globals = ctx.globals();
    let scratch = Object::new(ctx.clone())?;
    Class::<NativeHistogram>::define(&scratch)?;

    let bridge = Object::new(ctx.clone())?;
    bridge.set("now", Func::from(|| -> f64 { Performance::now() }))?;
    bridge.set(
        "nodeTimingSnapshot",
        Function::new(ctx.clone(), native_node_timing_snapshot)?,
    )?;
    bridge.set(
        "eventLoopStats",
        Function::new(ctx.clone(), native_event_loop_stats)?,
    )?;
    bridge.set(
        "setObserverMask",
        Function::new(ctx.clone(), native_set_observer_mask)?,
    )?;
    bridge.set("flushGc", Function::new(ctx.clone(), native_flush_gc)?)?;
    bridge.set(
        "createHistogram",
        Function::new(ctx.clone(), native_create_histogram)?,
    )?;
    bridge.set("createEld", Function::new(ctx.clone(), native_create_eld)?)?;

    let sym_native = Symbol::new_global(ctx.clone(), "lexe.perfNative")?;
    let def_prop: Function = ctx.eval(
        r#"(function (target, key, val) {
  Object.defineProperty(target, key, {
    value: val,
    writable: true,
    enumerable: false,
    configurable: true,
  });
})"#,
    )?;
    def_prop.call::<_, ()>((globals, sym_native, bridge))?;
    Ok(())
}

/// Install global `performance`, `Performance*` constructors, and observer layer once per context.
pub fn install_globals<'js>(ctx: &Ctx<'js>) -> Result<Object<'js>> {
    let sym_exports = Symbol::new_global(ctx.clone(), "lexe.perfExports")?;
    if let Some(existing) = ctx
        .globals()
        .get::<_, Option<Object<'_>>>(sym_exports.clone())?
    {
        return Ok(existing);
    }

    if ctx
        .globals()
        .get::<_, Option<Value<'_>>>("performance")?
        .is_none()
    {
        let instance = Class::instance(ctx.clone(), Performance::new())?;
        Performance::add_event_target_prototype(ctx)?;
        ctx.globals().set("performance", instance)?;
    }

    install_native_bridge(ctx)?;
    let exports_obj: Object = ctx.eval(PERF_OBSERVER_JS)?;
    Ok(exports_obj)
}

/// Install `perf_hooks` onto the module's default export.
pub fn install<'js>(ctx: &Ctx<'js>, default: &Object<'js>) -> Result<()> {
    let api = install_globals(ctx)?;
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
        let value: Value = api.get(name)?;
        default.set(name, value)?;
    }
    Ok(())
}
