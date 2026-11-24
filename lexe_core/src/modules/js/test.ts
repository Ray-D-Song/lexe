// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
//
// Embedded `node:test` — incremental Node 24.3-compatible test runner.

export type TestOptions = {
  concurrency?: boolean;
  only?: boolean;
  skip?: boolean;
  todo?: boolean;
  timeout?: number;
  signal?: AbortSignal;
};

export type RunOptions = {
  timeout?: number;
  files?: string[];
};

export type RunResult = {
  pass: number;
  fail: number;
  skip: number;
  todo: number;
  failed: { name: string; error: unknown }[];
};

type TestFn = (
  t: TestContext,
  done?: (err?: unknown) => void
) => void | Promise<void>;

type HookFn = () => void | Promise<void>;

type TestCase = TestOptions & {
  name: string;
  fn?: TestFn;
  kind: "test" | "skip" | "todo" | "only";
};

type Suite = TestOptions & {
  name: string;
  kind: "suite" | "skip" | "todo" | "only";
  tests: TestCase[];
  suites: Suite[];
  before: HookFn[];
  after: HookFn[];
  beforeEach: HookFn[];
  afterEach: HookFn[];
};

export class TestContext {
  readonly #name: string;
  readonly #diagnostics: string[] = [];
  #skipped = false;
  #todo = false;

  constructor(name: string) {
    this.#name = name;
  }

  get name(): string {
    return this.#name;
  }

  diagnostic(message: string): void {
    this.#diagnostics.push(String(message));
  }

  skip(message?: string): void {
    this.#skipped = true;
    if (message) this.diagnostic(message);
  }

  todo(message?: string): void {
    this.#todo = true;
    if (message) this.diagnostic(message);
  }

  async test(
    name: string,
    fnOrOptions?: TestFn | TestOptions,
    optionsOrFn?: TestFn | TestOptions
  ): Promise<void> {
    const { fn, options } = parseTestArgs(fnOrOptions, optionsOrFn);
    if (!fn) {
      throw new TypeError("test(): fn is required");
    }
    const child: TestCase = {
      name,
      fn,
      kind: "test",
      ...options,
    };
    await runTestCase(child, []);
  }

  get skipped(): boolean {
    return this.#skipped;
  }

  get todo(): boolean {
    return this.#todo;
  }
}

const DEFAULT_TIMEOUT_MS =
  parseInt((process.env as { TEST_TIMEOUT?: string }).TEST_TIMEOUT || "", 10) ||
  5000;

const root: Suite = createSuite("root", "suite");
let currentSuite: Suite = root;
let insideDescribe = false;

function createSuite(name: string, kind: Suite["kind"]): Suite {
  return {
    name,
    kind,
    tests: [],
    suites: [],
    before: [],
    after: [],
    beforeEach: [],
    afterEach: [],
  };
}

function parseTestArgs(
  fnOrOptions?: TestFn | TestOptions,
  optionsOrFn?: TestFn | TestOptions
): { fn?: TestFn; options: TestOptions } {
  let fn: TestFn | undefined;
  let options: TestOptions = {};
  if (typeof fnOrOptions === "function") {
    fn = fnOrOptions;
    if (typeof optionsOrFn === "object" && optionsOrFn !== null) {
      options = optionsOrFn;
    }
  } else if (typeof fnOrOptions === "object" && fnOrOptions !== null) {
    options = fnOrOptions;
    if (typeof optionsOrFn === "function") {
      fn = optionsOrFn;
    }
  }
  return { fn, options };
}

function registerTest(
  name: string,
  fnOrOptions?: TestFn | TestOptions,
  optionsOrFn?: TestFn | TestOptions,
  kind: TestCase["kind"] = "test"
): Promise<void> {
  const { fn, options } = parseTestArgs(fnOrOptions, optionsOrFn);
  const entry: TestCase = {
    name,
    fn,
    kind,
    ...options,
  };
  if (options.skip) entry.kind = "skip";
  if (options.todo) entry.kind = "todo";
  if (options.only) entry.kind = "only";
  currentSuite.tests.push(entry);

  if (insideDescribe) {
    return Promise.resolve();
  }
  if (!fn) {
    return Promise.resolve();
  }
  return runTestCase(entry, beforeEachHooksFor(currentSuite));
}

function registerSuite(
  name: string,
  fn: () => void,
  kind: Suite["kind"] = "suite"
): void {
  const suite = createSuite(name, kind);
  currentSuite.suites.push(suite);
  const prev = currentSuite;
  currentSuite = suite;
  insideDescribe = true;
  try {
    fn();
  } finally {
    insideDescribe = false;
    currentSuite = prev;
  }
}

function beforeEachHooksFor(suite: Suite): HookFn[] {
  const chain: Suite[] = [];
  let cur: Suite | undefined = suite;
  while (cur && cur.name !== "root") {
    chain.unshift(cur);
    cur = findParentSuite(cur);
  }
  const hooks: HookFn[] = [];
  for (const s of chain) {
    hooks.push(...s.beforeEach);
  }
  return hooks;
}

function findParentSuite(target: Suite): Suite | undefined {
  return findParentSuiteInner(root, target);
}

function findParentSuiteInner(
  node: Suite,
  target: Suite
): Suite | undefined {
  for (const child of node.suites) {
    if (child === target) return node;
    const found = findParentSuiteInner(child, target);
    if (found) return found;
  }
  return undefined;
}

function suiteHasOnly(suite: Suite): boolean {
  if (suite.kind === "only") return true;
  for (const t of suite.tests) {
    if (t.kind === "only" || t.only) return true;
  }
  for (const s of suite.suites) {
    if (suiteHasOnly(s)) return true;
  }
  return false;
}

function shouldRun(entry: { kind: string; only?: boolean }, parentOnly: boolean): boolean {
  if (entry.kind === "skip" || entry.kind === "todo") return false;
  if (parentOnly && entry.kind !== "only" && !entry.only) return false;
  return true;
}

async function runHooks(hooks: HookFn[], timeout: number): Promise<void> {
  for (const hook of hooks) {
    await runFn(hook, timeout, undefined, true);
  }
}

async function runFn(
  fn: HookFn | TestFn,
  timeout: number,
  ctx?: TestContext,
  hook = false
): Promise<void> {
  const fnText = Function.prototype.toString.call(fn);
  const usesDone = /\(\s*t\s*,\s*done\s*\)|\(\s*done\s*\)/.test(fnText);

  let timeoutId: ReturnType<typeof setTimeout> | undefined;
  const timeoutPromise = new Promise<never>((_, reject) => {
    timeoutId = setTimeout(
      () => reject(new Error(`Timeout after ${timeout}ms`)),
      timeout
    );
  });

  try {
    if (usesDone && !hook) {
      await Promise.race([
        new Promise<void>((resolve, reject) => {
          const done = (err?: unknown) => {
            if (err) reject(err);
            else resolve();
          };
          try {
            const ret = (fn as TestFn)(ctx!, done);
            if (ret && typeof (ret as Promise<void>).then === "function") {
              reject(
                new Error(
                  "Test function returned a Promise and used done() callback"
                )
              );
            }
          } catch (e) {
            reject(e);
          }
        }),
        timeoutPromise,
      ]);
    } else if (hook || fn.length === 0) {
      await Promise.race([
        Promise.resolve((fn as HookFn)()),
        timeoutPromise,
      ]);
    } else {
      await Promise.race([
        Promise.resolve((fn as TestFn)(ctx!)),
        timeoutPromise,
      ]);
    }
  } finally {
    if (timeoutId !== undefined) clearTimeout(timeoutId);
  }
}

async function runTestCase(
  testCase: TestCase,
  beforeEachHooks: HookFn[]
): Promise<{ status: "pass" | "fail" | "skip" | "todo"; error?: unknown }> {
  if (testCase.kind === "skip" || testCase.skip) {
    return { status: "skip" };
  }
  if (testCase.kind === "todo" || testCase.todo) {
    return { status: "todo" };
  }
  if (!testCase.fn) {
    return { status: "todo" };
  }

  const timeout = testCase.timeout ?? DEFAULT_TIMEOUT_MS;
  const ctx = new TestContext(testCase.name);

  try {
    await runHooks(beforeEachHooks, timeout);
    await runFn(testCase.fn, timeout, ctx);
    if (ctx.skipped) return { status: "skip" };
    if (ctx.todo) return { status: "todo" };
    return { status: "pass" };
  } catch (error) {
    return { status: "fail", error };
  }
}

async function runSuite(suite: Suite, parentOnly: boolean): Promise<RunResult> {
  const result: RunResult = {
    pass: 0,
    fail: 0,
    skip: 0,
    todo: 0,
    failed: [],
  };
  const onlyMode = parentOnly || suiteHasOnly(suite);

  if (!shouldRun(suite, parentOnly) && suite.kind !== "suite") {
    if (suite.kind === "skip") return result;
    if (suite.kind === "todo") {
      result.todo++;
      return result;
    }
  }

  await runHooks(suite.before, DEFAULT_TIMEOUT_MS);

  for (const child of suite.suites) {
    const sub = await runSuite(child, onlyMode);
    mergeResults(result, sub);
  }

  const beforeEachHooks = beforeEachHooksFor(suite);

  for (const testCase of suite.tests) {
    if (!shouldRun(testCase, onlyMode)) {
      if (testCase.kind === "skip" || testCase.skip) result.skip++;
      else if (testCase.kind === "todo" || testCase.todo) result.todo++;
      continue;
    }
    const out = await runTestCase(testCase, beforeEachHooks);
    if (out.status === "pass") result.pass++;
    else if (out.status === "fail") {
      result.fail++;
      result.failed.push({ name: testCase.name, error: out.error });
    } else if (out.status === "skip") result.skip++;
    else if (out.status === "todo") result.todo++;
  }

  await runHooks(suite.after, DEFAULT_TIMEOUT_MS);
  return result;
}

function mergeResults(into: RunResult, from: RunResult): void {
  into.pass += from.pass;
  into.fail += from.fail;
  into.skip += from.skip;
  into.todo += from.todo;
  into.failed.push(...from.failed);
}

export async function run(options?: RunOptions): Promise<RunResult> {
  const timeout = options?.timeout ?? DEFAULT_TIMEOUT_MS;
  void timeout;
  const result = await runSuite(root, false);
  if (result.fail > 0) {
    process.exitCode = 1;
  }
  __resetTestPlanForTests();
  return result;
}

export function test(
  name: string,
  fnOrOptions?: TestFn | TestOptions,
  optionsOrFn?: TestFn | TestOptions
): Promise<void> {
  return registerTest(name, fnOrOptions, optionsOrFn, "test");
}

export function suite(name: string, fn: () => void): void {
  registerSuite(name, fn, "suite");
}

export const describe = suite;

export function it(
  name: string,
  fnOrOptions?: TestFn | TestOptions,
  optionsOrFn?: TestFn | TestOptions
): Promise<void> {
  return test(name, fnOrOptions, optionsOrFn);
}

function hook(name: keyof Pick<Suite, "before" | "after" | "beforeEach" | "afterEach">) {
  return (fn: HookFn) => {
    currentSuite[name].push(fn);
  };
}

export const before = hook("before");
export const after = hook("after");
export const beforeEach = hook("beforeEach");
export const afterEach = hook("afterEach");

function defineTestVariant(kind: TestCase["kind"]) {
  return (
    name: string,
    fnOrOptions?: TestFn | TestOptions,
    optionsOrFn?: TestFn | TestOptions
  ) => registerTest(name, fnOrOptions, optionsOrFn, kind);
}

function defineSuiteVariant(kind: Suite["kind"]) {
  return (name: string, fn: () => void) => registerSuite(name, fn, kind);
}

test.skip = defineTestVariant("skip");
test.todo = defineTestVariant("todo");
test.only = defineTestVariant("only");
it.skip = test.skip;
it.todo = test.todo;
it.only = test.only;

describe.skip = defineSuiteVariant("skip");
describe.only = defineSuiteVariant("only");
suite.skip = describe.skip;
suite.only = describe.only;

/** Reset plan between programmatic `run()` calls in the same isolate (tests). */
export function __resetTestPlanForTests(): void {
  root.tests.length = 0;
  root.suites.length = 0;
  root.before.length = 0;
  root.after.length = 0;
  root.beforeEach.length = 0;
  root.afterEach.length = 0;
  currentSuite = root;
  insideDescribe = false;
}

export default test;
