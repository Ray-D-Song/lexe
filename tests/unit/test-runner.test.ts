import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnCapture } from "./test-utils";

const CWD = process.cwd();
const FIXTURES = `${CWD}/fixtures/test-runner`;

const runTestDir = (
  dir: string,
  env: Record<string, string> = {}
): Promise<{ code: number; stdout: string; stderr: string; elapsedMs: number }> => {
  const started = Date.now();
  return spawnCapture(process.argv0, ["test", "-d", dir], {
    env: { ...process.env, ...env } as Record<string, string>,
  }).then((result) => ({
    ...result,
    elapsedMs: Date.now() - started,
  }));
};

describe("test runner timeout and exit regressions", () => {
  it(
    "async timeout reports a single worker timeout without crash or resource errors",
    async () => {
      const { code, stdout, stderr, elapsedMs } = await runTestDir(
        `${FIXTURES}/async-timeout`
      );
      const output = stdout + stderr;

      expect(code).toBe(1);
      expect(output).toContain("Timeout after 300ms");
      expect(output).not.toContain(
        "Worker process exited with a non-zero exit code"
      );
      expect(output).not.toContain("does not properly clean up");
      expect(output).not.toContain("gc_obj_list");
      expect(output).not.toContain("JS_FreeRuntime");
      // Parent watchdog uses a different message; it must not also fire.
      expect(output).not.toContain("Test timed out after");

      // Worker should report before parent watchdog (300ms + 1s grace).
      expect(elapsedMs).toBeLessThan(2000);
    },
    10000
  );

  it(
    "callback failures clear timeout and finish immediately",
    async () => {
      const { code, stdout, stderr, elapsedMs } = await runTestDir(
        `${FIXTURES}/callback-early-fail`
      );
      const output = stdout + stderr;

      expect(code).toBe(1);
      expect(output).toContain("callback-promise-reject");
      expect(output).toContain("callback-sync-throw");
      expect(output).toContain("done-error");
      expect(output).not.toContain("Timeout after");
      expect(output).not.toContain("does not properly clean up");
      // Must finish well under the 5s per-test timeouts (uncleared timers fail this).
      expect(elapsedMs).toBeLessThan(3000);
    },
    10000
  );

  it(
    "sync hang is killed by parent watchdog around timeout + 1s",
    async () => {
      const { code, stdout, stderr, elapsedMs } = await runTestDir(
        `${FIXTURES}/sync-hang`
      );
      const output = stdout + stderr;

      expect(code).toBe(1);
      expect(output).toContain("Test timed out after 300ms");
      expect(elapsedMs).toBeGreaterThanOrEqual(1000);
      expect(elapsedMs).toBeLessThan(5000);
    },
    15000
  );

  it("lexe --version exits cleanly", async () => {
    const { code, stdout, stderr } = await spawnCapture(process.argv0, [
      "--version",
    ]);
    const output = stdout + stderr;
    expect(code).toBe(0);
    expect(output).not.toContain("gc_obj_list");
    expect(output).not.toContain("JS_FreeRuntime");
  });

  it("lexe -e exits cleanly", async () => {
    const { code, stdout, stderr } = await spawnCapture(process.argv0, [
      "-e",
      "console.log('ok')",
    ]);
    const output = stdout + stderr;
    expect(code).toBe(0);
    expect(stdout).toContain("ok");
    expect(output).not.toContain("gc_obj_list");
    expect(output).not.toContain("JS_FreeRuntime");
  });

  it("empty test directory exits cleanly", async () => {
    const emptyDir = await mkdtemp(join(tmpdir(), "lexe-empty-tests-"));
    try {
      const { code, stdout, stderr } = await spawnCapture(process.argv0, [
        "test",
        "-d",
        emptyDir,
      ]);
      const output = stdout + stderr;
      expect(code).toBe(0);
      expect(output).not.toContain("gc_obj_list");
      expect(output).not.toContain("JS_FreeRuntime");
    } finally {
      await rm(emptyDir, { recursive: true, force: true });
    }
  });
});
