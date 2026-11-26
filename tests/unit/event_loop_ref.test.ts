import { spawnCapture } from "./test-utils";

describe("event loop ref/unref", () => {
  it("unref timer allows process to exit before callback", async () => {
    const { code, stdout } = await spawnCapture(process.argv[0], [
      "-e",
      `
        setTimeout(() => console.log("late"), 500).unref();
        console.log("done");
      `,
    ]);
    expect(code).toBe(0);
    expect(stdout.trim()).toBe("done");
  });

  it("ref timer keeps process alive until callback", async () => {
    const { code, stdout } = await spawnCapture(process.argv[0], [
      "-e",
      `
        setTimeout(() => console.log("fired"), 30);
      `,
    ]);
    expect(code).toBe(0);
    expect(stdout.trim()).toBe("fired");
  });

  it("timer ref() after unref() keeps process alive", async () => {
    const { code, stdout } = await spawnCapture(process.argv[0], [
      "-e",
      `
        const t = setTimeout(() => console.log("ok"), 30);
        t.unref();
        t.ref();
      `,
    ]);
    expect(code).toBe(0);
    expect(stdout.trim()).toBe("ok");
  });

  it("timers/promises { ref: false } does not block exit alone", async () => {
    const { code, stdout } = await spawnCapture(process.argv[0], [
      "-e",
      `
        import("node:timers/promises").then(({ setTimeout }) => {
          setTimeout(500, "x", { ref: false });
          console.log("main");
        });
      `,
    ]);
    expect(code).toBe(0);
    expect(stdout.trim()).toBe("main");
  });

  it("unref net server allows exit while listening", async () => {
    const { code, stdout } = await spawnCapture(process.argv[0], [
      "-e",
      `
        const net = require("net");
        const s = net.createServer();
        s.listen(0, () => {
          s.unref();
          console.log("done");
        });
      `,
    ]);
    expect(code).toBe(0);
    expect(stdout.trim()).toBe("done");
  });

  it("unref dgram socket allows exit while bound", async () => {
    const { code, stdout } = await spawnCapture(process.argv[0], [
      "-e",
      `
        const dgram = require("dgram");
        const s = dgram.createSocket("udp4");
        s.bind(0, () => {
          s.unref();
          console.log("done");
        });
      `,
    ]);
    expect(code).toBe(0);
    expect(stdout.trim()).toBe("done");
  });

  it("unref http server allows exit while listening", async () => {
    const { code, stdout } = await spawnCapture(process.argv[0], [
      "-e",
      `
        const http = require("http");
        const s = http.createServer();
        s.listen(0, () => {
          s.unref();
          console.log("done");
        });
      `,
    ]);
    expect(code).toBe(0);
    expect(stdout.trim()).toBe("done");
  });

  it("unref child process allows exit while child runs", async () => {
    const { code, stdout } = await spawnCapture(process.argv[0], [
      "-e",
      `
        const { spawn } = require("child_process");
        const c = spawn("sleep", ["30"], { stdio: "ignore" });
        c.unref();
        console.log("done");
      `,
    ]);
    expect(code).toBe(0);
    expect(stdout.trim()).toBe("done");
  });

  it("unref process.stdin allows exit without active reader", async () => {
    const { code, stdout } = await spawnCapture(process.argv[0], [
      "-e",
      `
        process.stdin.unref();
        console.log("done");
      `,
    ]);
    expect(code).toBe(0);
    expect(stdout.trim()).toBe("done");
  });

  it("process.stdin.on('data') starts reading and keeps event loop alive until EOF", async () => {
    const { spawn } = await import("node:child_process");
    const result = await new Promise<{ code: number | null; stdout: string }>((resolve) => {
      const child = spawn(
        process.argv[0],
        [
          "-e",
          `
            process.stdin.setEncoding("utf8");
            let buf = "";
            process.stdin.on("data", (chunk) => { buf += chunk; });
            process.stdin.on("end", () => console.log("GOT:" + buf.trim()));
          `,
        ],
        { stdio: ["pipe", "pipe", "pipe"] },
      );
      let stdout = "";
      child.stdout.on("data", (c) => {
        stdout += c.toString();
      });
      setTimeout(() => {
        child.stdin.write("hello-pipe\n");
        child.stdin.end();
      }, 40);
      child.on("close", (code) => resolve({ code, stdout }));
    });
    expect(result.code).toBe(0);
    expect(result.stdout.trim()).toBe("GOT:hello-pipe");
  });
});
