describe("Temporal.PlainMonthDay", () => {
  it("can be constructed and created via from()", () => {
    const md1 = new Temporal.PlainMonthDay(2, 29);
    expect(md1[Symbol.toStringTag]).toBe("Temporal.PlainMonthDay");
    expect(md1.monthCode).toBe("M02");
    expect(md1.day).toBe(29);
    expect(md1.calendarId).toBe("iso8601");
    expect(md1.toString()).toBe("02-29");
    expect(md1.toJSON()).toBe("02-29");

    expect(Temporal.PlainMonthDay.from("--05-15").toString()).toBe("05-15");
    expect(Temporal.PlainMonthDay.from("2024-12-25").toString()).toBe("12-25");
    expect(Temporal.PlainMonthDay.from({ monthCode: "M07", day: 4 }).toString()).toBe("07-04");
  });

  it("supports overflow constrain and reject", () => {
    expect(Temporal.PlainMonthDay.from({ month: 2, day: 31 }).toString()).toBe("02-29");
    expect(() =>
      Temporal.PlainMonthDay.from({ month: 2, day: 31 }, { overflow: "reject" }),
    ).toThrow();
  });

  it("supports with, equals, toPlainDate, and valueOf", () => {
    const md = Temporal.PlainMonthDay.from("03-15");
    expect(md.equals("03-15")).toBe(true);
    expect(md.equals("03-16")).toBe(false);
    expect(md.with({ month: 4, day: 30 }).toString()).toBe("04-30");

    const pd = md.toPlainDate({ year: 2025 });
    expect(pd).toBeInstanceOf(Temporal.PlainDate);
    expect(pd.toString()).toBe("2025-03-15");
    expect(pd.toPlainMonthDay().toString()).toBe("03-15");

    expect(() => md.valueOf()).toThrow();
  });
});
