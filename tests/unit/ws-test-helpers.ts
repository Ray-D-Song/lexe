// Helpers for driving `http.WebSocket` against the Python echo fixture.
//
// These cases used to run in a child process, because a listener or a detached
// child left in the test worker made the worker exit non-zero at teardown. That
// was a harness gate rather than a test requirement, and it is gone: the worker
// now exits as soon as it has reported. So the fixture is started here, in the
// worker, and every case stops it in a `finally`.

import { spawn } from "node:child_process";
import path from "node:path";

export type Fixture = {
  port: number;
  stop: () => void;
};

/// Start `ws_echo_server.py` in `mode` and resolve once it reports its port.
export const startFixture = async (mode: string): Promise<Fixture> => {
  const fixturePath = path.resolve("tests/unit/fixtures/ws_echo_server.py");
  const child = spawn("python3", [fixturePath, "0", mode], {
    stdio: ["ignore", "pipe", "ignore"],
  });

  const port = await new Promise<number>((resolve, reject) => {
    let buffered = "";
    const timer = setTimeout(
      () => reject(new Error("ws fixture did not start")),
      5000
    );
    child.stdout.on("data", (chunk: any) => {
      buffered += chunk.toString();
      const match = /^(\d+)/m.exec(buffered);
      if (match) {
        clearTimeout(timer);
        resolve(Number(match[1]));
      }
    });
    child.on("error", (err: any) => {
      clearTimeout(timer);
      reject(err);
    });
  });

  const stop = () => {
    try {
      child.kill();
    } catch {}
  };

  return { port, stop };
};

/// Resolve with whatever `settle` is called with, or reject on timeout so a hung
/// socket fails the individual test instead of hanging the file.
export const withTimeout = <T>(
  run: (settle: (value: T) => void) => void,
  timeoutMs = 6000
): Promise<T> =>
  new Promise<T>((resolve, reject) => {
    const timer = setTimeout(
      () => reject(new Error("websocket case timed out")),
      timeoutMs
    );
    const settle = (value: T) => {
      clearTimeout(timer);
      resolve(value);
    };
    try {
      run(settle);
    } catch (err) {
      clearTimeout(timer);
      reject(err);
    }
  });
