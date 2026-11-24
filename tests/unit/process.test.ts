import process from "node:process";

import defaultImport from "node:process";
import legacyImport from "process";
import {
  chdir as namedChdir,
  cwd as namedCwd,
  id as namedId,
  pid as namedPid,
} from "node:process";
import { mkdirSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
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
