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
