// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
//
// Node-compatible `worker_threads` (Worker spawn via OS-thread QuickJS isolate).

import { EventEmitter } from "node:events";

declare global {
  var __lexeWorkerIsMainThread: (() => boolean) | undefined;
  var __lexeWorkerThreadId: (() => number) | undefined;
  var __lexeWorkerSpawn:
    | ((
        filename: string,
        options: Record<string, unknown>,
        onEvent: (event: string, arg?: unknown) => void
      ) => {
        threadId: number;
        postMessage: (data: unknown, transfer?: unknown[]) => void;
        terminate: () => void;
        ref: () => void;
        unref: () => void;
      })
    | undefined;
  var __lexeWorkerParentPostMessage:
    | ((data: unknown, transfer?: unknown[]) => void)
    | undefined;
  var __lexeWorkerBroadcastEnv:
    | ((key: unknown, value: unknown) => void)
    | undefined;
  var __lexeWorkerApplyEnvironmentSnapshot: ((snapshot: unknown) => void) | undefined;
  var __lexeWorkerApplyEnvPatch: ((key: unknown, value: unknown) => void) | undefined;
  var __lexeWorkerDispatchParentMessage: ((data: unknown) => void) | undefined;
  var __lexeCreateLoopRefController: () => {
    ref(): void;
    unref(): void;
  };
  var __lexePortCreatePair: (() => [number, number]) | undefined;
  var __lexePortPeer: ((id: number) => number | undefined) | undefined;
  var __lexePortClose: ((id: number) => void) | undefined;
  var __lexePortSend:
    | ((id: number, data: unknown, transfer?: { transfer: unknown[] }) => void)
    | undefined;
  var __lexePortAttach:
    | ((id: number, onMsg: (data: unknown) => void) => void)
    | undefined;
  var __lexePortEnsureLocalAttached: ((id: number) => void) | undefined;
  var __lexePortRebuildFromId: ((id: number) => unknown) | undefined;
  var __lexePortDetachSink: ((id: number) => void) | undefined;
  var __lexeVmMovePort:
    | ((sandbox: unknown, script: string, bridge: unknown) => unknown)
    | undefined;
  var __lexeWorkerPostToThread:
    | ((threadId: number, value: unknown, transfer?: { transfer: unknown[] }) => void)
    | undefined;
  var __lexeWorkerResourceLimits: Record<string, number> | undefined;
}

function readIsMainThread(): boolean {
  if (typeof globalThis.__lexeWorkerIsMainThread === "function") {
    return globalThis.__lexeWorkerIsMainThread();
  }
  return true;
}

function readThreadId(): number {
  if (typeof globalThis.__lexeWorkerThreadId === "function") {
    return globalThis.__lexeWorkerThreadId();
  }
  return 0;
}

export const isMainThread = readIsMainThread();
export const threadId = readThreadId();

/** Populated in worker child before the worker script is imported. */
export let workerData: unknown = isMainThread
  ? undefined
  : (globalThis as { workerData?: unknown }).workerData;

(globalThis as { __lexeSetWorkerData?: (v: unknown) => void }).__lexeSetWorkerData = (
  v: unknown
) => {
  workerData = v;
  defaultExport.workerData = v;
};

type LoopRefController = ReturnType<typeof __lexeCreateLoopRefController>;

/** Worker-side environment mirror (synced from main via bootstrap / patches). */
const workerEnvironmentData = new Map<unknown, unknown>();

const envBootstrap = (globalThis as { __lexeWorkerEnvBootstrap?: Map<unknown, unknown> })
  .__lexeWorkerEnvBootstrap;
if (envBootstrap) {
  for (const [k, v] of envBootstrap.entries()) {
    workerEnvironmentData.set(k, v);
  }
  delete (globalThis as { __lexeWorkerEnvBootstrap?: Map<unknown, unknown> })
    .__lexeWorkerEnvBootstrap;
}

globalThis.__lexeWorkerApplyEnvironmentSnapshot = (snapshot: unknown) => {
  workerEnvironmentData.clear();
  if (snapshot && typeof snapshot === "object") {
    for (const [k, v] of Object.entries(snapshot as Record<string, unknown>)) {
      workerEnvironmentData.set(k, v);
    }
  }
};

globalThis.__lexeWorkerApplyEnvPatch = (key: unknown, value: unknown) => {
  workerEnvironmentData.set(key, value);
};

const environmentData = new Map<unknown, unknown>();

function environmentSnapshotForSpawn(): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  for (const [k, v] of environmentData.entries()) {
    out[String(k)] = v;
  }
  return out;
}

function transferOptions(transfer?: unknown[]): { transfer: unknown[] } | undefined {
  if (transfer == null) return undefined;
  if (!Array.isArray(transfer)) {
    throw new TypeError('The "transferList" argument must be an instance of Array.');
  }
  if (transfer.length === 0) return undefined;
  return { transfer };
}

export interface ResourceLimits {
  maxYoungGenerationSizeMb?: number;
  maxOldGenerationSizeMb?: number;
  codeRangeSizeMb?: number;
  stackSizeMb?: number;
}

export interface WorkerOptions {
  workerData?: unknown;
  env?: Record<string, string> | typeof SHARE_ENV;
  argv?: string[];
  name?: string;
  eval?: boolean;
  stdin?: boolean;
  stdout?: boolean;
  stderr?: boolean;
  transferList?: unknown[];
  resourceLimits?: ResourceLimits;
}

export const SHARE_ENV = Symbol.for("nodejs.worker_threads.SHARE_ENV");

function normalizeResourceLimits(limits?: ResourceLimits): Required<ResourceLimits> {
  const rec = limits && typeof limits === "object" ? limits : {};
  const pick = (v: unknown, def: number) =>
    typeof v === "number" && Number.isFinite(v) && v >= 0 ? v : def;
  return {
    maxYoungGenerationSizeMb: pick(rec.maxYoungGenerationSizeMb, 16),
    maxOldGenerationSizeMb: pick(rec.maxOldGenerationSizeMb, 2048),
    codeRangeSizeMb: pick(rec.codeRangeSizeMb, 0),
    stackSizeMb: pick(rec.stackSizeMb, 4),
  };
}

export const resourceLimits: ResourceLimits = isMainThread
  ? {}
  : globalThis.__lexeWorkerResourceLimits ?? {};

function toDataCloneError(err: unknown): DOMException {
  if (
    err &&
    typeof err === "object" &&
    "name" in err &&
    (err as { name?: string }).name === "DataCloneError"
  ) {
    return err as DOMException;
  }
  const message =
    err && typeof err === "object" && "message" in err
      ? String((err as { message?: unknown }).message ?? err)
      : String(err ?? "The object could not be cloned.");
  return new DOMException(message, "DataCloneError");
}

type MessageListener = (data: unknown) => void;

const portIdSymbol = Symbol.for("lexe.internal.messagePortId");
const portTransferSymbol = Symbol.for("lexe.internal.messagePortTransfer");
const localPorts = new Map<number, MessagePort>();

export class MessagePort {
  #onmessage: ((ev: { data: unknown }) => void) | null = null;
  onmessageerror: ((ev: unknown) => void) | null = null;
  #peer: MessagePort | null = null;
  #portId = 0;
  #listeners = new Map<string, Set<MessageListener>>();
  #onceWrappers = new WeakMap<MessageListener, MessageListener>();
  #closed = false;
  #detached = false;
  #started = false;
  #attachedToHub = false;
  #explicitUnref = false;
  readonly #loopRef: LoopRefController;

  constructor(portId = 0) {
    this.#loopRef =
      typeof globalThis.__lexeCreateLoopRefController === "function"
        ? globalThis.__lexeCreateLoopRefController()
        : { ref() {}, unref() {} };
    if (portId > 0) {
      this.#portId = portId;
      localPorts.set(portId, this);
      Object.defineProperty(this, portIdSymbol, {
        value: portId,
        enumerable: false,
        configurable: true,
        writable: false,
      });
      Object.defineProperty(this, portTransferSymbol, {
        value: () => {
          if (this.#detached) return true;
          this.#detached = true;
          this.#closed = true;
          localPorts.delete(this.#portId);
          this.#loopRef.unref();
          return false;
        },
        enumerable: false,
        configurable: true,
        writable: false,
      });
    }
  }

  get onmessage(): ((ev: { data: unknown }) => void) | null {
    return this.#onmessage;
  }

  set onmessage(handler: ((ev: { data: unknown }) => void) | null) {
    this.#onmessage = typeof handler === "function" ? handler : null;
    if (this.#onmessage) {
      this.#started = true;
      if (!this.#hasLocalPeer()) {
        this._attachHub();
      }
    }
  }

  get isStarted(): boolean {
    return this.#started;
  }

  #hasLocalPeer(): boolean {
    if (this.#portId === 0 || typeof globalThis.__lexePortPeer !== "function") {
      return false;
    }
    const peerId = globalThis.__lexePortPeer(this.#portId);
    return peerId !== undefined && localPorts.has(peerId);
  }

  _setPeer(peer: MessagePort) {
    this.#peer = peer;
  }

  _attachHub(): void {
    if (
      this.#attachedToHub ||
      this.#closed ||
      this.#detached ||
      this.#portId === 0 ||
      typeof globalThis.__lexePortAttach !== "function"
    ) {
      return;
    }
    this.#attachedToHub = true;
    if (!this.#explicitUnref) {
      this.#loopRef.ref();
    }
    globalThis.__lexePortAttach(this.#portId, (data: unknown) => {
      if (!this.#closed && !this.#detached) {
        this.dispatchMessage(data);
      }
    });
  }

  start(): void {
    this.#started = true;
    if (!this.#hasLocalPeer()) {
      this._attachHub();
    }
  }

  close(): void {
    if (this.#closed) return;
    this.#closed = true;
    if (this.#portId > 0) {
      localPorts.delete(this.#portId);
      if (typeof globalThis.__lexePortClose === "function") {
        globalThis.__lexePortClose(this.#portId);
      }
    }
    this.#loopRef.unref();
    this.#dispatch("close", undefined);
  }

  postMessage(message: unknown, transfer?: unknown[]): void {
    if (this.#closed || this.#detached) return;

    let transferOpts: { transfer: unknown[] } | undefined;
    if (transfer != null) {
      if (!Array.isArray(transfer)) {
        throw new TypeError(
          'The "transferList" argument must be an instance of Array.'
        );
      }
      if (transfer.length > 0) {
        transferOpts = { transfer };
      }
    }

    if (this.#portId > 0 && typeof globalThis.__lexePortPeer === "function") {
      const peerId = globalThis.__lexePortPeer(this.#portId);
      const localPeer = peerId !== undefined ? localPorts.get(peerId) : undefined;
      if (localPeer) {
        let cloned: unknown;
        try {
          cloned = transferOpts
            ? structuredClone(message, transferOpts as object)
            : structuredClone(message);
        } catch (err) {
          throw toDataCloneError(err);
        }
        if (localPeer.#closed || localPeer.#detached) return;
        queueMicrotask(() => {
          if (localPeer.#closed || localPeer.#detached) return;
          localPeer.dispatchMessage(cloned);
        });
        return;
      }
      if (typeof globalThis.__lexePortSend === "function") {
        try {
          globalThis.__lexePortSend(this.#portId, message, transferOpts);
        } catch (err) {
          throw toDataCloneError(err);
        }
        return;
      }
    }

    let cloned: unknown;
    try {
      cloned = transferOpts
        ? structuredClone(message, transferOpts as object)
        : structuredClone(message);
    } catch (err) {
      throw toDataCloneError(err);
    }

    const peer = this.#peer;
    if (!peer || peer.#closed) return;

    queueMicrotask(() => {
      if (peer.#closed) return;
      peer.dispatchMessage(cloned);
    });
  }

  dispatchMessage(data: unknown): void {
    this.#dispatch("message", data);
    if (typeof this.onmessage === "function") {
      try {
        const ev =
          typeof MessageEvent === "function"
            ? new MessageEvent("message", { data })
            : { data };
        this.onmessage(ev);
      } catch {
        // Node continues after onmessage throws; other listeners already ran.
      }
    }
  }

  ref(): this {
    this.#explicitUnref = false;
    this.#loopRef.ref();
    return this;
  }

  unref(): this {
    this.#explicitUnref = true;
    this.#loopRef.unref();
    return this;
  }

  addEventListener(type: string, listener: MessageListener): void {
    let set = this.#listeners.get(type);
    if (!set) {
      set = new Set();
      this.#listeners.set(type, set);
    }
    set.add(listener);
    if (type === "message") {
      this.#started = true;
      if (!this.#hasLocalPeer()) {
        this._attachHub();
      }
    }
  }

  removeEventListener(type: string, listener: MessageListener): void {
    const set = this.#listeners.get(type);
    if (!set) return;
    set.delete(listener);
    const wrapped = this.#onceWrappers.get(listener);
    if (wrapped) {
      set.delete(wrapped);
    }
  }

  on(type: string, listener: MessageListener): this {
    this.addEventListener(type, listener);
    return this;
  }

  once(type: string, listener: MessageListener): this {
    const wrapped: MessageListener = (data: unknown) => {
      this.removeEventListener(type, wrapped);
      listener(data);
    };
    this.#onceWrappers.set(listener, wrapped);
    this.addEventListener(type, wrapped);
    return this;
  }

  addListener(type: string, listener: MessageListener): this {
    return this.on(type, listener);
  }

  off(type: string, listener: MessageListener): this {
    this.removeEventListener(type, listener);
    return this;
  }

  removeListener(type: string, listener: MessageListener): this {
    return this.off(type, listener);
  }

  removeAllListeners(type?: string): this {
    if (type === undefined) {
      this.#listeners.clear();
    } else {
      this.#listeners.delete(type);
    }
    return this;
  }

  #dispatch(type: string, data: unknown) {
    const set = this.#listeners.get(type);
    if (!set) return;
    for (const fn of [...set]) {
      try {
        fn(data);
      } catch {
        // Match Node: one listener throwing must not skip the rest.
      }
    }
  }
}

const workerParentListeners = new Set<MessageListener>();

/** Worker-side port talking to the parent over process IPC. */
class WorkerParentPort extends MessagePort {
  postMessage(message: unknown, transfer?: unknown[]): void {
    if (typeof globalThis.__lexeWorkerParentPostMessage !== "function") {
      throw new Error("worker parent IPC is unavailable");
    }
    try {
      globalThis.__lexeWorkerParentPostMessage(
        message,
        transferOptions(transfer)
      );
    } catch (err) {
      throw toDataCloneError(err);
    }
  }
}

let workerParentPort: WorkerParentPort | null = null;

globalThis.__lexeWorkerDispatchParentMessage = (data: unknown) => {
  workerParentPort?.dispatchMessage(data);
};

export const parentPort: WorkerParentPort | null = isMainThread
  ? null
  : (workerParentPort = new WorkerParentPort());

export class Worker extends EventEmitter {
  readonly threadId: number;
  readonly stdin: unknown;
  readonly stdout: unknown;
  readonly stderr: unknown;
  #exited = false;
  #resourceLimits: Required<ResourceLimits>;
  #handle: {
    postMessage: (data: unknown, transfer?: { transfer?: unknown[] }) => void;
    terminate: () => void;
    writeStdin: (chunk: unknown) => void;
    endStdin: () => void;
    ref: () => void;
    unref: () => void;
  };

  get resourceLimits(): ResourceLimits {
    return this.#exited ? {} : { ...this.#resourceLimits };
  }

  constructor(filename: string | URL, options: WorkerOptions = {}) {
    super();
    if (typeof globalThis.__lexeWorkerSpawn !== "function") {
      throw new Error("worker_threads.Worker is unavailable in this build");
    }
    const path = filename instanceof URL ? filename.href : String(filename);
    const resolvedLimits = normalizeResourceLimits(options.resourceLimits);
    this.#resourceLimits = resolvedLimits;

    const spawnOptions: Record<string, unknown> = {
      workerData: options.workerData ?? null,
      lexeEnvironmentSnapshot: environmentSnapshotForSpawn(),
      lexeResourceLimits: resolvedLimits,
    };
    if (options.env === SHARE_ENV) {
      spawnOptions.lexeEnvShare = true;
    } else if (options.env != null && typeof options.env === "object") {
      const envCopy: Record<string, string> = {};
      for (const [k, v] of Object.entries(options.env)) {
        envCopy[k] = String(v);
      }
      spawnOptions.lexeEnvMap = envCopy;
    }
    if (options.argv != null) spawnOptions.argv = options.argv;
    if (options.name != null) spawnOptions.name = options.name;
    if (options.eval) spawnOptions.eval = true;
    if (options.stdin) spawnOptions.stdin = true;
    if (options.stdout) spawnOptions.stdout = true;
    if (options.stderr) spawnOptions.stderr = true;
    const transferObj = transferOptions(options.transferList);
    if (transferObj) spawnOptions.lexeWorkerDataTransfer = transferObj;

    let stdoutStream: { push(chunk: unknown): void } | null = null;
    let stderrStream: { push(chunk: unknown): void } | null = null;
    let stdinStream: unknown = null;
    if (options.stdin || options.stdout || options.stderr) {
      const streamMod = require("node:stream") as {
        PassThrough: new () => { push(chunk: unknown): void };
        Writable: new (opts: {
          write(chunk: unknown, enc: unknown, cb: () => void): void;
          final?(cb: () => void): void;
        }) => unknown;
      };
      if (options.stdout) stdoutStream = new streamMod.PassThrough();
      if (options.stderr) stderrStream = new streamMod.PassThrough();
      if (options.stdin) {
        stdinStream = new streamMod.Writable({
          write(chunk, _enc, cb) {
            handle.writeStdin(chunk);
            cb();
          },
          final(cb) {
            handle.endStdin();
            cb();
          },
        });
      }
    }
    this.stdin = stdinStream;
    this.stdout = stdoutStream;
    this.stderr = stderrStream;

    const handle = globalThis.__lexeWorkerSpawn(path, spawnOptions, (event, arg) => {
      if (event === "stdout") {
        stdoutStream?.push(arg);
        return;
      }
      if (event === "stderr") {
        stderrStream?.push(arg);
        return;
      }
      if (event === "exit") {
        this.#exited = true;
        stdoutStream?.push(null);
        stderrStream?.push(null);
      }
      if (event === "exit" && arg != null && typeof arg === "object") {
        const rec = arg as { code?: unknown; signal?: unknown };
        this.emit("exit", rec.code ?? 0, rec.signal ?? null);
        return;
      }
      this.emit(event, arg);
    });
    this.threadId = handle.threadId;
    this.#handle = handle;
  }

  postMessage(value: unknown, transfer?: unknown[]): void {
    try {
      this.#handle.postMessage(value, transferOptions(transfer));
    } catch (err) {
      throw toDataCloneError(err);
    }
  }

  terminate(): Promise<number | undefined> {
    if (this.#exited) return Promise.resolve(undefined);
    const exited = new Promise<number>((resolve) => this.once("exit", resolve));
    this.#handle.terminate();
    return exited;
  }

  ref(): this {
    this.#handle.ref();
    return this;
  }

  unref(): this {
    this.#handle.unref();
    return this;
  }
}

globalThis.__lexePortEnsureLocalAttached = (peerId: number) => {
  const peer = localPorts.get(peerId);
  if (peer && peer.isStarted) peer._attachHub();
};

globalThis.__lexePortRebuildFromId = (portId: number) => {
  return new MessagePort(portId);
};

export class MessageChannel {
  port1: MessagePort;
  port2: MessagePort;
  constructor() {
    const [id1, id2] =
      typeof globalThis.__lexePortCreatePair === "function"
        ? globalThis.__lexePortCreatePair()
        : [0, 0];
    this.port1 = new MessagePort(id1);
    this.port2 = new MessagePort(id2);
    if (id1 === 0) {
      this.port1._setPeer(this.port2);
      this.port2._setPeer(this.port1);
    }
  }
}

export const BroadcastChannel = (globalThis as { BroadcastChannel?: unknown })
  .BroadcastChannel;

export function getEnvironmentData(key: unknown): unknown {
  const store = readIsMainThread() ? environmentData : workerEnvironmentData;
  if (!store.has(key)) {
    return undefined;
  }
  return structuredClone(store.get(key));
}

export function setEnvironmentData(key: unknown, value: unknown): void {
  if (!isMainThread) {
    throw new Error("setEnvironmentData is not supported in workers");
  }
  const cloned = structuredClone(value);
  environmentData.set(key, cloned);
  if (typeof globalThis.__lexeWorkerBroadcastEnv === "function") {
    globalThis.__lexeWorkerBroadcastEnv(key, cloned);
  }
}

export function markAsUntransferable(_object: unknown): void {}

export function markAsUncloneable(_object: unknown): void {}

export function isMarkedAsUntransferable(_object: unknown): boolean {
  return false;
}

const vmPortFactoryScript = `(function(bridge) {
  const { portId, portIdSym, portTransferSym, postMessage, attach, close, loopRef, toCloneErr } = bridge;
  class MessagePort {
    constructor() {
      this._onmessage = null;
      this.onmessageerror = null;
      this._listeners = new Map();
      this._onceWrappers = new WeakMap();
      this._closed = false;
      this._detached = false;
      this._started = false;
      this._attached = false;
      this._explicitUnref = false;
      this._loopRef = loopRef();
      Object.defineProperty(this, portIdSym, {
        value: portId,
        enumerable: false,
        configurable: true,
        writable: false,
      });
      Object.defineProperty(this, portTransferSym, {
        value: () => {
          if (this._detached) return true;
          this._detached = true;
          this._closed = true;
          this._loopRef.unref();
          return false;
        },
        enumerable: false,
        configurable: true,
        writable: false,
      });
    }
    get onmessage() { return this._onmessage; }
    set onmessage(h) {
      this._onmessage = typeof h === "function" ? h : null;
      if (this._onmessage) { this._started = true; this._attach(); }
    }
    _attach() {
      if (this._attached || this._closed || this._detached) return;
      this._attached = true;
      if (!this._explicitUnref) this._loopRef.ref();
      attach(portId, (data) => {
        if (this._closed || this._detached) return;
        this._dispatch(data);
      });
    }
    _dispatch(data) {
      if (this._closed || this._detached) return;
      if (typeof this._onmessage === "function") {
        try { this._onmessage({ data }); } catch {}
      }
      const set = this._listeners.get("message");
      if (set) {
        for (const fn of [...set]) {
          try { fn(data); } catch {}
        }
      }
      const evSet = this._listeners.get("__event__message");
      if (evSet) {
        for (const fn of [...evSet]) {
          try { fn({ type: "message", data, target: this, currentTarget: this }); } catch {}
        }
      }
    }
    postMessage(value, transfer) {
      if (this._closed || this._detached) return;
      try {
        postMessage(portId, value, transfer);
      } catch (err) {
        throw toCloneErr(err);
      }
    }
    start() { this._started = true; this._attach(); }
    close() {
      if (this._closed) return;
      this._closed = true;
      this._loopRef.unref();
      close(portId);
      const set = this._listeners.get("close");
      if (set) {
        for (const fn of [...set]) {
          try { fn(); } catch {}
        }
      }
    }
    on(ev, fn) {
      let set = this._listeners.get(ev);
      if (!set) { set = new Set(); this._listeners.set(ev, set); }
      set.add(fn);
      if (ev === "message") { this._started = true; this._attach(); }
      return this;
    }
    addListener(ev, fn) { return this.on(ev, fn); }
    once(ev, fn) {
      const w = (...args) => { this.off(ev, w); fn(...args); };
      this._onceWrappers.set(fn, w);
      return this.on(ev, w);
    }
    off(ev, fn) {
      const set = this._listeners.get(ev);
      if (set) {
        set.delete(fn);
        const w = this._onceWrappers.get(fn);
        if (w) { set.delete(w); this._onceWrappers.delete(fn); }
      }
      return this;
    }
    removeListener(ev, fn) { return this.off(ev, fn); }
    addEventListener(ev, fn) { return this.on("__event__" + ev, fn); }
    removeEventListener(ev, fn) { return this.off("__event__" + ev, fn); }
    ref() { this._explicitUnref = false; if (!this._closed && !this._detached) this._loopRef.ref(); return this; }
    unref() { this._explicitUnref = true; this._loopRef.unref(); return this; }
  }
  return new MessagePort();
})`;

export function moveMessagePortToContext(
  port: unknown,
  contextifiedSandbox: unknown
): unknown {
  if (
    !port ||
    typeof port !== "object" ||
    !(portIdSymbol in port) ||
    !(portTransferSymbol in port)
  ) {
    const err = new TypeError('The "port" argument must be an instance of MessagePort.');
    (err as { code?: string }).code = "ERR_INVALID_ARG_TYPE";
    throw err;
  }
  if (typeof globalThis.__lexeVmMovePort !== "function") {
    require("node:vm");
  }
  if (typeof globalThis.__lexeVmMovePort !== "function") {
    throw new Error("node:vm is unavailable for moveMessagePortToContext");
  }
  const portObj = port as Record<symbol, unknown>;
  const portId = portObj[portIdSymbol] as number;
  const detachFn = portObj[portTransferSymbol] as () => boolean;
  const alreadyDetached = detachFn.call(port);
  if (alreadyDetached) {
    throw new DOMException("MessagePort has already been detached", "DataCloneError");
  }
  globalThis.__lexePortDetachSink?.(portId);
  const peerId = globalThis.__lexePortPeer?.(portId);
  if (peerId !== undefined) {
    globalThis.__lexePortEnsureLocalAttached?.(peerId);
  }
  return globalThis.__lexeVmMovePort(contextifiedSandbox, vmPortFactoryScript, {
    portId,
    portIdSym: portIdSymbol,
    portTransferSym: portTransferSymbol,
    postMessage: (id: number, value: unknown, transfer?: unknown[]) =>
      globalThis.__lexePortSend?.(id, value, transferOptions(transfer)),
    attach: (id: number, cb: (data: unknown) => void) => globalThis.__lexePortAttach?.(id, cb),
    close: (id: number) => globalThis.__lexePortClose?.(id),
    loopRef: () =>
      typeof globalThis.__lexeCreateLoopRefController === "function"
        ? globalThis.__lexeCreateLoopRefController()
        : { ref() {}, unref() {} },
    toCloneErr: toDataCloneError,
  });
}

export async function postMessageToThread(
  targetThreadId: number,
  value: unknown,
  transferList?: unknown[],
  timeout?: number
): Promise<void> {
  if (typeof targetThreadId !== "number" || !Number.isInteger(targetThreadId) || targetThreadId < 0) {
    const err = new TypeError('The "threadId" argument must be a non-negative integer.');
    (err as { code?: string }).code = "ERR_INVALID_ARG_TYPE";
    throw err;
  }
  if (timeout !== undefined) {
    if (typeof timeout !== "number") {
      const err = new TypeError('The "timeout" argument must be of type number.');
      (err as { code?: string }).code = "ERR_INVALID_ARG_TYPE";
      throw err;
    }
    if (Number.isNaN(timeout) || timeout < 0) {
      const err = new RangeError('The "timeout" argument must be >= 0.');
      (err as { code?: string }).code = "ERR_OUT_OF_RANGE";
      throw err;
    }
  }
  if (typeof globalThis.__lexeWorkerPostToThread !== "function") {
    const err = new Error("Worker messaging is unavailable");
    (err as { code?: string }).code = "ERR_WORKER_MESSAGING_FAILED";
    throw err;
  }
  try {
    globalThis.__lexeWorkerPostToThread(
      targetThreadId,
      value,
      transferOptions(transferList)
    );
  } catch (err) {
    if (
      err &&
      typeof err === "object" &&
      "code" in err &&
      (err as { code?: string }).code === "ERR_WORKER_MESSAGING_FAILED"
    ) {
      throw err;
    }
    throw toDataCloneError(err);
  }
}

export function receiveMessageOnPort(_port: unknown): undefined {
  return undefined;
}

const defaultExport = {
  isMainThread,
  parentPort,
  workerData,
  threadId,
  SHARE_ENV,
  resourceLimits,
  Worker,
  MessagePort,
  MessageChannel,
  BroadcastChannel,
  getEnvironmentData,
  setEnvironmentData,
  markAsUntransferable,
  markAsUncloneable,
  isMarkedAsUntransferable,
  moveMessagePortToContext,
  postMessageToThread,
  receiveMessageOnPort,
};

export default defaultExport;
