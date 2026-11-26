import defaultImport from "node:tty";
import legacyImport from "tty";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

it("node:tty should be the same as tty", () => {
  expect(defaultImport).toStrictEqual(legacyImport);
});

it("WriteStream writes cleanly to non-stdio file descriptors", async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "lexe-tty-fd-"));
  const filePath = path.join(dir, "out.txt");
  const fd = fs.openSync(filePath, "w");
  try {
    const ws = new defaultImport.WriteStream(fd);
    expect(ws.isTTY).toBe(false);
    await new Promise<void>((resolve) => {
      ws.write("hello non-stdio tty\n", () => resolve());
    });
    ws.end();
    expect(fs.readFileSync(filePath, "utf8")).toBe("hello non-stdio tty\n");
  } finally {
    fs.closeSync(fd);
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

it("handles SIGWINCH signal delivery via process.kill without terminating", () => {
  if (process.platform === "win32") return;
  let sawSignal = false;
  const onSig = (sig: string) => {
    if (sig === "SIGWINCH") sawSignal = true;
  };
  process.on("signal", onSig);
  expect(() => process.kill(process.pid, "SIGWINCH")).not.toThrow();
  process.off("signal", onSig);
  expect(sawSignal).toBe(false); // async delivery; verifying no crash/exit
});

it("WriteStream.end handles callback overload without stringifying callback function", async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "lexe-tty-end-"));
  const filePath = path.join(dir, "end.txt");
  const fd = fs.openSync(filePath, "w");
  try {
    const ws = new defaultImport.WriteStream(fd);
    expect(ws.write("first\n", "utf8", undefined as any)).toBe(true);
    let finishEvents = 0;
    ws.on("finish", () => {
      finishEvents++;
    });
    await new Promise<void>((resolve) => {
      ws.end(() => resolve());
    });
    expect(finishEvents).toBe(1);
    expect(fs.readFileSync(filePath, "utf8")).toBe("first\n");

    const filePath2 = path.join(dir, "end2.txt");
    const fd2 = fs.openSync(filePath2, "w");
    try {
      const ws2 = new defaultImport.WriteStream(fd2);
      await new Promise<void>((resolve) => {
        ws2.end(undefined as any, () => resolve());
      });
      expect(fs.readFileSync(filePath2, "utf8")).toBe("");
    } finally {
      fs.closeSync(fd2);
    }
  } finally {
    fs.closeSync(fd);
    fs.rmSync(dir, { recursive: true, force: true });
  }
});
