import { fileURLToPath, pathToFileURL } from "node:url";

// Parity for `fileURLToPath` / `pathToFileURL`. Expectations were taken from
// Node 24.21 — including the one error that deliberately has no `code`.

type Thrown = { code: string | undefined; name: string; message: string };

const capture = (fn: () => unknown): Thrown | null => {
  try {
    fn();
    return null;
  } catch (e: any) {
    return { code: e.code, name: e.name, message: String(e.message) };
  }
};

describe("fileURLToPath basics", () => {
  it("decodes a plain file URL and drops query/hash", () => {
    expect(fileURLToPath("file:///tmp/a/b.txt")).toBe("/tmp/a/b.txt");
    expect(fileURLToPath("file:///tmp/a/b.txt?x=1")).toBe("/tmp/a/b.txt");
    expect(fileURLToPath("file:///tmp/a/b.txt#h")).toBe("/tmp/a/b.txt");
  });

  it("percent-decodes spaces and non-ASCII", () => {
    expect(fileURLToPath("file:///tmp/a/b%20c.txt")).toBe("/tmp/a/b c.txt");
    expect(fileURLToPath("file:///tmp/%E4%B8%AD%E6%96%87.txt")).toBe("/tmp/中文.txt");
  });

  it("accepts a URL object", () => {
    expect(fileURLToPath(new URL("file:///tmp/a"))).toBe("/tmp/a");
  });

  it("accepts the localhost host", () => {
    expect(fileURLToPath("file://localhost/a")).toBe("/a");
  });
});

describe("fileURLToPath errors carry Node's codes", () => {
  it("rejects an encoded / with ERR_INVALID_FILE_URL_PATH", () => {
    const e = capture(() => fileURLToPath("file:///tmp/a%2Fb.txt"));
    expect(e?.code).toBe("ERR_INVALID_FILE_URL_PATH");
    expect(e?.message).toContain("encoded / characters");
  });

  it("rejects a foreign host with ERR_INVALID_FILE_URL_HOST", () => {
    const e = capture(() => fileURLToPath("file://host/a"));
    expect(e?.code).toBe("ERR_INVALID_FILE_URL_HOST");
    expect(e?.message).toContain('must be "localhost" or empty');
  });

  it("rejects a non-file scheme with ERR_INVALID_URL_SCHEME", () => {
    const e = capture(() => fileURLToPath("http://x/y"));
    expect(e?.code).toBe("ERR_INVALID_URL_SCHEME");
  });

  it("rejects unparseable input with ERR_INVALID_URL", () => {
    expect(capture(() => fileURLToPath("a/b"))?.code).toBe("ERR_INVALID_URL");
    expect(capture(() => fileURLToPath("not a url"))?.code).toBe("ERR_INVALID_URL");
  });

  it("raises a bare URIError, with NO code, for a malformed escape", () => {
    // Node's only error here without a `code`. Parity beats internal
    // consistency: `assert` has one, so this one keeps none.
    for (const url of ["file:///tmp/%zz", "file:///tmp/%2"]) {
      const e = capture(() => fileURLToPath(url));
      expect(e).not.toBeNull();
      expect(e!.name).toBe("URIError");
      expect(e!.message).toBe("URI malformed");
      expect(e!.code).toBeUndefined();
    }
  });
});

describe("fileURLToPath options.windows", () => {
  it("converts a drive path when windows: true", () => {
    expect(fileURLToPath("file:///C:/x", { windows: true })).toBe("C:\\x");
  });

  it("leaves a drive path alone when windows: false", () => {
    expect(fileURLToPath("file:///C:/x", { windows: false })).toBe("/C:/x");
    // No options on POSIX behaves like windows: false.
    expect(fileURLToPath("file:///C:/x")).toBe("/C:/x");
  });

  it("rejects a non-absolute path when windows: true", () => {
    const e = capture(() => fileURLToPath("file:///tmp/a", { windows: true }));
    expect(e?.code).toBe("ERR_INVALID_FILE_URL_PATH");
    expect(e?.message).toContain("must be absolute");
  });
});

describe("pathToFileURL", () => {
  it("encodes characters that would otherwise change the URL", () => {
    expect(pathToFileURL("/tmp/a b.txt").href).toBe("file:///tmp/a%20b.txt");
    expect(pathToFileURL("/tmp/a%b.txt").href).toBe("file:///tmp/a%25b.txt");
    expect(pathToFileURL("/tmp/a#b.txt").href).toBe("file:///tmp/a%23b.txt");
    expect(pathToFileURL("/tmp/a?b.txt").href).toBe("file:///tmp/a%3Fb.txt");
  });

  it("encodes non-ASCII as UTF-8 percent escapes", () => {
    expect(pathToFileURL("/tmp/中文.txt").href).toBe("file:///tmp/%E4%B8%AD%E6%96%87.txt");
  });

  it("resolves a relative path against the cwd instead of failing", () => {
    // Node applies path.resolve() first, so this never throws.
    const url = pathToFileURL("a/b");
    expect(url.href.startsWith("file:///")).toBe(true);
    expect(url.href.endsWith("/a/b")).toBe(true);
    expect(fileURLToPath(url).endsWith("/a/b")).toBe(true);
  });

  it("preserves a trailing separator", () => {
    expect(pathToFileURL("/tmp/a/").href).toBe("file:///tmp/a/");
    expect(pathToFileURL("/tmp/a").href).toBe("file:///tmp/a");
  });
});

describe("round trip path -> URL -> path", () => {
  it("returns the original path for awkward characters", () => {
    // The case most likely to be missed: a literal `%` must survive both legs.
    const awkward = [
      "/tmp/plain.txt",
      "/tmp/a b.txt",
      "/tmp/a%b.txt",
      "/tmp/a#b.txt",
      "/tmp/a?b.txt",
      "/tmp/中文.txt",
    ];
    for (const path of awkward) {
      expect(fileURLToPath(pathToFileURL(path))).toBe(path);
    }
  });

  it("keeps a trailing separator across the round trip", () => {
    expect(fileURLToPath(pathToFileURL("/tmp/a/"))).toBe("/tmp/a/");
  });
});
