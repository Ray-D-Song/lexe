import { spawnCapture } from "./test-utils";

/**
 * HOOKING_MODE is a process-level once_cell Lazy. These behaviors must be
 * verified in separate Lexe processes so the first env read is controlled.
 */

const runner = process.argv0;

it("ALS works without LEXE_ASYNC_HOOKS", async () => {
  const script = `
    import { AsyncLocalStorage } from "async_hooks";
    const als = new AsyncLocalStorage();
    const v = await als.run("ctx", async () => {
      await Promise.resolve();
      await new Promise((r) => setTimeout(r, 5));
      return als.getStore();
    });
    if (v !== "ctx") {
      console.error("ALS store mismatch:", v);
      process.exit(1);
    }
    console.log("ALS_OK");
  `;
  const env = { ...process.env };
  delete env.LEXE_ASYNC_HOOKS;
  const { code, stdout, stderr } = await spawnCapture(
    runner,
    ["-e", script],
    { env }
  );
  expect(stderr).toBe("");
  expect(stdout).toContain("ALS_OK");
  expect(code).toBe(0);
});

it("createHook does not receive events without LEXE_ASYNC_HOOKS", async () => {
  const script = `
    import { createHook } from "async_hooks";
    let hits = 0;
    createHook({
      init() { hits++; },
      before() { hits++; },
      after() { hits++; },
      promiseResolve() { hits++; },
      destroy() { hits++; },
    }).enable();
    await Promise.resolve();
    await new Promise((r) => setTimeout(r, 10));
    if (hits !== 0) {
      console.error("unexpected user hook hits:", hits);
      process.exit(1);
    }
    console.log("HOOKS_QUIET");
  `;
  const env = { ...process.env };
  delete env.LEXE_ASYNC_HOOKS;
  const { code, stdout, stderr } = await spawnCapture(
    runner,
    ["-e", script],
    { env }
  );
  expect(stderr).toBe("");
  expect(stdout).toContain("HOOKS_QUIET");
  expect(code).toBe(0);
});

it("createHook receives events with LEXE_ASYNC_HOOKS=1", async () => {
  const script = `
    import { createHook } from "async_hooks";
    let hits = 0;
    createHook({
      init() { hits++; },
      before() { hits++; },
      after() { hits++; },
      promiseResolve() { hits++; },
    }).enable();
    await Promise.resolve();
    await new Promise((r) => setTimeout(r, 10));
    if (hits === 0) {
      console.error("expected user hook hits, got 0");
      process.exit(1);
    }
    console.log("HOOKS_HIT", hits);
  `;
  const env = {
    ...process.env,
    LEXE_ASYNC_HOOKS: "1",
  };
  const { code, stdout, stderr } = await spawnCapture(
    runner,
    ["-e", script],
    { env }
  );
  expect(stderr).toBe("");
  expect(stdout).toContain("HOOKS_HIT");
  expect(code).toBe(0);
});
