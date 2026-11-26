describe("Intl.supportedValuesOf", () => {
  it("should return an array", () => {
    const zones = Intl.supportedValuesOf("timeZone");
    expect(Array.isArray(zones)).toBe(true);
  });

  it("should contain common timezones", () => {
    const zones = Intl.supportedValuesOf("timeZone");
    expect(zones).toContain("America/New_York");
    expect(zones).toContain("America/Los_Angeles");
    expect(zones).toContain("America/Denver");
    expect(zones).toContain("Europe/London");
    expect(zones).toContain("Europe/Paris");
    expect(zones).toContain("Asia/Tokyo");
    expect(zones).toContain("UTC");
  });

  it("should return many timezones", () => {
    const zones = Intl.supportedValuesOf("timeZone");
    expect(zones.length).toBeGreaterThan(400);
  });
});

describe("Intl.DateTimeFormat", () => {
  describe("basic functionality", () => {
    it("should be defined globally", () => {
      expect(typeof Intl).toBe("object");
      expect(typeof Intl.DateTimeFormat).toBe("function");
    });

    it("should have correct Symbol.toStringTag", () => {
      const formatter = new Intl.DateTimeFormat("en-US");
      expect(formatter[Symbol.toStringTag]).toBe("Intl.DateTimeFormat");
    });
  });

  describe("formatToParts", () => {
    it("should return an array of parts", () => {
      const formatter = new Intl.DateTimeFormat("en-US", {
        timeZone: "UTC",
        year: "numeric",
        month: "2-digit",
        day: "2-digit",
      });
      const date = new Date("2022-03-02T15:45:34Z");
      const parts = formatter.formatToParts(date);

      expect(Array.isArray(parts)).toBe(true);
      expect(parts.length).toBeGreaterThan(0);
    });

    it("should format date in specified timezone", () => {
      const formatter = new Intl.DateTimeFormat("en-US", {
        timeZone: "America/Denver",
        hour12: false,
        year: "numeric",
        month: "2-digit",
        day: "2-digit",
        hour: "2-digit",
        minute: "2-digit",
        second: "2-digit",
      });
      // March 2, 2022 15:45:34 UTC = March 2, 2022 08:45:34 Denver (MST -7)
      const date = new Date("2022-03-02T15:45:34Z");
      const parts = formatter.formatToParts(date);

      const hourPart = parts.find((p: any) => p.type === "hour");
      expect(hourPart?.value).toBe("08");
    });

    it("should handle DST correctly", () => {
      const formatter = new Intl.DateTimeFormat("en-US", {
        timeZone: "America/Denver",
        hour12: false,
        hour: "2-digit",
      });

      // June 15, 2024 - DST is in effect (MDT -6)
      const summerDate = new Date("2024-06-15T18:00:00Z");
      const summerParts = formatter.formatToParts(summerDate);
      const summerHour = summerParts.find((p: any) => p.type === "hour");
      expect(summerHour?.value).toBe("12"); // 18 UTC - 6 = 12

      // January 15, 2024 - No DST (MST -7)
      const winterDate = new Date("2024-01-15T18:00:00Z");
      const winterParts = formatter.formatToParts(winterDate);
      const winterHour = winterParts.find((p: any) => p.type === "hour");
      expect(winterHour?.value).toBe("11"); // 18 UTC - 7 = 11
    });
  });

  describe("format", () => {
    it("should return a formatted string", () => {
      const formatter = new Intl.DateTimeFormat("en-US", {
        timeZone: "UTC",
        year: "numeric",
        month: "2-digit",
        day: "2-digit",
      });
      const date = new Date("2022-03-02T15:45:34Z");
      const result = formatter.format(date);

      expect(typeof result).toBe("string");
      expect(result).toContain("03");
      expect(result).toContain("02");
      expect(result).toContain("2022");
    });
  });

  describe("resolvedOptions", () => {
    it("should return resolved options", () => {
      const formatter = new Intl.DateTimeFormat("en-US", {
        timeZone: "America/Denver",
        year: "numeric",
        month: "2-digit",
      });
      const options = formatter.resolvedOptions();

      expect(options.timeZone).toBe("America/Denver");
      expect(options.locale).toBe("en-US");
      expect(options.year).toBe("numeric");
      expect(options.month).toBe("2-digit");
    });

    it("should default to system timezone when not specified", () => {
      const formatter = new Intl.DateTimeFormat("en-US");
      const options = formatter.resolvedOptions();

      expect(typeof options.timeZone).toBe("string");
      expect(options.timeZone.length).toBeGreaterThan(0);
    });
  });
});

describe("Intl.Segmenter", () => {
  it("segments graphemes including surrogate pairs with UTF-16 indices and containing()", () => {
    const seg = new Intl.Segmenter(undefined, { granularity: "grapheme" });
    expect(seg.resolvedOptions()).toEqual({ locale: "en-US", granularity: "grapheme" });
    const segments = seg.segment("A🚀B");
    const items = Array.from(segments);
    expect(items).toEqual([
      { segment: "A", index: 0, input: "A🚀B" },
      { segment: "🚀", index: 1, input: "A🚀B" },
      { segment: "B", index: 3, input: "A🚀B" },
    ]);
    expect(segments.containing(2)).toEqual({ segment: "🚀", index: 1, input: "A🚀B" });
    expect(segments.containing(99)).toBeUndefined();
  });

  it("segments words with isWordLike and sentences", () => {
    const wordSeg = new Intl.Segmenter("en", { granularity: "word" });
    const words = Array.from(wordSeg.segment("hi, world!"));
    expect(words.find((w: any) => w.segment === "hi").isWordLike).toBe(true);
    expect(words.find((w: any) => w.segment === ",").isWordLike).toBe(false);

    const sentSeg = new Intl.Segmenter("en", { granularity: "sentence" });
    const sents = Array.from(sentSeg.segment("Hello world. Next sentence!"));
    expect(sents.length).toBe(2);
  });
});

describe("Intl.NumberFormat", () => {
  it("formats currency, grouping, and compact notation", () => {
    const usd = new Intl.NumberFormat("en-US", { style: "currency", currency: "USD" });
    expect(usd.format(1234.5)).toBe("$1,234.50");
    expect(usd.format(-0.05)).toBe("-$0.05");

    const compact = new Intl.NumberFormat("en-US", {
      notation: "compact",
      maximumFractionDigits: 1,
    });
    expect(compact.format(1500)).toBe("1.5K");
    expect(compact.format(2000000)).toBe("2M");
  });
});

describe("Intl.ListFormat", () => {
  it("formats conjunction and disjunction lists", () => {
    const conj = new Intl.ListFormat("en-US", { style: "long", type: "conjunction" });
    expect(conj.format(["a", "b", "c"])).toBe("a, b, and c");
    const disj = new Intl.ListFormat("en", { type: "disjunction" });
    expect(disj.format(["foo", "bar"])).toBe("foo or bar");
  });
});

describe("Intl.Collator", () => {
  it("compares strings with numeric and sensitivity options", () => {
    const num = new Intl.Collator("en", { numeric: true, sensitivity: "base" });
    expect(["file10", "file2", "File1"].sort((a, b) => num.compare(a, b))).toEqual([
      "File1",
      "file2",
      "file10",
    ]);
  });
});
