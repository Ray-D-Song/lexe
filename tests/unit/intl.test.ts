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

describe("Intl.supportedValuesOf extended keys", () => {
  it("supports calendar, collation, currency, numberingSystem, and unit", () => {
    expect(Intl.supportedValuesOf("calendar")).toContain("iso8601");
    expect(Intl.supportedValuesOf("collation")).toContain("emoji");
    expect(Intl.supportedValuesOf("currency")).toContain("USD");
    expect(Intl.supportedValuesOf("numberingSystem")).toContain("latn");
    expect(Intl.supportedValuesOf("unit")).toContain("second");
  });
});

describe("Intl.Locale", () => {
  it("parses BCP 47 tags and extension subtags", () => {
    const loc = new Intl.Locale("zh-Hans-cn-u-ca-iso8601-hc-h23-kn");
    expect(loc[Symbol.toStringTag]).toBe("Intl.Locale");
    expect(loc.language).toBe("zh");
    expect(loc.script).toBe("Hans");
    expect(loc.region).toBe("CN");
    expect(loc.baseName).toBe("zh-Hans-CN");
    expect(loc.calendar).toBe("iso8601");
    expect(loc.hourCycle).toBe("h23");
    expect(loc.numeric).toBe(true);
    expect(loc.toString()).toBe("zh-Hans-CN-u-ca-iso8601-hc-h23-kn");
  });

  it("applies option overrides and supports maximize/minimize", () => {
    const loc = new Intl.Locale("en", { region: "GB", hourCycle: "h24" });
    expect(loc.toString()).toBe("en-GB-u-hc-h24");
    expect(new Intl.Locale("zh").maximize().baseName).toBe("zh-Hans-CN");
    expect(new Intl.Locale("zh-Hans-CN").minimize().baseName).toBe("zh");
    expect(new Intl.Locale("ar-SA").getTextInfo()).toEqual({ direction: "rtl" });
    expect(new Intl.Locale("en-US").getWeekInfo().firstDay).toBe(7);
  });

  it("throws RangeError on invalid tags or options", () => {
    expect(() => new Intl.Locale("not_a_valid_tag")).toThrow();
    expect(() => new Intl.Locale("en", { hourCycle: "h99" })).toThrow();
  });
});

describe("Intl.PluralRules", () => {
  it("selects English cardinal and ordinal forms", () => {
    const card = new Intl.PluralRules("en-US");
    expect(card[Symbol.toStringTag]).toBe("Intl.PluralRules");
    expect(card.select(0)).toBe("other");
    expect(card.select(1)).toBe("one");
    expect(card.select(2)).toBe("other");

    const ord = new Intl.PluralRules("en-US", { type: "ordinal" });
    expect(ord.select(1)).toBe("one");
    expect(ord.select(2)).toBe("two");
    expect(ord.select(3)).toBe("few");
    expect(ord.select(11)).toBe("other");
    expect(ord.select(21)).toBe("one");
    expect(ord.resolvedOptions().pluralCategories).toEqual(["one", "two", "few", "other"]);
  });

  it("handles Russian, Arabic, Chinese, and supportedLocalesOf", () => {
    const ru = new Intl.PluralRules("ru");
    expect(ru.select(1)).toBe("one");
    expect(ru.select(2)).toBe("few");
    expect(ru.select(5)).toBe("many");
    expect(ru.select(11)).toBe("many");

    const ar = new Intl.PluralRules("ar");
    expect(ar.select(0)).toBe("zero");
    expect(ar.select(1)).toBe("one");
    expect(ar.select(2)).toBe("two");
    expect(ar.select(5)).toBe("few");
    expect(ar.select(15)).toBe("many");

    const zh = new Intl.PluralRules("zh-CN");
    expect(zh.select(1)).toBe("other");
    expect(Intl.PluralRules.supportedLocalesOf(["en-us", "zh-CN"])).toEqual(["en-US", "zh-CN"]);
  });
});

describe("Intl.RelativeTimeFormat", () => {
  it("formats always and auto relative times and formatToParts", () => {
    const rtfAlways = new Intl.RelativeTimeFormat("en-US", { numeric: "always" });
    expect(rtfAlways[Symbol.toStringTag]).toBe("Intl.RelativeTimeFormat");
    expect(rtfAlways.format(-1, "day")).toBe("1 day ago");
    expect(rtfAlways.format(2, "days")).toBe("in 2 days");

    const rtfAuto = new Intl.RelativeTimeFormat("en", { numeric: "auto" });
    expect(rtfAuto.format(-1, "day")).toBe("yesterday");
    expect(rtfAuto.format(0, "day")).toBe("today");
    expect(rtfAuto.format(1, "day")).toBe("tomorrow");
    expect(rtfAuto.format(0, "second")).toBe("now");

    const parts = rtfAlways.formatToParts(3, "hours");
    expect(parts.find((p: any) => p.type === "integer")).toEqual({
      type: "integer",
      value: "3",
      unit: "hour",
    });

    const zh = new Intl.RelativeTimeFormat("zh-CN", { numeric: "auto" });
    expect(zh.format(-1, "day")).toBe("昨天");
    expect(zh.format(3, "day")).toBe("3天后");
  });
});

describe("Intl.DisplayNames", () => {
  it("resolves region, currency, script, language, calendar, and dateTimeField names", () => {
    const regions = new Intl.DisplayNames(["en"], { type: "region" });
    expect(regions[Symbol.toStringTag]).toBe("Intl.DisplayNames");
    expect(regions.of("US")).toBe("United States");
    expect(regions.of("JP")).toBe("Japan");

    const currencies = new Intl.DisplayNames(["en"], { type: "currency" });
    expect(currencies.of("USD")).toBe("US Dollar");
    expect(currencies.of("CNY")).toBe("Chinese Yuan");

    const langs = new Intl.DisplayNames(["en"], { type: "language" });
    expect(langs.of("zh-Hans")).toBe("Simplified Chinese");

    const zhFields = new Intl.DisplayNames(["zh-CN"], { type: "dateTimeField" });
    expect(zhFields.of("year")).toBe("年");

    const noneFallback = new Intl.DisplayNames(["en"], { type: "region", fallback: "none" });
    expect(noneFallback.of("ZZ")).toBeUndefined();
    expect(() => new Intl.DisplayNames(["en"], {} as any)).toThrow();
  });
});

describe("Intl.DurationFormat", () => {
  it("formats long, short, and digital durations", () => {
    const longFmt = new Intl.DurationFormat("en-US", { style: "long" });
    expect(longFmt[Symbol.toStringTag]).toBe("Intl.DurationFormat");
    expect(longFmt.format({ hours: 1, minutes: 30, seconds: 5 })).toBe(
      "1 hour, 30 minutes, and 5 seconds",
    );

    const digitalFmt = new Intl.DurationFormat("en-US", { style: "digital" });
    expect(digitalFmt.format({ hours: 1, minutes: 5, seconds: 9 })).toBe("1:05:09");
    expect(digitalFmt.format({ minutes: 5, seconds: 2, milliseconds: 450 })).toBe("05:02.45");

    const parts = longFmt.formatToParts({ days: 2 });
    expect(parts[0]).toEqual({ type: "integer", value: "2", unit: "day" });

    expect(() => longFmt.format({ hours: 1, minutes: -5 })).toThrow();
  });
});
