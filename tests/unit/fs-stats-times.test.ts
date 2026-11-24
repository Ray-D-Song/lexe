import fs from "node:fs";
import path from "node:path";
import os from "node:os";

// `Stats` timestamp fields. The `*Ms` fields are floats and are **not** the same
// number as the matching `Date`'s `getTime()`, which truncates to whole
// milliseconds. Every case below was measured against Node 24 first.
//
// When a filesystem cannot report a creation time, Node reports epoch zero for
// both `birthtimeMs` and `birthtime` — it does not fall back to `ctime`. Measured
// on `/proc` and `/sys`.

const IS_WINDOWS = os.platform() === "win32";

const tmp = (contents = "hello") => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "test-stats-times-"));
  const file = path.join(dir, "f.txt");
  fs.writeFileSync(file, contents);
  return { file, cleanup: () => fs.rmSync(dir, { recursive: true, force: true }) };
};

const call = <T = any>(fn: (cb: (err: any, ...rest: any[]) => void) => void): Promise<T> =>
  new Promise((resolve, reject) => {
    fn((err: any, ...rest: any[]) => (err ? reject(err) : resolve(rest as any as T)));
  });

// A timestamp with a sub-millisecond remainder, so `*Ms` and `getTime()` differ.
const SUB_MS = 1234567890.123456;

describe("Stats timestamps", () => {
  it("mtimeMs and atimeMs keep sub-millisecond precision", () => {
    const { file, cleanup } = tmp();
    fs.utimesSync(file, SUB_MS, SUB_MS);
    const stats = fs.statSync(file);
    // 1234567890.123456s * 1000 = 1234567890123.456 ms
    expect(stats.mtimeMs).toBeCloseTo(1234567890123.456, 3);
    expect(stats.atimeMs).toBeCloseTo(1234567890123.456, 3);
    cleanup();
  });

  it("mtimeMs is a float, not a truncated integer", () => {
    const { file, cleanup } = tmp();
    fs.utimesSync(file, SUB_MS, SUB_MS);
    const stats = fs.statSync(file);
    // The whole point of the float: truncating to an integer loses the 0.456.
    expect(Number.isInteger(stats.mtimeMs)).toBe(false);
    expect(stats.mtimeMs).not.toBe(Math.trunc(stats.mtimeMs));
    cleanup();
  });

  it("mtimeMs !== mtime.getTime() when the timestamp has a sub-millisecond part", () => {
    const { file, cleanup } = tmp();
    fs.utimesSync(file, SUB_MS, SUB_MS);
    const stats = fs.statSync(file);
    // This is the evidence that the `*Ms` getters cannot reuse the `Date` accessor.
    expect(stats.mtime.getTime()).toBe(1234567890123);
    expect(stats.mtimeMs).not.toBe(stats.mtime.getTime());
    expect(stats.mtimeMs - stats.mtime.getTime()).toBeCloseTo(0.456, 3);
    cleanup();
  });

  it("mtimeMs === mtime.getTime() on a whole-second timestamp", () => {
    const { file, cleanup } = tmp();
    // No sub-millisecond remainder, so there is nothing to truncate and the two
    // must agree. Without this case, the test above could pass on a broken
    // implementation that added a stray fraction to every value.
    fs.utimesSync(file, 1_000_000_000, 1_000_000_000);
    const stats = fs.statSync(file);
    expect(stats.mtimeMs).toBe(1_000_000_000_000);
    expect(stats.mtimeMs).toBe(stats.mtime.getTime());
    cleanup();
  });

  it("ctime is a real 2020s date, not an epoch-relative nanosecond count", () => {
    const { file, cleanup } = tmp();
    const stats = fs.statSync(file);
    // `ctime_nsec` is only the sub-second remainder; using it alone landed in 1970.
    expect(stats.ctime.getUTCFullYear()).toBeGreaterThan(2020);
    expect(stats.ctimeMs).toBeGreaterThan(1_600_000_000_000);
    // The `Date` truncates, so the remainder is always in [0, 1) — not "close to
    // zero", which would be a much tighter claim than the two can satisfy.
    expect(stats.ctimeMs - stats.ctime.getTime()).toBeGreaterThanOrEqual(0);
    expect(stats.ctimeMs - stats.ctime.getTime()).toBeLessThan(1);
    cleanup();
  });

  it("birthtime is a real 2020s date when the filesystem reports one", () => {
    const { file, cleanup } = tmp();
    const stats = fs.statSync(file);
    expect(stats.birthtime.getUTCFullYear()).toBeGreaterThan(2020);
    expect(stats.birthtimeMs - stats.birthtime.getTime()).toBeGreaterThanOrEqual(0);
    expect(stats.birthtimeMs - stats.birthtime.getTime()).toBeLessThan(1);
    cleanup();
  });

  it("ctimeMs and birthtimeMs track ctime and birthtime as floats", () => {
    const { file, cleanup } = tmp();
    const stats = fs.statSync(file);
    for (const [ms, date] of [
      [stats.atimeMs, stats.atime],
      [stats.mtimeMs, stats.mtime],
      [stats.ctimeMs, stats.ctime],
      [stats.birthtimeMs, stats.birthtime],
    ] as [number, Date][]) {
      expect(typeof ms).toBe("number");
      expect(Math.abs(ms - date.getTime())).toBeLessThan(1);
    }
    cleanup();
  });
});

// Node returns epoch zero for both `birthtimeMs` and `birthtime` when the
// filesystem has no creation time. `/proc` and `/sys` are the reliable cases on
// Linux; the check is skipped where they are not mounted or usable.
const PROC_FILE = "/proc/self/status";
// Only probes whether the file is readable. Deliberately does not look at
// `birthtimeMs` here: a broken `Stats` must fail the cases below, not silently
// skip them, and must not take down the whole module while it is being loaded.
const procAvailable = (() => {
  if (IS_WINDOWS || !fs.existsSync(PROC_FILE)) return false;
  try {
    fs.statSync(PROC_FILE);
    return true;
  } catch {
    return false;
  }
})();

describe("Stats birthtime fallback", () => {
  (procAvailable ? it : it.skip)(
    "birthtimeMs is 0, not ctime, when the filesystem has no creation time",
    () => {
      const stats = fs.statSync(PROC_FILE);
      expect(stats.birthtimeMs).toBe(0);
      // ctime on /proc is real, so a fallback to ctime would be visible here.
      expect(stats.ctimeMs).toBeGreaterThan(1_600_000_000_000);
    }
  );

  (procAvailable ? it : it.skip)("birthtime is the epoch, matching birthtimeMs", () => {
    const stats = fs.statSync(PROC_FILE);
    expect(stats.birthtime.getTime()).toBe(0);
    expect(stats.birthtime.getUTCFullYear()).toBe(1970);
    expect(stats.birthtimeMs).toBe(stats.birthtime.getTime());
  });
});

// One fix in `Stats` covers every access path; this pins that they agree.
//
// Each path is collected only when it exists, so this file passes on a tree
// where the descriptor family (`fstat`, `fstatSync`) is not there yet and
// covers it once it is.
const collectStatPaths = async (
  file: string,
  pick: (stats: any) => number
): Promise<Array<[string, number]>> => {
  const F = fs as any;
  const seen: Array<[string, number]> = [
    ["statSync", pick(fs.statSync(file))],
    ["lstatSync", pick(fs.lstatSync(file))],
  ];
  if (typeof F.fstatSync === "function") {
    const fd = fs.openSync(file, "r");
    seen.push(["fstatSync", pick(fs.fstatSync(fd))]);
    fs.closeSync(fd);
  }
  const [statCb] = await call<any>((cb) => fs.stat(file as any, cb));
  seen.push(["stat", pick(statCb)]);
  const [lstatCb] = await call<any>((cb) => fs.lstat(file as any, cb));
  seen.push(["lstat", pick(lstatCb)]);
  if (typeof F.fstat === "function") {
    const fd = fs.openSync(file, "r");
    const [fstatCb] = await call<any>((cb) => F.fstat(fd, cb));
    fs.closeSync(fd);
    seen.push(["fstat", pick(fstatCb)]);
  }
  seen.push(["promises.stat", pick(await fs.promises.stat(file))]);
  return seen;
};

describe("Stats timestamps agree across every access path", () => {
  it("reports the same mtimeMs from every access path", async () => {
    const { file, cleanup } = tmp();
    fs.utimesSync(file, SUB_MS, SUB_MS);
    const seen = await collectStatPaths(file, (stats) => stats.mtimeMs);

    for (const ms of seen.map(([, v]) => v)) {
      expect(ms.toFixed(3)).toBe(seen[0][1].toFixed(3));
    }
    expect(seen[0][1]).toBeCloseTo(1234567890123.456, 3);
    cleanup();
  });

  it("reports the same birthtimeMs from every access path", async () => {
    const { file, cleanup } = tmp();
    const values = (await collectStatPaths(file, (stats) => stats.birthtimeMs)).map(
      ([, v]) => v
    );

    for (const v of values) expect(v).toBe(values[0]);
    // Not the process age: `birthtimeMs` used to return `elapsed()`.
    expect(values[0]).toBeGreaterThan(1_600_000_000_000);
    cleanup();
  });
});
