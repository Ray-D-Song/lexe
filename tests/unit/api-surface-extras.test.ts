import consoleMod from "node:console";
import eventsMod, { EventEmitter } from "node:events";
import fsMod from "node:fs";
import httpMod from "node:http";
import netMod from "node:net";
import perfHooksMod from "node:perf_hooks";
import processMod from "node:process";
import streamMod from "node:stream";
import streamPromises from "node:stream/promises";
import utilMod from "node:util";
import workerThreadsMod from "node:worker_threads";
import zlibMod from "node:zlib";
import { Writable } from "node:stream";

describe("API-surface parity extras", () => {
  it("supports console.context, console.createTask, and internal stream fields", () => {
    const mod = consoleMod as unknown as Record<string, any>;
    expect(mod._ignoreErrors).toBe(true);
    expect(mod._stdout).toBe(processMod.stdout);
    expect(mod._stderr).toBe(processMod.stderr);
    expect(mod._times instanceof Map).toBe(true);
    expect(typeof mod._stdoutErrorHandler).toBe("function");
    expect(typeof mod._stderrErrorHandler).toBe("function");

    const chunks: string[] = [];
    const sink = new Writable({
      write(chunk, _enc, cb) {
        chunks.push(chunk.toString());
        cb();
      },
    });
    const ctxConsole = mod.context({ stdout: sink, stderr: sink });
    expect(ctxConsole._stdout).toBe(sink);
    expect(ctxConsole._stderr).toBe(sink);
    expect(ctxConsole._ignoreErrors).toBe(true);
    ctxConsole.log("hello-context");
    expect(chunks.join("")).toContain("hello-context");

    const task = mod.createTask("unit-task");
    expect(task.run(() => 42)).toBe(42);
    expect(() => mod.createTask(123)).toThrow();
    expect(() => task.run("not-a-fn")).toThrow();
  });

  it("supports events.init, EventTarget listener symbols, and usingDomains", () => {
    const ev = eventsMod as unknown as Record<string, any>;
    expect(typeof ev.kMaxEventTargetListeners).toBe("symbol");
    expect(typeof ev.kMaxEventTargetListenersWarned).toBe("symbol");
    expect(ev.usingDomains).toBe(false);
    expect(ev.init).toBe((EventEmitter as unknown as Record<string, any>).init);

    const target: Record<string, any> = {};
    ev.init.call(target);
    expect(target._events).toBeDefined();
    expect(target._eventsCount).toBe(0);
  });

  it("exposes top-level fs access constants matching fs.constants", () => {
    const fsAny = fsMod as unknown as Record<string, number>;
    expect(fsAny.F_OK).toBe(fsMod.constants.F_OK);
    expect(fsAny.R_OK).toBe(fsMod.constants.R_OK);
    expect(fsAny.W_OK).toBe(fsMod.constants.W_OK);
    expect(fsAny.X_OK).toBe(fsMod.constants.X_OK);
  });

  it("exposes http.CloseEvent, http.MessageEvent, and http._connectionListener", () => {
    const httpAny = httpMod as unknown as Record<string, any>;
    expect(httpAny.CloseEvent).toBe((globalThis as Record<string, any>).CloseEvent);
    expect(httpAny.MessageEvent).toBe((globalThis as Record<string, any>).MessageEvent);
    expect(typeof httpAny._connectionListener).toBe("function");

    const fakeSocket: Record<string, any> = {};
    const fakeServer = { timeout: 0 };
    httpAny._connectionListener.call(fakeServer, fakeSocket);
    expect(fakeSocket.server).toBe(fakeServer);
  });

  it("exposes net.Stream and net._normalizeArgs", () => {
    const netAny = netMod as unknown as Record<string, any>;
    expect(netAny.Stream).toBe(netMod.Socket);

    const cb = () => {};
    const [tcpOpts, tcpCb] = netAny._normalizeArgs([8080, "127.0.0.1", cb]);
    expect(tcpOpts).toEqual({ port: 8080, host: "127.0.0.1" });
    expect(tcpCb).toBe(cb);

    const [pipeOpts, pipeCb] = netAny._normalizeArgs(["/tmp/lexe.sock"]);
    expect(pipeOpts).toEqual({ path: "/tmp/lexe.sock" });
    expect(pipeCb).toBeNull();
  });

  it("exposes perf_hooks entry type and milestone constants", () => {
    const c = perfHooksMod.constants as unknown as Record<string, number>;
    expect(c.NODE_PERFORMANCE_ENTRY_TYPE_GC).toBe(0);
    expect(c.NODE_PERFORMANCE_ENTRY_TYPE_HTTP).toBe(1);
    expect(c.NODE_PERFORMANCE_ENTRY_TYPE_HTTP2).toBe(2);
    expect(c.NODE_PERFORMANCE_ENTRY_TYPE_NET).toBe(3);
    expect(c.NODE_PERFORMANCE_ENTRY_TYPE_DNS).toBe(4);
    expect(c.NODE_PERFORMANCE_MILESTONE_TIME_ORIGIN_TIMESTAMP).toBe(0);
    expect(c.NODE_PERFORMANCE_MILESTONE_TIME_ORIGIN).toBe(1);
    expect(c.NODE_PERFORMANCE_MILESTONE_ENVIRONMENT).toBe(2);
    expect(c.NODE_PERFORMANCE_MILESTONE_NODE_START).toBe(3);
    expect(c.NODE_PERFORMANCE_MILESTONE_V8_START).toBe(4);
    expect(c.NODE_PERFORMANCE_MILESTONE_LOOP_START).toBe(5);
    expect(c.NODE_PERFORMANCE_MILESTONE_LOOP_EXIT).toBe(6);
    expect(c.NODE_PERFORMANCE_MILESTONE_BOOTSTRAP_COMPLETE).toBe(7);
  });

  it("exposes process.debugPort, openStdin, reallyExit, threadCpuUsage, and Unix group helpers", () => {
    const proc = processMod as unknown as Record<string, any>;
    expect(proc.debugPort).toBe(9229);
    expect(typeof proc.reallyExit).toBe("function");
    expect(proc.openStdin()).toBe(processMod.stdin);

    const t0 = proc.threadCpuUsage();
    expect(typeof t0.user).toBe("number");
    expect(typeof t0.system).toBe("number");
    const t1 = proc.threadCpuUsage(t0);
    expect(t1.user).toBeGreaterThanOrEqual(0);
    expect(t1.system).toBeGreaterThanOrEqual(0);

    if (processMod.platform !== "win32") {
      const groups = proc.getgroups();
      expect(Array.isArray(groups)).toBe(true);
      expect(groups.every((g: unknown) => typeof g === "number")).toBe(true);
      expect(typeof proc.setgroups).toBe("function");
      expect(typeof proc.initgroups).toBe("function");
    }
  });

  it("exposes stream.promises and internal buffer helpers", () => {
    const st = streamMod as unknown as Record<string, any>;
    expect(st.promises).toBe(streamPromises);
    const bytes = new Uint8Array([10, 20, 30, 40]).subarray(1, 3);
    expect(st._isArrayBufferView(bytes)).toBe(true);
    expect(st._isUint8Array(bytes)).toBe(true);
    const buf = st._uint8ArrayToBuffer(bytes);
    expect(Buffer.isBuffer(buf)).toBe(true);
    expect(Array.from(buf)).toEqual([20, 30]);
  });

  it("exposes util.isArray, util.getCallSite, _errnoException, and _exceptionWithHostPort", () => {
    const u = utilMod as unknown as Record<string, any>;
    expect(u.isArray([1, 2])).toBe(true);
    expect(u.isArray({})).toBe(false);

    const sites = u.getCallSite(2);
    expect(Array.isArray(sites)).toBe(true);
    expect(sites.length).toBeGreaterThan(0);

    const e1 = u._errnoException(-2, "open", "'/missing'");
    expect(e1.errno).toBe(-2);
    expect(e1.code).toBe("ENOENT");
    expect(e1.syscall).toBe("open");
    expect(e1.message).toContain("ENOENT");

    const e2 = u._exceptionWithHostPort(-111, "connect", "127.0.0.1", 8080, "0.0.0.0:0");
    expect(e2.errno).toBe(-111);
    expect(e2.code).toBe("ECONNREFUSED");
    expect(e2.address).toBe("127.0.0.1");
    expect(e2.port).toBe(8080);
  });

  it("reports main-thread workerData as null and isInternalThread as false", () => {
    const wt = workerThreadsMod as unknown as Record<string, any>;
    expect(wt.workerData).toBeNull();
    expect(wt.isInternalThread).toBe(false);
  });

  it("exposes zlib.codes, top-level constants, BROTLI_*, and ZSTD_* constants", () => {
    const z = zlibMod as unknown as Record<string, any>;
    expect(z.DEFLATE).toBe(1);
    expect(z.GZIP).toBe(3);
    expect(z.UNZIP).toBe(7);
    expect(z.ZLIB_VERNUM).toBe(4880);
    expect(z.ZSTD_COMPRESS).toBe(10);
    expect(z.ZSTD_DECOMPRESS).toBe(11);
    expect(z.codes["0"]).toBe("Z_OK");
    expect(z.codes.Z_DATA_ERROR).toBe(-3);
    expect(Object.isFrozen(z.codes)).toBe(true);

    const c = zlibMod.constants as unknown as Record<string, number>;
    expect(c.BROTLI_ENCODE).toBe(9);
    expect(c.BROTLI_DECODE).toBe(8);
    expect(c.ZSTD_CLEVEL_DEFAULT).toBe(3);
    expect(c.ZSTD_c_compressionLevel).toBe(100);
    expect(c.ZSTD_e_end).toBe(2);
  });
});
