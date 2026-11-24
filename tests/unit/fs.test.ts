import defaultImport from "node:fs";
import legacyImport from "fs";

import path from "node:path";
import os from "node:os";
const IS_WINDOWS = os.platform() === "win32";

it("node:fs should be the same as fs", () => {
  expect(defaultImport).toStrictEqual(legacyImport);
});

const {
  constants,
  access: accessCb,
  accessSync,
  readdirSync,
  readFileSync,
  mkdtempSync,
  mkdirSync,
  renameSync,
  rmSync,
  rmdirSync,
  stat: statCb,
  statSync,
  lstat: lstatCb,
  lstatSync,
  symlinkSync,
  writeFileSync,
  appendFile: appendFileCb,
  appendFileSync,
  realpathSync,
  realpath,
  existsSync,
  watch,
  FSWatcher,
  createReadStream,
  promises,
} = defaultImport;

const {
  access,
  mkdir,
  mkdtemp,
  readdir,
  readFile,
  rename,
  rm,
  rmdir,
  lstat,
  symlink,
  writeFile,
  appendFile: appendFilePromise,
  realpath: realpathPromise,
} = promises;

function waitForWatchEvent(
  watcher: InstanceType<typeof FSWatcher>,
  action: () => void | Promise<void>
): Promise<[string, string | null]> {
  return new Promise((resolve, reject) => {
    const timeout = setTimeout(() => {
      watcher.close();
      reject(new Error("Timed out waiting for fs.watch event"));
    }, 5000);

    watcher.once("change", (eventType: string, filename: string | null) => {
      clearTimeout(timeout);
      resolve([eventType, filename]);
    });

    Promise.resolve(action()).catch((error) => {
      clearTimeout(timeout);
      watcher.close();
      reject(error);
    });
  });
}

describe("watch", () => {
  it("should watch file changes", async () => {
    const dirPath = await mkdtemp(path.join(os.tmpdir(), "watch-"));
    const filePath = path.join(dirPath, "file.txt");
    await writeFile(filePath, "before");

    const watcher = watch(filePath);
    try {
      const [eventType, filename] = await waitForWatchEvent(watcher, () =>
        writeFile(filePath, "after")
      );

      expect(["change", "rename"]).toContain(eventType);
      expect(filename === null || typeof filename === "string").toBeTruthy();
    } finally {
      watcher.close();
      await rm(dirPath, { recursive: true, force: true });
    }
  });

  it("should support listener argument, ref, unref, and close", async () => {
    const dirPath = await mkdtemp(path.join(os.tmpdir(), "watch-"));
    const filePath = path.join(dirPath, "file.txt");
    await writeFile(filePath, "before");

    let eventCount = 0;
    const watcher = watch(filePath, () => {
      eventCount++;
    });

    try {
      expect(watcher).toBeInstanceOf(FSWatcher);
      expect(watcher.ref()).toBe(watcher);
      expect(watcher.unref()).toBe(watcher);

      await waitForWatchEvent(watcher, () => writeFile(filePath, "after"));
      expect(eventCount).toBeGreaterThan(0);

      let closed = false;
      watcher.once("close", () => {
        closed = true;
      });
      watcher.close();
      watcher.close();
      expect(closed).toBeTruthy();
    } finally {
      watcher.close();
      await rm(dirPath, { recursive: true, force: true });
    }
  });

  it("should expose watch from node:fs and fs", () => {
    expect(defaultImport.watch).toBe(legacyImport.watch);
    expect(defaultImport.FSWatcher).toBe(legacyImport.FSWatcher);
  });
});

describe("readdir", () => {
  it("should read a directory", async () => {
    const dir = await readdir(".cargo");
    expect(dir).toEqual(["config.toml"]);
  });

  it("should read a directory with types", async () => {
    const dir = await readdir(".cargo", { withFileTypes: true });
    expect(dir).toEqual([
      {
        name: "config.toml",
        parentPath: ".cargo",
      },
    ]);
    expect(dir[0].isFile()).toBeTruthy();
  });

  it("should read a directory with types", async () => {
    const dir = await readdir(".cargo/", { withFileTypes: true });
    expect(dir).toEqual([
      {
        name: "config.toml",
        parentPath: ".cargo",
      },
    ]);
    expect(dir[0].isFile()).toBeTruthy();
  });

  it("should read a directory", async () => {
    const dir = await readdir(".cargo");
    expect(dir).toEqual(["config.toml"]);
  });

  it("should read a directory with recursive", async () => {
    const dir = await readdir("fixtures/fs/readdir", {
      recursive: true,
    });
    const compare = (a: string, b: string) => (a >= b ? 1 : -1);
    expect(dir.sort(compare)).toEqual(
      [
        IS_WINDOWS ? "recursive\\readdir.js" : "recursive/readdir.js",
        "recursive",
        "readdir.js",
      ].sort(compare)
    );
  });
});

describe("readdirSync", () => {
  it("should read a directory synchronously", () => {
    const dir = readdirSync(".cargo");
    expect(dir).toEqual(["config.toml"]);
  });

  it("should read a directory with types synchronously", () => {
    const dir = readdirSync(".cargo", {
      withFileTypes: true,
    });
    expect(dir).toEqual([
      {
        name: "config.toml",
        parentPath: ".cargo",
      },
    ]);
    expect(dir[0].isFile()).toBeTruthy();
  });

  it("should read a directory synchronously", () => {
    const dir = readdirSync(".cargo");
    expect(dir).toEqual(["config.toml"]);
  });

  it("should read a directory with recursive synchronously", () => {
    const dir = readdirSync("fixtures/fs/readdir", {
      recursive: true,
    });
    const compare = (a: string | Buffer, b: string | Buffer): number =>
      a >= b ? 1 : -1;
    expect(dir.sort(compare)).toEqual(
      [
        IS_WINDOWS ? "recursive\\readdir.js" : "recursive/readdir.js",
        "recursive",
        "readdir.js",
      ].sort(compare)
    );
  });
});

describe("readfile", () => {
  it("should read a file", async () => {
    const buf = await readFile("fixtures/hello.txt");
    const text = buf.toString();
    const base64Text = buf.toString("base64");
    const hexText = buf.toString("hex");

    expect(buf).toBeInstanceOf(Buffer);
    expect(buf).toBeInstanceOf(Uint8Array);
    expect(text).toEqual("hello world!");
    expect(base64Text).toEqual("aGVsbG8gd29ybGQh");
    expect(hexText).toEqual("68656c6c6f20776f726c6421");
  });

  it("should return a string when encoding is provided as option", async () => {
    const text = await readFile("fixtures/hello.txt", {
      encoding: "utf-8",
    });
    expect(typeof text).toEqual("string");
    expect(text).toEqual("hello world!");
  });

  it("should return a string when encoding is provided as string", async () => {
    const text = await readFile("fixtures/hello.txt", "utf-8");
    expect(typeof text).toEqual("string");
    expect(text).toEqual("hello world!");
  });

  it("should return a string when encoding is provided as string with different cases", async () => {
    // @ts-ignore
    const text = await readFile("fixtures/hello.txt", "Utf-8");
    expect(typeof text).toEqual("string");
    expect(text).toEqual("hello world!");
  });
});

describe("readfileSync", () => {
  it("should read a file synchronously", () => {
    const buf = readFileSync("fixtures/hello.txt");
    const text = buf.toString();
    const base64Text = buf.toString("base64");
    const hexText = buf.toString("hex");

    expect(buf).toBeInstanceOf(Buffer);
    expect(buf).toBeInstanceOf(Uint8Array);
    expect(text).toEqual("hello world!");
    expect(base64Text).toEqual("aGVsbG8gd29ybGQh");
    expect(hexText).toEqual("68656c6c6f20776f726c6421");
  });

  it("should return a string when encoding is provided as option synchronously", () => {
    const text = readFileSync("fixtures/hello.txt", {
      encoding: "utf-8",
    });
    expect(typeof text).toEqual("string");
    expect(text).toEqual("hello world!");
  });

  it("should return a string when encoding is provided as string synchronously", () => {
    const text = readFileSync("fixtures/hello.txt", "utf-8");
    expect(typeof text).toEqual("string");
    expect(text).toEqual("hello world!");
  });

  it("should return a string when encoding is provided as string with different cases synchronously", async () => {
    // @ts-ignore
    const text = readFileSync("fixtures/hello.txt", "Utf-8");
    expect(typeof text).toEqual("string");
    expect(text).toEqual("hello world!");
  });
});

describe("mkdtemp", () => {
  it("should create a temporary directory with a given prefix", async () => {
    // Create a temporary directory with the given prefix
    const prefix = "test-";
    const dirPath = await mkdtemp(path.join(os.tmpdir(), prefix));

    // Check that the directory exists
    const dirExists = await promises
      .stat(dirPath)
      .then(() => true)
      .catch(() => false);
    expect(dirExists).toBeTruthy();

    // Check that the directory has the correct prefix
    const dirPrefix = path.basename(dirPath).slice(0, prefix.length);
    expect(dirPrefix).toEqual(prefix);

    // Clean up the temporary directory
    await rmdir(dirPath);
  });
});

describe("mkdtempSync", () => {
  it("should create a temporary directory with a given prefix synchronously", () => {
    // Create a temporary directory with the given prefix
    const prefix = "test-";
    const dirPath = mkdtempSync(path.join(os.tmpdir(), prefix));

    // Check that the directory exists
    const dirExists = statSync(dirPath);
    expect(dirExists).toBeTruthy();

    // Check that the directory has the correct prefix
    const dirPrefix = path.basename(dirPath).slice(0, prefix.length);
    expect(dirPrefix).toEqual(prefix);

    // Clean up the temporary directory
    rmdirSync(dirPath);
  });
});

describe("mkdir", () => {
  it("should create a directory with the given path", async () => {
    const dirPath = await mkdtemp(path.join(os.tmpdir(), "test/test-"));

    //non recursive should reject
    await expect(mkdir(dirPath)).rejects.toThrow(/dir/);

    await mkdir(dirPath, { recursive: true });

    // Check that the directory exists
    const dirExists = await checkDirExists(dirPath);
    expect(dirExists).toBeTruthy();

    await rmdir(dirPath, { recursive: true });

    await mkdir(`${dirPath}/./`, { recursive: true });

    // Check that the directory exists
    const dirExists2 = await checkDirExists(dirPath);
    expect(dirExists2).toBeTruthy();

    // Clean up the directory
    await rmdir(dirPath, { recursive: true });
  });
});

describe("mkdirSync", () => {
  it("should create a directory with the given path synchronously", () => {
    const dirPath = mkdtempSync(path.join(os.tmpdir(), "test/test-"));

    //non recursive should reject
    expect(() => mkdirSync(dirPath)).toThrow(
      IS_WINDOWS ? /Can\'t create dir/ : /[fF]ile.*exists/
    );

    mkdirSync(dirPath, { recursive: true });

    // Check that the directory exists
    const dirExists = statSync(dirPath);
    expect(dirExists).toBeTruthy();

    // Clean up the directory
    rmdirSync(dirPath, { recursive: true });
  });
});

describe("writeFile", () => {
  it("should write a file", async () => {
    const tmpDir = await mkdtemp(path.join(os.tmpdir(), "test-"));
    const filePath = path.join(tmpDir, "test");
    const fileContents = "hello";
    await writeFile(filePath, fileContents);

    const contents = (await readFile(filePath)).toString();

    expect(fileContents).toEqual(contents);

    await rmdir(tmpDir, { recursive: true });
  });

  if (!IS_WINDOWS) {
    it("should write file with permissions", async () => {
      const tmpDir = await mkdtemp(path.join(os.tmpdir(), "test-"));
      const filePath = path.join(tmpDir, "test");
      const fileContents = "hello";
      const mode = 0o644;
      await writeFile(filePath, fileContents, { mode });

      const stats = statSync(filePath);
      expect(stats.mode & 0o777).toEqual(mode);

      await rmdir(tmpDir, { recursive: true });
    });
  }
});

describe("writeFileSync", () => {
  it("should write a file", () => {
    const tmpDir = mkdtempSync(path.join(os.tmpdir(), "test-"));
    const filePath = path.join(tmpDir, "test");
    const fileContents = "hello";
    writeFileSync(filePath, fileContents);

    const contents = readFileSync(filePath).toString();

    expect(fileContents).toEqual(contents);

    rmdirSync(tmpDir, { recursive: true });
  });

  if (!IS_WINDOWS) {
    it("should write file with permissions", async () => {
      const tmpDir = await mkdtemp(path.join(os.tmpdir(), "test-"));
      const filePath = path.join(tmpDir, "test");
      const fileContents = "hello";
      const mode = 0o644;
      writeFileSync(filePath, fileContents, { mode });

      const stats = statSync(filePath);
      expect(stats.mode & 0o777).toEqual(mode);

      rmdirSync(tmpDir, { recursive: true });
    });
  }
});

describe("rm", () => {
  it("should delete file and directory", async () => {
    const tmpDir = await mkdtemp(path.join(os.tmpdir(), "test-"));
    const filePath = path.join(tmpDir, "test");
    const fileContents = "hello";
    await writeFile(filePath, fileContents);

    const contents = (await readFile(filePath)).toString();
    expect(fileContents).toEqual(contents);

    // Should delete file
    await rm(filePath, { recursive: true });
    await expect(access(filePath)).rejects.toThrow(
      /[Nn]o such file or directory/
    );

    // Check dir still exists and then delete it
    await access(tmpDir);
    await rm(tmpDir, { recursive: true });
    await expect(access(filePath)).rejects.toThrow(
      /[Nn]o such file or directory/
    );
  });
  it("should throw an error if file does not exists", async () => {
    const tmpDir = mkdtempSync(path.join(os.tmpdir(), "test-"));
    const filePath = path.join(tmpDir, "test");

    await expect(rm(filePath, {})).rejects.toThrow(
      IS_WINDOWS ? /\(os error 2\)/ : /[Nn]o such file or directory/
    );
  });
  it("should not throw an error if file does not exists and force is used", async () => {
    const tmpDir = await mkdtemp(path.join(os.tmpdir(), "test-"));
    const filePath = path.join(tmpDir, "test");

    await expect(access(filePath)).rejects.toThrow(
      /[Nn]o such file or directory/
    );

    // Should not throw an exception since it does not exists
    await rm(filePath, { force: true, recursive: true });
  });
});
describe("rmSync", () => {
  it("should delete file and directory with rm synchronously", async () => {
    const tmpDir = mkdtempSync(path.join(os.tmpdir(), "test-"));
    const filePath = path.join(tmpDir, "test");
    const fileContents = "hello";
    await writeFile(filePath, fileContents);

    const contents = readFileSync(filePath).toString();

    expect(fileContents).toEqual(contents);

    // Should delete file
    rmSync(filePath, { recursive: true });
    expect(() => accessSync(filePath)).toThrow(/[Nn]o such file or directory/);

    // Check dir still exists and then delete it
    accessSync(tmpDir);
    rmSync(tmpDir, { recursive: true });
    expect(() => accessSync(tmpDir)).toThrow(/[Nn]o such file or directory/);
  });
  it("should throw an error if file does not exists with rm synchronously", async () => {
    const tmpDir = mkdtempSync(path.join(os.tmpdir(), "test-"));
    const filePath = path.join(tmpDir, "test");

    expect(() => rmSync(filePath, {})).toThrow(
      IS_WINDOWS ? /\(os error 2\)/ : /[Nn]o such file or directory/
    );
  });
  it("should not throw an error if file does not exists and force is used with rm synchronously", async () => {
    const tmpDir = mkdtempSync(path.join(os.tmpdir(), "test-"));
    const filePath = path.join(tmpDir, "test");

    expect(() => accessSync(filePath)).toThrow(/[Nn]o such file or directory/);

    // Should not throw an exception since it does not exists
    rmSync(filePath, { force: true, recursive: true });
  });
});

describe("access", () => {
  it("should access a file", async () => {
    const filePath = "fixtures/hello.txt";
    await access(filePath);
  });

  it("should handle execute permission check", async () => {
    const filePath = "fixtures/hello.txt";
    if (IS_WINDOWS) {
      // On Windows, X_OK doesn't check Unix-style execute bits
      // Windows determines executability by file extension, so X_OK typically succeeds
      await access(filePath, constants.X_OK);
    } else {
      // On Unix, X_OK throws for files without execute permission
      await expect(access(filePath, constants.X_OK)).rejects.toThrow(
        /[pP]ermission denied/
      );
    }
  });

  it("should throw if not exists", async () => {
    const filePath = "fixtures/nothing";
    await expect(access(filePath)).rejects.toThrow(
      /[nN]o such file or directory/
    );
  });

  it("should access a file", async () => {
    const filePath = "fixtures/hello.txt";
    await access(filePath);
  });
});

describe("accessSync", () => {
  it("should access a file synchronously", () => {
    const filePath = "fixtures/hello.txt";
    accessSync(filePath);
  });

  it("should handle execute permission check synchronously", () => {
    const filePath = "fixtures/hello.txt";
    if (IS_WINDOWS) {
      // On Windows, X_OK doesn't check Unix-style execute bits
      // Windows determines executability by file extension, so X_OK typically succeeds
      accessSync(filePath, constants.X_OK);
    } else {
      // On Unix, X_OK throws for files without execute permission
      expect(() => accessSync(filePath, constants.X_OK)).toThrow(
        /[pP]ermission denied/
      );
    }
  });

  it("should throw if not exists synchronously", () => {
    const filePath = "fixtures/nothing";
    expect(() => accessSync(filePath)).toThrow(/[Nn]o such file or directory/);
  });
});

describe("rename", () => {
  it("should rename a directory", async () => {
    const tmpDir = await mkdtemp(path.join(os.tmpdir(), "test-"));
    const oldPath = path.join(tmpDir, "old");
    const newPath = path.join(tmpDir, "new");

    await mkdir(oldPath);
    await rename(oldPath, newPath);

    const oldDirExists = await checkDirExists(oldPath);
    const newDirExists = await checkDirExists(newPath);

    expect(oldDirExists).toBeFalsy();
    expect(newDirExists).toBeTruthy();

    // Cleanup
    await rmdir(tmpDir, { recursive: true });
  });

  it("should throw error if source doesn't exist", async () => {
    const tmpDir = await mkdtemp(path.join(os.tmpdir(), "test-"));
    const oldPath = path.join(tmpDir, "nonexistent");
    const newPath = path.join(tmpDir, "new");

    await expect(rename(oldPath, newPath)).rejects.toThrow(
      IS_WINDOWS ? /Can't rename/ : /[Nn]o such file or directory/
    );

    await rmdir(tmpDir, { recursive: true });
  });
});

describe("renameSync", () => {
  it("should rename a directory synchronously", () => {
    const tmpDir = mkdtempSync(path.join(os.tmpdir(), "test-"));
    const oldPath = path.join(tmpDir, "old");
    const newPath = path.join(tmpDir, "new");

    mkdirSync(oldPath);
    renameSync(oldPath, newPath);

    // Check if old path doesn't exist (should throw)
    // Windows previously returned "Can't stat"; shared fs errors use ENOENT-style messages.
    expect(() => statSync(oldPath)).toThrow(
      /[Nn]o such file or directory|os error 2|ENOENT/
    );

    // Check if new path exists and is a directory
    const newDirStat = statSync(newPath);
    expect(newDirStat.isDirectory()).toBeTruthy();

    // Cleanup
    rmdirSync(tmpDir, { recursive: true });
  });

  it("should throw error if source doesn't exist synchronously", () => {
    const tmpDir = mkdtempSync(path.join(os.tmpdir(), "test-"));
    const oldPath = path.join(tmpDir, "nonexistent");
    const newPath = path.join(tmpDir, "new");

    expect(() => renameSync(oldPath, newPath)).toThrow(
      IS_WINDOWS ? /Can't rename/ : /[Nn]o such file or directory/
    );

    rmdirSync(tmpDir, { recursive: true });
  });
});

describe("symlink", () => {
  it("should create a symlink", async () => {
    const tmpDir = mkdtempSync(path.join(os.tmpdir(), "test-"));
    const filePath = path.join(tmpDir, "file");
    const linkPath = path.join(tmpDir, "link");

    const expectedContent = "hello world";
    await writeFile(filePath, expectedContent);
    await symlink(filePath, linkPath);

    // Check if new path exists and is a symlink
    const linkStat = await lstat(linkPath);
    expect(linkStat.isSymbolicLink()).toBeTruthy();

    // Verify symlink works by reading content through it
    const content = await readFile(linkPath, "utf-8");
    expect(content).toBe(expectedContent);

    // Cleanup
    rmdirSync(tmpDir, { recursive: true });
  });
});

describe("symlinkSync", () => {
  it("should create a symlink synchronously", () => {
    const tmpDir = mkdtempSync(path.join(os.tmpdir(), "test-"));
    const filePath = path.join(tmpDir, "file");
    const linkPath = path.join(tmpDir, "link");

    const expectedContent = "hello world";
    writeFileSync(filePath, expectedContent);
    symlinkSync(filePath, linkPath);

    // Check if new path exists and is a symlink
    const linkStat = lstatSync(linkPath);
    expect(linkStat.isSymbolicLink()).toBeTruthy();

    // Verify symlink works by reading content through it
    const content = readFileSync(linkPath, "utf-8");
    expect(content).toBe(expectedContent);

    // Cleanup
    rmdirSync(tmpDir, { recursive: true });
  });
});

describe("realpath", () => {
  it("should resolve absolute and relative paths synchronously", () => {
    const tmpDir = mkdtempSync(path.join(os.tmpdir(), "realpath-"));
    const filePath = path.join(tmpDir, "file.txt");
    writeFileSync(filePath, "data");

    const absolute = realpathSync(filePath);
    expect(path.isAbsolute(absolute)).toBeTruthy();
    expect(absolute).toBe(path.join(realpathSync(tmpDir), "file.txt"));
    expect(realpathSync(absolute)).toBe(absolute);

    const relativeFile = path.relative(process.cwd(), filePath);
    expect(realpathSync(relativeFile)).toBe(absolute);

    rmdirSync(tmpDir, { recursive: true });
  });

  it("should resolve directories and follow symlinks", () => {
    const tmpDir = mkdtempSync(path.join(os.tmpdir(), "realpath-"));
    const filePath = path.join(tmpDir, "file.txt");
    const linkPath = path.join(tmpDir, "link.txt");
    writeFileSync(filePath, "data");
    symlinkSync(filePath, linkPath);

    expect(path.isAbsolute(realpathSync(tmpDir))).toBeTruthy();
    expect(realpathSync(linkPath)).toBe(realpathSync(filePath));

    rmdirSync(tmpDir, { recursive: true });
  });

  it("should expose realpathSync.native and realpath.native", () => {
    expect(typeof realpathSync.native).toBe("function");
    expect(typeof realpath.native).toBe("function");

    const tmpDir = mkdtempSync(path.join(os.tmpdir(), "realpath-"));
    const filePath = path.join(tmpDir, "file.txt");
    writeFileSync(filePath, "data");

    expect(realpathSync.native(filePath)).toBe(realpathSync(filePath));

    rmdirSync(tmpDir, { recursive: true });
  });

  it("should accept Buffer and file: URL inputs", async () => {
    const { pathToFileURL } = await import("node:url");
    const tmpDir = mkdtempSync(path.join(os.tmpdir(), "realpath-"));
    const filePath = path.join(tmpDir, "file.txt");
    writeFileSync(filePath, "data");
    const expected = realpathSync(filePath);

    expect(realpathSync(Buffer.from(filePath))).toBe(expected);
    expect(realpathSync(pathToFileURL(filePath))).toBe(expected);

    rmdirSync(tmpDir, { recursive: true });
  });

  it("realpathSync accepts broad ArrayBufferView inputs (not CRS-restricted)", () => {
    const tmpDir = mkdtempSync(path.join(os.tmpdir(), "realpath-"));
    const filePath = path.join(tmpDir, "file.txt");
    writeFileSync(filePath, "data");
    const expected = realpathSync(filePath);
    const bytes = Buffer.from(filePath, "utf8");
    const ab = bytes.buffer.slice(
      bytes.byteOffset,
      bytes.byteOffset + bytes.byteLength
    );

    // UTF-8 path bytes via ArrayBuffer / DataView / Int8Array must resolve.
    expect(realpathSync(ab)).toBe(expected);
    expect(realpathSync(new DataView(ab))).toBe(expected);
    expect(realpathSync(new Int8Array(bytes))).toBe(expected);

    // Uint16Array is accepted (enters FS resolution); must not sync-TypeError.
    // Content is raw LE u16 bytes, so ENOENT is expected — not TypeError.
    try {
      realpathSync(new Uint16Array([0x2f]));
    } catch (err: any) {
      expect(err).not.toBeInstanceOf(TypeError);
      expect(String(err.message || err)).not.toMatch(
        /must be of type string or an instance of Buffer or URL/i
      );
    }

    rmdirSync(tmpDir, { recursive: true });
  });

  it("should reject non-file URLs and invalid UTF-8 Buffer", async () => {
    expect(() => realpathSync(new URL("https://example.com/x"))).toThrow(
      /scheme file/i
    );
    expect(() => realpathSync(Buffer.from([0xff, 0xfe, 0xfd]))).toThrow(
      /UTF-8/i
    );
  });

  it("should reject encoded path separators in file: URL input", () => {
    expect(() => realpathSync(new URL("file:///tmp/a%2Fb"))).toThrow(
      /encoded \/ characters/i
    );
  });

  it("should support encoding options including buffer/hex/base64", () => {
    const tmpDir = mkdtempSync(path.join(os.tmpdir(), "realpath-"));
    const filePath = path.join(tmpDir, "file.txt");
    writeFileSync(filePath, "data");
    const expected = realpathSync(filePath);

    expect(realpathSync(filePath, "utf8")).toBe(expected);
    expect(realpathSync(filePath, { encoding: "utf8" })).toBe(expected);
    expect(realpathSync(filePath, "")).toBe(expected);
    expect(realpathSync(filePath, { encoding: "" })).toBe(expected);

    const asBuffer = realpathSync(filePath, "buffer");
    expect(Buffer.isBuffer(asBuffer)).toBeTruthy();
    expect(asBuffer.toString("utf8")).toBe(expected);

    const asHex = realpathSync(filePath, { encoding: "hex" });
    expect(asHex).toBe(Buffer.from(expected).toString("hex"));

    const asBase64 = realpathSync(filePath, "base64");
    expect(asBase64).toBe(Buffer.from(expected).toString("base64"));

    rmdirSync(tmpDir, { recursive: true });
  });

  it("should reject illegal encodings before I/O on all entry points", async () => {
    const missing = path.join(
      os.tmpdir(),
      `realpath-missing-encoding-${Date.now()}`
    );

    expect(() => realpathSync(missing, "nope")).toThrow(/encoding is not supported/i);
    expect(() => realpathSync(".", { encoding: "nope" })).toThrow(
      /encoding is not supported/i
    );

    await expect(realpathPromise(missing, "nope")).rejects.toThrow(
      /encoding is not supported/i
    );

    expect(() =>
      realpath(".", "nope", () => {
        throw new Error("callback must not run for illegal encoding");
      })
    ).toThrow(/encoding is not supported/i);
  });

  it("should invoke callback asynchronously with (error, result)", async () => {
    const tmpDir = mkdtempSync(path.join(os.tmpdir(), "realpath-"));
    const filePath = path.join(tmpDir, "file.txt");
    writeFileSync(filePath, "data");
    const expected = realpathSync(filePath);

    let syncFlag = false;
    const result = await new Promise<string>((resolve, reject) => {
      realpath(filePath, (err, resolved) => {
        if (err) {
          reject(err);
          return;
        }
        expect(syncFlag).toBeTruthy();
        resolve(resolved);
      });
      syncFlag = true;
    });
    expect(result).toBe(expected);

    const nativeResult = await new Promise<string>((resolve, reject) => {
      realpath.native(filePath, { encoding: "utf8" }, (err, resolved) => {
        if (err) {
          reject(err);
          return;
        }
        resolve(resolved);
      });
    });
    expect(nativeResult).toBe(expected);

    rmdirSync(tmpDir, { recursive: true });
  });

  it("should reject missing paths with code/path/syscall", async () => {
    const missing = path.join(
      os.tmpdir(),
      `realpath-missing-${Date.now()}-${Math.random()}`
    );

    try {
      realpathSync(missing);
      expect(false).toBeTruthy();
    } catch (err: any) {
      expect(err.code).toBe("ENOENT");
      expect(err.path).toBe(missing);
      expect(err.syscall).toBe("realpath");
    }

    await expect(realpathPromise(missing)).rejects.toMatchObject({
      code: "ENOENT",
      path: missing,
      syscall: "realpath",
    });

    const callbackErr = await new Promise<any>((resolve) => {
      realpath(missing, (err) => resolve(err));
    });
    expect(callbackErr.code).toBe("ENOENT");
    expect(callbackErr.path).toBe(missing);
    expect(callbackErr.syscall).toBe("realpath");
  });

  it("should work via fs.promises and fs/promises", async () => {
    const promisesMod = await import("fs/promises");
    const tmpDir = mkdtempSync(path.join(os.tmpdir(), "realpath-"));
    const filePath = path.join(tmpDir, "file.txt");
    writeFileSync(filePath, "data");
    const expected = realpathSync(filePath);

    expect(await realpathPromise(filePath)).toBe(expected);
    expect(await promisesMod.realpath(filePath)).toBe(expected);
    expect(await promisesMod.default.realpath(filePath)).toBe(expected);
    expect(promisesMod.realpath).toBe(promisesMod.default.realpath);

    rmdirSync(tmpDir, { recursive: true });
  });
});

describe("callback access/stat/lstat and existsSync", () => {
  it("named and default exports should be the same function references", () => {
    expect(defaultImport.access).toBe(accessCb);
    expect(defaultImport.stat).toBe(statCb);
    expect(defaultImport.lstat).toBe(lstatCb);
    expect(defaultImport.existsSync).toBe(existsSync);
  });

  it("promisify(fs.stat) should return Stats with isFile()", async () => {
    const { promisify } = await import("util");
    const tmpDir = mkdtempSync(path.join(os.tmpdir(), "stat-"));
    const filePath = path.join(tmpDir, "file.txt");
    writeFileSync(filePath, "data");

    const stats = await promisify(statCb)(filePath);
    expect(stats.isFile()).toBeTruthy();

    rmdirSync(tmpDir, { recursive: true });
  });

  it("promisify(fs.lstat) should report symbolic links", async () => {
    const { promisify } = await import("util");
    const tmpDir = mkdtempSync(path.join(os.tmpdir(), "lstat-"));
    const filePath = path.join(tmpDir, "file.txt");
    const linkPath = path.join(tmpDir, "link");
    writeFileSync(filePath, "data");
    symlinkSync(filePath, linkPath);

    const stats = await promisify(lstatCb)(linkPath);
    expect(stats.isSymbolicLink()).toBeTruthy();

    rmdirSync(tmpDir, { recursive: true });
  });

  it("promisify(fs.access) should resolve for existing and reject for missing", async () => {
    const { promisify } = await import("util");
    const tmpDir = mkdtempSync(path.join(os.tmpdir(), "access-"));
    const filePath = path.join(tmpDir, "file.txt");
    const missing = path.join(tmpDir, "missing.txt");
    writeFileSync(filePath, "data");

    await promisify(accessCb)(filePath);
    await expect(promisify(accessCb)(missing)).rejects.toMatchObject({
      code: "ENOENT",
      path: missing,
      syscall: "access",
    });

    rmdirSync(tmpDir, { recursive: true });
  });

  it("callback APIs should not run synchronously on the current stack", async () => {
    const tmpDir = mkdtempSync(path.join(os.tmpdir(), "cb-async-"));
    const filePath = path.join(tmpDir, "file.txt");
    writeFileSync(filePath, "data");

    let syncFlag = true;
    const done = new Promise<void>((resolve, reject) => {
      statCb(filePath, (err: Error | null) => {
        try {
          expect(syncFlag).toBeFalsy();
          expect(err).toBeNull();
          resolve();
        } catch (e) {
          reject(e);
        }
      });
    });
    syncFlag = false;
    await done;

    rmdirSync(tmpDir, { recursive: true });
  });

  it("callback errors should include code, path, and syscall", async () => {
    const missing = path.join(
      os.tmpdir(),
      `stat-missing-${Date.now()}-${Math.random()}`
    );

    const err = await new Promise<any>((resolve) => {
      statCb(missing, (e) => resolve(e));
    });
    expect(err.code).toBe("ENOENT");
    expect(err.path).toBe(missing);
    expect(err.syscall).toBe("stat");
  });

  it("existsSync should report files, directories, and missing paths", () => {
    const tmpDir = mkdtempSync(path.join(os.tmpdir(), "exists-"));
    const filePath = path.join(tmpDir, "file.txt");
    writeFileSync(filePath, "data");

    expect(existsSync(filePath)).toBeTruthy();
    expect(existsSync(tmpDir)).toBeTruthy();

    rmSync(filePath);
    expect(existsSync(filePath)).toBeFalsy();

    rmdirSync(tmpDir, { recursive: true });
  });

  it("should throw for bigint Stats options", () => {
    expect(() =>
      statCb("fixtures/hello.txt", { bigint: true }, () => {})
    ).toThrow(/BigIntStats is not supported/);
    expect(() =>
      lstatCb("fixtures/hello.txt", { bigint: true }, () => {})
    ).toThrow(/BigIntStats is not supported/);
    expect(() => statSync("fixtures/hello.txt", { bigint: true })).toThrow(
      /BigIntStats is not supported/
    );
  });

  it("access callback should reject invalid mode values", () => {
    expect(() => accessCb("fixtures/hello.txt", -1, () => {})).toThrow(
      /out of range|mode/
    );
    expect(() => accessCb("fixtures/hello.txt", Number.NaN, () => {})).toThrow(
      /out of range|mode/
    );
    expect(() => accessCb("fixtures/hello.txt", 1.5, () => {})).toThrow(
      /out of range|mode/
    );
  });
});

describe("createReadStream", () => {
  it("reads a small file fully", async () => {
    const tmpDir = mkdtempSync(path.join(os.tmpdir(), "raster-crs-"));
    const filePath = path.join(tmpDir, "small.txt");
    writeFileSync(filePath, "hello-createReadStream");
    try {
      const stream = createReadStream(filePath);
      expect(stream.path).toBe(filePath);
      const chunks: Buffer[] = [];
      for await (const chunk of stream) {
        chunks.push(Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk));
      }
      expect(Buffer.concat(chunks).toString()).toBe("hello-createReadStream");
    } finally {
      rmSync(tmpDir, { recursive: true, force: true });
    }
  });

  it("reads multiple chunks for large highWaterMark-limited files", async () => {
    const tmpDir = mkdtempSync(path.join(os.tmpdir(), "raster-crs-"));
    const filePath = path.join(tmpDir, "large.bin");
    const payload = Buffer.alloc(200, 0x61);
    writeFileSync(filePath, payload);
    try {
      const stream = createReadStream(filePath, { highWaterMark: 64 });
      const chunks: Buffer[] = [];
      for await (const chunk of stream) {
        chunks.push(Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk));
      }
      expect(chunks.length).toBeGreaterThan(1);
      expect(Buffer.concat(chunks).equals(payload)).toBe(true);
    } finally {
      rmSync(tmpDir, { recursive: true, force: true });
    }
  });

  it("supports start/end range and encoding", async () => {
    const tmpDir = mkdtempSync(path.join(os.tmpdir(), "raster-crs-"));
    const filePath = path.join(tmpDir, "range.txt");
    writeFileSync(filePath, "0123456789");
    try {
      const stream = createReadStream(filePath, {
        start: 2,
        end: 5,
        encoding: "utf8",
      });
      let body = "";
      for await (const chunk of stream) {
        body += chunk;
      }
      expect(body).toBe("2345");
    } finally {
      rmSync(tmpDir, { recursive: true, force: true });
    }
  });

  it("emits error for missing files", async () => {
    const stream = createReadStream(path.join(os.tmpdir(), "no-such-raster-crs-file"));
    await expect(
      (async () => {
        for await (const _ of stream) {
          // drain
        }
      })()
    ).rejects.toBeTruthy();
  });

  it("rejects unsupported fd option", () => {
    expect(() =>
      createReadStream("fixtures/hello.txt", { fd: 3 } as any)
    ).toThrow(/fd is not supported/);
  });

  it("rejects autoClose:false (no close()/fd surface)", () => {
    expect(() =>
      createReadStream("fixtures/hello.txt", { autoClose: false } as any)
    ).toThrow(/autoClose:false is not supported/);
  });

  it("accepts Buffer and file URL paths", async () => {
    const tmpDir = mkdtempSync(path.join(os.tmpdir(), "raster-crs-"));
    const filePath = path.join(tmpDir, "pathlike.txt");
    writeFileSync(filePath, "pathlike-ok");
    try {
      const fromBuffer = createReadStream(Buffer.from(filePath, "utf8"));
      let body = "";
      for await (const chunk of fromBuffer) {
        body += Buffer.isBuffer(chunk) ? chunk.toString() : String(chunk);
      }
      expect(body).toBe("pathlike-ok");

      const { pathToFileURL } = await import("node:url");
      const fromUrl = createReadStream(pathToFileURL(filePath));
      body = "";
      for await (const chunk of fromUrl) {
        body += Buffer.isBuffer(chunk) ? chunk.toString() : String(chunk);
      }
      expect(body).toBe("pathlike-ok");
    } finally {
      rmSync(tmpDir, { recursive: true, force: true });
    }
  });

  it("rejects invalid UTF-8 Buffer and non-Uint8 ArrayBufferViews", () => {
    expect(() => createReadStream(Buffer.from([0xff]))).toThrow(
      /invalid UTF-8|Uint8Array|Buffer or URL/i
    );
    expect(() => createReadStream(new Uint16Array([0x2f]) as any)).toThrow(
      /string or an instance of Buffer or URL/i
    );
    expect(() => createReadStream(new DataView(new ArrayBuffer(1)) as any)).toThrow(
      /string or an instance of Buffer or URL/i
    );
  });

  it("rejects detached Uint8Array and Buffer without panicking", () => {
    const ab = new ArrayBuffer(8);
    const u8 = new Uint8Array(ab);
    ab.transfer();
    expect(() => createReadStream(u8)).toThrow(TypeError);

    // Buffer sharing a transferable ArrayBuffer
    const ab2 = new ArrayBuffer(32);
    const buf = Buffer.from(ab2);
    buf.write("hello-path");
    ab2.transfer();
    expect(() => createReadStream(buf)).toThrow(TypeError);
  });
});

// Helper function to check if directory exists
const checkDirExists = async (dirPath: string) => {
  return await promises
    .stat(dirPath)
    .then(() => true)
    .catch(() => false);
};

// `defaultImport` is the fs namespace; aliases keep the new tests readable.
const fs = defaultImport as any;
const fsp = fs.promises as any;
const join = path.join;
const tmpdir = os.tmpdir;

describe("createWriteStream", () => {
  const tmp = (name: string) => join(tmpdir(), `raster-ws-${process.pid}-${name}`);

  it("writes string and Buffer chunks and closes the handle", async () => {
    const file = tmp("basic.txt");
    const ws = fs.createWriteStream(file);
    ws.write("hello ");
    ws.write(Buffer.from("buffer"));
    ws.end("-end");
    await new Promise<void>((resolve, reject) => {
      ws.on("finish", resolve);
      ws.on("error", reject);
    });
    expect(fs.readFileSync(file, "utf8")).toBe("hello buffer-end");
    // Node exposes bytesWritten on the stream.
    expect(ws.bytesWritten).toBe("hello buffer-end".length);
    fs.rmSync(file, { force: true });
  });

  it("appends when flags is 'a'", async () => {
    const file = tmp("append.txt");
    fs.writeFileSync(file, "first");
    await new Promise<void>((resolve, reject) => {
      const ws = fs.createWriteStream(file, { flags: "a" });
      ws.on("finish", resolve);
      ws.on("error", reject);
      ws.end("+second");
    });
    expect(fs.readFileSync(file, "utf8")).toBe("first+second");
    fs.rmSync(file, { force: true });
  });

  it("round-trips through createReadStream", async () => {
    const file = tmp("roundtrip.txt");
    await new Promise<void>((resolve, reject) => {
      const ws = fs.createWriteStream(file);
      ws.on("finish", resolve);
      ws.on("error", reject);
      ws.end("round trip payload");
    });

    const chunks: Buffer[] = [];
    await new Promise<void>((resolve, reject) => {
      const rs = fs.createReadStream(file);
      rs.on("data", (c: Buffer) => chunks.push(c));
      rs.on("end", resolve);
      rs.on("error", reject);
    });
    expect(Buffer.concat(chunks).toString()).toBe("round trip payload");
    fs.rmSync(file, { force: true });
  });

  it("rejects unsupported options instead of silently ignoring them", () => {
    expect(() =>
      fs.createWriteStream(tmp("x.txt"), { fd: 3 } as any)
    ).toThrow(/fd is not supported/);
    expect(() =>
      fs.createWriteStream(tmp("x.txt"), { autoClose: false } as any)
    ).toThrow(/autoClose/);
  });
});

describe("fs error metadata", () => {
  it("attaches code/syscall/path like Node", async () => {
    await expect(
      (readFile as any)("/definitely-not-here/x")
    ).rejects.toMatchObject({
      code: "ENOENT",
      syscall: "open",
      path: "/definitely-not-here/x",
    });

    expect(() => fs.readFileSync("/definitely-not-here/y")).toThrow();
    try {
      fs.readFileSync("/definitely-not-here/y");
    } catch (err: any) {
      expect(err.code).toBe("ENOENT");
      expect(err.syscall).toBe("open");
      expect(err.path).toBe("/definitely-not-here/y");
    }
  });

  it("mkdtemp/mkdir failures carry a code", async () => {
    await expect(
      (mkdir as any)("/definitely-not-here/deep/dir")
    ).rejects.toMatchObject({ code: "ENOENT", syscall: "mkdir" });
  });
});

describe("fs class exports", () => {
  it("Stats is exported and matches stat results", () => {
    const st = fs.statSync(tmpdir());
    expect(typeof fs.Stats).toBe("function");
    expect(st).toBeInstanceOf(fs.Stats);
  });

  it("Dirent is exported and matches readdir results", () => {
    const entry = fs.readdirSync(tmpdir(), { withFileTypes: true })[0];
    expect(typeof fs.Dirent).toBe("function");
    expect(entry).toBeInstanceOf(fs.Dirent);
  });
});

describe("appendFile / appendFileSync", () => {
  const tmpFile = (name = "append.txt") => {
    const dir = mkdtempSync(path.join(os.tmpdir(), "test-append-"));
    return { dir, file: path.join(dir, name) };
  };

  it("creates the file when missing and appends when present", () => {
    const { dir, file } = tmpFile();
    appendFileSync(file, "a");
    expect(readFileSync(file, "utf8")).toBe("a");
    appendFileSync(file, "b");
    expect(readFileSync(file, "utf8")).toBe("ab");
    rmSync(dir, { recursive: true, force: true });
  });

  it("does not truncate an existing file", () => {
    const { dir, file } = tmpFile();
    writeFileSync(file, "keep");
    appendFileSync(file, "-more");
    expect(readFileSync(file, "utf8")).toBe("keep-more");
    rmSync(dir, { recursive: true, force: true });
  });

  it("keeps chunks in order across many appends", () => {
    const { dir, file } = tmpFile();
    for (let i = 0; i < 50; i++) appendFileSync(file, String(i % 10));
    expect(readFileSync(file, "utf8")).toBe(
      Array.from({ length: 50 }, (_, i) => String(i % 10)).join("")
    );
    rmSync(dir, { recursive: true, force: true });
  });

  it("mixes strings and buffers without tearing multibyte text", () => {
    const { dir, file } = tmpFile();
    appendFileSync(file, "héllo");
    appendFileSync(file, Buffer.from("→"));
    appendFileSync(file, "世界");
    expect(readFileSync(file, "utf8")).toBe("héllo→世界");
    rmSync(dir, { recursive: true, force: true });
  });

  it("accepts string, Buffer, TypedArray and DataView", () => {
    const { dir, file } = tmpFile();
    appendFileSync(file, "s");
    appendFileSync(file, Buffer.from("B"));
    appendFileSync(file, new Uint8Array([0x55]));
    appendFileSync(file, new DataView(new Uint8Array([0x44]).buffer));
    expect(readFileSync(file, "utf8")).toBe("sBUD");
    rmSync(dir, { recursive: true, force: true });
  });

  it("rejects other data types with ERR_INVALID_ARG_TYPE", () => {
    const { dir, file } = tmpFile();
    for (const bad of [42, {}, null, undefined]) {
      let err: any;
      try {
        appendFileSync(file, bad as any);
      } catch (e) {
        err = e;
      }
      expect(`${typeof bad}:${err && err.code}`).toBe(
        `${typeof bad}:ERR_INVALID_ARG_TYPE`
      );
      expect(err).toBeInstanceOf(TypeError);
    }
    rmSync(dir, { recursive: true, force: true });
  });

  it("honours the encoding option", () => {
    const { dir, file } = tmpFile();
    appendFileSync(file, "é", { encoding: "latin1" });
    expect(Array.from(readFileSync(file))).toEqual([0xe9]);
    rmSync(dir, { recursive: true, force: true });
  });

  it("honours the mode option on creation", () => {
    if (IS_WINDOWS) return;
    const { dir, file } = tmpFile();
    appendFileSync(file, "m", { mode: 0o600 });
    expect(statSync(file).mode & 0o777).toBe(0o600);
    rmSync(dir, { recursive: true, force: true });
  });

  it("throws EEXIST for flag 'ax' on an existing file", () => {
    const { dir, file } = tmpFile();
    writeFileSync(file, "x");
    let err: any;
    try {
      appendFileSync(file, "y", { flag: "ax" });
    } catch (e) {
      err = e;
    }
    expect(err && err.code).toBe("EEXIST");
    // and works when the file does not exist
    const fresh = path.join(dir, "fresh.txt");
    appendFileSync(fresh, "y", { flag: "ax" });
    expect(readFileSync(fresh, "utf8")).toBe("y");
    rmSync(dir, { recursive: true, force: true });
  });

  it("accepts flag 'a+' and still appends", () => {
    const { dir, file } = tmpFile();
    appendFileSync(file, "z", { flag: "a+" });
    appendFileSync(file, "w", { flag: "a+" });
    expect(readFileSync(file, "utf8")).toBe("zw");
    rmSync(dir, { recursive: true, force: true });
  });

  it("reports ERR_INVALID_ARG_VALUE for an unknown flag", () => {
    const { dir, file } = tmpFile();
    let err: any;
    try {
      appendFileSync(file, "q", { flag: "notaflag" });
    } catch (e) {
      err = e;
    }
    expect(err && err.code).toBe("ERR_INVALID_ARG_VALUE");
    rmSync(dir, { recursive: true, force: true });
  });

  it("maps path failures to the same codes as Node", () => {
    let missing: any;
    try {
      appendFileSync("/definitely-not-here-xyz/f.txt", "a");
    } catch (e) {
      missing = e;
    }
    expect(missing && missing.code).toBe("ENOENT");

    const dir = mkdtempSync(path.join(os.tmpdir(), "test-append-"));
    let isDir: any;
    try {
      appendFileSync(dir, "a");
    } catch (e) {
      isDir = e;
    }
    expect(isDir && isDir.code).toBe("EISDIR");
    rmSync(dir, { recursive: true, force: true });
  });

  it("appends through the callback form with an error-first callback", async () => {
    const { dir, file } = tmpFile();
    await new Promise<void>((resolve, reject) => {
      appendFileCb(file, "cb1", (err: any) => {
        if (err) reject(err);
        else resolve();
      });
    });
    expect(readFileSync(file, "utf8")).toBe("cb1");

    await new Promise<void>((resolve) => {
      appendFileCb("/definitely-not-here-xyz/f.txt", "x", (err: any) => {
        expect(err && err.code).toBe("ENOENT");
        resolve();
      });
    });
    rmSync(dir, { recursive: true, force: true });
  });

  it("appends through the promise form", async () => {
    const { dir, file } = tmpFile();
    await appendFilePromise(file, "p1");
    await appendFilePromise(file, "p2");
    expect(readFileSync(file, "utf8")).toBe("p1p2");
    await expect(
      appendFilePromise("/definitely-not-here-xyz/f.txt", "x")
    ).rejects.toMatchObject({ code: "ENOENT" });
    rmSync(dir, { recursive: true, force: true });
  });

  it("agrees between the sync, callback and promise forms", async () => {
    const { dir, file } = tmpFile();
    appendFileSync(file, "s");
    await appendFilePromise(file, "p");
    await new Promise<void>((resolve, reject) => {
      appendFileCb(file, "c", (err: any) => (err ? reject(err) : resolve()));
    });
    expect(readFileSync(file, "utf8")).toBe("spc");
    rmSync(dir, { recursive: true, force: true });
  });
});
