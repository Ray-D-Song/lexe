import fs from "node:fs";
import path from "node:path";
import os from "node:os";

// The callback-style asynchronous fd family: `fs.fstat`, `fs.ftruncate`,
// `fs.fchmod`, `fs.fsync`, `fs.fdatasync`, `fs.futimes`, `fs.readv`, `fs.writev`.
//
// Worth recording, because the docs read the other way: in Node these live on
// `fs` (callback style) and on `FileHandle` (promise style). **`fs.promises.read`
// and friends do not exist** — measured on Node 24, all ten are `undefined`.
//
// `fs.read` and `fs.write` are absent from Raster and not covered here.

const F = fs as any;

const tmp = (contents = "") => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "test-fdasync-"));
  const file = path.join(dir, "f.txt");
  fs.writeFileSync(file, contents);
  return {
    dir,
    file,
    cleanup: () => fs.rmSync(dir, { recursive: true, force: true }),
  };
};

const call = <T = any>(fn: (cb: (err: any, ...rest: any[]) => void) => void): Promise<T> =>
  new Promise((resolve, reject) => {
    fn((err: any, ...rest: any[]) => (err ? reject(err) : resolve(rest as any as T)));
  });

const expectBadFd = async (syscall: string, run: (cb: any) => void) => {
  let err: any;
  try {
    await call(run);
  } catch (e) {
    err = e;
  }
  expect(`${syscall}:${err && err.code}:${err && err.syscall}`).toBe(
    `${syscall}:EBADF:${syscall}`
  );
};

describe("async fd family", () => {
  it("fstat reports the descriptor's size", async () => {
    const { file, cleanup } = tmp("abcdef");
    const fd = fs.openSync(file, "r");
    const [stats] = await call<any>((cb) => F.fstat(fd, cb));
    expect(stats.size).toBe(6);
    fs.closeSync(fd);
    cleanup();
  });

  it("fstat on a bad descriptor is EBADF with syscall fstat", async () => {
    await expectBadFd("fstat", (cb) => F.fstat(999999, cb));
  });

  it("ftruncate shortens the file", async () => {
    const { file, cleanup } = tmp("abcdef");
    const fd = fs.openSync(file, "r+");
    await call((cb) => F.ftruncate(fd, 3, cb));
    fs.closeSync(fd);
    expect(fs.readFileSync(file, "utf8")).toBe("abc");
    cleanup();
  });

  it("ftruncate on a bad descriptor is EBADF", async () => {
    await expectBadFd("ftruncate", (cb) => F.ftruncate(999999, 1, cb));
  });

  it("fchmod sets the mode", async () => {
    if (os.platform() === "win32") return;
    const { file, cleanup } = tmp("x");
    const fd = fs.openSync(file, "r+");
    await call((cb) => F.fchmod(fd, 0o600, cb));
    fs.closeSync(fd);
    expect(fs.statSync(file).mode & 0o777).toBe(0o600);
    cleanup();
  });

  it("fchmod on a bad descriptor is EBADF", async () => {
    await expectBadFd("fchmod", (cb) => F.fchmod(999999, 0o600, cb));
  });

  it("fsync completes and reports EBADF for a bad descriptor", async () => {
    const { file, cleanup } = tmp("x");
    const fd = fs.openSync(file, "r+");
    await call((cb) => F.fsync(fd, cb));
    fs.closeSync(fd);
    cleanup();
    await expectBadFd("fsync", (cb) => F.fsync(999999, cb));
  });

  it("fdatasync completes and reports EBADF for a bad descriptor", async () => {
    const { file, cleanup } = tmp("x");
    const fd = fs.openSync(file, "r+");
    await call((cb) => F.fdatasync(fd, cb));
    fs.closeSync(fd);
    cleanup();
    await expectBadFd("fdatasync", (cb) => F.fdatasync(999999, cb));
  });

  it("futimes sets the file times", async () => {
    const { file, cleanup } = tmp("x");
    const fd = fs.openSync(file, "r+");
    await call((cb) => F.futimes(fd, 1_000_000, 1_000_000, cb));
    fs.closeSync(fd);
    // Whole-second timestamp, so `mtimeMs` and `mtime.getTime()` would agree.
    expect(Math.round(fs.statSync(file).mtime.getTime() / 1000)).toBe(1_000_000);
    cleanup();
  });

  it("futimes on a bad descriptor is EBADF", async () => {
    await expectBadFd("futimes", (cb) => F.futimes(999999, 1, 1, cb));
  });

  it("readv fills each buffer from the shared cursor", async () => {
    const { file, cleanup } = tmp("0123456789");
    const fd = fs.openSync(file, "r");
    const bufs = [Buffer.alloc(2), Buffer.alloc(2)];
    const [n] = await call<any>((cb) => F.readv(fd, bufs, null, cb));
    expect(n).toBe(4);
    expect(bufs.map((b) => b.toString())).toEqual(["01", "23"]);
    // The cursor advanced, so a plain readSync continues from there.
    const next = Buffer.alloc(2);
    expect(fs.readSync(fd, next, 0, 2, null)).toBe(2);
    expect(next.toString()).toBe("45");
    fs.closeSync(fd);
    cleanup();
  });

  it("readv on a bad descriptor is EBADF", async () => {
    await expectBadFd("read", (cb) => F.readv(999999, [Buffer.alloc(1)], null, cb));
  });

  it("readv rejects an empty iovec with EINVAL while writev returns 0", async () => {
    // Measured on Node: these two disagree. `readv([])` is EINVAL with syscall
    // `read`; `writev([])` is a successful write of nothing.
    const { file, cleanup } = tmp("abc");
    const fd = fs.openSync(file, "r+");

    let err: any;
    try {
      await call((cb) => F.readv(fd, [], null, cb));
    } catch (e) {
      err = e;
    }
    expect(`${err && err.code}:${err && err.syscall}`).toBe("EINVAL:read");

    const [n] = await call<any>((cb) => F.writev(fd, [], null, cb));
    expect(n).toBe(0);

    fs.closeSync(fd);
    cleanup();
  });

  it("accepts a mixed Buffer/TypedArray iovec", async () => {
    const { file, cleanup } = tmp("0123456789");
    const fd = fs.openSync(file, "r");
    const bufs: any[] = [Buffer.alloc(2), new Uint8Array(2)];
    const [n] = await call<any>((cb) => F.readv(fd, bufs, null, cb));
    expect(n).toBe(4);
    fs.closeSync(fd);
    cleanup();
  });

  it("rejects non-binary iovec entries", async () => {
    const { file, cleanup } = tmp("abc");
    const fd = fs.openSync(file, "r");
    let err: any;
    try {
      await call((cb) => F.readv(fd, [1], null, cb));
    } catch (e) {
      err = e;
    }
    expect(err && err.code).toBe("ERR_INVALID_ARG_TYPE");
    fs.closeSync(fd);
    cleanup();
  });

  it("writev writes every buffer and reports the total", async () => {
    const { file, cleanup } = tmp("0123456789");
    const fd = fs.openSync(file, "r+");
    const [n] = await call<any>((cb) =>
      F.writev(fd, [Buffer.from("AB"), new Uint8Array([0x43])], 0, cb)
    );
    expect(n).toBe(3);
    fs.closeSync(fd);
    expect(fs.readFileSync(file, "utf8")).toBe("ABC3456789");
    cleanup();
  });

  it("writev on a bad descriptor is EBADF", async () => {
    await expectBadFd("write", (cb) => F.writev(999999, [Buffer.from("x")], 0, cb));
  });

  it("writev to /dev/full reports ENOSPC", async () => {
    // The one case where a wrong short-write semantic would silently lose data.
    let fd: number;
    try {
      fd = fs.openSync("/dev/full", "w");
    } catch {
      return; // not present on this platform
    }
    try {
      let err: any;
      try {
        await call((cb) => F.writev(fd, [Buffer.from("ab"), Buffer.from("cd")], 0, cb));
      } catch (e) {
        err = e;
      }
      expect(err && err.code).toBe("ENOSPC");
    } finally {
      fs.closeSync(fd);
    }
  });
});
