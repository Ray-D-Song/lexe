import fs from "node:fs";
import fsp from "node:fs/promises";
import os from "node:os";
import path from "node:path";

// Covers the `fs` surface the compat backlog listed as missing: the stream and
// directory classes on the namespace, and the `fs/promises` entries that had no
// implementation at all.

let dir = "";

const makeDir = () => fs.mkdtempSync(path.join(os.tmpdir(), "lexe-fs-streams-"));

beforeAll(() => {
  dir = makeDir();
});

afterAll(() => {
  fs.rmSync(dir, { recursive: true, force: true });
});

it("exports ReadStream, WriteStream and Dir", () => {
  expect(typeof fs.ReadStream).toBe("function");
  expect(typeof fs.WriteStream).toBe("function");
  expect(typeof fs.Dir).toBe("function");
});

it("createReadStream produces a ReadStream", () => {
  const file = path.join(dir, "read.txt");
  fs.writeFileSync(file, "content");
  const stream = fs.createReadStream(file);
  expect(stream instanceof fs.ReadStream).toBe(true);
  expect(stream instanceof require("node:stream").Readable).toBe(true);
  stream.destroy();
});

it("createWriteStream produces a WriteStream", () => {
  const file = path.join(dir, "write.txt");
  const stream = fs.createWriteStream(file);
  expect(stream instanceof fs.WriteStream).toBe(true);
  expect(stream instanceof require("node:stream").Writable).toBe(true);
  stream.destroy();
});

it("opendir yields entries and is a Dir", async () => {
  const target = makeDir();
  fs.writeFileSync(path.join(target, "one.txt"), "1");
  fs.writeFileSync(path.join(target, "two.txt"), "2");

  const dirHandle = await fsp.opendir(target);
  expect(dirHandle instanceof fs.Dir).toBe(true);

  const names: string[] = [];
  for await (const entry of dirHandle) names.push(entry.name);
  expect(names.sort()).toEqual(["one.txt", "two.txt"]);

  await dirHandle.close();
  fs.rmSync(target, { recursive: true, force: true });
});

it("cp copies a file and a directory tree", async () => {
  const src = path.join(dir, "cp-src");
  fs.mkdirSync(src, { recursive: true });
  fs.writeFileSync(path.join(src, "a.txt"), "alpha");

  const fileCopy = path.join(dir, "cp-file.txt");
  await fsp.cp(path.join(src, "a.txt"), fileCopy);
  expect(fs.readFileSync(fileCopy, "utf8")).toBe("alpha");

  const dirCopy = path.join(dir, "cp-dir");
  await fsp.cp(src, dirCopy, { recursive: true });
  expect(fs.readFileSync(path.join(dirCopy, "a.txt"), "utf8")).toBe("alpha");
});

it("link and readlink", async () => {
  const file = path.join(dir, "link-source.txt");
  fs.writeFileSync(file, "linked");

  const hard = path.join(dir, "link-hard.txt");
  await fsp.link(file, hard);
  expect(fs.readFileSync(hard, "utf8")).toBe("linked");

  const sym = path.join(dir, "link-sym.txt");
  await fsp.symlink(file, sym);
  expect(await fsp.readlink(sym)).toBe(file);
  expect(Buffer.isBuffer(await fsp.readlink(sym, { encoding: "buffer" }))).toBe(true);
});

it("truncate", async () => {
  const file = path.join(dir, "truncate.txt");
  fs.writeFileSync(file, "0123456789");
  await fsp.truncate(file, 4);
  expect(fs.readFileSync(file, "utf8")).toBe("0123");
});

it("chown and lchown accept the caller's own uid/gid", async () => {
  const file = path.join(dir, "chown.txt");
  fs.writeFileSync(file, "x");
  const uid = process.getuid();
  const gid = process.getgid();
  await expect(fsp.chown(file, uid, gid)).resolves.toBeUndefined();
  await expect(fsp.lchown(file, uid, gid)).resolves.toBeUndefined();
});

it("utimes and lutimes", async () => {
  const file = path.join(dir, "utimes.txt");
  fs.writeFileSync(file, "x");
  const seconds = Math.floor(Date.now() / 1000) - 60;
  await expect(fsp.utimes(file, seconds, seconds)).resolves.toBeUndefined();
  await expect(fsp.lutimes(file, seconds, seconds)).resolves.toBeUndefined();
});

it("statfs reports block sizes", async () => {
  const stats = await fsp.statfs(dir);
  expect(typeof stats.bsize).toBe("number");
  expect(stats.bsize).toBeGreaterThan(0);
  expect(typeof stats.blocks).toBe("number");
});

it("glob yields matching paths relative to cwd", async () => {
  const target = makeDir();
  fs.mkdirSync(path.join(target, "nested"), { recursive: true });
  fs.writeFileSync(path.join(target, "top.txt"), "1");
  fs.writeFileSync(path.join(target, "nested", "deep.txt"), "2");
  fs.writeFileSync(path.join(target, "nested", "skip.js"), "3");

  const found: string[] = [];
  for await (const match of fsp.glob("**/*.txt", { cwd: target })) {
    found.push(match as string);
  }
  expect(found.sort()).toEqual(["nested/deep.txt", "top.txt"]);

  fs.rmSync(target, { recursive: true, force: true });
});
