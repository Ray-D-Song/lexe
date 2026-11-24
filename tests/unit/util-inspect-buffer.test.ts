import util from "node:util";

// `util.inspect(Buffer)` uses Node's dedicated `<Buffer ...>` form: lowercase
// two-digit hex, a hard 50-byte cap, and a trailing space when empty. Buffer
// does *not* go through the generic typed-array path, and it ignores
// `maxArrayLength` / `depth` because Node's implementation bakes those in.
//
// Expectations taken from Node 24.21.

const inspect = (value: any, options?: any) =>
  options === undefined ? util.inspect(value) : util.inspect(value, options);

describe("util.inspect(Buffer) hex form", () => {
  it("prints an empty buffer with Node's trailing space", () => {
    // Node emits "<Buffer >" — the space is part of the output, not a typo.
    expect(inspect(Buffer.alloc(0))).toBe("<Buffer >");
  });

  it("prints single bytes as two-digit lowercase hex", () => {
    expect(inspect(Buffer.from([0]))).toBe("<Buffer 00>");
    expect(inspect(Buffer.from([0xff]))).toBe("<Buffer ff>");
    expect(inspect(Buffer.from([0x0a]))).toBe("<Buffer 0a>");
    expect(inspect(Buffer.from([0, 1, 255]))).toBe("<Buffer 00 01 ff>");
  });

  it("prints exactly 50 bytes without truncating", () => {
    const out = inspect(Buffer.alloc(50, 0xab));
    expect(out.startsWith("<Buffer ab ab")).toBe(true);
    expect(out.endsWith(">")).toBe(true);
    expect(out).not.toContain("more byte");
    expect(out.split(" ").length).toBe(50 + 1);
  });
});

describe("util.inspect(Buffer) truncation", () => {
  it("reports one remaining byte in the singular", () => {
    const out = inspect(Buffer.alloc(51, 0xab));
    expect(out.endsWith(" ... 1 more byte>")).toBe(true);
    // Plural would be wrong here.
    expect(out).not.toContain("1 more bytes");
  });

  it("reports multiple remaining bytes in the plural", () => {
    expect(inspect(Buffer.alloc(52, 0xab)).endsWith(" ... 2 more bytes>")).toBe(true);
    expect(inspect(Buffer.alloc(100, 0xab)).endsWith(" ... 50 more bytes>")).toBe(true);
    expect(inspect(Buffer.alloc(101, 0xab)).endsWith(" ... 51 more bytes>")).toBe(true);
  });

  it("shows exactly 50 bytes before the ellipsis", () => {
    const out = inspect(Buffer.alloc(100, 0xab));
    const shown = out.slice("<Buffer ".length, out.indexOf(" ..."));
    expect(shown.split(" ").length).toBe(50);
  });
});

describe("util.inspect(Buffer) ignores array options", () => {
  it("does not honour maxArrayLength", () => {
    // Node's Buffer inspect has the 50-byte cap baked in; the option is inert.
    const fifty = Buffer.alloc(50, 0xab);
    expect(inspect(fifty, { maxArrayLength: 10 })).toBe(inspect(fifty));
    expect(inspect(fifty, { maxArrayLength: 5 })).toBe(inspect(fifty));
    expect(inspect(fifty, { maxArrayLength: 0 })).toBe(inspect(fifty));
    expect(inspect(fifty, { maxArrayLength: null })).toBe(inspect(fifty));
  });

  it("still caps at 50 when maxArrayLength is raised", () => {
    const hundred = Buffer.alloc(100, 0xab);
    expect(inspect(hundred, { maxArrayLength: Infinity })).toBe(inspect(hundred));
  });
});

describe("util.inspect(Buffer) nesting", () => {
  it("uses the buffer form inside objects and arrays", () => {
    expect(inspect({ buf: Buffer.from([0, 1, 255]) })).toBe("{ buf: <Buffer 00 01 ff> }");
    expect(inspect([Buffer.from([0, 1, 255])])).toBe("[ <Buffer 00 01 ff> ]");
  });

  it("still expands a nested buffer at depth: 0", () => {
    // The generic depth limit must not swallow this: Node prints the buffer's
    // full form even with no depth allowance left.
    expect(inspect({ buf: Buffer.from([0, 1, 255]) }, { depth: 0 })).toBe(
      "{ buf: <Buffer 00 01 ff> }"
    );
  });

  it("expands a nested buffer one level down", () => {
    expect(inspect({ a: { b: Buffer.from([1, 2]) } }, { depth: 1 })).toBe(
      "{ a: { b: <Buffer 01 02> } }"
    );
  });
});

describe("util.inspect(Buffer) versus typed arrays", () => {
  it("keeps Uint8Array on its own decimal form", () => {
    // Deliberately different from Buffer: decimal, a length suffix, brackets.
    const out = inspect(new Uint8Array([0, 1, 255]));
    expect(out).toContain("Uint8Array");
    expect(out).toContain("[");
    expect(out).not.toContain("<Buffer");
  });

  it("does not apply Buffer truncation to a plain Uint8Array", () => {
    const out = inspect(new Uint8Array(60));
    expect(out).not.toContain("more bytes");
  });
});
