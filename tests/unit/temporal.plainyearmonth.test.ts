describe("Temporal.PlainYearMonth", () => {
  it("can be constructed and created via from()", () => {
    const ym1 = new Temporal.PlainYearMonth(2024, 3);
    expect(ym1[Symbol.toStringTag]).toBe("Temporal.PlainYearMonth");
    expect(ym1.year).toBe(2024);
    expect(ym1.month).toBe(3);
    expect(ym1.monthCode).toBe("M03");
    expect(ym1.calendarId).toBe("iso8601");
    expect(ym1.daysInMonth).toBe(31);
    expect(ym1.daysInYear).toBe(366);
    expect(ym1.monthsInYear).toBe(12);
    expect(ym1.inLeapYear).toBe(true);
    expect(ym1.toString()).toBe("2024-03");
    expect(ym1.toJSON()).toBe("2024-03");

    const ym2 = Temporal.PlainYearMonth.from("2024-03");
    expect(ym1.equals(ym2)).toBe(true);

    const ym3 = Temporal.PlainYearMonth.from({ year: 2024, monthCode: "M02" });
    expect(ym3.toString()).toBe("2024-02");
    expect(ym3.daysInMonth).toBe(29);
  });

  it("supports overflow constrain and reject", () => {
    expect(Temporal.PlainYearMonth.from({ year: 2024, month: 15 }).toString()).toBe("2024-12");
    expect(() =>
      Temporal.PlainYearMonth.from({ year: 2024, month: 15 }, { overflow: "reject" }),
    ).toThrow();
  });

  it("supports compare, with, add, subtract, until, since, and toPlainDate", () => {
    const ym1 = Temporal.PlainYearMonth.from("2023-11");
    const ym2 = Temporal.PlainYearMonth.from("2025-02");

    expect(Temporal.PlainYearMonth.compare(ym1, ym2)).toBe(-1);
    expect(Temporal.PlainYearMonth.compare(ym2, ym1)).toBe(1);
    expect(Temporal.PlainYearMonth.compare(ym1, "2023-11")).toBe(0);

    expect(ym1.add({ months: 3 }).toString()).toBe("2024-02");
    expect(ym2.subtract({ years: 1, months: 1 }).toString()).toBe("2024-01");
    expect(ym1.with({ month: 6 }).toString()).toBe("2023-06");

    const u = ym1.until(ym2);
    expect(u.years).toBe(1);
    expect(u.months).toBe(3);

    const uMonths = ym1.until(ym2, { largestUnit: "month" });
    expect(uMonths.years).toBe(0);
    expect(uMonths.months).toBe(15);

    const s = ym2.since(ym1);
    expect(s.years).toBe(1);
    expect(s.months).toBe(3);

    const pd = ym1.toPlainDate({ day: 20 });
    expect(pd).toBeInstanceOf(Temporal.PlainDate);
    expect(pd.toString()).toBe("2023-11-20");
    expect(pd.toPlainYearMonth().toString()).toBe("2023-11");
  });

  it("rejects invalid units and valueOf", () => {
    const ym = Temporal.PlainYearMonth.from("2024-01");
    expect(() => ym.add({ days: 5 })).toThrow();
    expect(() => ym.valueOf()).toThrow();
  });
});
