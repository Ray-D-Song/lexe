import defaultImport from "node:buffer";
import legacyImport from "buffer";

it("node:buffer should be the same as buffer", () => {
  expect(defaultImport).toStrictEqual(legacyImport);
});

const { Buffer } = defaultImport;

describe("Buffer.alloc", () => {
  it("should create a buffer with specified size and fill with zeros (default fill)", () => {
    const size = 10;
    const buffer = Buffer.alloc(size);

    expect(buffer.length).toEqual(size);

    for (const byte of buffer) {
      expect(byte).toEqual(0);
    }
  });

  it("should create a buffer with specified size and fill with a string value", () => {
    const size = 8;
    const fillString = "abc";
    const buffer = Buffer.alloc(size, fillString);

    expect(buffer.toString()).toEqual("abcabcab");
  });

  it("should create a buffer with specified size and fill with an encoded string value", () => {
    const size = 8;
    const fillString = "616263";
    const buffer = Buffer.alloc(size, fillString, "hex");

    expect(buffer.toString()).toEqual("abcabcab");
  });

  it("should create a buffer with specified size and fill with a Buffer value", () => {
    const size = 6;
    const fillBuffer = Buffer.from([1, 2, 3]);
    const buffer = Buffer.alloc(size, fillBuffer);

    expect(buffer).toStrictEqual(Buffer.from([1, 2, 3, 1, 2, 3]));
  });

  it("should create a buffer with specified size and fill with a Uint8Array value", () => {
    const size = 5;
    const fillUint8Array = new Uint8Array([5, 10, 15]);
    const buffer = Buffer.alloc(size, fillUint8Array);

    expect(buffer).toStrictEqual(Buffer.from([5, 10, 15, 5, 10]));
  });

  it("should create a buffer with specified size and fill with an integer value", () => {
    const size = 4;
    const fillInteger = 42;
    const buffer = Buffer.alloc(size, fillInteger);

    for (const byte of buffer) {
      expect(byte).toEqual(fillInteger);
    }
  });

  it("should throw an error when fill argument is invalid", () => {
    const size = 10;
    let buffer = Buffer.alloc(size, true as any);
    for (const byte of buffer) {
      expect(byte).toEqual(0);
    }
  });
});

describe("Buffer.allocUnsafe", () => {
  it("should create a buffer of the specified size", () => {
    const size = 10;
    const buffer = Buffer.allocUnsafe(size);

    expect(buffer.length).toEqual(size);
    for (const byte of buffer) {
      expect(byte).toBeDefined();
    }
  });

  it("should create an empty buffer when size is 0", () => {
    const size = 0;
    const buffer = Buffer.allocUnsafe(size);

    expect(buffer.length).toEqual(size);
  });

  it("should throw a TypeError when size is negative", () => {
    expect(() => {
      const size = -1;
      const buffer = Buffer.allocUnsafe(size);
    }).toThrow(TypeError);
  });
});

describe("Buffer.allocUnsafeSlow", () => {
  it("should create a buffer of the specified size", () => {
    const size = 10;
    const buffer = Buffer.allocUnsafeSlow(size);

    expect(buffer.length).toEqual(size);
    for (const byte of buffer) {
      expect(byte).toBeDefined();
    }
  });

  it("should create an empty buffer when size is 0", () => {
    const size = 0;
    const buffer = Buffer.allocUnsafeSlow(size);

    expect(buffer.length).toEqual(size);
  });

  it("should throw a TypeError when size is negative", () => {
    expect(() => {
      const size = -1;
      const buffer = Buffer.allocUnsafeSlow(size);
    }).toThrow(TypeError);
  });
});

describe("Buffer.byteLength", () => {
  it("should return the correct byte length for ASCII string", () => {
    const length = Buffer.byteLength("Hello");

    expect(length).toEqual(5);
  });

  it("should return the correct byte length for UTF-8 string", () => {
    const length = Buffer.byteLength("👋");

    expect(length).toEqual(4);
  });

  it("should return the correct byte length for UTF-8 string", () => {
    const length = Buffer.byteLength("你好");

    expect(length).toEqual(6);
  });

  it("should return the correct byte length for a buffer", () => {
    const buffer = Buffer.from([1, 2, 3, 4, 5]);
    const length = Buffer.byteLength(buffer);

    expect(length).toEqual(5);
  });

  it("should return the correct byte length for a hex-encoded string", () => {
    const length = Buffer.byteLength("deadbeef", "hex");

    expect(length).toEqual(4);
  });

  it("should return the correct byte length for a base64-encoded string", () => {
    const length = Buffer.byteLength("SGVsbG8gV29ybGQ=", "base64");

    expect(length).toEqual(11);
  });
});

describe("Buffer.concat", () => {
  it("should concatenate buffers", () => {
    const buffer1 = Buffer.from("Hello");
    const buffer2 = Buffer.from(" ");
    const buffer3 = Buffer.from("World");
    const resultBuffer = Buffer.concat([buffer1, buffer2, buffer3]);

    expect(resultBuffer.toString()).toEqual("Hello World");
  });

  it("should handle empty buffers in the array", () => {
    const buffer1 = Buffer.from("Hello");
    const buffer2 = Buffer.from("");
    const buffer3 = Buffer.from("World");
    const resultBuffer = Buffer.concat([buffer1, buffer2, buffer3]);

    expect(resultBuffer.toString()).toEqual("HelloWorld");
  });

  it("should handle an array with a single buffer", () => {
    const buffer = Buffer.from("SingleBuffer");
    const resultBuffer = Buffer.concat([buffer]);

    expect(resultBuffer.toString()).toEqual("SingleBuffer");
  });

  it("should handle an empty array of buffers", () => {
    const resultBuffer = Buffer.concat([]);

    expect(resultBuffer.toString()).toEqual("");
  });

  it("should throw an error when the list contains a non-buffer", () => {
    expect(() => {
      const buffer1 = Buffer.from("Hello");
      const invalidBuffer = "InvalidBuffer";
      Buffer.concat([buffer1, invalidBuffer as any]);
    }).toThrow(TypeError);
  });

  it("should throw an error when the totalLength is too large", () => {
    expect(() => {
      const buffer1 = Buffer.from("Hello");
      const buffer2 = Buffer.alloc(2 ** 32); // 1 GB buffer
      Buffer.concat([buffer1, buffer2], 2 ** 33); // totalLength exceeding maximum allowed
    }).toThrow(RangeError);
  });

  it("should concatenate buffers with specified totalLength", () => {
    const buffer1 = Buffer.from("123");
    const buffer2 = Buffer.from("4567");
    const buffer3 = Buffer.from("89");
    const resultBuffer = Buffer.concat([buffer1, buffer2, buffer3], 4);

    expect(resultBuffer.toString()).toEqual("1234");

    const resultBuffer2 = Buffer.concat([buffer1, buffer2, buffer3], 3);

    expect(resultBuffer2.toString()).toEqual("123");
  });

  it("should throw an error when totalLength is less than the actual length of concatenated buffers", () => {
    const buffer1 = Buffer.from("Hello");
    const buffer2 = Buffer.from("World");
    const resultBuffer = Buffer.concat([buffer1, buffer2], 999);

    expect(resultBuffer.toString()).toEqual("HelloWorld");
    expect(resultBuffer.length).toEqual(buffer1.length + buffer2.length);
  });
});

describe("Buffer.from", () => {
  it("should create a buffer from a string with utf-8 encoding", () => {
    const input = "Hello, world!";
    const buffer = Buffer.from(input, "utf-8");

    expect(buffer.toString()).toEqual(input);
  });

  it("should create a buffer from an array of bytes", () => {
    const byteArray = [65, 66, 67, 68, 69]; // ASCII values of A, B, C, D, E
    const buffer = Buffer.from(byteArray);

    for (let i = 0; i < byteArray.length; i++) {
      expect(buffer[i]).toEqual(byteArray[i]);
    }
  });

  it("should create a buffer from a string with base64 encoding", () => {
    const input = "SGVsbG8sIHdvcmxkIQ==";
    const buffer = Buffer.from(input, "base64");
    expect(buffer.toString()).toEqual("Hello, world!");

    const input2 = "SGVsbG8sIHdvcmxkIQ";
    const buffer2 = Buffer.from(input2, "base64");
    expect(buffer2.toString()).toEqual("Hello, world!");
  });

  it("should create a buffer from a string with base64 encoding that contains / or +", () => {
    const input = "PD8+MTIz";
    const buffer = Buffer.from(input, "base64");
    expect(buffer.toString()).toEqual("<?>123");

    const input3 = "PD8/PjEyMw==";
    const buffer3 = Buffer.from(input3, "base64");
    expect(buffer3.toString()).toEqual("<??>123");
  });

  // https://en.wikipedia.org/wiki/Base64#URL_applications
  it("should create a buffer from a string with URL safe base64 encoding that contains _ or -", () => {
    const input = "PD8-MTIz";
    const buffer = Buffer.from(input, "base64");
    expect(buffer.toString()).toEqual("<?>123");

    const input3 = "PD8_PjEyMw";
    const buffer3 = Buffer.from(input3, "base64");
    expect(buffer3.toString()).toEqual("<??>123");
  });

  it("should create a buffer from a string with hex encoding", () => {
    const input = "48656c6c6f2c20776f726c6421";
    const buffer = Buffer.from(input, "hex");

    expect(buffer.toString()).toEqual("Hello, world!");
  });

  it("should create a buffer from a single ASCII character in utf16le encoding", () => {
    // ASCII character 'A' (U+0041) = [0x41, 0x00] in utf16le
    const input = "A";
    const buffer = Buffer.from(input, "utf16le");
    expect(buffer.length).toEqual(2);
    expect(buffer[0]).toEqual(0x41);
    expect(buffer[1]).toEqual(0x00);
    expect(buffer.toString("utf16le")).toEqual(input);
  });

  it("should create a buffer from multiple ASCII characters in utf16le encoding", () => {
    const input2 = "Hello";
    const buffer2 = Buffer.from(input2, "utf16le");
    expect(buffer2.length).toEqual(10); // 5 characters * 2 bytes
    expect(buffer2.toString("utf16le")).toEqual(input2);
  });

  it("should create a buffer from a Unicode BMP character in utf16le encoding", () => {
    // Unicode BMP character '中' (U+4E2D) = [0x2D, 0x4E] in utf16le
    const input3 = "中";
    const buffer3 = Buffer.from(input3, "utf16le");
    expect(buffer3.length).toEqual(2);
    expect(buffer3[0]).toEqual(0x2d);
    expect(buffer3[1]).toEqual(0x4e);
    expect(buffer3.toString("utf16le")).toEqual(input3);
  });

  it("should create a buffer from an emoji (astral plane character) in utf16le encoding", () => {
    // Emoji '😀' (U+1F600) = surrogate pair [0xD83D, 0xDE00] = [0x3D, 0xD8, 0x00, 0xDE] in utf16le
    const input4 = "😀";
    const buffer4 = Buffer.from(input4, "utf16le");
    expect(buffer4.length).toEqual(4);
    expect(buffer4[0]).toEqual(0x3d);
    expect(buffer4[1]).toEqual(0xd8);
    expect(buffer4[2]).toEqual(0x00);
    expect(buffer4[3]).toEqual(0xde);
    expect(buffer4.toString("utf16le")).toEqual(input4);
  });

  it("should keep a lone surrogate when creating a buffer in utf16le encoding", () => {
    const input4 = "🌍🌎".slice(1);
    expect(Buffer.from(input4, "utf16le").toString("hex")).toEqual("0ddf3cd80edf");
  });

  it("should create a buffer from a portion of an array with offset and length", () => {
    const byteArray = [65, 66, 67, 68, 69]; // ASCII values of A, B, C, D, E
    const offset = 1;
    const length = 3;

    // @ts-ignore
    const buffer = Buffer.from(byteArray, offset, length);

    expect(buffer.length).toEqual(length);
    for (let i = 0; i < length; i++) {
      expect(buffer[i]).toEqual(byteArray[offset + i]);
    }
  });

  it("should handle offset and length overflows", () => {
    const byteArray = [65, 66, 67, 68, 69]; // ASCII values of A, B, C, D, E
    let length = 99;
    let offset = 0;
    // @ts-ignore
    let buffer = Buffer.from(byteArray, offset, length);
    expect(buffer.length).toEqual(byteArray.length);
    for (let i = 0; i < length; i++) {
      expect(buffer[i]).toEqual(byteArray[offset + i]);
    }

    // @ts-ignore
    buffer = Buffer.from(byteArray, 99, 2);
    expect(buffer.length).toEqual(0);

    // @ts-ignore
    buffer = Buffer.from(byteArray, 99, 999);
    expect(buffer.length).toEqual(0);
  });

  it("should use same memory for sub arrays", () => {
    const typedArray = new Uint8Array([65, 66, 67, 68, 69]);

    const a = Buffer.from(typedArray.buffer);
    const b = Buffer.from(typedArray.subarray(1, 4));
    const c = Buffer.from(a);

    expect(a.buffer).toStrictEqual(b.buffer);
    expect(a.toString()).toEqual("ABCDE");
    expect(b.toString()).toEqual("BCD");
    expect(c.toString()).toEqual("ABCDE");

    typedArray.set([70, 71], 1);

    expect(a.toString()).toEqual("AFGDE");
    expect(b.toString()).toEqual("FGD");
    expect(c.toString()).toEqual("ABCDE");
  });
});

describe("Buffer.isBuffer", () => {
  it("should return true when the object being tested is an instance of Buffer", () => {
    const buffer = Buffer.from("Hello, world!");

    expect(Buffer.isBuffer(buffer)).toEqual(true);
  });

  it("should return false when the object being tested is not an instance of Buffer", () => {
    expect(Buffer.isBuffer(false)).toEqual(false);
    expect(Buffer.isBuffer(undefined)).toEqual(false);
    expect(Buffer.isBuffer(null)).toEqual(false);
    expect(Buffer.isBuffer("Buffer")).toEqual(false);
    expect(Buffer.isBuffer(Buffer)).toEqual(false);
  });
});

describe("Buffer.isEncoding", () => {
  it("should return true when input is a valid encoding name", () => {
    expect(Buffer.isEncoding("utf8")).toEqual(true);
    expect(Buffer.isEncoding("hex")).toEqual(true);
    expect(Buffer.isEncoding("base64")).toEqual(true);
  });

  it("should return false when input is not a valid encoding name", () => {
    expect(Buffer.isEncoding(false as unknown as string)).toEqual(false);
    expect(Buffer.isEncoding(undefined as unknown as string)).toEqual(false);
    expect(Buffer.isEncoding(null as unknown as string)).toEqual(false);
    expect(Buffer.isEncoding("utf8/8")).toEqual(false);
  });
});

// Test prototype methods
describe("copy", () => {
  it("should copy the entire source buffer to the destination buffer", () => {
    const bufSrc = Buffer.from("abcdefghijklmnopqrstuvwxyz");
    const bufDest = Buffer.from("**************************");
    expect(bufSrc.copy(bufDest)).toEqual(26);
    expect(bufDest.toString()).toEqual("abcdefghijklmnopqrstuvwxyz");
  });

  it("should copy the entire source buffer starting from a specified offset in the destination buffer", () => {
    const bufSrc = Buffer.from("abcdefghijklmnopqrstuvwxyz");
    const bufDest = Buffer.from("**************************");
    expect(bufSrc.copy(bufDest, 5)).toEqual(21);
    expect(bufDest.toString()).toEqual("*****abcdefghijklmnopqrstu");
  });

  it("should copy a portion of the source buffer starting from a specified source offset to the destination buffer at a specified offset", () => {
    const bufSrc = Buffer.from("abcdefghijklmnopqrstuvwxyz");
    const bufDest = Buffer.from("**************************");
    expect(bufSrc.copy(bufDest, 5, 10)).toEqual(16);
    expect(bufDest.toString()).toEqual("*****klmnopqrstuvwxyz*****");
  });

  it("should copy a specific range of the source buffer to the destination buffer at a specified offset", () => {
    const bufSrc = Buffer.from("abcdefghijklmnopqrstuvwxyz");
    const bufDest = Buffer.from("**************************");
    expect(bufSrc.copy(bufDest, 5, 10, 15)).toEqual(5);
    expect(bufDest.toString()).toEqual("*****klmno****************");
  });

  it("should return 0 and not modify the destination buffer when the source start index is greater than the source end index", () => {
    const bufSrc = Buffer.from("abcdefghijklmnopqrstuvwxyz");
    const bufDest = Buffer.from("**************************");
    expect(bufSrc.copy(bufDest, 5, 10, 9)).toEqual(0);
    expect(bufDest.toString()).toEqual("**************************");
  });

  it("should clamp when source is longer than remaining target space (mysql packet shape)", () => {
    const src = Buffer.from([1, 2, 3, 4, 5, 6, 7, 8]);
    const dest = Buffer.alloc(4, 0xff);
    expect(src.copy(dest, 0, 0, 8)).toEqual(4);
    expect([...dest]).toEqual([1, 2, 3, 4]);
  });

  it("should return 0 when targetStart == target.length", () => {
    const src = Buffer.from([1, 2, 3]);
    const dest = Buffer.from([9, 9, 9]);
    expect(src.copy(dest, dest.length)).toEqual(0);
    expect([...dest]).toEqual([9, 9, 9]);
  });

  it("should return 0 when targetStart > target.length", () => {
    const src = Buffer.from([1, 2, 3]);
    const dest = Buffer.from([9, 9, 9]);
    expect(src.copy(dest, dest.length + 1)).toEqual(0);
    expect([...dest]).toEqual([9, 9, 9]);
  });

  it("should return 0 when sourceStart == source.length", () => {
    const src = Buffer.from([1, 2, 3]);
    const dest = Buffer.from([9, 9, 9]);
    expect(src.copy(dest, 0, src.length)).toEqual(0);
  });

  it("should throw when sourceStart > source.length", () => {
    const src = Buffer.from([1, 2, 3]);
    const dest = Buffer.from([9, 9, 9]);
    expect(() => src.copy(dest, 0, src.length + 1)).toThrow(RangeError);
  });

  it("should clamp sourceEnd greater than source.length", () => {
    const src = Buffer.from([1, 2, 3]);
    const dest = Buffer.alloc(3, 0);
    expect(src.copy(dest, 0, 0, 100)).toEqual(3);
    expect([...dest]).toEqual([1, 2, 3]);
  });

  it("should throw RangeError for negative indices", () => {
    const src = Buffer.from([1, 2, 3]);
    const dest = Buffer.alloc(3);
    expect(() => src.copy(dest, -1)).toThrow(RangeError);
    expect(() => src.copy(dest, 0, -1)).toThrow(RangeError);
    expect(() => src.copy(dest, 0, 0, -1)).toThrow(RangeError);
  });

  it("should floor fractional indices (Node Math.floor)", () => {
    const src = Buffer.from([10, 20, 30, 40]);
    const dest = Buffer.alloc(4, 0);
    // floor(1.9)=1, floor(0.5)=0, floor(2.7)=2
    expect(src.copy(dest, 1.9 as unknown as number, 0.5 as unknown as number, 2.7 as unknown as number)).toEqual(2);
    expect([...dest]).toEqual([0, 10, 20, 0]);
  });

  it("should copy with non-zero byteOffset subarrays", () => {
    const backing = Buffer.from([0, 1, 2, 3, 4, 5, 6, 7]);
    const src = backing.subarray(2, 6); // [2,3,4,5]
    const destBacking = Buffer.from([9, 9, 9, 9, 9, 9]);
    const dest = destBacking.subarray(1, 5); // 4 bytes
    expect(src.copy(dest, 1, 1, 3)).toEqual(2);
    expect([...dest]).toEqual([9, 3, 4, 9]);
    expect([...destBacking]).toEqual([9, 9, 3, 4, 9, 9]);
  });

  it("should handle forward and backward overlapping copies on the same Buffer", () => {
    const buf = Buffer.from([1, 2, 3, 4, 5, 6]);
    expect(buf.copy(buf, 2, 0, 3)).toEqual(3); // forward overlap
    expect([...buf]).toEqual([1, 2, 1, 2, 3, 6]);

    const buf2 = Buffer.from([1, 2, 3, 4, 5, 6]);
    expect(buf2.copy(buf2, 0, 2, 5)).toEqual(3); // backward overlap
    expect([...buf2]).toEqual([3, 4, 5, 4, 5, 6]);
  });

  it("should handle overlapping subarrays that share a backing ArrayBuffer", () => {
    const backing = Buffer.from([1, 2, 3, 4, 5, 6]);
    const a = backing.subarray(0, 4);
    const b = backing.subarray(2, 6);
    expect(a.copy(b, 0, 0, 3)).toEqual(3);
    expect([...backing]).toEqual([1, 2, 1, 2, 3, 6]);
  });

  it("should leave bytes outside the target view unchanged", () => {
    const dest = Buffer.from([1, 2, 3, 4, 5, 6]);
    const view = dest.subarray(2, 5);
    const src = Buffer.from([7, 8, 9]);
    src.copy(view);
    expect([...dest]).toEqual([1, 2, 7, 8, 9, 6]);
  });

  it("should coerce non-number indices via ToNumber (Node style)", () => {
    const src = Buffer.from([1, 2, 3, 4]);
    const dest = Buffer.alloc(4, 0);

    // null → 0 for sourceEnd → no copy
    expect(src.copy(dest, 0, 0, null as unknown as number)).toEqual(0);
    expect([...dest]).toEqual([0, 0, 0, 0]);

    // true → 1 for sourceEnd → copy one byte
    expect(src.copy(dest, 0, 0, true as unknown as number)).toEqual(1);
    expect(dest[0]).toEqual(1);

    // string "1" → 1 for targetStart
    const dest2 = Buffer.alloc(4, 0);
    expect(src.copy(dest2, "1" as unknown as number, 0, 1)).toEqual(1);
    expect([...dest2]).toEqual([0, 1, 0, 0]);

    // invalid string → NaN → toInteger default 0 for sourceEnd → no copy
    const dest3 = Buffer.alloc(4, 0xff);
    expect(src.copy(dest3, 0, 0, "nope" as unknown as number)).toEqual(0);
    expect([...dest3]).toEqual([0xff, 0xff, 0xff, 0xff]);
  });

  it("matches Node toInteger for NaN, Infinity, 2**53, and negative fractions", () => {
    const src = Buffer.from([1, 2, 3, 4]);

    // sourceEnd = NaN/Infinity → toInteger(..., 0) → 0 bytes
    const dest = Buffer.alloc(4, 0xff);
    expect(src.copy(dest, 0, 0, Number.NaN)).toEqual(0);
    expect([...dest]).toEqual([0xff, 0xff, 0xff, 0xff]);

    const dest2 = Buffer.alloc(4, 0xff);
    expect(src.copy(dest2, 0, 0, Number.POSITIVE_INFINITY)).toEqual(0);
    expect([...dest2]).toEqual([0xff, 0xff, 0xff, 0xff]);

    // targetStart = 2**53 (integer kept) → >= length → 0 bytes
    const dest3 = Buffer.alloc(4, 0xff);
    expect(src.copy(dest3, 2 ** 53, 0, 4)).toEqual(0);
    expect([...dest3]).toEqual([0xff, 0xff, 0xff, 0xff]);

    // sourceStart = 2**53 → RangeError
    expect(() => src.copy(Buffer.alloc(4), 0, 2 ** 53)).toThrow(RangeError);

    // sourceEnd = 2**53 (integer kept) → clamped to source.length → full copy
    const dest4 = Buffer.alloc(4, 0);
    expect(src.copy(dest4, 0, 0, 2 ** 53)).toEqual(4);
    expect([...dest4]).toEqual([1, 2, 3, 4]);

    // -0.5 uses Math.floor → -1 → RangeError (not trunc → 0)
    expect(() => src.copy(Buffer.alloc(4), -0.5 as unknown as number)).toThrow(
      RangeError
    );
    expect(() => src.copy(Buffer.alloc(4), 0, -0.5 as unknown as number)).toThrow(
      RangeError
    );
    expect(() =>
      src.copy(Buffer.alloc(4), 0, 0, -0.5 as unknown as number)
    ).toThrow(RangeError);

    // NaN on targetStart → toInteger 0 → copy from 0
    const dest5 = Buffer.alloc(4, 0);
    expect(src.copy(dest5, Number.NaN, 0, 2)).toEqual(2);
    expect([...dest5]).toEqual([1, 2, 0, 0]);

    // Infinity on sourceStart → toInteger 0 → copy from 0 (not a RangeError)
    const dest6 = Buffer.alloc(4, 0);
    expect(src.copy(dest6, 0, Number.POSITIVE_INFINITY, 2)).toEqual(2);
    expect([...dest6]).toEqual([1, 2, 0, 0]);

    // String / valueOf forms of 2**53 are NOT NumberIsInteger on the original value:
    // they go through ToNumber → toInteger(..., 0).
    const huge = "9007199254740992"; // 2**53
    const dest7 = Buffer.alloc(4, 0xff);
    expect(src.copy(dest7, huge as unknown as number, 0, 2)).toEqual(2);
    expect([...dest7]).toEqual([1, 2, 0xff, 0xff]);

    const dest8 = Buffer.alloc(4, 0);
    expect(
      src.copy(dest8, 0, huge as unknown as number, 4)
    ).toEqual(4); // sourceStart coerced → 0
    expect([...dest8]).toEqual([1, 2, 3, 4]);

    const dest9 = Buffer.alloc(4, 0xff);
    expect(
      src.copy(dest9, 0, 0, huge as unknown as number)
    ).toEqual(0); // sourceEnd coerced → 0
    expect([...dest9]).toEqual([0xff, 0xff, 0xff, 0xff]);

    const viaValueOf = { valueOf: () => 2 ** 53 };
    const dest10 = Buffer.alloc(4, 0xff);
    expect(src.copy(dest10, viaValueOf as unknown as number, 0, 2)).toEqual(2);
    expect([...dest10]).toEqual([1, 2, 0xff, 0xff]);
  });

  it("checks indices in Node order (does not evaluate later args after RangeError)", () => {
    const src = Buffer.from([1, 2, 3, 4]);
    const dst = Buffer.alloc(4, 9);

    // targetStart fails first — Symbol as sourceStart must not produce TypeError
    expect(() =>
      src.copy(dst, -1, Symbol("skip") as unknown as number)
    ).toThrow(RangeError);

    // sourceStart out of range — Symbol as sourceEnd must not produce TypeError
    expect(() =>
      src.copy(dst, 0, src.length + 1, Symbol("skip") as unknown as number)
    ).toThrow(RangeError);

    // sourceStart out of range after valueOf must not call sourceEnd.valueOf()
    let sourceEndCalled = false;
    const ab = new ArrayBuffer(4);
    const src2 = Buffer.from(ab);
    src2.set([1, 2, 3, 4]);
    const sourceStart = {
      valueOf() {
        // @ts-expect-error transfer may be present
        ab.transfer();
        return 1; // > detached length 0 → RangeError
      },
    };
    const sourceEnd = {
      valueOf() {
        sourceEndCalled = true;
        return 4;
      },
    };
    expect(() =>
      src2.copy(
        Buffer.alloc(4),
        0,
        sourceStart as unknown as number,
        sourceEnd as unknown as number
      )
    ).toThrow(RangeError);
    expect(sourceEndCalled).toEqual(false);
  });

  it("returns 0 when index valueOf detaches or resizes the backing buffer", () => {
    // targetStart.valueOf() detaches source
    {
      const ab = new ArrayBuffer(4);
      const src = Buffer.from(ab);
      src.set([1, 2, 3, 4]);
      const dst = Buffer.alloc(4, 9);
      const index = {
        valueOf() {
          // @ts-expect-error transfer may be present
          ab.transfer();
          return 0;
        },
      };
      expect(src.copy(dst, index as unknown as number, 0, 4)).toEqual(0);
      expect([...dst]).toEqual([9, 9, 9, 9]);
    }

    // sourceStart conversion detaches target
    {
      const ab = new ArrayBuffer(4);
      const src = Buffer.from([1, 2, 3, 4]);
      const dst = Buffer.from(ab);
      dst.fill(9);
      const index = {
        valueOf() {
          // @ts-expect-error transfer may be present
          ab.transfer();
          return 0;
        },
      };
      expect(src.copy(dst, 0, index as unknown as number, 4)).toEqual(0);
    }

    // resizable source: only wrap construction; assertions stay outside catch
    let abZero: ArrayBuffer | undefined;
    try {
      abZero = new ArrayBuffer(4, { maxByteLength: 16 });
    } catch {
      abZero = undefined;
    }
    if (abZero) {
      const src = Buffer.from(abZero);
      src.set([1, 2, 3, 4]);
      const dst = Buffer.alloc(4, 9);
      const ab = abZero;
      const index = {
        valueOf() {
          // @ts-expect-error resize may be present
          ab.resize(0);
          return 0;
        },
      };
      expect(src.copy(dst, index as unknown as number, 0, 4)).toEqual(0);
      expect([...dst]).toEqual([9, 9, 9, 9]);
    }

    // resizable source shrunk so the fixed-length Buffer view is no longer valid:
    // as_raw() fails → copy returns 0 (safe; no UAF). Node may track length and copy 2.
    let abShrink: ArrayBuffer | undefined;
    try {
      abShrink = new ArrayBuffer(8, { maxByteLength: 16 });
    } catch {
      abShrink = undefined;
    }
    if (abShrink) {
      const src = Buffer.from(abShrink);
      src.set([1, 2, 3, 4, 5, 6, 7, 8]);
      const dst = Buffer.alloc(8, 9);
      const ab = abShrink;
      const index = {
        valueOf() {
          // @ts-expect-error resize may be present
          ab.resize(2);
          return 0;
        },
      };
      expect(src.copy(dst, index as unknown as number, 0, 4)).toEqual(0);
      expect([...dst]).toEqual([9, 9, 9, 9, 9, 9, 9, 9]);
    }

    // resize then grow again: must use post-coercion pointer/length (not UAF)
    let abGrow: ArrayBuffer | undefined;
    try {
      abGrow = new ArrayBuffer(4, { maxByteLength: 16 });
    } catch {
      abGrow = undefined;
    }
    if (abGrow) {
      const src = Buffer.from(abGrow);
      src.set([1, 2, 3, 4]);
      const dst = Buffer.alloc(4, 9);
      const ab = abGrow;
      const index = {
        valueOf() {
          // @ts-expect-error resize may be present
          ab.resize(0);
          // @ts-expect-error resize may be present
          ab.resize(4);
          new Uint8Array(ab).set([5, 6, 7, 8]);
          return 0;
        },
      };
      expect(src.copy(dst, index as unknown as number, 0, 4)).toEqual(4);
      expect([...dst]).toEqual([5, 6, 7, 8]);
    }
  });

  it("ignores shadowed byteOffset on subarray for copy and numeric write", () => {
    const backing = Buffer.from([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]);
    const view = backing.subarray(2, 5); // [3,4,5]
    Object.defineProperty(view, "byteOffset", { value: 0, configurable: true });

    const dest = Buffer.alloc(3, 0);
    expect(view.copy(dest)).toEqual(3);
    expect([...dest]).toEqual([3, 4, 5]);

    // subarray with shadowed offset/length still crops from the real view
    const view3 = Buffer.from([1, 2, 3, 4, 5, 6, 7, 8]).subarray(4, 8);
    Object.defineProperty(view3, "byteOffset", { value: 0, configurable: true });
    Object.defineProperty(view3, "byteLength", { value: 100, configurable: true });
    Object.defineProperty(view3, "length", { value: 100, configurable: true });
    const sub = view3.subarray(0, 2);
    expect(sub.length).toEqual(2);
    expect([...sub]).toEqual([5, 6]);

    // Numeric write must hit the real view range (offset 2), not backing start.
    const writeBacking = Buffer.alloc(12, 0xaa);
    const view2 = writeBacking.subarray(2, 10); // 8 bytes
    Object.defineProperty(view2, "byteOffset", { value: 0, configurable: true });
    expect(view2.writeDoubleLE(1.5, 0)).toEqual(8);
    expect(writeBacking[0]).toEqual(0xaa); // first byte of backing unchanged
    expect(writeBacking[1]).toEqual(0xaa);
    expect(view2.readDoubleLE(0)).toEqual(1.5);
  });
});

describe("equals", () => {
  it("should return true for equal buffers", () => {
    expect(Buffer.from("abc").equals(Buffer.from("abc"))).toEqual(true);
  });

  it("should return false for unequal content or length", () => {
    expect(Buffer.from("abc").equals(Buffer.from("abd"))).toEqual(false);
    expect(Buffer.from("abc").equals(Buffer.from("ab"))).toEqual(false);
  });

  it("should return true for empty buffers", () => {
    expect(Buffer.alloc(0).equals(Buffer.alloc(0))).toEqual(true);
  });

  it("should accept Uint8Array and reject invalid args", () => {
    expect(Buffer.from([1, 2]).equals(new Uint8Array([1, 2]))).toEqual(true);
    expect(() => Buffer.from([1]).equals("nope" as unknown as Uint8Array)).toThrow(
      TypeError
    );
  });
});

describe("subarray", () => {
  it("should create a subarray from a buffer with the specified start and end indices", () => {
    const buffer = Buffer.from("Hello, world!");
    const subBuffer = buffer.subarray(7, 12);

    expect(subBuffer.toString()).toEqual("world");
  });

  it("should return a subarray from the start index to the end of the buffer when end index is omitted", () => {
    const buffer = Buffer.from("Hello, world!");
    const subBuffer = buffer.subarray(7);

    expect(subBuffer.toString()).toEqual("world!");
  });

  it("should return an empty buffer when the start index equals the end index", () => {
    const buffer = Buffer.from("Hello, world!");
    const subBuffer = buffer.subarray(5, 5);

    expect(subBuffer.length).toEqual(0);
    expect(subBuffer.toString()).toEqual("");
  });

  it("should create a subarray with the same content as the original buffer when start and end indices cover the entire buffer", () => {
    const buffer = Buffer.from("Hello, world!");
    const subBuffer = buffer.subarray(0, buffer.length);

    expect(subBuffer.toString()).toEqual("Hello, world!");
    expect(subBuffer).not.toBe(buffer); // Should be a new buffer, not the original one
  });

  it("should handle negative start and end indices", () => {
    const buffer = Buffer.from("Hello, world!");
    const subBuffer = buffer.subarray(-6, -1);

    expect(subBuffer.toString()).toEqual("world");
  });

  it("should handle out-of-bounds start and end indices", () => {
    const buffer = Buffer.from("Hello, world!");

    const subBuffer1 = buffer.subarray(-100, 5);
    expect(subBuffer1.toString()).toEqual("Hello");

    const subBuffer2 = buffer.subarray(0, 100);
    expect(subBuffer2.toString()).toEqual("Hello, world!");

    const subBuffer3 = buffer.subarray(50, 100);
    expect(subBuffer3.length).toEqual(0);
  });

  it("should share memory with the original buffer", () => {
    const buffer = Buffer.from("Hello, world!");
    const subBuffer = buffer.subarray(0, 5);

    const lowerCaseH = "h".charCodeAt(0);
    subBuffer[0] = lowerCaseH;
    expect(buffer[0]).toEqual(lowerCaseH);
    expect(subBuffer.toString()).toEqual("hello");
  });

  it("should not throw errors when start and end are out of order, but should return an empty buffer", () => {
    const buffer = Buffer.from("Hello, world!");
    const subBuffer = buffer.subarray(10, 5);

    expect(subBuffer.length).toEqual(0);
    expect(subBuffer.toString()).toEqual("");
  });

  it("should create subarray views that share memory with the original Buffer", () => {
    const origin = Buffer.from("Hello, World!");
    const sub = origin.subarray(7, 12);

    expect(origin.toString()).toEqual("Hello, World!");
    expect(sub.toString()).toEqual("World");

    expect(origin.subarray(1, 3).toString()).toEqual("el");
    expect(sub.subarray(1, 3).toString()).toEqual("or");
  });
});

describe("toString", () => {
  it("should convert buffer to a string with utf-8 encoding", () => {
    const input = "Hello, world!";
    const buffer = Buffer.from(input);

    expect(buffer.toString("utf-8")).toEqual(input);
  });

  it("should convert buffer to a string with base64 encoding", () => {
    const input = "SGVsbG8sIHdvcmxkIQ==";
    const buffer = Buffer.from(input, "base64");

    expect(buffer.toString("base64")).toEqual(input);
  });

  it("should convert buffer to a string with hex encoding", () => {
    const input = "48656c6c6f2c20776f726c6421";
    const buffer = Buffer.from(input, "hex");

    expect(buffer.toString("hex")).toEqual(input);
  });

  it("should convert buffer to hex string with hex encoding", () => {
    const buffer = Buffer.from("Hello");
    const hexString = buffer.toString("hex");

    expect(hexString).toEqual("48656c6c6f");
  });

  it("should convert buffer to utf-8 string with start parameter", () => {
    const buffer = Buffer.from("Hello, world!");
    const result = buffer.toString("utf-8", 7);

    expect(result).toEqual("world!");
  });

  it("should convert buffer to utf-8 string with start and end parameters", () => {
    const buffer = Buffer.from("Hello, world!");
    const result = buffer.toString("utf-8", 7, 12);

    expect(result).toEqual("world");
  });

  it("should handle negative start parameter", () => {
    const buffer = Buffer.from("Hello, world!");
    const result = buffer.toString("utf-8", -6);

    expect(result).toEqual("Hello, world!");
  });

  it("should handle negative end parameter", () => {
    const buffer = Buffer.from("Hello, world!");
    const result = buffer.toString("utf-8", 0, -1);

    expect(result).toEqual("");
  });

  it("should handle both negative start and end parameters", () => {
    const buffer = Buffer.from("Hello, world!");
    const result = buffer.toString("utf-8", -6, -1);

    expect(result).toEqual("");
  });
});

describe("write", () => {
  it("should write a UTF-8 string into a buffer and return the correct byte length", () => {
    const buf1 = Buffer.alloc(15);
    expect(buf1.write("こんにちは", "utf-8")).toEqual(15); // "こんにちは" means 'hello' in japanese.
    expect(buf1.toString("utf8")).toEqual("こんにちは");
  });

  it("should write a hex string into a buffer and correctly convert it to UTF-8", () => {
    const buf2 = Buffer.alloc(15);
    expect(buf2.write("68656c6c6f", "hex")).toEqual(5); // 68656c6c6f -> 'hello'
    expect(buf2.toString("utf8").substring(0, 5)).toEqual("hello");
  });

  it("should write a UTF-8 string into a buffer with an explicit offset of 0", () => {
    const buf1 = Buffer.alloc(15);
    expect(buf1.write("こんにちは", 0, "utf-8")).toEqual(15);
    expect(buf1.toString("utf8")).toEqual("こんにちは");
  });

  it("should write a hex string into a buffer with an explicit offset of 0", () => {
    const buf2 = Buffer.alloc(15);
    expect(buf2.write("68656c6c6f", 0, "hex")).toEqual(5);
    expect(buf2.toString("utf8").substring(0, 5)).toEqual("hello");
  });

  it("should write a UTF-8 string at offset 12 and store only part of it", () => {
    const buf1 = Buffer.alloc(15);
    expect(buf1.write("こんにちは", 12, "utf-8")).toEqual(3);
    expect(buf1.toString("utf8").substring(12)).toEqual("こ");
  });

  it("should write a hex string at offset 12 and store only part of it", () => {
    const buf2 = Buffer.alloc(15);
    expect(buf2.write("68656c6c6f", 12, "hex")).toEqual(3);
    expect(buf2.toString("utf8").substring(12)).toEqual("hel");
  });

  it("should write only the first 3 bytes of a UTF-8 string and store a partial character", () => {
    const buf1 = Buffer.alloc(15);
    expect(buf1.write("こんにちは", 0, 3, "utf-8")).toEqual(3);
    expect(buf1.toString("utf8").substring(0, 1)).toEqual("こ"); // Returning characters instead of bytes
  });

  it("should write only the first 3 bytes of a hex string and correctly store the data", () => {
    const buf2 = Buffer.alloc(15);
    expect(buf2.write("68656c6c6f", 0, 3, "hex")).toEqual(3);
    expect(buf2.toString("utf8").substring(0, 3)).toEqual("hel");
  });

  it("should write a UTF-8 string at offset 9 with a length of 12 bytes and store part of it", () => {
    const buf1 = Buffer.alloc(15);
    expect(buf1.write("こんにちは", 9, 12, "utf-8")).toEqual(6);
    expect(buf1.toString("utf8").substring(9, 12)).toEqual("こん");
  });

  it("should write a hex string at offset 9 with a length of 12 bytes and store part of it", () => {
    const buf2 = Buffer.alloc(15);
    expect(buf2.write("68656c6c6f", 9, 12, "hex")).toEqual(5);
    expect(buf2.toString("utf8").substring(9, 12)).toEqual("hel");
  });
});

describe("writeBigInt64BE", () => {
  it("should write a 64-bit BigInteger in big-endian format at the beginning of the buffer", () => {
    const buf = Buffer.alloc(16);
    expect(buf.writeBigInt64BE(0x0102030405060708n)).toEqual(8);
    expect(buf).toEqual(
      Buffer.from([1, 2, 3, 4, 5, 6, 7, 8, 0, 0, 0, 0, 0, 0, 0, 0])
    );
  });

  it("should write a 64-bit BigInteger in big-endian format at the specified offset in the buffer", () => {
    const buf = Buffer.alloc(16);
    expect(buf.writeBigInt64BE(0x0102030405060708n, 8)).toEqual(16);
    expect(buf).toEqual(
      Buffer.from([0, 0, 0, 0, 0, 0, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8])
    );
  });

  it("should throw a RangeError if the offset is out of bounds", () => {
    expect(() => {
      const buf = Buffer.alloc(16);
      buf.writeBigInt64BE(0x0102030405060708n, 9);
    }).toThrow(RangeError);
  });
});

describe("writeBigInt64LE", () => {
  it("should write a 64-bit BigInteger in little-endian format at the beginning of the buffer", () => {
    const buf = Buffer.alloc(16);
    expect(buf.writeBigInt64LE(0x0102030405060708n)).toEqual(8);
    expect(buf).toEqual(
      Buffer.from([8, 7, 6, 5, 4, 3, 2, 1, 0, 0, 0, 0, 0, 0, 0, 0])
    );
  });

  it("should write a 64-bit BigInteger in little-endian format at the specified offset in the buffer", () => {
    const buf = Buffer.alloc(16);
    expect(buf.writeBigInt64LE(0x0102030405060708n, 8)).toEqual(16);
    expect(buf).toEqual(
      Buffer.from([0, 0, 0, 0, 0, 0, 0, 0, 8, 7, 6, 5, 4, 3, 2, 1])
    );
  });

  it("should throw a RangeError if the offset is out of bounds", () => {
    expect(() => {
      const buf = Buffer.alloc(16);
      buf.writeBigInt64LE(0x0102030405060708n, 9);
    }).toThrow(RangeError);
  });
});

describe("writeDoubleBE", () => {
  it("should write a 64-bit Double in big-endian format at the beginning of the buffer", () => {
    const buf = Buffer.alloc(16);
    expect(buf.writeDoubleBE(123.456)).toEqual(8);
    expect(buf).toEqual(
      Buffer.from([64, 94, 221, 47, 26, 159, 190, 119, 0, 0, 0, 0, 0, 0, 0, 0])
    );
  });

  it("should write a 64-bit Double in big-endian format at the specified offset in the buffer", () => {
    const buf = Buffer.alloc(16);
    expect(buf.writeDoubleBE(123.456, 8)).toEqual(16);
    expect(buf).toEqual(
      Buffer.from([0, 0, 0, 0, 0, 0, 0, 0, 64, 94, 221, 47, 26, 159, 190, 119])
    );
  });

  it("should throw a RangeError if the offset is out of bounds", () => {
    expect(() => {
      const buf = Buffer.alloc(16);
      buf.writeDoubleBE(123.456, 9);
    }).toThrow(RangeError);
  });
});

describe("writeDoubleLE", () => {
  it("should write a 64-bit Double in little-endian format at the beginning of the buffer", () => {
    const buf = Buffer.alloc(16);
    expect(buf.writeDoubleLE(123.456)).toEqual(8);
    expect(buf).toEqual(
      Buffer.from([119, 190, 159, 26, 47, 221, 94, 64, 0, 0, 0, 0, 0, 0, 0, 0])
    );
  });

  it("should write a 64-bit Double in little-endian format at the specified offset in the buffer", () => {
    const buf = Buffer.alloc(16);
    expect(buf.writeDoubleLE(123.456, 8)).toEqual(16);
    expect(buf).toEqual(
      Buffer.from([0, 0, 0, 0, 0, 0, 0, 0, 119, 190, 159, 26, 47, 221, 94, 64])
    );
  });

  it("should throw a RangeError if the offset is out of bounds", () => {
    expect(() => {
      const buf = Buffer.alloc(16);
      buf.writeDoubleLE(123.456, 9);
    }).toThrow(RangeError);
  });

  it("should accept integer Number values (mysql bind path writeDoubleLE(7))", () => {
    const buf = Buffer.alloc(8);
    expect(buf.writeDoubleLE(7)).toEqual(8);
    expect(buf.readDoubleLE(0)).toEqual(7);
  });

  it("should accept fractional Number values writeDoubleLE(7.5)", () => {
    const buf = Buffer.alloc(8);
    expect(buf.writeDoubleLE(7.5)).toEqual(8);
    expect(buf.readDoubleLE(0)).toEqual(7.5);
  });

  it("should read and write floats with non-zero byteOffset subarray", () => {
    const backing = Buffer.alloc(16, 0xaa);
    const view = backing.subarray(4, 12);
    expect(view.writeDoubleLE(3.5)).toEqual(8);
    expect(view.readDoubleLE(0)).toEqual(3.5);
    // Bytes outside the view stay 0xaa
    expect(backing[0]).toEqual(0xaa);
    expect(backing[3]).toEqual(0xaa);
    expect(backing[12]).toEqual(0xaa);
    expect(backing[15]).toEqual(0xaa);
  });
});

describe("writeFloatBE", () => {
  it("should write a 32-bit Float in big-endian format at the beginning of the buffer", () => {
    const buf = Buffer.alloc(8);
    expect(buf.writeFloatBE(0xcafebabe)).toEqual(4);
    expect(buf).toEqual(Buffer.from([79, 74, 254, 187, 0, 0, 0, 0]));
  });

  it("should write a 32-bit Float in big-endian format at the specified offset in the buffer", () => {
    const buf = Buffer.alloc(8);
    expect(buf.writeFloatBE(0xcafebabe, 4)).toEqual(8);
    expect(buf).toEqual(Buffer.from([0, 0, 0, 0, 79, 74, 254, 187]));
  });

  it("should throw a RangeError if the offset is out of bounds", () => {
    expect(() => {
      const buf = Buffer.alloc(8);
      buf.writeFloatBE(0xcafebabe, 5);
    }).toThrow(RangeError);
  });
});

describe("writeFloatLE", () => {
  it("should write a 32-bit Float in little-endian format at the beginning of the buffer", () => {
    const buf = Buffer.alloc(8);
    expect(buf.writeFloatLE(0xcafebabe)).toEqual(4);
    expect(buf).toEqual(Buffer.from([187, 254, 74, 79, 0, 0, 0, 0]));
  });

  it("should write a 32-bit Float in little-endian format at the specified offset in the buffer", () => {
    const buf = Buffer.alloc(8);
    expect(buf.writeFloatLE(0xcafebabe, 4)).toEqual(8);
    expect(buf).toEqual(Buffer.from([0, 0, 0, 0, 187, 254, 74, 79]));
  });

  it("should throw a RangeError if the offset is out of bounds", () => {
    expect(() => {
      const buf = Buffer.alloc(8);
      buf.writeFloatLE(0xcafebabe, 5);
    }).toThrow(RangeError);
  });
});

describe("writeInt8", () => {
  it("should write a 8-bit integer at the beginning of the buffer", () => {
    const buf = Buffer.alloc(2);
    expect(buf.writeInt8(0x01)).toEqual(1);
    expect(buf).toEqual(Buffer.from([1, 0]));
  });

  it("should write a 8-bit integer at the specified offset in the buffer", () => {
    const buf = Buffer.alloc(2);
    expect(buf.writeInt8(0x01, 1)).toEqual(2);
    expect(buf).toEqual(Buffer.from([0, 1]));
  });

  it("should throw a RangeError if the offset is out of bounds", () => {
    expect(() => {
      const buf = Buffer.alloc(2);
      buf.writeInt8(0x01, 3);
    }).toThrow(RangeError);
  });
});

describe("writeInt16BE", () => {
  it("should write a 16-bit integer in big-endian format at the beginning of the buffer", () => {
    const buf = Buffer.alloc(4);
    expect(buf.writeInt16BE(0x0102)).toEqual(2);
    expect(buf).toEqual(Buffer.from([1, 2, 0, 0]));
  });

  it("should write a 16-bit integer in big-endian format at the specified offset in the buffer", () => {
    const buf = Buffer.alloc(4);
    expect(buf.writeInt16BE(0x0102, 2)).toEqual(4);
    expect(buf).toEqual(Buffer.from([0, 0, 1, 2]));
  });

  it("should throw a RangeError if the offset is out of bounds", () => {
    expect(() => {
      const buf = Buffer.alloc(4);
      buf.writeInt16BE(0x0102, 3);
    }).toThrow(RangeError);
  });
});

describe("writeInt16LE", () => {
  it("should write a 16-bit integer in little-endian format at the beginning of the buffer", () => {
    const buf = Buffer.alloc(4);
    expect(buf.writeInt16LE(0x0102)).toEqual(2);
    expect(buf).toEqual(Buffer.from([2, 1, 0, 0]));
  });

  it("should write a 16-bit integer in little-endian format at the specified offset in the buffer", () => {
    const buf = Buffer.alloc(4);
    expect(buf.writeInt16LE(0x0102, 2)).toEqual(4);
    expect(buf).toEqual(Buffer.from([0, 0, 2, 1]));
  });

  it("should throw a RangeError if the offset is out of bounds", () => {
    expect(() => {
      const buf = Buffer.alloc(4);
      buf.writeInt16LE(0x0102, 3);
    }).toThrow(RangeError);
  });
});

describe("writeInt32BE", () => {
  it("should write a 32-bit integer in big-endian format at the beginning of the buffer", () => {
    const buf = Buffer.alloc(8);
    expect(buf.writeInt32BE(0x01020304)).toEqual(4);
    expect(buf).toEqual(Buffer.from([1, 2, 3, 4, 0, 0, 0, 0]));
  });

  it("should write a 32-bit integer in big-endian format at the specified offset in the buffer", () => {
    const buf = Buffer.alloc(8);
    expect(buf.writeInt32BE(0x01020304, 4)).toEqual(8);
    expect(buf).toEqual(Buffer.from([0, 0, 0, 0, 1, 2, 3, 4]));
  });

  it("should throw a RangeError if the offset is out of bounds", () => {
    expect(() => {
      const buf = Buffer.alloc(8);
      buf.writeInt32BE(0x01020304, 5);
    }).toThrow(RangeError);
  });
});

describe("writeInt32LE", () => {
  it("should write a 32-bit integer in little-endian format at the beginning of the buffer", () => {
    const buf = Buffer.alloc(8);
    expect(buf.writeInt32LE(0x05060708)).toEqual(4);
    expect(buf).toEqual(Buffer.from([8, 7, 6, 5, 0, 0, 0, 0]));
  });

  it("should write a 32-bit integer in little-endian format at the specified offset in the buffer", () => {
    const buf = Buffer.alloc(8);
    expect(buf.writeInt32LE(0x05060708, 4)).toEqual(8);
    expect(buf).toEqual(Buffer.from([0, 0, 0, 0, 8, 7, 6, 5]));
  });

  it("should throw a RangeError if the offset is out of bounds", () => {
    expect(() => {
      const buf = Buffer.alloc(8);
      buf.writeInt32LE(0x05060708, 5);
    }).toThrow(RangeError);
  });
});

describe("writeUInt8", () => {
  it("should write a 8-bit unsigned integer at the beginning of the buffer", () => {
    const buf = Buffer.alloc(2);
    expect(buf.writeUInt8(2)).toEqual(1);
    expect(buf).toEqual(Buffer.from([2, 0]));
  });

  it("should write a 8-bit unsigned integer at the specified offset in the buffer", () => {
    const buf = Buffer.alloc(4);
    expect(buf.writeUInt8(0x3, 0)).toEqual(1);
    expect(buf.writeUInt8(0x4, 1)).toEqual(2);
    expect(buf.writeUInt8(0x23, 2)).toEqual(3);
    expect(buf.writeUInt8(0x42, 3)).toEqual(4);
    expect(buf).toEqual(Buffer.from([3, 4, 35, 66]));
  });

  it("should throw a RangeError if the offset is out of bounds", () => {
    expect(() => {
      const buf = Buffer.alloc(2);
      buf.writeInt8(0x01, 3);
    }).toThrow(RangeError);
  });
});

describe("writeUInt16BE", () => {
  it("should write a 16-bit unsigned integer in big-endian format at the beginning of the buffer", () => {
    const buf = Buffer.alloc(4);
    expect(buf.writeUInt16BE(0xdead)).toEqual(2);
    expect(buf).toEqual(Buffer.from([222, 173, 0, 0]));
  });

  it("should write a 16-bit unsigned integer in big-endian format at the specified offset in the buffer", () => {
    const buf = Buffer.alloc(4);
    expect(buf.writeUInt16BE(0xbeef, 2)).toEqual(4);
    expect(buf).toEqual(Buffer.from([0, 0, 190, 239]));
  });

  it("should throw a RangeError if the offset is out of bounds", () => {
    expect(() => {
      const buf = Buffer.alloc(4);
      buf.writeUInt16BE(0x0102, 3);
    }).toThrow(RangeError);
  });
});

describe("writeUInt16LE", () => {
  it("should write a 16-bit unsigned integer in little-endian format at the beginning of the buffer", () => {
    const buf = Buffer.alloc(4);
    expect(buf.writeUInt16LE(0xdead)).toEqual(2);
    expect(buf).toEqual(Buffer.from([173, 222, 0, 0]));
  });

  it("should write a 16-bit unsigned integer in little-endian format at the specified offset in the buffer", () => {
    const buf = Buffer.alloc(4);
    expect(buf.writeUInt16LE(0xbeef, 2)).toEqual(4);
    expect(buf).toEqual(Buffer.from([0, 0, 239, 190]));
  });

  it("should throw a RangeError if the offset is out of bounds", () => {
    expect(() => {
      const buf = Buffer.alloc(4);
      buf.writeUInt16LE(0x0304, 3);
    }).toThrow(RangeError);
  });
});

describe("writeUInt32BE", () => {
  it("should write a 32-bit unsigned integer in big-endian format at the beginning of the buffer", () => {
    const buf = Buffer.alloc(8);
    expect(buf.writeUInt32BE(0xfeedface)).toEqual(4);
    expect(buf).toEqual(Buffer.from([254, 237, 250, 206, 0, 0, 0, 0]));
  });

  it("should write a 32-bit unsigned integer in big-endian format at the specified offset in the buffer", () => {
    const buf = Buffer.alloc(8);
    expect(buf.writeUInt32BE(0xfeedface, 4)).toEqual(8);
    expect(buf).toEqual(Buffer.from([0, 0, 0, 0, 254, 237, 250, 206]));
  });

  it("should throw a RangeError if the offset is out of bounds", () => {
    expect(() => {
      const buf = Buffer.alloc(8);
      buf.writeUInt32BE(0x01020304, 5);
    }).toThrow(RangeError);
  });
});

describe("writeUInt32LE", () => {
  it("should write a 32-bit unsigned integer in little-endian format at the beginning of the buffer", () => {
    const buf = Buffer.alloc(8);
    expect(buf.writeUInt32LE(0xfeedface)).toEqual(4);
    expect(buf).toEqual(Buffer.from([206, 250, 237, 254, 0, 0, 0, 0]));
  });

  it("should write a 32-bit unsigned integer in little-endian format at the specified offset in the buffer", () => {
    const buf = Buffer.alloc(8);
    expect(buf.writeUInt32LE(0x01020304, 4)).toEqual(8);
    expect(buf).toEqual(Buffer.from([0, 0, 0, 0, 4, 3, 2, 1]));
  });

  it("should throw a RangeError if the offset is out of bounds", () => {
    expect(() => {
      const buf = Buffer.alloc(8);
      buf.writeUInt32LE(0x01020304, 5);
    }).toThrow(RangeError);
  });
});

describe("readBigInt64BE", () => {
  it("should read a 64-bit signed BigInt in big-endian format from the beginning of the buffer", () => {
    const buf = Buffer.from([1, 2, 3, 4, 5, 6, 7, 8, 0, 0, 0, 0, 0, 0, 0, 0]);
    expect(buf.readBigInt64BE()).toEqual(0x0102030405060708n);
  });

  it("should read a 64-bit signed BigInt in big-endian format from the specified offset", () => {
    const buf = Buffer.from([0, 0, 0, 0, 0, 0, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8]);
    expect(buf.readBigInt64BE(8)).toEqual(0x0102030405060708n);
  });

  it("should throw a RangeError if the offset is out of bounds", () => {
    expect(() => {
      const buf = Buffer.alloc(16);
      buf.readBigInt64BE(9);
    }).toThrow(RangeError);
  });
});

describe("readBigInt64LE", () => {
  it("should read a 64-bit signed BigInt in little-endian format from the beginning of the buffer", () => {
    const buf = Buffer.from([8, 7, 6, 5, 4, 3, 2, 1, 0, 0, 0, 0, 0, 0, 0, 0]);
    expect(buf.readBigInt64LE()).toEqual(0x0102030405060708n);
  });

  it("should read a 64-bit signed BigInt in little-endian format from the specified offset", () => {
    const buf = Buffer.from([0, 0, 0, 0, 0, 0, 0, 0, 8, 7, 6, 5, 4, 3, 2, 1]);
    expect(buf.readBigInt64LE(8)).toEqual(0x0102030405060708n);
  });

  it("should throw a RangeError if the offset is out of bounds", () => {
    expect(() => {
      const buf = Buffer.alloc(16);
      buf.readBigInt64LE(9);
    }).toThrow(RangeError);
  });
});

describe("readDoubleBE", () => {
  it("should read a 64-bit Double in big-endian format from the beginning of the buffer", () => {
    const buf = Buffer.from([
      64, 94, 221, 47, 26, 159, 190, 119, 0, 0, 0, 0, 0, 0, 0, 0,
    ]);
    expect(buf.readDoubleBE()).toBeCloseTo(123.456);
  });

  it("should read a 64-bit Double in big-endian format from the specified offset", () => {
    const buf = Buffer.from([
      0, 0, 0, 0, 0, 0, 0, 0, 64, 94, 221, 47, 26, 159, 190, 119,
    ]);
    expect(buf.readDoubleBE(8)).toBeCloseTo(123.456);
  });

  it("should throw a RangeError if the offset is out of bounds", () => {
    expect(() => {
      const buf = Buffer.alloc(16);
      buf.readDoubleBE(9);
    }).toThrow(RangeError);
  });
});

describe("readDoubleLE", () => {
  it("should read a 64-bit Double in little-endian format from the beginning of the buffer", () => {
    const buf = Buffer.from([
      119, 190, 159, 26, 47, 221, 94, 64, 0, 0, 0, 0, 0, 0, 0, 0,
    ]);
    expect(buf.readDoubleLE()).toBeCloseTo(123.456);
  });

  it("should read a 64-bit Double in little-endian format from the specified offset", () => {
    const buf = Buffer.from([
      0, 0, 0, 0, 0, 0, 0, 0, 119, 190, 159, 26, 47, 221, 94, 64,
    ]);
    expect(buf.readDoubleLE(8)).toBeCloseTo(123.456);
  });

  it("should throw a RangeError if the offset is out of bounds", () => {
    expect(() => {
      const buf = Buffer.alloc(16);
      buf.readDoubleLE(9);
    }).toThrow(RangeError);
  });
});

describe("readFloatBE", () => {
  it("should read a 32-bit Float in big-endian format from the beginning of the buffer", () => {
    const buf = Buffer.from([79, 74, 254, 187, 0, 0, 0, 0]);
    expect(buf.readFloatBE()).toBeCloseTo(0xcafebabe, -4);
  });

  it("should read a 32-bit Float in big-endian format from the specified offset", () => {
    const buf = Buffer.from([0, 0, 0, 0, 79, 74, 254, 187]);
    expect(buf.readFloatBE(4)).toBeCloseTo(0xcafebabe, -4);
  });

  it("should throw a RangeError if the offset is out of bounds", () => {
    expect(() => {
      const buf = Buffer.alloc(8);
      buf.readFloatBE(5);
    }).toThrow(RangeError);
  });
});

describe("readFloatLE", () => {
  it("should read a 32-bit Float in little-endian format from the beginning of the buffer", () => {
    const buf = Buffer.from([187, 254, 74, 79, 0, 0, 0, 0]);
    expect(buf.readFloatLE()).toBeCloseTo(0xcafebabe, -4);
  });

  it("should read a 32-bit Float in little-endian format from the specified offset", () => {
    const buf = Buffer.from([0, 0, 0, 0, 187, 254, 74, 79]);
    expect(buf.readFloatLE(4)).toBeCloseTo(0xcafebabe, -4);
  });

  it("should throw a RangeError if the offset is out of bounds", () => {
    expect(() => {
      const buf = Buffer.alloc(8);
      buf.readFloatLE(5);
    }).toThrow(RangeError);
  });
});

describe("readInt8", () => {
  it("should read a 8-bit integer from the beginning of the buffer", () => {
    const buf = Buffer.from([1, 0]);
    expect(buf.readInt8()).toEqual(0x01);
  });

  it("should read a 8-bit integer from the specified offset", () => {
    const buf = Buffer.from([0, 1]);
    expect(buf.readInt8(1)).toEqual(0x01);
  });

  it("should throw a RangeError if the offset is out of bounds", () => {
    expect(() => {
      const buf = Buffer.alloc(2);
      buf.readInt8(3);
    }).toThrow(RangeError);
  });
});

describe("readInt16BE", () => {
  it("should read a 16-bit integer in big-endian format from the beginning of the buffer", () => {
    const buf = Buffer.from([1, 2, 0, 0]);
    expect(buf.readInt16BE()).toEqual(0x0102);
  });

  it("should read a 16-bit integer in big-endian format from the specified offset", () => {
    const buf = Buffer.from([0, 0, 1, 2]);
    expect(buf.readInt16BE(2)).toEqual(0x0102);
  });

  it("should throw a RangeError if the offset is out of bounds", () => {
    expect(() => {
      const buf = Buffer.alloc(4);
      buf.readInt16BE(3);
    }).toThrow(RangeError);
  });
});

describe("readInt16LE", () => {
  it("should read a 16-bit integer in little-endian format from the beginning of the buffer", () => {
    const buf = Buffer.from([2, 1, 0, 0]);
    expect(buf.readInt16LE()).toEqual(0x0102);
  });

  it("should read a 16-bit integer in little-endian format from the specified offset", () => {
    const buf = Buffer.from([0, 0, 2, 1]);
    expect(buf.readInt16LE(2)).toEqual(0x0102);
  });

  it("should throw a RangeError if the offset is out of bounds", () => {
    expect(() => {
      const buf = Buffer.alloc(4);
      buf.readInt16LE(3);
    }).toThrow(RangeError);
  });
});

describe("readInt32BE", () => {
  it("should read a 32-bit integer in big-endian format from the beginning of the buffer", () => {
    const buf = Buffer.from([1, 2, 3, 4, 0, 0, 0, 0]);
    expect(buf.readInt32BE()).toEqual(0x01020304);
  });

  it("should read a 32-bit integer in big-endian format from the specified offset", () => {
    const buf = Buffer.from([0, 0, 0, 0, 1, 2, 3, 4]);
    expect(buf.readInt32BE(4)).toEqual(0x01020304);
  });

  it("should throw a RangeError if the offset is out of bounds", () => {
    expect(() => {
      const buf = Buffer.alloc(8);
      buf.readInt32BE(5);
    }).toThrow(RangeError);
  });
});

describe("readInt32LE", () => {
  it("should read a 32-bit integer in little-endian format from the beginning of the buffer", () => {
    const buf = Buffer.from([8, 7, 6, 5, 0, 0, 0, 0]);
    expect(buf.readInt32LE()).toEqual(0x05060708);
  });

  it("should read a 32-bit integer in little-endian format from the specified offset", () => {
    const buf = Buffer.from([0, 0, 0, 0, 8, 7, 6, 5]);
    expect(buf.readInt32LE(4)).toEqual(0x05060708);
  });

  it("should throw a RangeError if the offset is out of bounds", () => {
    expect(() => {
      const buf = Buffer.alloc(8);
      buf.readInt32LE(5);
    }).toThrow(RangeError);
  });
});

describe("readUInt8", () => {
  it("should read an 8-bit unsigned integer from the beginning of the buffer", () => {
    const buf = Buffer.from([2, 0]);
    expect(buf.readUInt8()).toEqual(2);
  });

  it("should read an 8-bit unsigned integer from the specified offset", () => {
    const buf = Buffer.from([3, 4, 35, 66]);
    expect(buf.readUInt8(0)).toEqual(0x3);
    expect(buf.readUInt8(1)).toEqual(0x4);
    expect(buf.readUInt8(2)).toEqual(0x23);
    expect(buf.readUInt8(3)).toEqual(0x42);
  });

  it("should throw a RangeError if the offset is out of bounds", () => {
    expect(() => {
      const buf = Buffer.alloc(2);
      buf.readUInt8(3);
    }).toThrow(RangeError);
  });
});

describe("readUInt16BE", () => {
  it("should read a 16-bit unsigned integer in big-endian format from the beginning of the buffer", () => {
    const buf = Buffer.from([222, 173, 0, 0]);
    expect(buf.readUInt16BE()).toEqual(0xdead);
  });

  it("should read a 16-bit unsigned integer in big-endian format from the specified offset", () => {
    const buf = Buffer.from([0, 0, 190, 239]);
    expect(buf.readUInt16BE(2)).toEqual(0xbeef);
  });

  it("should throw a RangeError if the offset is out of bounds", () => {
    expect(() => {
      const buf = Buffer.alloc(4);
      buf.readUInt16BE(3);
    }).toThrow(RangeError);
  });
});

describe("readUInt16LE", () => {
  it("should read a 16-bit unsigned integer in little-endian format from the beginning of the buffer", () => {
    const buf = Buffer.from([173, 222, 0, 0]);
    expect(buf.readUInt16LE()).toEqual(0xdead);
  });

  it("should read a 16-bit unsigned integer in little-endian format from the specified offset", () => {
    const buf = Buffer.from([0, 0, 239, 190]);
    expect(buf.readUInt16LE(2)).toEqual(0xbeef);
  });

  it("should throw a RangeError if the offset is out of bounds", () => {
    expect(() => {
      const buf = Buffer.alloc(4);
      buf.readUInt16LE(3);
    }).toThrow(RangeError);
  });
});

describe("readUInt32BE", () => {
  it("should read a 32-bit unsigned integer in big-endian format from the beginning of the buffer", () => {
    const buf = Buffer.from([254, 237, 250, 206, 0, 0, 0, 0]);
    expect(buf.readUInt32BE()).toEqual(0xfeedface);
  });

  it("should read a 32-bit unsigned integer in big-endian format from the specified offset", () => {
    const buf = Buffer.from([0, 0, 0, 0, 254, 237, 250, 206]);
    expect(buf.readUInt32BE(4)).toEqual(0xfeedface);
  });

  it("should throw a RangeError if the offset is out of bounds", () => {
    expect(() => {
      const buf = Buffer.alloc(8);
      buf.readUInt32BE(5);
    }).toThrow(RangeError);
  });
});

describe("readUInt32LE", () => {
  it("should read a 32-bit unsigned integer in little-endian format from the beginning of the buffer", () => {
    const buf = Buffer.from([206, 250, 237, 254, 0, 0, 0, 0]);
    expect(buf.readUInt32LE()).toEqual(0xfeedface);
  });

  it("should read a 32-bit unsigned integer in little-endian format from the specified offset", () => {
    const buf = Buffer.from([0, 0, 0, 0, 4, 3, 2, 1]);
    expect(buf.readUInt32LE(4)).toEqual(0x01020304);
  });

  it("should throw a RangeError if the offset is out of bounds", () => {
    expect(() => {
      const buf = Buffer.alloc(8);
      buf.readUInt32LE(5);
    }).toThrow(RangeError);
  });
});

describe("Blob class", () => {
  it("should construct a new Blob object with the provided data and options", () => {
    const blobData = ["Hello, world!"];
    const blobOptions = { type: "text/plain" };
    const blob = new Blob(blobData, blobOptions);

    expect(blob.size).toEqual(blobData[0].length);
    expect(blob.type).toEqual(blobOptions.type);
  });

  it("should create a Blob with default type if options.type is not provided", () => {
    const blobData = ["Hello, world!"];
    const blob = new Blob(blobData);

    expect(blob.size).toEqual(blobData[0].length);
    expect(blob.type).toEqual("");
  });

  it("should create a Blob with an empty array if no data is provided", () => {
    // @ts-ignore
    const blob = new Blob();

    expect(blob.size).toEqual(0);
    expect(blob.type).toEqual("");
  });

  it("should handle line endings properly", async () => {
    const text = "This\r\n is a \ntest\r\n string";

    // @ts-ignore
    const blob = new Blob([text], {
      // @ts-ignore
      endings: "native",
    });

    expect(blob.type).toEqual("");
    if (process.platform != "win32") {
      expect(blob.size < text.length).toBeTruthy();
      expect(await blob.text()).toEqual(text.replace(/\r\n/g, "\n"));
    }
  });

  it("should return an ArrayBuffer with the arrayBuffer() method", async () => {
    const blobData = ["Hello, world!"];
    const blob = new Blob(blobData, { type: "text/plain" });

    const arrayBuffer = await blob.arrayBuffer();

    expect(arrayBuffer).toBeInstanceOf(ArrayBuffer);
  });

  it("should return an Uint8Array with the bytes() method", async () => {
    const blobData = ["Hello, world!"];
    const blob = new Blob(blobData, { type: "text/plain" });

    const bytes = await blob.bytes();

    expect(bytes).toBeInstanceOf(Uint8Array);
  });

  it("should return a DataView with the slice method", () => {
    const blobData = ["Hello, world!"];
    const blob = new Blob(blobData, { type: "text/plain" });

    const slicedBlob = blob.slice(0, 5, "text/plain");

    expect(slicedBlob instanceof Blob).toBeTruthy();
    expect(slicedBlob.size).toEqual(5);
    expect(slicedBlob.type).toEqual("text/plain");
  });
});

describe("File class", () => {
  it("should construct a new File", () => {
    const file = new File(["Hello, world!"], "hello.txt", {
      type: "text/plain",
    });

    expect(file.size).toBe(13);
    expect(file.type).toBe("text/plain");
    expect(file.name).toBe("hello.txt");
  });

  it("should return the correct lastModified date", () => {
    const fileWithDate = new File([], "file.bin", {
      lastModified: new Date(Date.UTC(2017, 1, 1, 0, 0, 0, 0)).getTime(),
    });
    expect(fileWithDate.lastModified).toBe(1485907200000);
  });

  it("has a name", () => {
    const file = new File(["file content"], "example.txt");
    expect(file.name).toBe("example.txt");
  });

  it("has content", () => {
    const file = new File(["file content"], "example.txt");
    expect(file.size).toBeGreaterThan(0);
  });

  it("has a size", () => {
    const file = new File(["file content"], "example.txt");
    expect(file.size).toBeGreaterThan(0);
  });

  it("has a type", () => {
    const file = new File(["file content"], "example.txt", {
      type: "text/plain",
    });
    expect(file.type).toBe("text/plain");
  });

  it("has last modified date", () => {
    const file = new File(["file content"], "example.txt");
    const now = new Date();
    expect(file.lastModified * 0.9999).toBeLessThanOrEqual(now.getTime());
  });

  it("can slice file", () => {
    const file = new File(["file content"], "example.txt");
    const slice = file.slice(0, 5);
    expect(slice).toBeInstanceOf(Blob);
    expect(slice.size).toBe(5);
  });

  it("can read file as text", async () => {
    const file = new File(["file content"], "example.txt");
    const text = await file.text();
    expect(text).toBe("file content");
  });

  it("can read file as arrayBuffer", async () => {
    const file = new File([1, 2, 3, 4] as any, "example.txt");
    const arrayBuffer = await file.arrayBuffer();
    const uint8Array = new Uint8Array(arrayBuffer);
    expect(Array.from(uint8Array)).toStrictEqual([49, 50, 51, 52]);
    expect(uint8Array.length).toBe(4);
  });

  it("is an instance of Blob", () => {
    const file = new File(["file content"], "example.txt");
    expect(file).toBeInstanceOf(Blob);
  });
});
