import { spawn } from "node:child_process";
import { join } from "node:path";

const TIMEOUT = 20_000;

// These cases run a standalone .mjs file instead of "-e" source. The defect
// only reproduces at a module's top level: there poll_timers drains the timer
// callback's promise continuation itself, so a timer registered from that
// continuation is missing from the "is the timer list empty" snapshot taken
// before the callbacks ran. Inside a nested async function the continuation is
// drained by a later poll, which masks the stale snapshot.
const runFixture = (
  name: string
): Promise<{ code: number; stdout: string; stderr: string }> =>
  new Promise((resolve, reject) => {
    let stdout = "";
    let stderr = "";
    const child = spawn(process.argv[0], [
      join(process.cwd(), "tests/unit", name),
    ]);
    const timer = setTimeout(() => {
      child.kill("SIGKILL");
    }, TIMEOUT - 2_000);
    child.stdout.on("data", (d) => (stdout += d.toString()));
    child.stderr.on("data", (d) => (stderr += d.toString()));
    child.on("close", (code) => {
      clearTimeout(timer);
      resolve({ code: code ?? -1, stdout, stderr });
    });
    child.on("error", (e) => {
      clearTimeout(timer);
      reject(e);
    });
  });

describe("timer loop after a callback registers a new timer", () => {
  it(
    "fires a timer registered from a timer continuation while an HTTP connection stays open",
    async () => {
      const { code, stdout } = await runFixture(
        "timers_after_callback_with_connection.fixture.mjs"
      );

      expect(stdout).toContain("response");
      expect(stdout).toContain("first-timer");
      expect(stdout).toContain("second-timer");
      expect(code).toBe(0);
    },
    TIMEOUT
  );

  // Reverse guard: the loop must still be allowed to exit. A fix that keeps the
  // loop alive whenever it saw any callback would leave this one hanging.
  it(
    "still exits after nested timers registered from a continuation, with no connection",
    async () => {
      const { code, stdout } = await runFixture(
        "timers_nested_no_connection.fixture.mjs"
      );

      expect(stdout).toContain("outer");
      expect(stdout).toContain("inner");
      expect(code).toBe(0);
    },
    TIMEOUT
  );
});
