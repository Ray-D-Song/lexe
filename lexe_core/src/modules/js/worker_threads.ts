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
export const workerData: unknown = isMainThread
  ? undefined
  : (globalThis as { workerData?: unknown }).workerData;

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

export interface WorkerOptions {
  workerData?: unknown;
  env?: Record<string, string>;
  argv?: string[];
  name?: string;
}

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

export class MessagePort {
  onmessage: ((ev: { data: unknown }) => void) | null = null;
  onmessageerror: ((ev: unknown) => void) | null = null;
  #peer: MessagePort | null = null;
  #listeners = new Map<string, Set<MessageListener>>();
  #closed = false;
  readonly #loopRef: LoopRefController;

  constructor() {
    this.#loopRef =
      typeof globalThis.__lexeCreateLoopRefController === "function"
        ? globalThis.__lexeCreateLoopRefController()
        : { ref() {}, unref() {} };
  }

  _setPeer(peer: MessagePort) {
    this.#peer = peer;
  }

  start(): void {}

  close(): void {
    if (this.#closed) return;
    this.#closed = true;
    this.#loopRef.unref();
    this.#dispatch("close", undefined);
  }

  postMessage(message: unknown, transfer?: unknown[]): void {
    if (this.#closed) return;

    let transferOptions: { transfer: unknown[] } | undefined;
    if (transfer != null) {
      if (!Array.isArray(transfer)) {
        throw new TypeError(
          'The "transferList" argument must be an instance of Array.'
        );
      }
      if (transfer.length > 0) {
        transferOptions = { transfer };
      }
    }

    let cloned: unknown;
    try {
      cloned = transferOptions
        ? structuredClone(message, transferOptions as object)
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
        this.onmessage({ data });
      } catch {
        // Node continues after onmessage throws; other listeners already ran.
      }
    }
  }

  ref(): this {
    this.#loopRef.ref();
    return this;
  }

  unref(): this {
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
  }

  removeEventListener(type: string, listener: MessageListener): void {
    this.#listeners.get(type)?.delete(listener);
  }

  on(type: string, listener: MessageListener): this {
    this.addEventListener(type, listener);
    return this;
  }

  off(type: string, listener: MessageListener): this {
    this.removeEventListener(type, listener);
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
  #exited = false;
  #handle: {
    postMessage: (data: unknown, transfer?: { transfer?: unknown[] }) => void;
    terminate: () => void;
    ref: () => void;
    unref: () => void;
  };

  constructor(filename: string | URL, options: WorkerOptions = {}) {
    super();
    if (typeof globalThis.__lexeWorkerSpawn !== "function") {
      throw new Error("worker_threads.Worker is unavailable in this build");
    }
    const path = filename instanceof URL ? filename.href : String(filename);
    const spawnOptions: Record<string, unknown> = {
      workerData: options.workerData ?? null,
      lexeEnvironmentSnapshot: environmentSnapshotForSpawn(),
    };
    if (options.env != null) spawnOptions.env = options.env;
    if (options.argv != null) spawnOptions.argv = options.argv;
    if (options.name != null) spawnOptions.name = options.name;
    if (options.eval) spawnOptions.eval = true;

    const handle = globalThis.__lexeWorkerSpawn(path, spawnOptions, (event, arg) => {
      if (event === "exit") this.#exited = true;
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

export class MessageChannel {
  port1: MessagePort;
  port2: MessagePort;
  constructor() {
    this.port1 = new MessagePort();
    this.port2 = new MessagePort();
    this.port1._setPeer(this.port2);
    this.port2._setPeer(this.port1);
  }
}

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

export function moveMessagePortToContext(
  _port: unknown,
  _context: unknown
): never {
  throw new Error(
    "worker_threads.moveMessagePortToContext is not implemented"
  );
}

export function receiveMessageOnPort(_port: unknown): undefined {
  return undefined;
}

export default {
  isMainThread,
  parentPort,
  workerData,
  threadId,
  Worker,
  MessagePort,
  MessageChannel,
  getEnvironmentData,
  setEnvironmentData,
  markAsUntransferable,
  markAsUncloneable,
  isMarkedAsUntransferable,
  moveMessagePortToContext,
  receiveMessageOnPort,
};
