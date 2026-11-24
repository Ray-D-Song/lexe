import buffer, { Blob as ModuleBlob, File as ModuleFile, isAscii, isUtf8, resolveObjectURL, transcode } from "node:buffer";

const SOURCES: Record<string, number[]> = {
  ascii: [0x68, 0x65, 0x6c, 0x6c, 0x6f],
  latin: [0x63, 0x61, 0x66, 0xe9],
  hi: [0x80, 0xff, 0x41],
  u8multi: [...Buffer.from("\u20ac\u00fc\u{1f600}\u00e9", "utf8")],
  u8bad: [0x61, 0xff, 0x62, 0xc0, 0x80, 0x63],
  u8trunc: [0x61, 0xe2, 0x82],
  u8surr: [0xed, 0xa0, 0x80, 0x41],
  u16: [...Buffer.from("a\u20ac\u{1f600}", "utf16le")],
  u16odd: [0x61, 0x00, 0x62],
  u16lone: [0x00, 0xd8, 0x41, 0x00],
  u16loneLow: [0x00, 0xdc, 0x41, 0x00],
  bom: [0xef, 0xbb, 0xbf, 0x61],
  empty: [],
  u8arr: [0x68, 0x69],
};

// [source, from, to, expected hex | "!" + error code], recorded from Node v24.21.0.
const MATRIX: Array<[string, string, string, string]> = [
  ["ascii", "utf8", "utf8", "68656c6c6f"],
  ["ascii", "utf8", "ucs2", "680065006c006c006f00"],
  ["ascii", "utf8", "latin1", "68656c6c6f"],
  ["ascii", "utf8", "ascii", "68656c6c6f"],
  ["ascii", "ucs2", "utf8", "e695a8e6b1ac"],
  ["ascii", "ucs2", "ucs2", "68656c6cfdff"],
  ["ascii", "ucs2", "latin1", "3f3f"],
  ["ascii", "ucs2", "ascii", "3f3f"],
  ["ascii", "latin1", "utf8", "68656c6c6f"],
  ["ascii", "latin1", "ucs2", "680065006c006c006f00"],
  ["ascii", "latin1", "latin1", "68656c6c6f"],
  ["ascii", "latin1", "ascii", "68656c6c6f"],
  ["ascii", "ascii", "utf8", "68656c6c6f"],
  ["ascii", "ascii", "ucs2", "680065006c006c006f00"],
  ["ascii", "ascii", "latin1", "68656c6c6f"],
  ["ascii", "ascii", "ascii", "68656c6c6f"],
  ["latin", "utf8", "utf8", "636166efbfbd"],
  ["latin", "utf8", "ucs2", "!U_INVALID_CHAR_FOUND"],
  ["latin", "utf8", "latin1", "6361663f"],
  ["latin", "utf8", "ascii", "6361663f"],
  ["latin", "ucs2", "utf8", "e685a3eea5a6"],
  ["latin", "ucs2", "ucs2", "636166e9"],
  ["latin", "ucs2", "latin1", "3f3f"],
  ["latin", "ucs2", "ascii", "3f3f"],
  ["latin", "latin1", "utf8", "636166c3a9"],
  ["latin", "latin1", "ucs2", "630061006600e900"],
  ["latin", "latin1", "latin1", "636166e9"],
  ["latin", "latin1", "ascii", "6361663f"],
  ["latin", "ascii", "utf8", "636166efbfbd"],
  ["latin", "ascii", "ucs2", "630061006600e900"],
  ["latin", "ascii", "latin1", "6361663f"],
  ["latin", "ascii", "ascii", "6361663f"],
  ["hi", "utf8", "utf8", "efbfbdefbfbd41"],
  ["hi", "utf8", "ucs2", "!U_INVALID_CHAR_FOUND"],
  ["hi", "utf8", "latin1", "3f3f41"],
  ["hi", "utf8", "ascii", "3f3f41"],
  ["hi", "ucs2", "utf8", "efbe80"],
  ["hi", "ucs2", "ucs2", "80fffdff"],
  ["hi", "ucs2", "latin1", "3f"],
  ["hi", "ucs2", "ascii", "3f"],
  ["hi", "latin1", "utf8", "c280c3bf41"],
  ["hi", "latin1", "ucs2", "8000ff004100"],
  ["hi", "latin1", "latin1", "80ff41"],
  ["hi", "latin1", "ascii", "3f3f41"],
  ["hi", "ascii", "utf8", "efbfbdefbfbd41"],
  ["hi", "ascii", "ucs2", "8000ff004100"],
  ["hi", "ascii", "latin1", "3f3f41"],
  ["hi", "ascii", "ascii", "3f3f41"],
  ["u8multi", "utf8", "utf8", "e282acc3bcf09f9880c3a9"],
  ["u8multi", "utf8", "ucs2", "ac20fc003dd800dee900"],
  ["u8multi", "utf8", "latin1", "3ffc3fe9"],
  ["u8multi", "utf8", "ascii", "3f3f3f3f"],
  ["u8multi", "ucs2", "utf8", "e88ba2ec8eacef82bce9a29fec8e80"],
  ["u8multi", "ucs2", "ucs2", "e282acc3bcf09f9880c3fdff"],
  ["u8multi", "ucs2", "latin1", "3f3f3f3f3f"],
  ["u8multi", "ucs2", "ascii", "3f3f3f3f3f"],
  ["u8multi", "latin1", "utf8", "c3a2c282c2acc383c2bcc3b0c29fc298c280c383c2a9"],
  ["u8multi", "latin1", "ucs2", "e2008200ac00c300bc00f0009f0098008000c300a900"],
  ["u8multi", "latin1", "latin1", "e282acc3bcf09f9880c3a9"],
  ["u8multi", "latin1", "ascii", "3f3f3f3f3f3f3f3f3f3f3f"],
  ["u8multi", "ascii", "utf8", "efbfbdefbfbdefbfbdefbfbdefbfbdefbfbdefbfbdefbfbdefbfbdefbfbdefbfbd"],
  ["u8multi", "ascii", "ucs2", "e2008200ac00c300bc00f0009f0098008000c300a900"],
  ["u8multi", "ascii", "latin1", "3f3f3f3f3f3f3f3f3f3f3f"],
  ["u8multi", "ascii", "ascii", "3f3f3f3f3f3f3f3f3f3f3f"],
  ["u8bad", "utf8", "utf8", "61efbfbd62efbfbdefbfbd63"],
  ["u8bad", "utf8", "ucs2", "!U_INVALID_CHAR_FOUND"],
  ["u8bad", "utf8", "latin1", "613f623f3f63"],
  ["u8bad", "utf8", "ascii", "613f623f3f63"],
  ["u8bad", "ucs2", "utf8", "efbda1ec81a2e68e80"],
  ["u8bad", "ucs2", "ucs2", "61ff62c08063"],
  ["u8bad", "ucs2", "latin1", "3f3f3f"],
  ["u8bad", "ucs2", "ascii", "3f3f3f"],
  ["u8bad", "latin1", "utf8", "61c3bf62c380c28063"],
  ["u8bad", "latin1", "ucs2", "6100ff006200c00080006300"],
  ["u8bad", "latin1", "latin1", "61ff62c08063"],
  ["u8bad", "latin1", "ascii", "613f623f3f63"],
  ["u8bad", "ascii", "utf8", "61efbfbd62efbfbdefbfbd63"],
  ["u8bad", "ascii", "ucs2", "6100ff006200c00080006300"],
  ["u8bad", "ascii", "latin1", "613f623f3f63"],
  ["u8bad", "ascii", "ascii", "613f623f3f63"],
  ["u8trunc", "utf8", "utf8", "61efbfbd"],
  ["u8trunc", "utf8", "ucs2", "!U_INVALID_CHAR_FOUND"],
  ["u8trunc", "utf8", "latin1", "613f"],
  ["u8trunc", "utf8", "ascii", "613f"],
  ["u8trunc", "ucs2", "utf8", "ee89a1"],
  ["u8trunc", "ucs2", "ucs2", "61e2fdff"],
  ["u8trunc", "ucs2", "latin1", "3f"],
  ["u8trunc", "ucs2", "ascii", "3f"],
  ["u8trunc", "latin1", "utf8", "61c3a2c282"],
  ["u8trunc", "latin1", "ucs2", "6100e2008200"],
  ["u8trunc", "latin1", "latin1", "61e282"],
  ["u8trunc", "latin1", "ascii", "613f3f"],
  ["u8trunc", "ascii", "utf8", "61efbfbdefbfbd"],
  ["u8trunc", "ascii", "ucs2", "6100e2008200"],
  ["u8trunc", "ascii", "latin1", "613f3f"],
  ["u8trunc", "ascii", "ascii", "613f3f"],
  ["u8surr", "utf8", "utf8", "efbfbdefbfbdefbfbd41"],
  ["u8surr", "utf8", "ucs2", "!U_INVALID_CHAR_FOUND"],
  ["u8surr", "utf8", "latin1", "3f3f3f41"],
  ["u8surr", "utf8", "ascii", "3f3f3f41"],
  ["u8surr", "ucs2", "utf8", "ea83ade48680"],
  ["u8surr", "ucs2", "ucs2", "eda08041"],
  ["u8surr", "ucs2", "latin1", "3f3f"],
  ["u8surr", "ucs2", "ascii", "3f3f"],
  ["u8surr", "latin1", "utf8", "c3adc2a0c28041"],
  ["u8surr", "latin1", "ucs2", "ed00a00080004100"],
  ["u8surr", "latin1", "latin1", "eda08041"],
  ["u8surr", "latin1", "ascii", "3f3f3f41"],
  ["u8surr", "ascii", "utf8", "efbfbdefbfbdefbfbd41"],
  ["u8surr", "ascii", "ucs2", "ed00a00080004100"],
  ["u8surr", "ascii", "latin1", "3f3f3f41"],
  ["u8surr", "ascii", "ascii", "3f3f3f41"],
  ["u16", "utf8", "utf8", "6100efbfbd203defbfbd00efbfbd"],
  ["u16", "utf8", "ucs2", "!U_INVALID_CHAR_FOUND"],
  ["u16", "utf8", "latin1", "61003f203d3f003f"],
  ["u16", "utf8", "ascii", "61003f203d3f003f"],
  ["u16", "ucs2", "utf8", "61e282acf09f9880"],
  ["u16", "ucs2", "ucs2", "6100ac203dd800de"],
  ["u16", "ucs2", "latin1", "613f3f"],
  ["u16", "ucs2", "ascii", "613f3f"],
  ["u16", "latin1", "utf8", "6100c2ac203dc39800c39e"],
  ["u16", "latin1", "ucs2", "61000000ac0020003d00d8000000de00"],
  ["u16", "latin1", "latin1", "6100ac203dd800de"],
  ["u16", "latin1", "ascii", "61003f203d3f003f"],
  ["u16", "ascii", "utf8", "6100efbfbd203defbfbd00efbfbd"],
  ["u16", "ascii", "ucs2", "61000000ac0020003d00d8000000de00"],
  ["u16", "ascii", "latin1", "61003f203d3f003f"],
  ["u16", "ascii", "ascii", "61003f203d3f003f"],
  ["u16odd", "utf8", "utf8", "610062"],
  ["u16odd", "utf8", "ucs2", "610000006200"],
  ["u16odd", "utf8", "latin1", "610062"],
  ["u16odd", "utf8", "ascii", "610062"],
  ["u16odd", "ucs2", "utf8", "61"],
  ["u16odd", "ucs2", "ucs2", "6100fdff"],
  ["u16odd", "ucs2", "latin1", "61"],
  ["u16odd", "ucs2", "ascii", "61"],
  ["u16odd", "latin1", "utf8", "610062"],
  ["u16odd", "latin1", "ucs2", "610000006200"],
  ["u16odd", "latin1", "latin1", "610062"],
  ["u16odd", "latin1", "ascii", "610062"],
  ["u16odd", "ascii", "utf8", "610062"],
  ["u16odd", "ascii", "ucs2", "610000006200"],
  ["u16odd", "ascii", "latin1", "610062"],
  ["u16odd", "ascii", "ascii", "610062"],
  ["u16lone", "utf8", "utf8", "00efbfbd4100"],
  ["u16lone", "utf8", "ucs2", "!U_INVALID_CHAR_FOUND"],
  ["u16lone", "utf8", "latin1", "003f4100"],
  ["u16lone", "utf8", "ascii", "003f4100"],
  ["u16lone", "ucs2", "utf8", "!U_INVALID_CHAR_FOUND"],
  ["u16lone", "ucs2", "ucs2", "fdff4100"],
  ["u16lone", "ucs2", "latin1", "3f41"],
  ["u16lone", "ucs2", "ascii", "3f41"],
  ["u16lone", "latin1", "utf8", "00c3984100"],
  ["u16lone", "latin1", "ucs2", "0000d80041000000"],
  ["u16lone", "latin1", "latin1", "00d84100"],
  ["u16lone", "latin1", "ascii", "003f4100"],
  ["u16lone", "ascii", "utf8", "00efbfbd4100"],
  ["u16lone", "ascii", "ucs2", "0000d80041000000"],
  ["u16lone", "ascii", "latin1", "003f4100"],
  ["u16lone", "ascii", "ascii", "003f4100"],
  ["u16loneLow", "utf8", "utf8", "00efbfbd4100"],
  ["u16loneLow", "utf8", "ucs2", "!U_INVALID_CHAR_FOUND"],
  ["u16loneLow", "utf8", "latin1", "003f4100"],
  ["u16loneLow", "utf8", "ascii", "003f4100"],
  ["u16loneLow", "ucs2", "utf8", "!U_INVALID_CHAR_FOUND"],
  ["u16loneLow", "ucs2", "ucs2", "fdff4100"],
  ["u16loneLow", "ucs2", "latin1", "3f41"],
  ["u16loneLow", "ucs2", "ascii", "3f41"],
  ["u16loneLow", "latin1", "utf8", "00c39c4100"],
  ["u16loneLow", "latin1", "ucs2", "0000dc0041000000"],
  ["u16loneLow", "latin1", "latin1", "00dc4100"],
  ["u16loneLow", "latin1", "ascii", "003f4100"],
  ["u16loneLow", "ascii", "utf8", "00efbfbd4100"],
  ["u16loneLow", "ascii", "ucs2", "0000dc0041000000"],
  ["u16loneLow", "ascii", "latin1", "003f4100"],
  ["u16loneLow", "ascii", "ascii", "003f4100"],
  ["bom", "utf8", "utf8", "efbbbf61"],
  ["bom", "utf8", "ucs2", "fffe6100"],
  ["bom", "utf8", "latin1", "61"],
  ["bom", "utf8", "ascii", "61"],
  ["bom", "ucs2", "utf8", "ebafafe686bf"],
  ["bom", "ucs2", "ucs2", "efbbbf61"],
  ["bom", "ucs2", "latin1", "3f3f"],
  ["bom", "ucs2", "ascii", "3f3f"],
  ["bom", "latin1", "utf8", "c3afc2bbc2bf61"],
  ["bom", "latin1", "ucs2", "ef00bb00bf006100"],
  ["bom", "latin1", "latin1", "efbbbf61"],
  ["bom", "latin1", "ascii", "3f3f3f61"],
  ["bom", "ascii", "utf8", "efbfbdefbfbdefbfbd61"],
  ["bom", "ascii", "ucs2", "ef00bb00bf006100"],
  ["bom", "ascii", "latin1", "3f3f3f61"],
  ["bom", "ascii", "ascii", "3f3f3f61"],
  ["empty", "utf8", "utf8", ""],
  ["empty", "utf8", "ucs2", ""],
  ["empty", "utf8", "latin1", ""],
  ["empty", "utf8", "ascii", ""],
  ["empty", "ucs2", "utf8", ""],
  ["empty", "ucs2", "ucs2", ""],
  ["empty", "ucs2", "latin1", ""],
  ["empty", "ucs2", "ascii", ""],
  ["empty", "latin1", "utf8", ""],
  ["empty", "latin1", "ucs2", ""],
  ["empty", "latin1", "latin1", ""],
  ["empty", "latin1", "ascii", ""],
  ["empty", "ascii", "utf8", ""],
  ["empty", "ascii", "ucs2", ""],
  ["empty", "ascii", "latin1", ""],
  ["empty", "ascii", "ascii", ""],
  ["u8arr", "utf8", "utf8", "6869"],
  ["u8arr", "utf8", "ucs2", "68006900"],
  ["u8arr", "utf8", "latin1", "6869"],
  ["u8arr", "utf8", "ascii", "6869"],
  ["u8arr", "ucs2", "utf8", "e6a5a8"],
  ["u8arr", "ucs2", "ucs2", "6869"],
  ["u8arr", "ucs2", "latin1", "3f"],
  ["u8arr", "ucs2", "ascii", "3f"],
  ["u8arr", "latin1", "utf8", "6869"],
  ["u8arr", "latin1", "ucs2", "68006900"],
  ["u8arr", "latin1", "latin1", "6869"],
  ["u8arr", "latin1", "ascii", "6869"],
  ["u8arr", "ascii", "utf8", "6869"],
  ["u8arr", "ascii", "ucs2", "68006900"],
  ["u8arr", "ascii", "latin1", "6869"],
  ["u8arr", "ascii", "ascii", "6869"],
];

function each<T>(rows: T[]) {
  return (title: string, fn: (...args: any[]) => void) => {
    for (const row of rows) {
      const args: any[] = Array.isArray(row) ? row : [row];
      let next = 0;
      it(
        title.replace(/%s/g, () => String(args[next++])),
        () => fn(...args)
      );
    }
  };
}

const hex = (value: Uint8Array) => Buffer.from(value).toString("hex");

const capture = (fn: () => unknown): { code?: string; message?: string; value?: unknown } => {
  try {
    return { value: fn() };
  } catch (error: any) {
    return { code: error.code, message: error.message };
  }
};

describe("buffer.transcode", () => {
  each(MATRIX)("%s %s -> %s", (source, from, to, expected) => {
    const result = capture(() => transcode(Buffer.from(SOURCES[source]), from as any, to as any));
    if (expected.startsWith("!")) {
      expect(result.code).toBe(expected.slice(1));
      expect(result.message).toBe(`Unable to transcode Buffer [${expected.slice(1)}]`);
    } else {
      expect(Buffer.isBuffer(result.value)).toBe(true);
      expect(hex(result.value as Uint8Array)).toBe(expected);
    }
  });

  it("reports the ICU error number on the error", () => {
    let error: any;
    try {
      transcode(Buffer.from([0xff]), "utf8", "ucs2");
    } catch (e) {
      error = e;
    }
    expect(error.errno).toBe(10);
    expect(error.code).toBe("U_INVALID_CHAR_FOUND");
  });

  it("normalizes encoding names", () => {
    const source = Buffer.from("hi");
    expect(hex(transcode(source, "UTF8", "UCS-2" as any))).toBe("68006900");
    expect(hex(transcode(source, "binary", "utf-16le"))).toBe("68006900");
    expect(hex(transcode(source, undefined as any, undefined as any))).toBe("6869");
    expect(hex(transcode(source, "" as any, "utf8"))).toBe("6869");
  });

  each(["hex", "base64", "utf16be", "usc2", "unknown", 5])("rejects the unsupported encoding %s", (name) => {
    for (const args of [[name, "utf8"], ["utf8", name]]) {
      const result = capture(() => transcode(Buffer.from("hi"), args[0] as any, args[1] as any));
      expect(result.code).toBe("U_ILLEGAL_ARGUMENT_ERROR");
    }
  });

  it("returns an empty buffer for an empty source without validating encodings", () => {
    const result = transcode(Buffer.alloc(0), "foo" as any, "bar" as any);
    expect(Buffer.isBuffer(result)).toBe(true);
    expect(result.length).toBe(0);
  });

  it("accepts a plain Uint8Array but rejects other sources", () => {
    expect(hex(transcode(new Uint8Array([0x68, 0x69]), "utf8", "ucs2"))).toBe("68006900");
    for (const bad of ["x", 1, undefined, null, [1], new ArrayBuffer(2), new Uint16Array(1)]) {
      const result = capture(() => transcode(bad as any, "utf8", "ucs2"));
      expect(result.code).toBe("ERR_INVALID_ARG_TYPE");
      expect(result.message).toContain('The "source" argument must be an instance of Buffer or Uint8Array. Received');
    }
    expect(capture(() => (transcode as any)()).code).toBe("ERR_INVALID_ARG_TYPE");
  });

  it("has Node's arity", () => {
    expect(transcode.length).toBe(3);
  });
});

describe("buffer.isUtf8 / buffer.isAscii", () => {
  // [name, bytes, isUtf8, isAscii], recorded from Node v24.21.0.
  const CASES: Array<[string, number[], boolean, boolean]> = [
    ["empty", [], true, true],
    ["ascii", [0x41, 0x7f], true, true],
    ["lone 0x80", [0x80], false, false],
    ["2 byte", [0xc3, 0xa9], true, false],
    ["3 byte", [0xe2, 0x82, 0xac], true, false],
    ["4 byte", [0xf0, 0x9f, 0x98, 0x80], true, false],
    ["overlong 2 (c0 80)", [0xc0, 0x80], false, false],
    ["overlong 2 (c1 bf)", [0xc1, 0xbf], false, false],
    ["overlong 3", [0xe0, 0x80, 0x80], false, false],
    ["overlong 4", [0xf0, 0x80, 0x80, 0x80], false, false],
    ["surrogate d800", [0xed, 0xa0, 0x80], false, false],
    ["surrogate dfff", [0xed, 0xbf, 0xbf], false, false],
    ["U+10FFFF", [0xf4, 0x8f, 0xbf, 0xbf], true, false],
    ["above U+10FFFF", [0xf4, 0x90, 0x80, 0x80], false, false],
    ["0xf5 lead", [0xf5, 0x80, 0x80, 0x80], false, false],
    ["0xff", [0xff], false, false],
    ["0xfe", [0xfe], false, false],
    ["truncated 2", [0xc3], false, false],
    ["truncated 3", [0xe2, 0x82], false, false],
    ["truncated 4", [0xf0, 0x9f, 0x98], false, false],
    ["bad continuation", [0xe2, 0x41, 0x82], false, false],
    ["BOM", [0xef, 0xbb, 0xbf], true, false],
    ["NUL", [0x00, 0x41], true, true],
    ["valid then invalid", [0x61, 0xc3, 0xa9, 0x62, 0xff], false, false],
    ["U+FFFE", [0xef, 0xbf, 0xbe], true, false],
  ];

  each(CASES)("%s", (_name, bytes, utf8, ascii) => {
    const view = Buffer.from(bytes);
    expect(isUtf8(view)).toBe(utf8);
    expect(isAscii(view)).toBe(ascii);
  });

  it("accepts ArrayBuffer, SharedArrayBuffer and every TypedArray", () => {
    const bytes = new Uint8Array([0xc3, 0xa9, 0x41]);
    expect([isUtf8(bytes.buffer), isAscii(bytes.buffer)]).toEqual([true, false]);
    expect([isUtf8(new SharedArrayBuffer(2)), isAscii(new SharedArrayBuffer(2))]).toEqual([true, true]);
    expect(isAscii(new Uint16Array([0x4141]))).toBe(true);
    expect(isAscii(new Uint16Array([0x4181]))).toBe(false);
    expect(isAscii(new Uint32Array([0x41414141]))).toBe(true);
    expect(isUtf8(new Float64Array([1]))).toBe(false);
  });

  it("looks only at the bytes a view covers", () => {
    const backing = new Uint8Array([0x41, 0xff, 0x41]);
    const view = backing.subarray(0, 1);
    expect(isUtf8(view)).toBe(true);
    expect(isAscii(view)).toBe(true);
    expect(isAscii(backing.subarray(1, 2))).toBe(false);
  });

  each([
    ["DataView", new DataView(new ArrayBuffer(2))],
    ["string", "a"],
    ["undefined", undefined],
    ["null", null],
    ["array", [0x41]],
    ["object", {}],
    ["number", 1],
  ])("rejects %s", (_name, value) => {
    for (const fn of [isUtf8, isAscii]) {
      const result = capture(() => fn(value as any));
      expect(result.code).toBe("ERR_INVALID_ARG_TYPE");
      expect(result.message).toContain(
        'The "input" argument must be an instance of ArrayBuffer, Buffer, or TypedArray. Received'
      );
    }
  });

  it("has Node's arity", () => {
    expect(isUtf8.length).toBe(1);
    expect(isAscii.length).toBe(1);
  });
});

describe("buffer constants and legacy exports", () => {
  it("kMaxLength and kStringMaxLength agree with buffer.constants", () => {
    expect(typeof buffer.kMaxLength).toBe("number");
    expect(buffer.kMaxLength).toBeGreaterThan(0);
    expect(buffer.kMaxLength).toBe(buffer.constants.MAX_LENGTH);
    expect(buffer.kStringMaxLength).toBe(buffer.constants.MAX_STRING_LENGTH);
  });

  it("INSPECT_MAX_BYTES is a validated accessor defaulting to 50", () => {
    const descriptor = Object.getOwnPropertyDescriptor(buffer, "INSPECT_MAX_BYTES")!;
    expect(typeof descriptor.get).toBe("function");
    expect(typeof descriptor.set).toBe("function");
    expect(descriptor.enumerable).toBe(true);
    expect(buffer.INSPECT_MAX_BYTES).toBe(50);
    try {
      buffer.INSPECT_MAX_BYTES = 7;
      expect(buffer.INSPECT_MAX_BYTES).toBe(7);
      expect(capture(() => (buffer.INSPECT_MAX_BYTES = -1)).code).toBe("ERR_OUT_OF_RANGE");
      expect(capture(() => (buffer.INSPECT_MAX_BYTES = NaN)).code).toBe("ERR_OUT_OF_RANGE");
      expect(capture(() => ((buffer as any).INSPECT_MAX_BYTES = "x")).code).toBe("ERR_INVALID_ARG_TYPE");
      expect(buffer.INSPECT_MAX_BYTES).toBe(7);
    } finally {
      buffer.INSPECT_MAX_BYTES = 50;
    }
  });

  it("SlowBuffer returns a Buffer and validates its size", () => {
    const warn = process.emitWarning;
    process.emitWarning = (() => {}) as any;
    try {
      const slow = (buffer as any).SlowBuffer(3);
      expect(Buffer.isBuffer(slow)).toBe(true);
      expect(slow.length).toBe(3);
      expect(Buffer.isBuffer(new (buffer as any).SlowBuffer(2))).toBe(true);
      expect(capture(() => (buffer as any).SlowBuffer(-1)).code).toBe("ERR_OUT_OF_RANGE");
      expect(capture(() => (buffer as any).SlowBuffer(NaN)).code).toBe("ERR_OUT_OF_RANGE");
      expect(capture(() => (buffer as any).SlowBuffer("3")).code).toBe("ERR_INVALID_ARG_TYPE");
      expect(capture(() => (buffer as any).SlowBuffer()).code).toBe("ERR_INVALID_ARG_TYPE");
    } finally {
      process.emitWarning = warn;
    }
  });

  it("re-exports the global Blob and File", () => {
    expect(ModuleBlob).toBe(globalThis.Blob);
    expect(ModuleFile).toBe(globalThis.File);
    expect(buffer.Blob).toBe(globalThis.Blob);
    expect(buffer.File).toBe(globalThis.File);
  });
});

describe("URL.createObjectURL / buffer.resolveObjectURL", () => {
  it("round-trips a Blob into an equal but distinct Blob", async () => {
    const blob = new Blob(["hello"], { type: "Text/Plain" });
    const url = URL.createObjectURL(blob);
    try {
      expect(url).toMatch(/^blob:nodedata:[0-9a-f-]{36}$/);
      const resolved = resolveObjectURL(url)!;
      expect(resolved).toBeInstanceOf(Blob);
      expect(resolved).not.toBe(blob);
      expect(resolved.size).toBe(5);
      expect(resolved.type).toBe("text/plain");
      expect(await resolved.text()).toBe("hello");
      expect(resolveObjectURL(url)).not.toBe(resolved);
      expect(resolveObjectURL(`${url}#fragment`)).toBeDefined();
    } finally {
      URL.revokeObjectURL(url);
    }
  });

  it("resolves a File to a plain Blob", () => {
    const file = new File(["abc"], "a.txt", { type: "text/plain" });
    const url = URL.createObjectURL(file);
    try {
      const resolved: any = resolveObjectURL(url);
      expect(resolved.size).toBe(3);
      expect(resolved.type).toBe("text/plain");
      expect(resolved.name).toBeUndefined();
    } finally {
      URL.revokeObjectURL(url);
    }
  });

  it("hands out a distinct URL for every call", () => {
    const blob = new Blob(["x"]);
    const a = URL.createObjectURL(blob);
    const b = URL.createObjectURL(blob);
    expect(a).not.toBe(b);
    URL.revokeObjectURL(a);
    expect(resolveObjectURL(a)).toBeUndefined();
    expect(resolveObjectURL(b)).toBeDefined();
    URL.revokeObjectURL(b);
  });

  it("revokeObjectURL forgets the URL and is idempotent", () => {
    const url = URL.createObjectURL(new Blob(["x"]));
    URL.revokeObjectURL(url);
    expect(resolveObjectURL(url)).toBeUndefined();
    expect(() => URL.revokeObjectURL(url)).not.toThrow();
    expect(() => URL.revokeObjectURL("zzz")).not.toThrow();
    expect(() => URL.revokeObjectURL(undefined as any)).not.toThrow();
    expect(capture(() => (URL.revokeObjectURL as any)()).code).toBe("ERR_MISSING_ARGS");
  });

  it("revokeObjectURL ignores the scheme and base but not extra colons", () => {
    const blob = new Blob(["x"]);
    const first = URL.createObjectURL(blob);
    const id = first.split(":")[2];
    URL.revokeObjectURL(`${first}:extra`);
    expect(resolveObjectURL(first)).toBeDefined();
    URL.revokeObjectURL(`blob:whatever:${id}`);
    expect(resolveObjectURL(first)).toBeUndefined();
  });

  each(["nodedata:x", "blob:nodedata:nope", "blob:nodedata:a:b", "blob:", "", "not a url", undefined, null, 5])(
    "resolveObjectURL(%s) is undefined",
    (value) => {
      expect(resolveObjectURL(value as any)).toBeUndefined();
    }
  );

  it("resolveObjectURL only honours the nodedata base", () => {
    const url = URL.createObjectURL(new Blob(["x"]));
    try {
      expect(resolveObjectURL(`blob:other:${url.split(":")[2]}`)).toBeUndefined();
    } finally {
      URL.revokeObjectURL(url);
    }
  });

  it("stringifies its argument", () => {
    const url = URL.createObjectURL(new Blob(["x"]));
    try {
      expect(resolveObjectURL({ toString: () => url } as any)).toBeDefined();
    } finally {
      URL.revokeObjectURL(url);
    }
  });

  each([undefined, null, "str", 1, {}, new Uint8Array(1), new ArrayBuffer(1)])(
    "createObjectURL rejects %s",
    (value) => {
      const result = capture(() => URL.createObjectURL(value as any));
      expect(result.code).toBe("ERR_INVALID_ARG_TYPE");
      expect(result.message).toContain('The "obj" argument must be an instance of Blob. Received');
    }
  );

  it("installs enumerable, writable, configurable static methods", () => {
    for (const name of ["createObjectURL", "revokeObjectURL"]) {
      const descriptor = Object.getOwnPropertyDescriptor(URL, name)!;
      expect([descriptor.enumerable, descriptor.writable, descriptor.configurable]).toEqual([true, true, true]);
      expect((URL as any)[name].length).toBe(1);
    }
  });
});
