import fs from "node:fs";
import path from "node:path";
import os from "node:os";

// The asynchronous and synchronous fd family: `fs.read`, `fs.write`,
// `fs.fstat`, `fs.ftruncate`, `fs.fchmod`, `fs.fchown`, `fs.fsync`,
// `fs.fdatasync`, `fs.futimes`, `fs.readv`, `fs.writev`, and their Sync twins.

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

  it("readv and writev pass buffers as the 3rd callback argument", async () => {
    const { file, cleanup } = tmp("abcdef");
    const fd = fs.openSync(file, "r+");
    try {
      const wbufs = [Buffer.from("XY"), Buffer.from("ZW")];
      const [wn, wOut] = await call<any>((cb) => F.writev(fd, wbufs, 0, cb));
      expect(wn).toBe(4);
      expect(wOut).toBe(wbufs);

      const rbufs = [Buffer.alloc(2), Buffer.alloc(2)];
      const [rn, rOut] = await call<any>((cb) => F.readv(fd, rbufs, 0, cb));
      expect(rn).toBe(4);
      expect(rOut).toBe(rbufs);
      expect(rbufs.map((b) => b.toString())).toEqual(["XY", "ZW"]);
    } finally {
      fs.closeSync(fd);
      cleanup();
    }
  });

  it("fs.read supports positional, options object, and buffer-only overloads", async () => {
    const { file, cleanup } = tmp("0123456789");
    const fd = fs.openSync(file, "r");
    try {
      const b1 = Buffer.alloc(6);
      const [n1, out1] = await call<any>((cb) => F.read(fd, b1, 1, 4, 2, cb));
      expect(n1).toBe(4);
      expect(out1).toBe(b1);
      expect(b1.subarray(1, 5).toString()).toBe("2345");

      const b2 = Buffer.alloc(3);
      const [n2, out2] = await call<any>((cb) =>
        F.read(fd, { buffer: b2, offset: 0, length: 3, position: 6 }, cb)
      );
      expect(n2).toBe(3);
      expect(out2).toBe(b2);
      expect(b2.toString()).toBe("678");

      const b3 = Buffer.alloc(4);
      const [n3, out3] = await call<any>((cb) => F.read(fd, b3, cb));
      expect(n3).toBe(4);
      expect(out3).toBe(b3);
      expect(b3.toString()).toBe("0123");
    } finally {
      fs.closeSync(fd);
      cleanup();
    }
  });

  it("fs.write supports buffer, string, and options object overloads", async () => {
    const { file, cleanup } = tmp("0123456789");
    const fd = fs.openSync(file, "r+");
    try {
      const src = Buffer.from("_ABC_");
      const [wn1, wout1] = await call<any>((cb) => F.write(fd, src, 1, 3, 2, cb));
      expect(wn1).toBe(3);
      expect(wout1).toBe(src);

      const [wn2, wout2] = await call<any>((cb) => F.write(fd, "ZZ", 6, "utf8", cb));
      expect(wn2).toBe(2);
      expect(wout2).toBe("ZZ");

      const src2 = Buffer.from("QQ");
      const [wn3, wout3] = await call<any>((cb) =>
        F.write(fd, src2, { offset: 0, length: 2, position: 8 }, cb)
      );
      expect(wn3).toBe(2);
      expect(wout3).toBe(src2);

      expect(fs.readFileSync(file, "utf8")).toBe("01ABC5ZZQQ");
    } finally {
      fs.closeSync(fd);
      cleanup();
    }
  });

  it("fs.fstat supports { bigint: true } and fs.fchown sets owner", async () => {
    const { file, cleanup } = tmp("hello");
    const fd = fs.openSync(file, "r+");
    try {
      const [bigStats] = await call<any>((cb) => F.fstat(fd, { bigint: true }, cb));
      expect(typeof bigStats.size).toBe("bigint");
      expect(bigStats.size).toBe(5n);

      if (os.platform() !== "win32") {
        await call((cb) => F.fchown(fd, process.getuid(), process.getgid(), cb));
      }
    } finally {
      fs.closeSync(fd);
      cleanup();
    }
  });

  it("supports all synchronous fd variants", () => {
    const { file, cleanup } = tmp("0123456789");
    const fd = fs.openSync(file, "r+");
    try {
      const big = F.fstatSync(fd, { bigint: true });
      expect(typeof big.size).toBe("bigint");
      expect(big.size).toBe(10n);

      expect(F.writevSync(fd, [Buffer.from("AB"), Buffer.from("CD")], 1)).toBe(4);
      const rbufs = [Buffer.alloc(2), Buffer.alloc(2)];
      expect(F.readvSync(fd, rbufs, 1)).toBe(4);
      expect(rbufs.map((b) => b.toString())).toEqual(["AB", "CD"]);

      F.ftruncateSync(fd, 6);
      expect(fs.fstatSync(fd).size).toBe(6);

      F.futimesSync(fd, 1_100_000, 1_100_000);
      expect(Math.round(fs.fstatSync(fd).mtime.getTime() / 1000)).toBe(1_100_000);

      F.fsyncSync(fd);
      F.fdatasyncSync(fd);

      if (os.platform() !== "win32") {
        F.fchmodSync(fd, 0o600);
        expect(fs.fstatSync(fd).mode & 0o777).toBe(0o600);
        F.fchownSync(fd, process.getuid(), process.getgid());
      }
    } finally {
      fs.closeSync(fd);
      cleanup();
    }
  });

  it("handles explicit undefined optional arguments and numeric open flags", async () => {
    const { dir, file, cleanup } = tmp("hello");
    const subDir = path.join(dir, "sub");
    const linkPath = path.join(dir, "link.txt");
    try {
      await fs.promises.mkdir(subDir, undefined as any);
      const entries = await fs.promises.readdir(dir, undefined as any);
      expect(entries).toContain("sub");

      const content = await fs.promises.readFile(file, undefined as any);
      expect(Buffer.isBuffer(content)).toBe(true);
      expect((content as Buffer).toString()).toBe("hello");

      const fh = await fs.promises.open(file, undefined as any, undefined as any);
      const readRes = await fh.read(Buffer.alloc(5), 0, undefined as any, undefined as any);
      expect(readRes.bytesRead).toBe(5);
      expect(readRes.buffer.toString()).toBe("hello");
      await fh.close();

      const [cbFd] = await call<[number]>((cb) =>
        F.open(file, "r", undefined, cb)
      );
      expect(typeof cbFd).toBe("number");
      fs.closeSync(cbFd);

      const syncFd = fs.openSync(file, undefined as any, undefined as any);
      expect(typeof syncFd).toBe("number");
      fs.closeSync(syncFd);

      if (os.platform() !== "win32" && typeof fs.constants.O_NOFOLLOW === "number") {
        const fhNum = await fs.promises.open(
          file,
          (fs.constants.O_RDONLY | fs.constants.O_NOFOLLOW) as any,
          undefined as any
        );
        await fhNum.close();

        fs.symlinkSync(file, linkPath, undefined as any);
        let loopErr: any;
        try {
          await fs.promises.open(
            linkPath,
            (fs.constants.O_RDONLY | fs.constants.O_NOFOLLOW) as any,
            undefined as any
          );
        } catch (e) {
          loopErr = e;
        }
        expect(loopErr?.code).toBe("ELOOP");
      }
    } finally {
      cleanup();
    }
  });
});
