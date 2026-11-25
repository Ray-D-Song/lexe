import process from "node:process";

import defaultImport from "node:process";
import legacyImport from "process";
import {
  chdir as namedChdir,
  cwd as namedCwd,
  id as namedId,
  pid as namedPid,
} from "node:process";
import { mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { spawnCapture } from "./test-utils";

it("node:process should be the same as process", () => {
  expect(defaultImport).toStrictEqual(legacyImport);
});

const {
  env,
  cwd,
  chdir,
  argv0,
  argv,
  platform,
  arch,
  hrtime,
  release,
  version,
  versions,
  exit,
  on,
  once,
  off,
  emit,
  pid,
  id,
} = defaultImport;

it("should have a process env", () => {
  expect(env).toEqual(process.env);
});

it("should have a process cwd", () => {
  expect(cwd()).toEqual(process.cwd());
});

describe("process.pid", () => {
  it("is a positive integer equal to process.id", () => {
    expect(typeof process.pid).toBe("number");
    expect(Number.isInteger(process.pid)).toBeTruthy();
    expect(process.pid).toBeGreaterThan(0);
    expect(process.pid).toBe(process.id);
    expect(pid).toBe(process.pid);
    expect(id).toBe(process.id);
    expect(namedPid).toBe(process.pid);
    expect(namedId).toBe(process.id);
  });

  it("is enumerable, non-writable, configurable (Node-compatible descriptor)", () => {
    const desc = Object.getOwnPropertyDescriptor(process, "pid");
    expect(desc).toBeDefined();
    expect(desc!.enumerable).toBe(true);
    expect(desc!.writable).toBe(false);
    expect(desc!.configurable).toBe(true);
    const original = process.pid;
    try {
      // @ts-expect-error pid is readonly
      process.pid = original + 1;
    } catch {
      // Strict-mode / QuickJS throw TypeError on non-writable assignment.
    }
    expect(process.pid).toBe(original);
  });

  it("keeps process.id as a writable legacy data property", () => {
    const desc = Object.getOwnPropertyDescriptor(process, "id");
    expect(desc).toBeDefined();
    expect(desc!.writable).toBe(true);
    expect(desc!.enumerable).toBe(true);
    const original = process.id;
    process.id = original + 1;
    expect(process.id).toBe(original + 1);
    process.id = original;
    expect(process.id).toBe(original);
    // pid is independent of later id writes after init
    expect(process.pid).toBe(original);
  });
});

describe("process.chdir", () => {
  it("changes and restores the process cwd (absolute and relative)", () => {
    const original = process.cwd();
    const tempRoot = join(original, `.chdir-test-${process.pid}-${Date.now()}`);
    const child = join(tempRoot, "child");
    mkdirSync(child, { recursive: true });

    try {
      chdir(original);
      expect(cwd()).toBe(original);

      process.chdir(tempRoot);
      expect(process.cwd()).toBe(tempRoot);
      expect(namedCwd()).toBe(tempRoot);

      process.chdir("child");
      expect(process.cwd()).toBe(child);

      process.chdir("..");
      expect(process.cwd()).toBe(tempRoot);

      namedChdir(original);
      expect(process.cwd()).toBe(original);
    } finally {
      try {
        process.chdir(original);
      } catch {
        // ignore restore failure so cleanup still runs
      }
      rmSync(tempRoot, { recursive: true, force: true });
    }
  });

  it("throws ENOENT for missing paths with code/path/syscall", () => {
    const original = process.cwd();
    const missing = join(original, `.chdir-missing-${process.pid}-${Date.now()}`);
    try {
      process.chdir(missing);
      throw new Error("expected chdir to throw");
    } catch (err: any) {
      expect(err.code).toBe("ENOENT");
      expect(err.path).toBe(missing);
      expect(err.syscall).toBe("chdir");
      expect(String(err.message)).toContain("ENOENT");
      expect(String(err.message)).toContain("chdir");
    } finally {
      process.chdir(original);
    }
  });

  it("throws ENOTDIR when path is a file", () => {
    const original = process.cwd();
    const filePath = join(original, `.chdir-file-${process.pid}-${Date.now()}`);
    writeFileSync(filePath, "not-a-dir");
    try {
      process.chdir(filePath);
      throw new Error("expected chdir to throw");
    } catch (err: any) {
      // Node uses ENOTDIR; some platforms may report a related code.
      expect(["ENOTDIR", "ENOENT", "EINVAL", "UNKNOWN"]).toContain(err.code);
      expect(err.path).toBe(filePath);
      expect(err.syscall).toBe("chdir");
    } finally {
      try {
        process.chdir(original);
      } catch {
        // ignore
      }
      rmSync(filePath, { force: true });
    }
  });

  it("throws TypeError for non-string directory arguments", () => {
    const original = process.cwd();
    try {
      // @ts-expect-error intentional invalid arg
      expect(() => process.chdir(undefined)).toThrow(TypeError);
      // @ts-expect-error intentional invalid arg
      expect(() => process.chdir(42)).toThrow(TypeError);
      // @ts-expect-error intentional invalid arg
      expect(() => process.chdir({})).toThrow(TypeError);
      // @ts-expect-error intentional invalid arg
      expect(() => process.chdir()).toThrow(TypeError);
    } finally {
      process.chdir(original);
    }
  });
});

it("should have a process argv0", () => {
  expect(argv0).toEqual(process.argv0);
});

it("should have a process argv", () => {
  expect(argv).toEqual(process.argv);
});

it("should have a process platform", () => {
  expect(platform).toEqual(process.platform);
});

it("should have a process arch", () => {
  expect(arch).toEqual(process.arch);
});

it("should have a process hrtime", () => {
  expect(hrtime.bigint() > 0).toBeTruthy();
});

it("should have a process release", () => {
  expect(release).toEqual(process.release);
});

it("should have a process version", () => {
  expect(version).toEqual(process.version);
});

it("should have a process versions", () => {
  expect(versions).toEqual(process.versions);
});

it("should have a process exit", () => {
  expect(exit).toEqual(process.exit);
});

describe("Node compat identity", () => {
  it("should advertise Node 24.3.0 identity", () => {
    expect(process.version).toBe("v24.3.0");
    expect(process.versions.node).toBe("24.3.0");
    expect(typeof process.versions.lexe).toBe("string");
    expect(process.versions.lexe.length > 0).toBeTruthy();
    expect(process.release.name).toBe("lexe");
  });
});

describe("EventEmitter methods", () => {
  it("should register, emit, and remove listeners with on/once/off/emit", () => {
    const seen: number[] = [];
    const listener = (n: number) => {
      seen.push(n);
    };

    process.on("compat-test", listener);
    process.emit("compat-test", 1);
    process.off("compat-test", listener);
    process.emit("compat-test", 2);
    expect(seen).toEqual([1]);
  });

  it("once should only fire once", () => {
    let count = 0;
    process.once("compat-once", () => {
      count += 1;
    });
    process.emit("compat-once");
    process.emit("compat-once");
    expect(count).toBe(1);
  });

  it("process.exit should emit exit synchronously before terminating", async () => {
    const { code, stderr } = await spawnCapture(process.argv[0], [
      "-e",
      `
        process.on("exit", (code) => {
          console.error("exit:" + code);
        });
        process.exit(42);
      `,
    ]);
    expect(code).toBe(42);
    expect(stderr).toContain("exit:42");
  });

  it("should not expose internal test helpers on process", () => {
    expect((process as any).__emitExit).toBeUndefined();
    expect(Object.keys(process).includes("__emitExit")).toBeFalsy();
  });

  it("EventEmitter methods should not be own enumerable properties", () => {
    expect(Object.prototype.hasOwnProperty.call(process, "on")).toBeFalsy();
    expect(Object.keys(process).includes("on")).toBeFalsy();
    expect(typeof process.on).toBe("function");
  });

  it("dynamic import should ignore user-added enumerable process fields", async () => {
    (process as any).userField = 123;
    try {
      const mod = await import("process");
      expect(mod.default).toBe(process);
      expect((mod as any).userField).toBeUndefined();
      expect(mod.on).toBe(process.on);
    } finally {
      delete (process as any).userField;
    }
  });

  it("global process and node:process should share event methods", () => {
    expect(process.on).toBe(on);
    expect(process.once).toBe(once);
    expect(process.off).toBe(off);
    expect(process.emit).toBe(emit);
    expect(defaultImport.on).toBe(process.on);
    expect(defaultImport).toBe(process);
    expect(globalThis.process).toBe(process);
  });
});

describe("unhandledRejection timing", () => {
  it("should not emit unhandledRejection when catch attaches in the same turn", async () => {
    let seen = 0;
    const listener = () => {
      seen += 1;
    };
    process.on("unhandledRejection", listener);
    try {
      const p = Promise.reject(new Error("handled"));
      p.catch(() => {});
      await new Promise((r) => setImmediate(r));
      expect(seen).toBe(0);
    } finally {
      process.off("unhandledRejection", listener);
    }
  });

  it("should emit unhandledRejection for truly unhandled rejections", async () => {
    let seen = 0;
    const listener = () => {
      seen += 1;
    };
    process.on("unhandledRejection", listener);
    try {
      Promise.reject(new Error("unhandled"));
      await new Promise((r) => setImmediate(r));
      expect(seen).toBe(1);
    } finally {
      process.off("unhandledRejection", listener);
    }
  });

  it("should not leak rejection tracker helpers on globalThis", () => {
    expect((globalThis as any).__lexeRejectionPending).toBeUndefined();
    expect((globalThis as any).__lexeRejectionSchedule).toBeUndefined();
  });
});

describe("process lifecycle drain", () => {
  it("emits exit on natural drain without process.exit", async () => {
    const { code, stderr } = await spawnCapture(process.argv[0], [
      "-e",
      `process.on("exit", (c) => { console.error("exit:" + c); });`,
    ]);
    expect(code).toBe(0);
    expect(stderr).toContain("exit:0");
  });

  it("emits beforeExit when the loop would otherwise drain", async () => {
    const { code, stderr } = await spawnCapture(process.argv[0], [
      "-e",
      `
        let sawBeforeExit = false;
        process.on("beforeExit", () => { sawBeforeExit = true; });
        process.on("exit", () => {
          console.error("beforeExit:" + sawBeforeExit);
        });
      `,
    ]);
    expect(code).toBe(0);
    expect(stderr).toContain("beforeExit:true");
  });

  it("waits for referenced timers before exit", async () => {
    const start = Date.now();
    const { code, stderr } = await spawnCapture(process.argv[0], [
      "-e",
      `
        process.on("exit", (c) => console.error("exit:" + c));
        setTimeout(() => {}, 80).ref();
      `,
    ]);
    expect(code).toBe(0);
    expect(stderr).toContain("exit:0");
    expect(Date.now() - start).toBeGreaterThanOrEqual(40);
  });
});

describe("extended Node process APIs", () => {
  it("exposes execPath, execArgv, ppid, uptime, title, config, features, allowedNodeEnvironmentFlags", async () => {
    expect(typeof process.execPath).toBe("string");
    expect(process.execPath.length).toBeGreaterThan(0);
    expect(Array.isArray(process.execArgv)).toBe(true);
    expect(typeof process.ppid).toBe("number");
    expect(process.ppid).toBeGreaterThanOrEqual(0);
    const ppidDesc = Object.getOwnPropertyDescriptor(process, "ppid");
    expect(ppidDesc?.enumerable).toBe(true);
    expect(ppidDesc?.writable).toBe(false);

    expect(typeof process.uptime()).toBe("number");
    expect(process.uptime()).toBeGreaterThanOrEqual(0);

    const oldTitle = process.title;
    process.title = "lexe-test-title";
    expect(process.title).toBe("lexe-test-title");
    process.title = oldTitle;

    expect(typeof process.config).toBe("object");
    expect(typeof process.config.variables).toBe("object");
    expect(process.features.tls).toBe(true);
    expect(process.allowedNodeEnvironmentFlags instanceof Set).toBe(true);
    expect(process.allowedNodeEnvironmentFlags.has("--no-warnings")).toBe(true);
    expect(process.allowedNodeEnvironmentFlags.has("--permission")).toBe(true);

    const { code, stdout } = await spawnCapture(process.argv[0], [
      "--no-warnings",
      "--enable-source-maps",
      "-e",
      "console.log(JSON.stringify({ execArgv: process.execArgv, sm: process.sourceMapsEnabled }))",
    ]);
    expect(code).toBe(0);
    expect(JSON.parse(stdout.trim())).toEqual({
      execArgv: ["--no-warnings", "--enable-source-maps"],
      sm: true,
    });
  });

  it("reports memoryUsage(), memoryUsage.rss(), cpuUsage(), resourceUsage(), constrainedMemory(), availableMemory()", () => {
    const mem = process.memoryUsage();
    expect(mem.rss).toBeGreaterThan(0);
    expect(mem.heapTotal).toBeGreaterThan(0);
    expect(mem.heapUsed).toBeGreaterThan(0);
    expect(typeof mem.external).toBe("number");
    expect(typeof mem.arrayBuffers).toBe("number");
    expect(process.memoryUsage.rss()).toBeGreaterThan(0);

    const c1 = process.cpuUsage();
    expect(c1.user).toBeGreaterThanOrEqual(0);
    expect(c1.system).toBeGreaterThanOrEqual(0);
    const c2 = process.cpuUsage(c1);
    expect(c2.user).toBeGreaterThanOrEqual(0);
    expect(c2.system).toBeGreaterThanOrEqual(0);
    expect(() => process.cpuUsage("bad" as any)).toThrow();
    expect(() => process.cpuUsage({ user: -1, system: 0 })).toThrow();

    const ru = process.resourceUsage();
    for (const key of [
      "userCPUTime",
      "systemCPUTime",
      "maxRSS",
      "minorPageFault",
      "majorPageFault",
      "voluntaryContextSwitches",
      "involuntaryContextSwitches",
    ]) {
      expect(typeof (ru as any)[key]).toBe("number");
    }

    expect(typeof process.constrainedMemory()).toBe("number");
    expect(process.constrainedMemory()).toBeGreaterThanOrEqual(0);
    expect(typeof process.availableMemory()).toBe("number");
    expect(process.availableMemory()).toBeGreaterThanOrEqual(0);
  });

  it("supports umask(), ref/unref helpers, getActiveResourcesInfo(), and synced sourceMapsEnabled", () => {
    const origMask = process.umask();
    try {
      const prev1 = process.umask(0o027);
      expect(prev1).toBe(origMask);
      expect(process.umask()).toBe(0o027);
      const prev2 = process.umask("022");
      expect(prev2).toBe(0o027);
      expect(process.umask()).toBe(0o022);
    } finally {
      process.umask(origMask);
    }
    expect(() => process.umask("not-octal")).toThrow();

    let refCalls = 0;
    let unrefCalls = 0;
    const handle = {
      ref() {
        refCalls += 1;
      },
      unref() {
        unrefCalls += 1;
      },
    };
    process.ref(handle);
    process.unref(handle);
    process.ref(null);
    process.unref(123);
    expect(refCalls).toBe(1);
    expect(unrefCalls).toBe(1);

    const timer = setTimeout(() => {}, 5000);
    try {
      const active = process.getActiveResourcesInfo();
      expect(Array.isArray(active)).toBe(true);
      expect(active.length).toBeGreaterThan(0);
    } finally {
      clearTimeout(timer);
    }

    const req = createRequire(import.meta.url);
    const mod = req("node:module");
    const origSm = process.sourceMapsEnabled;
    try {
      process.setSourceMapsEnabled(true);
      expect(process.sourceMapsEnabled).toBe(true);
      expect(mod.getSourceMapsSupport().enabled).toBe(true);
      mod.setSourceMapsSupport(false);
      expect(process.sourceMapsEnabled).toBe(false);
      expect(() => process.setSourceMapsEnabled("yes" as any)).toThrow();
    } finally {
      process.setSourceMapsEnabled(origSm);
    }
  });

  it("loads .env files without overwriting existing env vars and throws ENOENT when missing", () => {
    const envPath = join(tmpdir(), `lexe-env-${process.pid}-${Date.now()}.env`);
    writeFileSync(
      envPath,
      [
        "# leading comment",
        "LEXE_TEST_PLAIN=hello # inline comment",
        'export LEXE_TEST_EXPORTED="line1\\nline2"',
        "LEXE_TEST_SINGLE='literal # not comment'",
        "LEXE_TEST_EXISTING=from-file",
      ].join("\n")
    );
    process.env.LEXE_TEST_EXISTING = "already-set";
    try {
      process.loadEnvFile(envPath);
      expect(process.env.LEXE_TEST_PLAIN).toBe("hello");
      expect(process.env.LEXE_TEST_EXPORTED).toBe("line1\nline2");
      expect(process.env.LEXE_TEST_SINGLE).toBe("literal # not comment");
      expect(process.env.LEXE_TEST_EXISTING).toBe("already-set");
    } finally {
      delete process.env.LEXE_TEST_PLAIN;
      delete process.env.LEXE_TEST_EXPORTED;
      delete process.env.LEXE_TEST_SINGLE;
      delete process.env.LEXE_TEST_EXISTING;
      rmSync(envPath, { force: true });
    }

    let missingErr: any;
    try {
      process.loadEnvFile(envPath + ".missing");
    } catch (e) {
      missingErr = e;
    }
    expect(missingErr?.code).toBe("ENOENT");
    expect(missingErr?.syscall).toBe("open");
  });

  it("supports report.getReport() and report.writeReport()", () => {
    const reportObj = process.report.getReport(new Error("sample-err"));
    expect(reportObj.header.reportVersion).toBe(5);
    expect(reportObj.header.nodejsVersion).toBe(process.version);
    expect(reportObj.javascriptStack.message).toContain("sample-err");
    expect(typeof reportObj.resourceUsage.maxRSS).toBe("number");

    const outPath = join(tmpdir(), `lexe-report-${process.pid}-${Date.now()}.json`);
    try {
      const written = process.report.writeReport(outPath);
      expect(written).toBe(outPath);
      const parsed = JSON.parse(readFileSync(outPath, "utf8"));
      expect(parsed.header.trigger).toBe("WriteReport");
      expect(parsed.header.filename).toBe(outPath);
    } finally {
      rmSync(outPath, { force: true });
    }
  });

  it("supports setUncaughtExceptionCaptureCallback and finalization in subprocesses", async () => {
    expect(process.hasUncaughtExceptionCaptureCallback()).toBe(false);
    expect(() => process.setUncaughtExceptionCaptureCallback("bad" as any)).toThrow();
    const dummy = () => {};
    process.setUncaughtExceptionCaptureCallback(dummy);
    expect(process.hasUncaughtExceptionCaptureCallback()).toBe(true);
    let dupErr: any;
    try {
      process.setUncaughtExceptionCaptureCallback(() => {});
    } catch (e) {
      dupErr = e;
    }
    expect(dupErr?.code).toBe("ERR_UNCAUGHT_EXCEPTION_CAPTURE_ALREADY_SET");
    process.setUncaughtExceptionCaptureCallback(null);
    expect(process.hasUncaughtExceptionCaptureCallback()).toBe(false);

    const syncCap = await spawnCapture(process.argv[0], [
      "-e",
      `
        process.setUncaughtExceptionCaptureCallback((err) => {
          console.log("sync-captured:" + err.message);
        });
        throw new Error("boom-sync");
      `,
    ]);
    expect(syncCap.code).toBe(0);
    expect(syncCap.stdout.trim()).toBe("sync-captured:boom-sync");

    const asyncCap = await spawnCapture(process.argv[0], [
      "-e",
      `
        process.setUncaughtExceptionCaptureCallback((err) => {
          console.log("async-captured:" + err.message);
        });
        setTimeout(() => { throw new Error("boom-async"); }, 10);
      `,
    ]);
    expect(asyncCap.code).toBe(0);
    expect(asyncCap.stdout.trim()).toBe("async-captured:boom-async");

    const finRes = await spawnCapture(process.argv[0], [
      "-e",
      `
        const events = [];
        const a = { name: "a" };
        const b = { name: "b" };
        const c = { name: "c" };
        process.finalization.register(a, (t, ev) => events.push(t.name + ":" + ev));
        process.finalization.registerBeforeExit(b, (t, ev) => events.push(t.name + ":" + ev));
        process.finalization.register(c, (t, ev) => events.push(t.name + ":" + ev));
        process.finalization.unregister(c);
        process.on("exit", () => console.error("fin:" + events.join(",")));
      `,
    ]);
    expect(finRes.code).toBe(0);
    expect(finRes.stderr).toContain("fin:");
    expect(finRes.stderr).toContain("b:beforeExit");
    expect(finRes.stderr).toContain("b:exit");
    expect(finRes.stderr).toContain("a:exit");
    expect(finRes.stderr).not.toContain("c:");
  });

  it("exposes process.permission only under --permission and enforces fs/child restrictions", async () => {
    expect(process.permission).toBeUndefined();

    const dir = join(tmpdir(), `lexe-perm-${process.pid}-${Date.now()}`);
    mkdirSync(dir, { recursive: true });
    const allowedFile = join(dir, "ok.txt");
    const deniedFile = join(tmpdir(), `lexe-perm-denied-${process.pid}-${Date.now()}.txt`);
    writeFileSync(allowedFile, "allowed-content");
    writeFileSync(deniedFile, "denied-content");

    try {
      const { code, stdout } = await spawnCapture(process.argv[0], [
        "--permission",
        `--allow-fs-read=${dir}`,
        "-e",
        `
          const fs = require("node:fs");
          const cp = require("node:child_process");
          const out = {
            hasReadOk: process.permission.has("fs.read", ${JSON.stringify(allowedFile)}),
            hasReadDenied: process.permission.has("fs.read", ${JSON.stringify(deniedFile)}),
            hasWrite: process.permission.has("fs.write", ${JSON.stringify(allowedFile)}),
            hasChild: process.permission.has("child"),
            readOk: fs.readFileSync(${JSON.stringify(allowedFile)}, "utf8"),
            readDeniedCode: null,
            writeDeniedCode: null,
            childDeniedCode: null,
          };
          try { fs.readFileSync(${JSON.stringify(deniedFile)}, "utf8"); } catch (e) { out.readDeniedCode = e.code; }
          try { fs.writeFileSync(${JSON.stringify(allowedFile)}, "nope"); } catch (e) { out.writeDeniedCode = e.code; }
          try { cp.spawn("echo", ["hi"]); } catch (e) { out.childDeniedCode = e.code; }
          console.log(JSON.stringify(out));
        `,
      ]);
      expect(code).toBe(0);
      expect(JSON.parse(stdout.trim())).toEqual({
        hasReadOk: true,
        hasReadDenied: false,
        hasWrite: false,
        hasChild: false,
        readOk: "allowed-content",
        readDeniedCode: "ERR_ACCESS_DENIED",
        writeDeniedCode: "ERR_ACCESS_DENIED",
        childDeniedCode: "ERR_ACCESS_DENIED",
      });
    } finally {
      rmSync(dir, { recursive: true, force: true });
      rmSync(deniedFile, { force: true });
    }
  });
});
