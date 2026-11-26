import fs from "node:fs";
import fsp from "node:fs/promises";
import os from "node:os";
import path from "node:path";

const F = fs as any;
const FP = fsp as any;

const makeTmpDir = () => fs.mkdtempSync(path.join(os.tmpdir(), "lexe-fs-parity-"));

const call = <T = any>(fn: (cb: (err: any, ...rest: any[]) => void) => void): Promise<T> =>
  new Promise((resolve, reject) => {
    fn((err: any, ...rest: any[]) => (err ? reject(err) : resolve(rest as any as T)));
  });

describe("fs and fs/promises full Node.js parity", () => {
  it("exports all Node.js fs and fs/promises members without undefined values", () => {
    for (const key of Object.keys(F)) {
      expect(F[key]).not.toBeUndefined();
    }
    const expectedCallbacks = [
      "unlink",
      "chmod",
      "lchmod",
      "chown",
      "lchown",
      "copyFile",
      "cp",
      "glob",
      "link",
      "lutimes",
      "mkdir",
      "mkdtemp",
      "opendir",
      "readlink",
      "rename",
      "rm",
      "rmdir",
      "statfs",
      "symlink",
      "truncate",
      "utimes",
      "read",
      "write",
      "fchown",
      "watchFile",
      "unwatchFile",
      "openAsBlob",
      "exists",
    ];
    for (const fnName of expectedCallbacks) {
      expect(typeof F[fnName]).toBe("function");
    }
    expect(F.FileReadStream).toBe(F.ReadStream);
    expect(F.FileWriteStream).toBe(F.WriteStream);

    for (const pName of ["unlink", "copyFile", "watch", "lchmod", "cp", "glob", "statfs"]) {
      expect(typeof FP[pName]).toBe("function");
    }
  });

  it("unlink, unlinkSync, and fsp.unlink remove files and symlinks and reject directories", async () => {
    const dir = makeTmpDir();
    try {
      const f1 = path.join(dir, "u1.txt");
      const f2 = path.join(dir, "u2.txt");
      const f3 = path.join(dir, "u3.txt");
      fs.writeFileSync(f1, "1");
      fs.writeFileSync(f2, "2");
      fs.writeFileSync(f3, "3");

      await call((cb) => F.unlink(f1, cb));
      expect(fs.existsSync(f1)).toBe(false);

      F.unlinkSync(f2);
      expect(fs.existsSync(f2)).toBe(false);

      await FP.unlink(f3);
      expect(fs.existsSync(f3)).toBe(false);

      // Unlinking a symlink removes the link without modifying target
      const target = path.join(dir, "target.txt");
      const link = path.join(dir, "sym.txt");
      fs.writeFileSync(target, "keep");
      fs.symlinkSync(target, link);
      await FP.unlink(link);
      expect(fs.existsSync(target)).toBe(true);

      // Unlinking a directory fails with EPERM / EISDIR
      const sub = path.join(dir, "subdir");
      fs.mkdirSync(sub);
      let err: any;
      try {
        F.unlinkSync(sub);
      } catch (e) {
        err = e;
      }
      expect(["EPERM", "EISDIR"]).toContain(err?.code);
      expect(err?.syscall).toBe("unlink");
    } finally {
      fs.rmSync(dir, { recursive: true, force: true });
    }
  });

  it("chmod callback and lchmod / lchmodSync / fsp.lchmod work cleanly", async () => {
    if (os.platform() === "win32") return;
    const dir = makeTmpDir();
    try {
      const file = path.join(dir, "chmod.txt");
      fs.writeFileSync(file, "x");
      await call((cb) => F.chmod(file, 0o600, cb));
      expect(fs.statSync(file).mode & 0o777).toBe(0o600);

      const sym = path.join(dir, "chmod-sym.txt");
      fs.symlinkSync(file, sym);
      if (os.platform() === "darwin") {
        await call((cb) => F.lchmod(sym, 0o700, cb));
        expect(fs.lstatSync(sym).mode & 0o777).toBe(0o700);
        F.lchmodSync(sym, 0o644);
        expect(fs.lstatSync(sym).mode & 0o777).toBe(0o644);
        await FP.lchmod(sym, 0o600);
        expect(fs.lstatSync(sym).mode & 0o777).toBe(0o600);
      } else {
        await expect(FP.lchmod(sym, 0o700)).rejects.toMatchObject({ code: "ENOTSUP" });
      }
    } finally {
      fs.rmSync(dir, { recursive: true, force: true });
    }
  });

  it("copyFile, copyFileSync, and fsp.copyFile support flags and error metadata", async () => {
    const dir = makeTmpDir();
    try {
      const src = path.join(dir, "src.txt");
      const dst = path.join(dir, "dst.txt");
      fs.writeFileSync(src, "copy-payload");

      F.copyFileSync(src, dst);
      expect(fs.readFileSync(dst, "utf8")).toBe("copy-payload");

      let exclErr: any;
      try {
        F.copyFileSync(src, dst, fs.constants.COPYFILE_EXCL);
      } catch (e) {
        exclErr = e;
      }
      expect(exclErr?.code).toBe("EEXIST");
      expect(exclErr?.dest).toBe(dst);

      await expect(FP.copyFile(src, dst, fs.constants.COPYFILE_EXCL)).rejects.toMatchObject({
        code: "EEXIST",
      });

      const dst2 = path.join(dir, "dst2.txt");
      await call((cb) => F.copyFile(src, dst2, cb));
      expect(fs.readFileSync(dst2, "utf8")).toBe("copy-payload");

      expect(() => F.copyFileSync(src, dst, 999)).toThrow();
    } finally {
      fs.rmSync(dir, { recursive: true, force: true });
    }
  });

  it("cp and cpSync support filter, errorOnExist, preserveTimestamps, and symlinks", async () => {
    const dir = makeTmpDir();
    try {
      const src = path.join(dir, "src");
      const dstSync = path.join(dir, "dst-sync");
      const dstAsync = path.join(dir, "dst-async");
      fs.mkdirSync(path.join(src, "sub"), { recursive: true });
      fs.writeFileSync(path.join(src, "keep.txt"), "keep");
      fs.writeFileSync(path.join(src, "skip.txt"), "skip");
      fs.writeFileSync(path.join(src, "sub", "nested.txt"), "nested");
      fs.utimesSync(path.join(src, "keep.txt"), 1_500_000, 1_500_000);

      F.cpSync(src, dstSync, {
        recursive: true,
        preserveTimestamps: true,
        filter: (s: string) => !s.endsWith("skip.txt"),
      });
      expect(fs.existsSync(path.join(dstSync, "keep.txt"))).toBe(true);
      expect(fs.existsSync(path.join(dstSync, "skip.txt"))).toBe(false);
      expect(fs.readFileSync(path.join(dstSync, "sub", "nested.txt"), "utf8")).toBe("nested");
      expect(Math.round(fs.statSync(path.join(dstSync, "keep.txt")).mtimeMs / 1000)).toBe(1_500_000);

      // Async filter in cpSync throws ERR_INVALID_RETURN_VALUE
      expect(() =>
        F.cpSync(path.join(src, "keep.txt"), path.join(dir, "bad.txt"), {
          filter: async () => true,
        })
      ).toThrow();

      // errorOnExist when force:false throws ERR_FS_CP_EEXIST
      let existErr: any;
      try {
        F.cpSync(path.join(src, "keep.txt"), path.join(dstSync, "keep.txt"), {
          force: false,
          errorOnExist: true,
        });
      } catch (e) {
        existErr = e;
      }
      expect(existErr?.code).toBe("ERR_FS_CP_EEXIST");

      // Async fsp.cp + callback fs.cp with async filter
      await FP.cp(src, dstAsync, {
        recursive: true,
        filter: async (s: string) => !s.endsWith("skip.txt"),
      });
      expect(fs.existsSync(path.join(dstAsync, "keep.txt"))).toBe(true);
      expect(fs.existsSync(path.join(dstAsync, "skip.txt"))).toBe(false);

      const dstCb = path.join(dir, "dst-cb");
      await call((cb) => F.cp(path.join(src, "keep.txt"), dstCb, cb));
      expect(fs.readFileSync(dstCb, "utf8")).toBe("keep");
    } finally {
      fs.rmSync(dir, { recursive: true, force: true });
    }
  });

  it("glob, globSync, and fsp.glob support pattern arrays, exclude, and withFileTypes", async () => {
    const dir = makeTmpDir();
    try {
      fs.mkdirSync(path.join(dir, "src"), { recursive: true });
      fs.mkdirSync(path.join(dir, "skipdir"), { recursive: true });
      fs.writeFileSync(path.join(dir, "a.ts"), "");
      fs.writeFileSync(path.join(dir, "b.md"), "");
      fs.writeFileSync(path.join(dir, "src", "c.ts"), "");
      fs.writeFileSync(path.join(dir, "skipdir", "d.ts"), "");

      const syncRes = F.globSync(["**/*.ts", "*.md"], {
        cwd: dir,
        exclude: ["skipdir/**"],
      });
      expect(syncRes.sort()).toEqual(["a.ts", "b.md", "src/c.ts"]);

      const fnExcluded = F.globSync("**/*.ts", {
        cwd: dir,
        exclude: (p: string) => p === "skipdir",
      });
      expect(fnExcluded.sort()).toEqual(["a.ts", "src/c.ts"]);

      const dirents = F.globSync("src/*.ts", { cwd: dir, withFileTypes: true });
      expect(dirents.length).toBe(1);
      expect(dirents[0]).toBeInstanceOf(fs.Dirent);
      expect(dirents[0].name).toBe("c.ts");
      expect(dirents[0].parentPath).toBe(path.join(dir, "src"));

      const [cbRes] = await call<any>((cb) => F.glob("*.md", { cwd: dir }, cb));
      expect(cbRes).toEqual(["b.md"]);

      const asyncItems: string[] = [];
      for await (const item of FP.glob(["**/*.ts"], { cwd: dir, exclude: ["skipdir/**"] })) {
        asyncItems.push(item);
      }
      expect(asyncItems.sort()).toEqual(["a.ts", "src/c.ts"]);
    } finally {
      fs.rmSync(dir, { recursive: true, force: true });
    }
  });

  it("opendir, opendirSync, Dir.readSync, and statfs / statfsSync work properly", async () => {
    const dir = makeTmpDir();
    try {
      fs.writeFileSync(path.join(dir, "one.txt"), "1");
      fs.writeFileSync(path.join(dir, "two.txt"), "2");

      const dSync = F.opendirSync(dir);
      expect(dSync).toBeInstanceOf(fs.Dir);
      const seen: string[] = [];
      for (;;) {
        const entry = dSync.readSync();
        if (entry === null) break;
        expect(entry).toBeInstanceOf(fs.Dirent);
        seen.push(entry.name);
      }
      expect(seen.sort()).toEqual(["one.txt", "two.txt"]);
      dSync.closeSync();
      expect(() => dSync.readSync()).toThrow();

      const [dCb] = await call<any>((cb) => F.opendir(dir, cb));
      const [firstEntry] = await call<any>((cb) => dCb.read(cb));
      expect(firstEntry).toBeInstanceOf(fs.Dirent);
      await dCb.close();

      const sfsSync = F.statfsSync(dir);
      expect(typeof sfsSync.bsize).toBe("number");
      expect(sfsSync.bsize).toBeGreaterThan(0);

      const sfsBig = F.statfsSync(dir, { bigint: true });
      expect(typeof sfsBig.bsize).toBe("bigint");
      expect(typeof sfsBig.blocks).toBe("bigint");

      const [sfsCb] = await call<any>((cb) => F.statfs(dir, { bigint: true }, cb));
      expect(typeof sfsCb.bsize).toBe("bigint");
    } finally {
      fs.rmSync(dir, { recursive: true, force: true });
    }
  });

  it("FileHandle supports position: -1, utimes, stat({ bigint }), readv/writev, appendFile, and streams", async () => {
    const dir = makeTmpDir();
    try {
      const file = path.join(dir, "fh.txt");
      fs.writeFileSync(file, "0123456789");

      const fh: any = await fsp.open(file, "r+");
      try {
        const a = Buffer.alloc(3);
        await fh.read(a, 0, 3, -1);
        const b = Buffer.alloc(6);
        // Also verifies partial buffer writes at offset > 0 do not clobber/panic
        const r2 = await fh.read(b, 2, 3, -1);
        expect(a.toString()).toBe("012");
        expect(r2.bytesRead).toBe(3);
        expect(b.subarray(2, 5).toString()).toBe("345");

        const sb = await fh.stat({ bigint: true });
        expect(typeof sb.size).toBe("bigint");
        expect(sb.size).toBe(10n);

        await fh.utimes(1_200_000, 1_200_000);
        expect(Math.round((await fh.stat()).mtimeMs / 1000)).toBe(1_200_000);

        await fh.write(Buffer.from("_XY_"), 1, 2, 0);
        expect(fs.readFileSync(file, "utf8")).toBe("XY23456789");

        const rv1 = Buffer.alloc(2);
        const rv2 = Buffer.alloc(2);
        const rvr = await fh.readv([rv1, rv2], 2);
        expect(rvr.bytesRead).toBe(4);
        expect([rv1.toString(), rv2.toString()]).toEqual(["23", "45"]);

        await fh.appendFile("+END");
        expect(fs.readFileSync(file, "utf8")).toBe("XY23456789+END");
      } finally {
        await fh.close();
      }
      expect(() => fs.fstatSync(fh.fd)).toThrow();

      // FileHandle.createReadStream / createWriteStream
      const streamFile = path.join(dir, "fh-stream.txt");
      const wfh: any = await fsp.open(streamFile, "w");
      await new Promise<void>((resolve, reject) => {
        const ws = wfh.createWriteStream();
        ws.on("close", resolve);
        ws.on("error", reject);
        ws.end("fh-stream-data");
      });
      const rfh: any = await fsp.open(streamFile, "r");
      let collected = "";
      for await (const chunk of rfh.createReadStream({ encoding: "utf8" })) {
        collected += chunk;
      }
      expect(collected).toBe("fh-stream-data");
    } finally {
      fs.rmSync(dir, { recursive: true, force: true });
    }
  });

  it("watchFile / unwatchFile and fsp.watch and openAsBlob and exists work", async () => {
    const dir = makeTmpDir();
    const target = path.join(dir, "watched.txt");
    fs.writeFileSync(target, "v1");
    try {
      // fs.exists
      expect(await new Promise((r) => F.exists(target, r))).toBe(true);
      expect(await new Promise((r) => F.exists(path.join(dir, "missing"), r))).toBe(false);

      // fs.openAsBlob
      const blob = await F.openAsBlob(target, { type: "text/plain" });
      expect(blob).toBeInstanceOf(Blob);
      expect(blob.type).toBe("text/plain");
      expect(blob.size).toBe(2);
      expect(await blob.text()).toBe("v1");

      // fs.watchFile & fs.unwatchFile
      let detected: any = null;
      const watcher = F.watchFile(target, { interval: 30, persistent: false, bigint: true }, (curr: any, prev: any) => {
        detected = { currSize: curr.size, prevSize: prev.size };
      });
      expect(typeof watcher.ref).toBe("function");
      expect(typeof watcher.unref).toBe("function");
      await new Promise((r) => setTimeout(r, 40));
      fs.writeFileSync(target, "v1-updated");
      const deadline = Date.now() + 2000;
      while (!detected && Date.now() < deadline) {
        await new Promise((r) => setTimeout(r, 30));
      }
      F.unwatchFile(target);
      expect(detected).not.toBeNull();
      expect(typeof detected.currSize).toBe("bigint");
      expect(detected.currSize).toBe(10n);

      // fs/promises.watch + AbortController
      const ac = new AbortController();
      const it = FP.watch(dir, { signal: ac.signal })[Symbol.asyncIterator]();
      setTimeout(() => fs.writeFileSync(target, "v2"), 30);
      const ev = await it.next();
      expect(ev.done).toBe(false);
      expect(typeof ev.value.eventType).toBe("string");
      ac.abort();
      await expect(it.next()).rejects.toMatchObject({ name: "AbortError", code: "ABORT_ERR" });
    } finally {
      F.unwatchFile(target);
      fs.rmSync(dir, { recursive: true, force: true });
    }
  });

  it("chmod and chmodSync report ENOENT error codes for missing files", async () => {
    const dir = makeTmpDir();
    const missing = path.join(dir, "missing.db-shm");
    try {
      await expect(FP.chmod(missing, 0o600)).rejects.toMatchObject({
        code: "ENOENT",
        syscall: "chmod",
      });
      let syncErr: any;
      try {
        fs.chmodSync(missing, 0o600);
      } catch (e) {
        syncErr = e;
      }
      expect(syncErr?.code).toBe("ENOENT");
      expect(syncErr?.syscall).toBe("chmod");
    } finally {
      fs.rmSync(dir, { recursive: true, force: true });
    }
  });
});
