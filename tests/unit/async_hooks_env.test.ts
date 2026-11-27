import { spawnCapture } from "./test-utils";

/**
 * Verify dynamic async_hooks activation in fresh Lexe processes where no env
 * flag or prior test hook is active.
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

it("createHook stays quiet until enabled and activates on demand without LEXE_ASYNC_HOOKS", async () => {
  const script = `
    import { createHook } from "async_hooks";
    let hits = 0;
    const hook = createHook({
      init() { hits++; },
      before() { hits++; },
      after() { hits++; },
      promiseResolve() { hits++; },
    });

    await Promise.resolve();
    await new Promise((r) => setTimeout(r, 5));
    if (hits !== 0) {
      console.error("unexpected hits before enable():", hits);
      process.exit(1);
    }

    if (hook.enable() !== hook) {
      console.error("enable() should return hook");
      process.exit(1);
    }

    await Promise.resolve();
    await new Promise((r) => setTimeout(r, 10));
    const hitsAfterEnable = hits;
    if (hitsAfterEnable === 0) {
      console.error("expected hits after enable(), got 0");
      process.exit(1);
    }

    if (hook.disable() !== hook) {
      console.error("disable() should return hook");
      process.exit(1);
    }

    await Promise.resolve();
    await new Promise((r) => setTimeout(r, 10));
    if (hits !== hitsAfterEnable) {
      console.error("unexpected hits after disable():", hits, hitsAfterEnable);
      process.exit(1);
    }
    console.log("HOOKS_DYNAMIC_OK", hitsAfterEnable);
  `;
  const env = { ...process.env };
  delete env.LEXE_ASYNC_HOOKS;
  const { code, stdout, stderr } = await spawnCapture(
    runner,
    ["-e", script],
    { env }
  );
  expect(stderr).toBe("");
  expect(stdout).toContain("HOOKS_DYNAMIC_OK");
  expect(code).toBe(0);
});

it("overlapping hooks keep tracking active until the last hook disables", async () => {
  const script = `
    import { createHook } from "async_hooks";
    let h1Hits = 0;
    let h2Hits = 0;
    const h1 = createHook({ init() { h1Hits++; } }).enable();
    const h2 = createHook({ init() { h2Hits++; } }).enable();

    await Promise.resolve();
    await new Promise((r) => setTimeout(r, 5));
    const h1BeforeDisable = h1Hits;
    const h2BeforeDisable = h2Hits;

    h1.disable();
    await Promise.resolve();
    await new Promise((r) => setTimeout(r, 5));
    if (h1Hits !== h1BeforeDisable) {
      console.error("disabled h1 continued receiving hits");
      process.exit(1);
    }
    if (h2Hits <= h2BeforeDisable) {
      console.error("enabled h2 stopped receiving hits");
      process.exit(1);
    }

    h2.disable();
    const h2AfterDisable = h2Hits;
    await Promise.resolve();
    await new Promise((r) => setTimeout(r, 5));
    if (h2Hits !== h2AfterDisable) {
      console.error("disabled h2 continued receiving hits");
      process.exit(1);
    }
    console.log("HOOKS_OVERLAP_OK");
  `;
  const env = { ...process.env };
  delete env.LEXE_ASYNC_HOOKS;
  const { code, stdout, stderr } = await spawnCapture(
    runner,
    ["-e", script],
    { env }
  );
  expect(stderr).toBe("");
  expect(stdout).toContain("HOOKS_OVERLAP_OK");
  expect(code).toBe(0);
});
