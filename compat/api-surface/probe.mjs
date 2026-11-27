#!/usr/bin/env node
import { createRequire, builtinModules, isBuiltin } from "node:module";
import process from "node:process";

const require = createRequire(import.meta.url);

// Canonical Node.js built-in modules tracked in docs/compatibility.md
const TRACKED_MODULES = [
  "assert",
  "assert/strict",
  "async_hooks",
  "buffer",
  "child_process",
  "cluster",
  "console",
  "constants",
  "crypto",
  "dgram",
  "diagnostics_channel",
  "dns",
  "dns/promises",
  "events",
  "fs",
  "fs/promises",
  "http",
  "http2",
  "https",
  "inspector",
  "inspector/promises",
  "module",
  "net",
  "os",
  "path",
  "path/posix",
  "path/win32",
  "perf_hooks",
  "process",
  "querystring",
  "readline",
  "readline/promises",
  "repl",
  "sea",
  "sqlite",
  "stream",
  "stream/consumers",
  "stream/promises",
  "stream/web",
  "string_decoder",
  "test",
  "timers",
  "timers/promises",
  "tls",
  "tty",
  "url",
  "util",
  "util/types",
  "v8",
  "vm",
  "wasi",
  "worker_threads",
  "zlib",
];

const NODE_ONLY_MODULES = new Set(["sea", "sqlite", "test"]);

const TRACKED_GLOBALS = [
  "AbortController",
  "AbortSignal",
  "Blob",
  "BroadcastChannel",
  "Buffer",
  "ByteLengthQueuingStrategy",
  "CloseEvent",
  "CompressionStream",
  "CountQueuingStrategy",
  "CustomEvent",
  "DOMException",
  "DecompressionStream",
  "ErrorEvent",
  "Event",
  "EventTarget",
  "File",
  "FormData",
  "Headers",
  "Intl",
  "MessageChannel",
  "MessagePort",
  "Performance",
  "PerformanceEntry",
  "PerformanceMark",
  "PerformanceMeasure",
  "PerformanceObserver",
  "PerformanceObserverEntryList",
  "PerformanceResourceTiming",
  "ProgressEvent",
  "ReadableStream",
  "Request",
  "Response",
  "TextDecoder",
  "TextDecoderStream",
  "TextEncoder",
  "TextEncoderStream",
  "TransformStream",
  "URL",
  "URLPattern",
  "URLSearchParams",
  "WebAssembly",
  "WritableStream",
  "atob",
  "btoa",
  "clearImmediate",
  "clearInterval",
  "clearTimeout",
  "console",
  "crypto",
  "fetch",
  "navigator",
  "performance",
  "process",
  "queueMicrotask",
  "setImmediate",
  "setInterval",
  "setTimeout",
  "structuredClone",
];

function classifyValue(val) {
  if (val === null) return "null";
  if (val === undefined) return "undefined";
  const t = typeof val;
  if (t === "function") return "function";
  if (t === "object") {
    if (Array.isArray(val)) return "array";
    return "object";
  }
  return t;
}

function symbolLabel(sym) {
  const key = Symbol.keyFor(sym);
  if (key !== undefined) return `Symbol.for(${key})`;
  return sym.toString();
}

function inspectConstructorSurface(fn) {
  const protoKeys =
    fn.prototype && typeof fn.prototype === "object"
      ? Object.getOwnPropertyNames(fn.prototype).sort()
      : [];
  const staticKeys = Object.getOwnPropertyNames(fn)
    .filter((k) => k !== "length" && k !== "name" && k !== "prototype" && k !== "arguments" && k !== "caller")
    .sort();
  return { protoKeys, staticKeys };
}

function safeInspectExports(obj) {
  if (obj === null || (typeof obj !== "object" && typeof obj !== "function")) {
    return {
      kind: classifyValue(obj),
      keys: {},
      symbols: [],
      constructors: {},
    };
  }
  const keys = {};
  const constructors = {};
  for (const name of Object.getOwnPropertyNames(obj).sort()) {
    let val;
    try {
      val = obj[name];
    } catch {
      keys[name] = "getter-threw";
      continue;
    }
    const kind = classifyValue(val);
    keys[name] = kind;
    if (kind === "function") {
      const surface = inspectConstructorSurface(val);
      if (surface.protoKeys.length > 1 || (/^[A-Z]/.test(name) && surface.staticKeys.length > 0)) {
        constructors[name] = surface;
      }
    }
  }
  const symbols = Object.getOwnPropertySymbols(obj).map(symbolLabel).sort();
  return { kind: classifyValue(obj), keys, symbols, constructors };
}

function safeReadNestedKeys(obj, prop) {
  try {
    const val = obj?.[prop];
    if (!val || typeof val !== "object") return null;
    if (Array.isArray(val)) return { kind: "array", values: [...val].sort() };
    const out = {};
    for (const k of Object.getOwnPropertyNames(val).sort()) {
      out[k] = classifyValue(val[k]);
    }
    return { kind: "object", keys: out };
  } catch {
    return null;
  }
}

async function probeModule(name) {
  const nodeSpec = `node:${name}`;
  const entry = {
    name,
    isBuiltinBare: isBuiltin(name),
    isBuiltinNode: isBuiltin(nodeSpec),
    cjsLoaded: false,
    esmLoaded: false,
    getBuiltinLoaded: false,
    bareCjsLoaded: false,
    cjs: null,
    esmKeys: null,
    esmCjsMissingInEsm: [],
    esmExtraOverCjs: [],
    constants: null,
    methods: null,
    statusCodes: null,
  };

  let cjsMod;
  try {
    cjsMod = require(nodeSpec);
    entry.cjsLoaded = true;
    entry.cjs = safeInspectExports(cjsMod);
    if (cjsMod && typeof cjsMod === "object") {
      entry.constants = safeReadNestedKeys(cjsMod, "constants");
      entry.methods = safeReadNestedKeys(cjsMod, "METHODS");
      entry.statusCodes = safeReadNestedKeys(cjsMod, "STATUS_CODES");
    }
  } catch (err) {
    entry.cjsError = err?.code || err?.message || String(err);
  }

  if (!NODE_ONLY_MODULES.has(name)) {
    try {
      require(name);
      entry.bareCjsLoaded = true;
    } catch {
      entry.bareCjsLoaded = false;
    }
  }

  if (typeof process.getBuiltinModule === "function") {
    try {
      const bm = process.getBuiltinModule(nodeSpec);
      entry.getBuiltinLoaded = bm !== undefined;
    } catch {
      entry.getBuiltinLoaded = false;
    }
  }

  try {
    const esmMod = await import(nodeSpec);
    entry.esmLoaded = true;
    const esmNames = Object.getOwnPropertyNames(esmMod).sort();
    entry.esmKeys = esmNames;
    if (entry.cjsLoaded && entry.cjs && typeof cjsMod === "object" && cjsMod !== null) {
      const cjsSet = new Set(Object.keys(entry.cjs.keys));
      const esmSet = new Set(esmNames);
      for (const k of cjsSet) {
        if (!esmSet.has(k)) entry.esmCjsMissingInEsm.push(k);
      }
      for (const k of esmSet) {
        if (k !== "default" && !cjsSet.has(k)) entry.esmExtraOverCjs.push(k);
      }
    }
  } catch (err) {
    entry.esmError = err?.code || err?.message || String(err);
  }

  return entry;
}

const modules = {};
for (const name of TRACKED_MODULES) {
  modules[name] = await probeModule(name);
}

const globals = {};
for (const name of TRACKED_GLOBALS) {
  const val = globalThis[name];
  const kind = classifyValue(val);
  globals[name] = {
    kind,
    ...(kind === "function" ? inspectConstructorSurface(val) : {}),
  };
}

const payload = {
  runtime: process.release?.name || "node",
  version: process.version,
  builtinModules: [...builtinModules].sort(),
  modules,
  globals,
};

console.log(JSON.stringify(payload));
