import hex from "lexe:hex";

describe("lexe:hex", () => {
  it("should encode/decode text", () => {
    let hello = "hello";
    const encoded = new TextEncoder().encode(hello);
    const decoded = new TextDecoder().decode(encoded);

    expect(decoded).toEqual(hello);
  });

  it("should encode/decode hex", () => {
    const byteArray = new TextEncoder().encode("hello");
    const encoded = hex.encode(byteArray);

    expect(encoded).toEqual("68656c6c6f");
  });
});

describe("TextDecoder", () => {
  it("should be able to decode UTF-16LE labels", () => {
    const ary_u8 = new Uint8Array([
      0x48, 0x00, 0xac, 0x20, 0x6c, 0x00, 0x6c, 0x00, 0x6f, 0x00, 0x20, 0x00,
      0x77, 0x00, 0x6f, 0x00, 0x72, 0x00, 0x6c, 0x00, 0x64, 0x00, 0x21, 0x00,
    ]);
    const decoded = new TextDecoder("UTF-16LE");
    expect(decoded.encoding).toEqual("utf-16le");
    expect(decoded.decode(ary_u8)).toEqual("H€llo world!");
  });

  it("should not be removed BOM", () => {
    const smile = "😄";
    const bomPlusSmile = new Uint8Array([0xef, 0xbb, 0xbf, 240, 159, 152, 132]);
    expect(new TextDecoder("utf8").decode(bomPlusSmile)).toEqual(smile);

    const decoded = new TextDecoder("utf8", { ignoreBOM: true });
    expect(decoded.encoding).toEqual("utf-8");
    expect(decoded.ignoreBOM).toBeTruthy();
    const encoded = new TextEncoder().encode(decoded.decode(bomPlusSmile));
    expect(encoded).toEqual(bomPlusSmile);
  });

  it("should be generated fatal error", () => {
    const illegalString = new Uint8Array([0xff, 0xfe, 0xfd]);
    try {
      const decoded = new TextDecoder("utf-8", { fatal: true });
      expect(decoded.fatal).toBeTruthy();
      const a = decoded.decode(illegalString);
    } catch (ex) {
      expect(ex.message).toEqual(
        "invalid utf-8 sequence of 1 bytes from index 0"
      );
    }
  });

  it("should be generated unsupported error", () => {
    try {
      const decoded = new TextDecoder("nonexistent_label");
    } catch (ex) {
      expect(ex.message).toEqual(
        'The "nonexistent_label" encoding is not supported'
      );
    }
  });

  it("streams utf-16 lead surrogate and odd byte across chunks", () => {
    const decoder = new TextDecoder("utf-16le");
    expect(
      decoder.decode(Uint8Array.of(0x00, 0xd8, 0x41), { stream: true })
    ).toBe("");
    expect(decoder.decode(Uint8Array.of(0x00), { stream: true })).toBe("\uFFFD" + "A");

    const decoderBe = new TextDecoder("utf-16be");
    expect(
      decoderBe.decode(Uint8Array.of(0xd8, 0x00, 0x00), { stream: true })
    ).toBe("");
    expect(decoderBe.decode(Uint8Array.of(0x41), { stream: true })).toBe(
      "\uFFFD" + "A"
    );
  });

  it("replaces incomplete bom prefixes on flush", () => {
    const lossy = new TextDecoder("utf-8");
    lossy.decode(Uint8Array.of(0xef), { stream: true });
    expect(lossy.decode()).toBe("\uFFFD");

    const fatal = new TextDecoder("utf-8", { fatal: true });
    fatal.decode(Uint8Array.of(0xef), { stream: true });
    expect(() => fatal.decode()).toThrow(TypeError);
  });

  it("replaces combined utf-16 lead surrogate and odd byte on flush", () => {
    const decoder = new TextDecoder("utf-16le");
    decoder.decode(Uint8Array.of(0x00, 0xd8, 0x41), { stream: true });
    expect(decoder.decode()).toBe("\uFFFD");

    const decoderBe = new TextDecoder("utf-16be");
    decoderBe.decode(Uint8Array.of(0xd8, 0x00, 0x00), { stream: true });
    expect(decoderBe.decode()).toBe("\uFFFD");
  });
});

describe("TextEncoder", () => {
  it("should be able to encodeInto of surrogate pair character(Short Array)", () => {
    const hono = "🔥";
    const encoded = new TextEncoder();

    const u8Array3 = new Uint8Array(3);
    const result3 = encoded.encodeInto(hono, u8Array3);
    expect(result3.read).toEqual(0);
    expect(result3.written).toEqual(0);
    expect(u8Array3).toEqual(new Uint8Array([0, 0, 0]));
  });

  it("should be able to encodeInto of surrogate pair character(Equal Length Array)", () => {
    const hono = "🔥";
    const encoded = new TextEncoder();

    const u8Array4 = new Uint8Array(4);
    const result4 = encoded.encodeInto(hono, u8Array4);
    expect(result4.read).toEqual(2);
    expect(result4.written).toEqual(4);
    expect(u8Array4).toEqual(new Uint8Array([240, 159, 148, 165]));
  });

  it("should be able to encodeInto of surrogate pair character(Long Array)", () => {
    const hono = "🔥";
    const encoded = new TextEncoder();

    const u8Array5 = new Uint8Array(5);
    const result5 = encoded.encodeInto(hono, u8Array5);
    expect(result5.read).toEqual(2);
    expect(result5.written).toEqual(4);
    expect(u8Array5).toEqual(new Uint8Array([240, 159, 148, 165, 0]));
  });

  it("should be able to encodeInto and decode", () => {
    const hono = "hono - [炎] means flame🔥 in Japanese";
    const encoded = new TextEncoder();

    const u8Array40 = new Uint8Array(40);
    const resultHono = encoded.encodeInto(hono, u8Array40);
    expect(resultHono.read).toEqual(36);
    expect(resultHono.written).toEqual(40);
    expect(u8Array40).toEqual(
      new Uint8Array([
        104, 111, 110, 111, 32, 45, 32, 91, 231, 130, 142, 93, 32, 109, 101, 97,
        110, 115, 32, 102, 108, 97, 109, 101, 240, 159, 148, 165, 32, 105, 110,
        32, 74, 97, 112, 97, 110, 101, 115, 101,
      ])
    );
    expect(new TextDecoder().decode(u8Array40)).toEqual(hono);
  });

  it("handles undefined label/options like WHATWG default utf-8 and coerces null to 'null'", () => {
    const bytes = new TextEncoder().encode("hello-utf8");
    const dec1 = new TextDecoder(undefined);
    expect(dec1.encoding).toBe("utf-8");
    expect(dec1.decode(bytes)).toBe("hello-utf8");

    const dec2 = new TextDecoder(undefined, undefined);
    expect(dec2.encoding).toBe("utf-8");
    expect(dec2.decode(bytes)).toBe("hello-utf8");

    expect(() => new TextDecoder(null as any)).toThrow(RangeError);
  });
});

describe("WHATWG legacy encodings", () => {
  it("decodes CJK encodings (gbk, gb18030, big5, shift_jis, euc-jp, iso-2022-jp, euc-kr)", () => {
    const gbk = new TextDecoder("gb2312");
    expect(gbk.encoding).toBe("gbk");
    expect(gbk.decode(Uint8Array.of(0xd6, 0xd0, 0xce, 0xc4))).toBe("中文");

    const gb18030 = new TextDecoder("  GB18030\t");
    expect(gb18030.encoding).toBe("gb18030");
    // Euro sign in GB18030: 0xA2 0xE3; 4-byte supplementary (U+1F4DE): 0x94 0x39 0xDF 0x36
    expect(gb18030.decode(Uint8Array.of(0xa2, 0xe3))).toBe("€");
    expect(gb18030.decode(Uint8Array.of(0x94, 0x39, 0xdf, 0x36))).toBe("📞");

    const big5 = new TextDecoder("big5");
    expect(big5.encoding).toBe("big5");
    expect(big5.decode(Uint8Array.of(0xa4, 0xa4, 0xa4, 0xe5))).toBe("中文");

    const sjis = new TextDecoder("sjis");
    expect(sjis.encoding).toBe("shift_jis");
    expect(sjis.decode(Uint8Array.of(0x93, 0xfa, 0x96, 0x7b, 0x8c, 0xea))).toBe("日本語");

    const eucJp = new TextDecoder("euc-jp");
    expect(eucJp.encoding).toBe("euc-jp");
    expect(eucJp.decode(Uint8Array.of(0xc6, 0xfc, 0xcb, 0xdc, 0xb8, 0xec))).toBe("日本語");

    const iso2022 = new TextDecoder("iso-2022-jp");
    expect(iso2022.encoding).toBe("iso-2022-jp");
    expect(
      iso2022.decode(Uint8Array.of(0x1b, 0x24, 0x42, 0x46, 0x7c, 0x4b, 0x5c, 0x1b, 0x28, 0x42))
    ).toBe("日本");

    const eucKr = new TextDecoder("ks_c_5601-1987");
    expect(eucKr.encoding).toBe("euc-kr");
    expect(eucKr.decode(Uint8Array.of(0xc7, 0xd1, 0xb1, 0xdb))).toBe("한글");
  });

  it("decodes single-byte encodings and replacement / x-user-defined", () => {
    const win1251 = new TextDecoder("cp1251");
    expect(win1251.encoding).toBe("windows-1251");
    expect(win1251.decode(Uint8Array.of(0xcf, 0xf0, 0xe8, 0xe2, 0xe5, 0xf2))).toBe("Привет");

    const iso2 = new TextDecoder("iso-8859-2");
    expect(iso2.encoding).toBe("iso-8859-2");
    expect(iso2.decode(Uint8Array.of(0xc8, 0xf8))).toBe("Čř");

    const koi8 = new TextDecoder("koi8-r");
    expect(koi8.encoding).toBe("koi8-r");
    expect(koi8.decode(Uint8Array.of(0xf0, 0xd2, 0xc9, 0xd7, 0xc5, 0xd4))).toBe("Привет");

    const userDef = new TextDecoder("x-user-defined");
    expect(userDef.encoding).toBe("x-user-defined");
    expect(userDef.decode(Uint8Array.of(0x41, 0x80))).toBe("A\uF780");

    const rep = new TextDecoder("iso-2022-kr");
    expect(rep.encoding).toBe("replacement");
    expect(rep.decode(new Uint8Array())).toBe("");
    expect(rep.decode(Uint8Array.of(0x41, 0x42))).toBe("\uFFFD");
    const repFatal = new TextDecoder("replacement", { fatal: true });
    expect(repFatal.decode(new Uint8Array())).toBe("");
    expect(() => repFatal.decode(Uint8Array.of(0x41))).toThrow(TypeError);
  });

  it("streams multi-byte legacy encodings and honors fatal mode via TextDecoder, util.TextDecoder, and TextDecoderStream", async () => {
    const decoder = new TextDecoder("gbk");
    expect(decoder.decode(Uint8Array.of(0xd6), { stream: true })).toBe("");
    expect(decoder.decode(Uint8Array.of(0xd0, 0xce), { stream: true })).toBe("中");
    expect(decoder.decode(Uint8Array.of(0xc4))).toBe("文");

    // Trailing incomplete byte on flush becomes U+FFFD in lossy mode, TypeError in fatal mode.
    decoder.decode(Uint8Array.of(0xd6), { stream: true });
    expect(decoder.decode()).toBe("\uFFFD");

    const fatalGbk = new TextDecoder("gbk", { fatal: true });
    expect(fatalGbk.decode(Uint8Array.of(0x80))).toBe("€");
    expect(() => fatalGbk.decode(Uint8Array.of(0x81, 0x30))).toThrow(TypeError);
    fatalGbk.decode(Uint8Array.of(0xd6), { stream: true });
    expect(() => fatalGbk.decode()).toThrow(TypeError);
    // Cleanly usable after error
    expect(fatalGbk.decode(Uint8Array.of(0xd6, 0xd0))).toBe("中");

    const util = await import("node:util");
    expect(new util.TextDecoder("shift_jis").decode(Uint8Array.of(0x93, 0xfa))).toBe("日");

    const rs = new ReadableStream({
      start(c) {
        c.enqueue(Uint8Array.of(0xd6));
        c.enqueue(Uint8Array.of(0xd0, 0xce));
        c.enqueue(Uint8Array.of(0xc4));
        c.close();
      },
    });
    const out: string[] = [];
    const reader = rs.pipeThrough(new TextDecoderStream("gbk")).getReader();
    while (true) {
      const { value, done } = await reader.read();
      if (done) break;
      out.push(value);
    }
    expect(out.join("")).toBe("中文");
  });
});
