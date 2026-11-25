import defaultImport from "node:child_process";
import legacyImport from "child_process";

import { openSync, closeSync, readFileSync, rmSync } from "node:fs";
import { join } from "node:path";
import { platform, tmpdir } from "node:os";
import process from "node:process";
const IS_WINDOWS = platform() === "win32";

it("node:child_process should be the same as child_process", () => {
  expect(defaultImport).toStrictEqual(legacyImport);
});

const { spawn, spawnSync, exec, execFile, execSync, execFileSync, fork, ChildProcess } =
  defaultImport;

describe("spawn", () => {
  it("should spawn a child process", (done) => {
    // Use cross-platform commands
    const command = IS_WINDOWS ? "cmd" : "ls";
    const args = IS_WINDOWS ? ["/c", "dir"] : ["-l"];
    const child = spawn(command, args);
    child.on("exit", (code) => {
      try {
        expect(code).toEqual(0);
        done();
      } catch (error) {
        done(error);
      }
    });
  });
  it("should spawn in a diffrent directory", (done) => {
    // Use cross-platform commands for getting current directory
    const child = IS_WINDOWS
      ? spawn("cmd", ["/c", "cd"], { cwd: "./tests" })
      : spawn("pwd", { cwd: "./tests" });
    let output = "";
    child.stdout.on("data", (data) => {
      output += data.toString();
    });

    child.on("close", (code) => {
      try {
        // Split by either / or \ for cross-platform compatibility
        const dir = output.trim().split(/[/\\]/).at(-1);
        expect(dir).toEqual("tests");
        expect(code).toEqual(0);
        done();
      } catch (error) {
        done(error);
      }
    });
  });
  it("should capture output from the child process", (done) => {
    const command = "echo";
    const args = ["Hello, World!"];
    const child = spawn(command, args);
    let output = "";
    child.stdout.on("data", (data) => {
      output += data.toString();
    });

    child.on("close", (code) => {
      try {
        expect(output.trim()).toEqual(args[0]);
        expect(code).toEqual(0);
        done();
      } catch (error) {
        done(error);
      }
    });
  });

  it("should send input to the child process", (done) => {
    // Use cross-platform approach: Windows uses 'findstr .*' to echo all input, Unix uses 'cat'
    const command = IS_WINDOWS ? "findstr" : "cat";
    const args = IS_WINDOWS ? [".*"] : [];
    const input = "Hello, world!";
    const child = spawn(command, args);

    child.stdin.write(input);
    child.stdin.end();

    let output = "";
    child.stdout.on("data", (data) => {
      output += data.toString();
    });

    child.on("close", (code) => {
      try {
        expect(code).toEqual(0);
        expect(output.trim()).toEqual(input);
        done();
      } catch (error) {
        done(error);
      }
    });
  });

  it("should handle errors from the child process", (done) => {
    if (process.env._VIRTUAL_ENV) {
      //QEMU spawns nonexistent-command successfully
      return done();
    }
    const command = "nonexistent-command";
    const child = spawn(command);
    child.on("error", (err) => {
      try {
        expect(err).toBeTruthy();
        done();
      } catch (error) {
        done(error);
      }
    });
  });

  it("should handle child process termination", (done) => {
    // Use cross-platform long-running command: Windows uses 'ping -n 999 localhost', Unix uses 'sleep 999'
    const child = IS_WINDOWS
      ? spawn("ping", ["-n", "999", "localhost"])
      : spawn("sleep", ["999"]);

    child.on("exit", (code, signal) => {
      try {
        if (IS_WINDOWS) {
          // lexe on Windows returns exit code 1 when killed
          expect(code).toEqual(1);
        } else {
          expect(code).toBeNull();
        }
        expect(signal).toEqual("SIGKILL");
        done();
      } catch (error) {
        done(error);
      }
    });

    setTimeout(() => {
      child.kill("SIGKILL"); //SIGINT does not forward to children on Linux
    }, 50);
  });

  it("should handle child process stdio inherit", (done) => {
    const child = spawn("echo", ["123"], { stdio: "inherit" });
    child.on("exit", (code) => {
      try {
        expect(code).toEqual(0);
        done();
      } catch (error) {
        done(error);
      }
    });
  });
  it("should handle child process stdio ignore", (done) => {
    const child = spawn("echo", ["123"], { stdio: "ignore" });
    child.on("exit", (code) => {
      try {
        expect(code).toEqual(0);
        done();
      } catch (error) {
        done(error);
      }
    });
  });

  it("should have a process exitCode", async () => {
    const testExitCode = async (
      exitCodeValue: number | string,
      expectedCode: number
    ) => {
      const proc = spawn(process.argv0, [
        "-e",
        `process.exitCode = ${exitCodeValue}`,
      ]);
      await new Promise<void>((resolve) => {
        proc.on("exit", (code) => {
          expect(code).toEqual(expectedCode);
          resolve();
        });
      });
    };

    await testExitCode(241212341, 181);
    await testExitCode(-1, 255);
    await testExitCode(1, 1);
    await testExitCode(-1231231231, 1);
    await testExitCode(266, 10);
    await testExitCode("266", 10);
  });

  // Skip on Windows - detached process behavior differs significantly
  (IS_WINDOWS ? it.skip : it)(
    "should handle detached child process termination",
    (done) => {
      const sleepCmd = "spawn('sleep', ['999']";

      const parentProc = spawn(process.argv0, [
        "-e",
        `
        import {spawn} from "child_process";
        const child = ${sleepCmd}, {
          detached: true,
          stdio: 'ignore'
        });
        console.log(child.pid.toString());
      `,
      ]);

      let detachedPidString = "";
      parentProc.stdout.on("data", (data) => {
        detachedPidString += data.toString();
        // Kill parent once we have the PID - parent would otherwise wait for detached child
        parentProc.kill();
      });

      parentProc.on("error", (err) => {
        done(err);
      });

      parentProc.on("close", () => {
        try {
          const detachedPid = parseInt(detachedPidString.trim());
          expect(detachedPid).toBeGreaterThan(0);
          // Verify detached process survived parent termination
          const exists = process.kill(detachedPid, 0);
          expect(exists).toBe(true);
          // Clean up the detached process
          process.kill(detachedPid, "SIGKILL");
          done();
        } catch (error) {
          done(error);
        }
      });
    }
  );
});

describe("fork (unix IPC)", () => {
  it("exchanges clone_wire messages over fork IPC", async () => {
    if (IS_WINDOWS) {
      return;
    }
    const childPath = join(process.cwd(), "tests/unit/child_process_fork.fixture.mjs");
    const child = fork(childPath);
    expect(child instanceof ChildProcess).toBe(true);
    expect(child.connected).toBe(true);
    const reply = new Promise<unknown>((resolve) => {
      child.once("message", resolve);
    });
    child.send({ ping: 1 });
    expect(await reply).toEqual({ pong: 1 });
    child.kill();
    await new Promise<void>((resolve) => child.once("exit", () => resolve()));
  });

  it("supports serialization: 'json' and disconnect()", async () => {
    if (IS_WINDOWS) {
      return;
    }
    const childPath = join(process.cwd(), "tests/unit/child_process_fork.fixture.mjs");
    const child = fork(childPath, [], { serialization: "json" });
    expect(child.connected).toBe(true);
    const reply = new Promise<unknown>((resolve) => {
      child.once("message", resolve);
    });
    child.send({ ping: 42 });
    expect(await reply).toEqual({ pong: 42 });

    const disconnected = new Promise<void>((resolve) => {
      child.once("disconnect", () => resolve());
    });
    child.disconnect();
    await disconnected;
    expect(child.connected).toBe(false);

    child.kill();
    await new Promise<void>((resolve) => child.once("exit", () => resolve()));
  });
});

describe("spawnSync, execSync, and execFileSync", () => {
  it("spawnSync captures Buffer output, status, and output array", () => {
    const res = spawnSync("echo", ["hello-sync"]);
    expect(res.pid).toBeGreaterThan(0);
    expect(res.status).toBe(0);
    expect(res.signal).toBeNull();
    expect(Buffer.isBuffer(res.stdout)).toBe(true);
    expect(Buffer.isBuffer(res.stderr)).toBe(true);
    expect(res.output[0]).toBeNull();
    expect(res.stdout.toString().trim()).toBe("hello-sync");
  });

  it("spawnSync supports input, encoding, and shell", () => {
    const cmd = IS_WINDOWS ? "findstr .*" : "cat";
    const res = spawnSync(cmd, [], {
      input: "sync-input-line\n",
      encoding: "utf8",
      shell: true,
    });
    expect(res.status).toBe(0);
    expect(typeof res.stdout).toBe("string");
    expect(res.stdout.trim()).toBe("sync-input-line");
  });

  it("spawnSync returns error object without throwing on ENOENT", () => {
    const res = spawnSync("definitely-nonexistent-binary-lexe-xyz");
    expect(res.pid).toBe(0);
    expect(res.output).toBeNull();
    expect(res.status).toBeNull();
    expect(res.error).toBeDefined();
    expect(res.error.code).toBe("ENOENT");
  });

  it("spawnSync enforces timeout and maxBuffer", () => {
    if (!IS_WINDOWS) {
      const timed = spawnSync("sleep", ["5"], { timeout: 30, killSignal: "SIGKILL" });
      expect(timed.error?.code).toBe("ETIMEDOUT");
      expect(timed.signal).toBe("SIGKILL");

      const capped = spawnSync("sh", ["-c", "head -c 4096 /dev/zero | tr '\\0' 'a'"], {
        maxBuffer: 128,
      });
      expect(capped.error?.code).toBe("ENOBUFS");
    }
  });

  it("execSync and execFileSync return Buffer or decoded string and throw with metadata on failure", () => {
    const buf = execSync("echo exec-sync-buf");
    expect(Buffer.isBuffer(buf)).toBe(true);
    expect(buf.toString().trim()).toBe("exec-sync-buf");

    const str = execFileSync("echo", ["exec-file-sync"], { encoding: "utf8" });
    expect(typeof str).toBe("string");
    expect(str.trim()).toBe("exec-file-sync");

    if (!IS_WINDOWS) {
      let caught: any;
      try {
        execSync("echo out-msg; echo err-msg >&2; exit 7", { encoding: "utf8" });
      } catch (err) {
        caught = err;
      }
      expect(caught).toBeDefined();
      expect(caught.status).toBe(7);
      expect(caught.pid).toBeGreaterThan(0);
      expect(String(caught.stdout)).toContain("out-msg");
      expect(String(caught.stderr)).toContain("err-msg");
    }
  });
});

describe("spawn options, properties, stdio, and unref", () => {
  it("exposes spawnfile, spawnargs, spawn event, exitCode, signalCode, and killed", async () => {
    const child = spawn("echo", ["props"]);
    expect(child.spawnfile).toBe("echo");
    expect(child.spawnargs).toEqual(["echo", "props"]);
    expect(child.exitCode).toBeNull();
    expect(child.signalCode).toBeNull();
    expect(child.killed).toBe(false);
    expect(child.connected).toBe(false);

    let sawSpawn = false;
    child.once("spawn", () => {
      sawSpawn = true;
    });
    const code = await new Promise<number | null>((resolve) => {
      child.once("close", (c) => resolve(c));
    });
    expect(sawSpawn).toBe(true);
    expect(code).toBe(0);
    expect(child.exitCode).toBe(0);
    expect(child.signalCode).toBeNull();
  });

  it("validates serialization option and handles pre-aborted / mid-flight AbortSignal", async () => {
    expect(() => spawn("echo", ["ok"], { serialization: "bogus" as any })).toThrow();

    const preAc = new AbortController();
    preAc.abort();
    const preChild = spawn("sleep", ["5"], { signal: preAc.signal });
    const preErr = await new Promise<any>((resolve) => {
      preChild.once("error", resolve);
    });
    expect(preErr.name).toBe("AbortError");
    expect(preErr.code).toBe("ABORT_ERR");

    if (!IS_WINDOWS) {
      const ac = new AbortController();
      const child = spawn("sleep", ["10"], { signal: ac.signal });
      const errPromise = new Promise<any>((resolve) => child.once("error", resolve));
      setTimeout(() => ac.abort(), 30);
      const err = await errPromise;
      expect(err.name).toBe("AbortError");
      expect(err.code).toBe("ABORT_ERR");
    }
  });

  it("supports timeout + killSignal, argv0, and overlapped stdio", async () => {
    if (!IS_WINDOWS) {
      const argv0Child = spawn("sh", ["-c", "echo $0"], { argv0: "custom-lexe-argv0" });
      let out = "";
      argv0Child.stdout.on("data", (d: Buffer) => {
        out += d.toString();
      });
      await new Promise<void>((resolve) => argv0Child.once("close", () => resolve()));
      expect(out.trim()).toBe("custom-lexe-argv0");

      const timedChild = spawn("sleep", ["10"], { timeout: 40, killSignal: "SIGKILL" });
      const [, sig] = await new Promise<[any, any]>((resolve) => {
        timedChild.once("close", (c, s) => resolve([c, s]));
      });
      expect(sig).toBe("SIGKILL");
      expect(timedChild.signalCode).toBe("SIGKILL");
    }

    const overlapped = spawn("echo", ["ov"], { stdio: ["overlapped", "pipe", "ignore"] });
    expect(overlapped.stderr).toBeNull();
    let ovOut = "";
    overlapped.stdout.on("data", (d: Buffer) => {
      ovOut += d.toString();
    });
    await new Promise<void>((resolve) => overlapped.once("close", () => resolve()));
    expect(ovOut.trim()).toBe("ov");
  });

  it("supports numeric fd without closing parent fd and extra fd 3 pipe on Unix", async () => {
    if (IS_WINDOWS) {
      return;
    }
    const outFile = join(tmpdir(), `lexe-cp-fd-${process.pid}-${Date.now()}.txt`);
    const fd = openSync(outFile, "w+");
    try {
      const fdChild = spawn("echo", ["fd-write-ok"], { stdio: ["ignore", fd, "ignore"] });
      await new Promise<void>((resolve) => fdChild.once("close", () => resolve()));
    } finally {
      // Parent fd must remain valid after child exits.
      closeSync(fd);
    }
    expect(readFileSync(outFile, "utf8").trim()).toBe("fd-write-ok");
    rmSync(outFile, { force: true });

    const pipeChild = spawn("sh", ["-c", "read line <&3; echo got:$line >&3"], {
      stdio: ["ignore", "pipe", "pipe", "pipe"],
    });
    expect(pipeChild.stdio.length).toBe(4);
    const p3 = pipeChild.stdio[3];
    let p3Out = "";
    p3.on("data", (d: Buffer) => {
      p3Out += d.toString();
    });
    p3.write("ping3\n");
    await new Promise<void>((resolve) => pipeChild.once("close", () => resolve()));
    expect(p3Out.trim()).toBe("got:ping3");
  });

  it("unref() still emits exit/close when another timer keeps the loop alive", async () => {
    const child = spawn("echo", ["unref-still-fires"]);
    child.unref();
    const events: string[] = [];
    await new Promise<void>((resolve) => {
      const keepAlive = setTimeout(() => {}, 1000);
      child.once("exit", (code) => events.push(`exit:${code}`));
      child.once("close", (code) => {
        events.push(`close:${code}`);
        clearTimeout(keepAlive);
        resolve();
      });
    });
    expect(events).toEqual(["exit:0", "close:0"]);
  });

  it("exec and execFile support encoding, timeout, and maxBuffer", async () => {
    const buf = await new Promise<Buffer>((resolve, reject) => {
      exec("echo async-exec-buf", { encoding: "buffer" }, (err: any, stdout: any) => {
        if (err) reject(err);
        else resolve(stdout);
      });
    });
    expect(Buffer.isBuffer(buf)).toBe(true);
    expect(buf.toString().trim()).toBe("async-exec-buf");

    const fileStr = await new Promise<string>((resolve, reject) => {
      execFile("echo", ["async-exec-file"], { encoding: "utf8" }, (err: any, stdout: any) => {
        if (err) reject(err);
        else resolve(stdout);
      });
    });
    expect(fileStr.trim()).toBe("async-exec-file");

    if (!IS_WINDOWS) {
      const mbErr = await new Promise<any>((resolve) => {
        exec(
          "head -c 1024 /dev/zero | tr '\\0' 'x'",
          { maxBuffer: 64 },
          (err: any) => resolve(err)
        );
      });
      expect(mbErr?.code).toBe("ERR_CHILD_PROCESS_STDIO_MAXBUFFER");
    }
  });
});